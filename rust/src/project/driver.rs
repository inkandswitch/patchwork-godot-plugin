use crate::diff::differ::ProjectDiff;
use crate::fs::file_utils::FileSystemEvent;
use crate::helpers::branch::BranchState;
use crate::helpers::utils::{CommitInfo, CommitMetadata};
use crate::interop::godot_accessors::{EditorFilesystemAccessor, PatchworkEditorAccessor};
use crate::project::branch_db::{BranchDb, HistoryRef};
use crate::project::connection::{RemoteConnection, RemoteConnectionEvent, RemoteConnectionStatus};
use crate::project::document_watcher::DocumentWatcher;
use crate::project::peer_watcher::PeerWatcher;
use crate::project::sync_automerge_to_fs::SyncAutomergeToFileSystem;
use crate::project::sync_fs_to_automerge::SyncFileSystemToAutomerge;
use automerge::ChangeHash;
use futures::StreamExt;
use samod::{ConcurrencyConfig, DocHandle, DocumentId, Repo};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

/// The main driver for the project.
/// Hooks together all the various controllers.
/// When this object is constructed, it is started. When the handle is dropped, it shuts down.
#[derive(Clone, Debug)]
pub struct Driver {
    inner: Arc<DriverInner>,
}

#[derive(Debug)]
pub struct DriverInner {
    connection: RemoteConnection,
    branch_db: BranchDb,
    peer_watcher: PeerWatcher,
    document_watcher: DocumentWatcher,
    sync_automerge_to_fs: SyncAutomergeToFileSystem,
    sync_fs_to_automerge: SyncFileSystemToAutomerge,
    // TODO (Lilith): Currently the differ is broken because it can't be sent across threads due to the Variant cache.
    // Figure out a way to fix that. One option is maybe a global singleton cache only on one thread? Or just killing Variants in the differ, which is ideal.
    // differ: Differ,
}

impl Driver {
    async fn parse_gitignore(
        project_dir: &PathBuf,
        gitignore_path: &PathBuf,
    ) -> Vec<glob::Pattern> {
        let mut ignore_globs = Vec::new();

        let content = match tokio::fs::read_to_string(gitignore_path).await {
            Ok(content) => content,
            Err(_) => {
                tracing::error!("Couldn't read gitignore file at {:?}", gitignore_path);
                return ignore_globs;
            }
        };

        for line in content.lines() {
            // trim any comments and whitespace
            let line = line.trim().split('#').next().unwrap_or_default().trim();
            if line.is_empty() {
                continue;
            }
            let mut new_line = if line.starts_with("/") {
                line.to_string()
            } else {
                project_dir.join(line).to_string_lossy().to_string()
            };
            let new_line = if new_line.ends_with("/") {
                // just remove the trailing slash
                new_line.pop();
                new_line
            } else {
                new_line
            };
            match glob::Pattern::new(&new_line) {
                Ok(glob) => ignore_globs.push(glob),
                Err(e) => tracing::error!(
                    "Invalid glob while parsing gitignore {:?}! Error: {}",
                    gitignore_path,
                    e
                ),
            }
        }
        ignore_globs
    }

    /// Creates a new instance of [Driver].
    /// Causes tasks to run in the background. To cancel everything, drop the handle.
    /// If we couldn't start the driver, [None] is returned.
    pub async fn new(
        server_url: String,
        project_path: PathBuf,
        storage_directory: PathBuf,
        metadata_id: Option<DocumentId>,
    ) -> Option<Self> {
        // TODO (Lilith): ensure we make this work across the ENTIRE program. Initialize it only once, etc.
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .thread_name("GodotProjectDriver: worker thread")
            .build()
            .unwrap();

        let _ = runtime.enter();
        let storage = samod::storage::TokioFilesystemStorage::new(storage_directory);
        let repo = Repo::build_tokio()
            .with_concurrency(ConcurrencyConfig::Threadpool(
                rayon::ThreadPoolBuilder::new().build().unwrap(),
            ))
            .with_storage(storage)
            .load()
            .await;

        let project_path = PathBuf::from(project_path);

        // Construct the ignore globs, and link in the gitignores
        let ignore_globs = vec![
            "**/.DS_Store",
            "**/thumbs.db",
            "**/desktop.ini",
            "**/patchwork.cfg",
            "**/addons/patchwork*",
            "**/target/*",
            "**/.*",
        ]
        .iter()
        .filter_map(|p| glob::Pattern::new(p).ok())
        .chain(
            Self::parse_gitignore(&project_path, &project_path.join(".gitignore"))
                .await
                .into_iter(),
        )
        .chain(
            Self::parse_gitignore(&project_path, &project_path.join(".patchworkignore"))
                .await
                .into_iter(),
        )
        .chain(
            Self::parse_gitignore(&project_path, &project_path.join(".gdignore"))
                .await
                .into_iter(),
        )
        .collect();

        // Start the connection
        let connection = RemoteConnection::new(repo.clone(), server_url);
        let branch_db = BranchDb::new(repo.clone(), project_path, ignore_globs);

        let metadata_handle = match metadata_id {
            // If we're expecting an existing ID, try and fetch it.
            Some(id) => {
                let Some(handle) = Self::get_metadata_handle(&repo, &id, &connection).await else {
                    return None;
                };
                handle
            }
            // If we need to make a new ID, make a doc.
            None => branch_db.create_metadata_doc().await,
        };

        // The document watcher will auto-ingest the provided metadata handle.
        let document_watcher =
            DocumentWatcher::new(repo.clone(), branch_db.clone(), metadata_handle);
        let peer_watcher = PeerWatcher::new(repo.clone(), branch_db.clone());
        let sync_automerge_to_fs = SyncAutomergeToFileSystem::new(branch_db.clone());
        let sync_fs_to_automerge = SyncFileSystemToAutomerge::new(branch_db.clone());
        // let differ = Differ::new(branch_db.clone());

        // At this point, if we loaded an existing project, we may not have checked it out yet.
        // We'll discover that while processing updates, and check it out then.

        Some(Driver {
            inner: DriverInner {
                connection,
                branch_db,
                peer_watcher,
                document_watcher,
                sync_automerge_to_fs,
                sync_fs_to_automerge,
                // differ,
            }
            .into(),
        })
    }

    pub async fn set_username(&self, username: Option<String>) {
        self.inner.branch_db.set_username(username).await;
    }

    /// If we're connected to the server, returns true.
    /// Otherwise, retries the server connection on state change until it is either connected,
    /// or we give up, then returns true if success or false if failure.
    async fn ensure_server_connection(connection: &RemoteConnection, retries: i32) -> bool {
        // We must subscribe to the events stream BEFORE checking the status.
        // This is so that between two lines of code, the status doesn't change before we've inited our stream.
        let events = connection.events();
        tokio::pin!(events);
        match connection.status() {
            RemoteConnectionStatus::Connected => true,
            RemoteConnectionStatus::Disconnected => {
                let mut connected = false;
                // try 3 times
                for _ in 0..retries {
                    if let Some(RemoteConnectionEvent::Connected) = events.next().await {
                        connected = true;
                        break;
                    }
                }
                connected
            }
        }
    }

    async fn get_metadata_handle(
        repo: &Repo,
        metadata_id: &DocumentId,
        connection: &RemoteConnection,
    ) -> Option<DocHandle> {
        // TODO: This is awkward; instead it would be great to create a fake old document and have it updated from the server asynchronously.
        // For now we must wait til the server connects, if we don't have the doc ID locally.
        // Before we continue, we must acquire a handle to the metadata document.
        // There are three cases to handle:
        //  a: The document exists on the local repository.
        //  b: The document exists on the server, and not the local repository.
        //  c: The document doesn't exist at all.

        // First, we check the local repository.
        let Ok(metadata_handle) = repo.find(metadata_id.clone()).await else {
            tracing::error!("Can't start the driver; the repo was immediately stopped!");
            return None;
        };

        match metadata_handle {
            // We found it locally, or the server connected REALLY quickly.
            Some(metadata_handle) => Some(metadata_handle),
            // We didn't find it locally. Try the server for a bit.
            None => {
                // If the hang gets annoying when starting, we could set this to 1 to reduce it to a minimum.
                if !Self::ensure_server_connection(&connection, 3).await {
                    tracing::error!(
                        "Couldn't find the metadata doc handle locally, and the server couldn't connect!"
                    );
                    return None;
                }

                // Try again on the server, if we were able to connect
                match repo.find(metadata_id.clone()).await {
                    Ok(Some(handle)) => Some(handle),
                    Ok(None) => {
                        tracing::error!(
                            "Couldn't find the metadata doc handle, even after connecting to the server!"
                        );
                        return None;
                    }
                    Err(e) => {
                        tracing::error!(
                            "Can't start the driver; the repo was immediately stopped!"
                        );
                        return None;
                    }
                }
            }
        }
    }

    pub fn merge_branch(
        &mut self,
        source_branch_doc_id: DocumentId,
        target_branch_doc_id: DocumentId,
    ) {
        // TODO
    }

    pub fn create_merge_preview_branch_between(
        &mut self,
        source_branch_doc_id: DocumentId,
        target_branch_doc_id: DocumentId,
    ) {
        // TODO (Lilith)
    }

    pub fn create_revert_preview_branch_for(
        &mut self,
        branch_doc_id: DocumentId,
        revert_to: Vec<ChangeHash>,
    ) {
        // TODO (Lilith)
    }

    pub fn delete_branch(&mut self, branch_doc_id: DocumentId) {
        // TODO (Lilith)
    }

    pub fn revert_to_heads(&mut self, to_revert_to: Vec<ChangeHash>) {
        // TODO (Lilith)
        // let branch_state = self.get_checked_out_branch_state().unwrap();
        // let heads = branch_state.doc_handle.with_document(|d| {
        // 	d.get_heads()
        // });
        // let content = self.get_changed_file_content_between(Some(branch_state.doc_handle.document_id().clone()), branch_state.doc_handle.document_id().clone(), heads.clone(), to_revert_to.clone(), true);
        // let files = content.into_iter().map(|event| {
        // 	match event {
        // 		FileSystemEvent::FileCreated(path, content) => (path, content),
        // 		FileSystemEvent::FileModified(path, content) => (path, content),
        // 		FileSystemEvent::FileDeleted(path) => (path, FileContent::Deleted),
        // 	}
        // }).collect::<Vec<(PathBuf, FileContent)>>();
        // self.sync_files_at(branch_state.doc_handle.clone(), files, Some(heads), Some(to_revert_to), false);
        // self.checked_out_branch_state = CheckedOutBranchState::CheckingOut(branch_state.doc_handle.document_id().clone(), None);
    }

    pub async fn get_diff(&self, before: &HistoryRef, after: &HistoryRef) -> ProjectDiff {
        // self.differ.get_diff(before, after).await
        ProjectDiff::default()
    }

    pub async fn get_metadata_doc(&self) -> Option<DocumentId> {
        self.inner
            .branch_db
            .get_metadata_state()
            .await
            .map(|(id, _)| id)
    }

    pub fn safe_to_update_godot() -> bool {
        return !(EditorFilesystemAccessor::is_scanning()
            || PatchworkEditorAccessor::is_editor_importing()
            || PatchworkEditorAccessor::is_changing_scene()
            || PatchworkEditorAccessor::unsaved_files_open());
    }

    pub async fn get_main_branch(&self) -> Option<DocumentId> {
        self.inner
            .branch_db
            .get_metadata_state()
            .await
            .map(|(_, doc)| DocumentId::from_str(&doc.main_doc_id).unwrap())
    }

    pub async fn get_branch_name(&self, id: &DocumentId) -> Option<String> {
        let Some(state) = self.inner.branch_db.get_branch_state(id).await else {
            return None;
        };
        Some(state.lock().await.name.clone())
    }

    pub async fn get_branch_state(&self, id: &DocumentId) -> Option<BranchState> {
        let Some(state) = self.inner.branch_db.get_branch_state(id).await else {
            return None;
        };
        Some(state.lock().await.clone())
    }

    /// Returns the changes from the current branch.
    pub async fn get_changes(&self) -> Vec<CommitInfo> {
        let checked_out = self.inner.branch_db.get_checked_out_ref_mut().await;
        let checked_out = checked_out.read().await;
        let Some(checked_out) = checked_out.as_ref() else {
            tracing::info!("Can't get changes; nothing checked out!");
            return Vec::new();
        };

        let Some(branch_state) = self
            .inner
            .branch_db
            .get_branch_state(&checked_out.branch)
            .await
        else {
            tracing::info!("Can't get the checked out branch state; something must be wrong");
            return Vec::new();
        };

        // TODO: we probably don't need to lock the branch state for this whole method
        let branch_state = branch_state.lock().await;
        let handle = branch_state.doc_handle.clone();
        let doc_id = handle.document_id();

        let last_acked_heads = self
            .inner
            .peer_watcher
            .get_server_info()
            .await
            .as_ref()
            .and_then(|info| info.docs.get(&doc_id))
            .and_then(|state| state.last_acked_heads.clone());

        let changes = handle.with_document(move |d| {
            d.get_changes(&[])
                .to_vec()
                .iter()
                .map(|c| {
                    CommitInfo {
                        hash: c.hash(),
                        timestamp: c.timestamp(),
                        metadata: c
                            .message()
                            .and_then(|m| serde_json::from_str::<CommitMetadata>(&m).ok()),
                        synced: false,           // set later
                        summary: "".to_string(), // set later
                    }
                })
                .collect::<Vec<CommitInfo>>()
        });

        // Check to see what the most recent synced commit is.
        let mut synced_until_index = -1;
        for (i, change) in changes.iter().enumerate() {
            if last_acked_heads
                .as_ref()
                .is_some_and(|f| f.contains(&change.hash))
            {
                synced_until_index = i as i32;
            }
        }

        changes
            .into_iter()
            .enumerate()
            .map(|(i, change)| CommitInfo {
                synced: (i as i32) <= synced_until_index,
                ..change
            })
            .collect()
    }

    pub async fn get_checked_out_ref(&self) -> Option<HistoryRef> {
        let checked_out_ref = self.inner.branch_db.get_checked_out_ref_mut().await;
        return checked_out_ref.read().await.clone();
    }

    /// Sync the project state as best we can.
    /// Make sure not to run two of these at once, for safety.
    /// (It would likely be OK state-wise but weird results might happen on the UI side.)
    /// Returns a vector of filesystem changes we performed.
    pub async fn sync(&self) -> Vec<FileSystemEvent> {
        // TODO (Lilith): There are inefficiencies with this strategy.
        // Basically, every time we save a file, it'll do a bunch of extra work.
        // It will first commit the changes, then it will check out the changes we just committed.
        // No files will be written of course, but it will still walk the tree of changes.
        // The same happens in reverse: when we check out a ref, it will attempt to commit and not
        // find any actual changes.
        // Maybe that's OK, we need to profile to see if it's a problem.
        let changes = if Self::safe_to_update_godot() {
            let goal_ref = {
                let current_ref_lock = self.inner.branch_db.get_checked_out_ref_mut().await;
                let current_ref_guard = current_ref_lock.read().await;

                let branch = match current_ref_guard.as_ref() {
                    Some(r) => r.branch.clone(),
                    None => {
                        // If we didn't find a current ref, nothing is checked out.
                        // Let's fix that by checking out the main branch.
                        let Some(main_branch) = self.inner.branch_db.get_main_branch().await else {
                            tracing::error!(
                                "No metadata doc checked out, or otherwise couldn't get main branch. Skipping sync!"
                            );
                            return Vec::new();
                        };
                        main_branch
                    }
                };
                let Some(goal_ref) = self.inner.branch_db.get_latest_ref_on_branch(&branch).await
                else {
                    tracing::error!("Couldn't get the goal ref for branch {}", branch);
                    return Vec::new();
                };
                goal_ref
                // guard dropped here
            };
            // TODO (Lilith): minor problem, the checked out ref could change between these lines when the guard is dropped.
            // That said, uhhhh I think that's fine? We're already syncing, so other sync methods shouldn't effect it.
            // If we're doing some branch edits, like a merge or revert preview or something, that could mean we
            // never checkout our desired ref... but we need to rethink this for branch swapping anyways.
            // For branch swapping, we either need to checkout a ref elsewhere during a merge or something and prevent sync.
            // Or, we could set a request_checkout tokio::Watch here on the driver that will check out a branch at the next opportunity.
            self.inner.sync_automerge_to_fs.checkout_ref(goal_ref).await
        } else {
            Vec::new()
        };

        // Apply any watched FS updates to Automerge
        self.inner.sync_fs_to_automerge.commit().await;
        changes
    }
}
