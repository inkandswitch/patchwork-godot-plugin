use crate::diff::differ::ProjectDiff;
use crate::fs::file_utils::FileSystemEvent;
use crate::helpers::branch::BranchState;
use crate::helpers::utils::{CommitInfo, summarize_changes};
use crate::interop::godot_accessors::{
    EditorFilesystemAccessor, PatchworkConfigAccessor, PatchworkEditorAccessor,
};
use crate::project::branch_db::HistoryRef;
use crate::project::driver::Driver;
use crate::project::main_thread_block::MainThreadBlock;
use crate::project::project_api::ChangeViewModel;
use automerge::ChangeHash;
use futures::future::join_all;
use samod::DocumentId;
use tracing::instrument;
use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;
use std::{collections::HashMap, str::FromStr};
use tokio::runtime::Runtime;
use tokio::sync::{Mutex, MutexGuard, OwnedMutexGuard, watch};

/// Manages the state and operations of a Patchwork project within Godot.
/// Its API is exposed to GDScript via the GodotProject struct.
#[derive(Debug)]
pub struct Project {
    // Sync
    main_thread_block: MainThreadBlock,
    // These are here so we don't needlessly block during process
    changes_rx: Option<watch::Receiver<Vec<CommitInfo>>>,
    checked_out_ref_rx: Option<watch::Receiver<Option<HistoryRef>>>,

    // Project driver. If some, is running.
    // I'd prefer this not be a mutex, but we need to move it into temporary threads in order to dispatch async code from sync code.
    // What's annoying is that we never actually block on this mutex!
    pub(super) driver: Arc<Mutex<Option<Driver>>>,
    project_dir: PathBuf,
    pub(super) runtime: Runtime,

    // Tracked changes for the UI
    pub(super) history: Vec<ChangeHash>,
    pub(super) changes: HashMap<ChangeHash, CommitInfo>,
    last_known_branch: Option<DocumentId>,

    // Cached diffs between refs
    pub(super) diff_cache: RefCell<HashMap<(HistoryRef, HistoryRef), ProjectDiff>>,
}

/// The default server URL used for syncing Patchwork projects. Can be overridden by user or project configuration.
const DEFAULT_SERVER_URL: &str = "24.199.97.236:8085";

/// Notifications that can be emitted via process and consumed by GodotProject, in order to trigger signals to GDScript.
pub enum GodotProjectSignal {
    CheckedOutBranch,
    ChangesIngested,
}

impl Project {
    pub fn new(project_dir: PathBuf) -> Self {
        // TODO (Lilith): ensure we make this work across the ENTIRE program, not just the driver.
        // For now this encapsulates everything we multi-thread, since Project is the barrier for public async access.
        // So it's fine. But if we want other code besides the driver to be multi-threaded...
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("patchwork-driver-worker")
            .build()
            .unwrap();

        Self {
            main_thread_block: MainThreadBlock::new(),
            changes_rx: None,
            checked_out_ref_rx: None,
            driver: Arc::new(Mutex::new(None)),
            project_dir,
            runtime,
            history: Vec::new(),
            changes: HashMap::new(),
            last_known_branch: None,
            diff_cache: RefCell::new(HashMap::new()),
        }
    }

    fn ingest_changes(&mut self, changes: Vec<CommitInfo>) {
        tracing::info!("Ingesting changes...");

        self.history.clear();
        self.changes.clear();

        // Consume changes into self.changes
        for change in changes {
            self.history.push(change.hash);
            self.changes.insert(change.hash, change);
        }
    }

    pub fn get_cached_diff(&self, before: HistoryRef, after: HistoryRef) -> ProjectDiff {
        self.diff_cache
            .borrow_mut()
            .entry((before.clone(), after.clone()))
            .or_insert_with(|| self.get_diff(before, after))
            .clone()
    }

    pub fn clear_diff_cache(&self) {
        self.diff_cache.borrow_mut().clear();
    }

    // Do not run this on anything except the main thread!
    pub fn safe_to_update_godot() -> bool {
        return !(EditorFilesystemAccessor::is_scanning()
            || PatchworkEditorAccessor::is_editor_importing()
            || PatchworkEditorAccessor::is_changing_scene()
            || PatchworkEditorAccessor::unsaved_files_open());
    }

    pub fn get_diff(&self, before: HistoryRef, after: HistoryRef) -> ProjectDiff {
        let Some(driver) = self.driver.blocking_lock().as_ref() else {
            return ProjectDiff::default();
        };
        ProjectDiff::default()

        // TODO (Lilith): make this code work; ProjectDiff includes variants which can't be pushed across threads
        // TODO: these can get expensive; propagate a spinner to the UI
        // self.runtime.block_on(
        // 	self.runtime.spawn(async move {
        // 		driver.get_diff(&before, &after).await
        // 	}).await.unwrap()
        // )
    }

    pub fn start(&mut self) {
        if self.driver.blocking_lock().is_some() {
            tracing::error!("Driver is already started!");
            return;
        }

        let storage_dir = self.project_dir.join(".patchwork");
        let server_url = {
            let project = PatchworkConfigAccessor::get_project_value("server_url", "");
            let user = PatchworkConfigAccessor::get_user_value("server_url", "");
            if !project.is_empty() {
                tracing::info!("Using project override for server url: {:?}", project);
                project
            } else if !user.is_empty() {
                tracing::info!("Using user override for server url: {:?}", user);
                user
            } else {
                let default = DEFAULT_SERVER_URL.to_string();
                tracing::info!("Using default server url: {:?}", default);
                default
            }
        };

        // If the metadata ID is not a valid document ID, give up.
        // If it's an empty string, returns None so we can make a new doc.
        let metadata_id = match Some(PatchworkConfigAccessor::get_project_value(
            "project_doc_id",
            "",
        ))
        .filter(|s| !s.is_empty())
        {
            Some(s) => match DocumentId::from_str(&s) {
                Ok(id) => Some(id),
                Err(_) => {
                    tracing::error!("Invalid metadata document ID! Not starting driver.");
                    return;
                }
            },
            None => None,
        };

        // TODO (Lilith): Add support back for initially checking out a branch; probably once we're syncing
        // let checked_out_branch_doc_id = PatchworkConfigAccessor::get_project_value("checked_out_branch_doc_id", "");
        tracing::info!(
            "Starting GodotProject with metadata doc id: {:?}",
            metadata_id
        );

        let project_dir = self.project_dir.clone();
        let block = self.main_thread_block.clone();

        // TODO: Don't block on main thread for checkin
        *self.driver.blocking_lock() = self
            .runtime
            .block_on(
                // I think it's correct to spawn this on a different task explicitly, because block_on runs the future on the current thread, not a worker thread.
                self.runtime.spawn(async move {
                    Driver::new(block, server_url, project_dir, storage_dir, metadata_id).await
                }),
            )
            .unwrap();

        if self.driver.blocking_lock().is_none() {
            tracing::error!("Could not start the driver!");
            return;
        }

        let driver = self.driver.blocking_lock();
        self.changes_rx = Some(driver.as_ref().unwrap().get_changes_rx());
        self.checked_out_ref_rx = Some(driver.as_ref().unwrap().get_ref_rx());
    }

    pub fn stop(&mut self) {
        self.driver.blocking_lock().take();
    }

    pub(super) fn get_checked_out_branch_state(&self) -> Option<BranchState> {
        self.with_driver_blocking(|driver| async move {
            if driver.is_none() {
                return None;
            }
            let branch_state = match driver.as_ref().unwrap().get_checked_out_ref().await {
                Some(id) => driver.as_ref().unwrap().get_branch_state(&id.branch).await,
                None => None,
            };
            branch_state.clone()
        })
    }

    /// Jank utility function to lock on the driver and run on a different thread.
    /// Allows us to easily block on async code when we need the driver.
    pub(super) fn with_driver_blocking<F, Fut, R>(&self, f: F) -> R
    where
        F: FnOnce(OwnedMutexGuard<Option<Driver>>) -> Fut + Send + 'static,
        Fut: Future<Output = R> + Send + 'static,
        R: Send + 'static,
    {
        let driver = self.driver.clone();

        self.runtime
            .block_on(self.runtime.spawn(async move {
                let driver = driver.lock_owned().await;
                f(driver).await
            }))
            .unwrap()
    }

    #[instrument(skip_all)]
    pub fn process(&mut self, _delta: f64) -> (Vec<FileSystemEvent>, Vec<GodotProjectSignal>) {
        tracing::trace!("Running project process...");
        let fs_changes = {
            let mut driver_guard = self.driver.blocking_lock();
            if driver_guard.is_none() {
                return (Vec::new(), Vec::new());
            }
            // Run the blocking sync
            driver_guard
                .as_ref()
                .unwrap()
                .set_safe_to_update_editor(Self::safe_to_update_godot());
            let block = self.main_thread_block.clone();
            tracing::trace!("Blocking for dependents...");
            self.runtime
                .block_on(self.runtime.spawn(async move {
                    block.checkpoint().await;
                }))
                .unwrap();
            tracing::trace!("Done blocking.");

            // Consume any modified files to send to Godot
            driver_guard.as_mut().unwrap().get_filesystem_changes()
        };

        let mut signals = Vec::new();

        // Ingest changes if the driver produced a new changeset
        let changes = {
            let rx = self.changes_rx.as_mut().unwrap();
            if rx.has_changed().unwrap_or(false) {
                rx.mark_unchanged();
                signals.push(GodotProjectSignal::ChangesIngested);
                Some(rx.borrow().clone())
            } else {
                None
            }
        };

        if let Some(changes) = changes {
            self.ingest_changes(changes);
        }

        // Check to see if we need to produce a CheckedOutBranch signal
        let rx = self.checked_out_ref_rx.as_mut().unwrap();
        if rx.has_changed().unwrap_or(false) {
            signals.push(GodotProjectSignal::CheckedOutBranch);
            rx.mark_unchanged();
        }

        // TODO (Lilith): VERY IMPORTANT, set the patchwork config branch ID here!!!
        // So that we save the branch ID for future checkouts.

        tracing::trace!("Done with process.");
        (fs_changes, signals)
    }
}
