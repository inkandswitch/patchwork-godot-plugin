use crate::diff::differ::ProjectDiff;
use crate::fs::file_utils::FileSystemEvent;
use crate::helpers::utils::{CommitInfo, summarize_changes};
use crate::interop::godot_accessors::{EditorFilesystemAccessor, PatchworkConfigAccessor, PatchworkEditorAccessor};
use crate::project::branch_db::HistoryRef;
use crate::project::driver::Driver;
use crate::project::project_api::ChangeViewModel;
use automerge::ChangeHash;
use futures::future::join_all;
use samod::DocumentId;
use std::cell::RefCell;
use std::path::PathBuf;
use std::time::SystemTime;
use std::{collections::HashMap, str::FromStr};
use tokio::runtime::Runtime;

/// Manages the state and operations of a Patchwork project within Godot.
/// Its API is exposed to GDScript via the GodotProject struct.
#[derive(Debug)]
pub struct Project {
    // Project driver. If some, is running
    pub(super) driver: Option<Driver>,
    project_dir: PathBuf,
    pub(super) runtime: Runtime,

    // Tracked changes for the UI
    pub(super) history: Vec<ChangeHash>,
    pub(super) changes: HashMap<ChangeHash, CommitInfo>,
    last_ingest: (SystemTime, i32),
    ingest_requested: bool,
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
            driver: None,
            project_dir,
            runtime,
            history: Vec::new(),
            changes: HashMap::new(),
            last_ingest: (SystemTime::UNIX_EPOCH, 0),
            ingest_requested: true,
            last_known_branch: None,
            diff_cache: RefCell::new(HashMap::new()),
        }
    }

    /// Expensive operation to ingest all branch changes from automerge into the project data.
    /// Should be called when we think there are new changes to process.
    fn ingest_changes(&mut self) {
        let Some(driver) = self.driver.clone() else {
            tracing::error!("Driver not started, can't ingest changes!");
            return;
        };

        // tracing::info!("Ingesting changes...");

        let changes = self
            .runtime
            .block_on(self.runtime.spawn(async move {
                let changes =
                    driver
                        .get_changes()
                        .await
                        .into_iter()
                        .map(async |change| CommitInfo {
                            summary: Self::get_change_summary(&change, driver.clone())
                                .await
                                .unwrap_or("Invalid data".to_string()),
                            ..change
                        });
                // a little awk. Parallelization doesn't really matter here at all and is maybe bad (bc locks).
                // but otherwise we're iteratively constructing a vec, probably doesn't matter
                join_all(changes).await
            }))
            .unwrap();

		self.history.clear();
		self.changes.clear();

        // Consume changes into self.changes
        for change in changes {
            self.history.push(change.hash);
            self.changes.insert(change.hash, change);
        }
    }

    async fn get_change_summary(change: &CommitInfo, driver: Driver) -> Option<String> {
        let meta = change.metadata.as_ref();
        let author = meta?.username.clone().unwrap_or("Anonymous".to_string());

        // merge commit
        if let Some(merge_info) = &meta?.merge_metadata {
            let merged_branch = driver
                .get_branch_name(&merge_info.merged_branch_id.clone())
                .await
                .unwrap_or(merge_info.merged_branch_id.to_string());
            return Some(format!("↪ {author} merged {merged_branch}"));
        }

        // revert commit
        if let Some(revert_info) = &meta?.reverted_to {
            let heads = revert_info
                .iter()
                .map(|s| &s[..7])
                .collect::<Vec<&str>>()
                .join(", ");
            return Some(format!("↩ {author} reverted to {heads}"));
        }

        // initial commit
        if change.is_setup() {
            return Some(format!("Initialized repository"));
        }

        return Some(summarize_changes(&author, meta?.changed_files.as_ref()?));
    }

    /// Request for a change ingestion to be dispatched.
    fn request_ingestion(&mut self) {
        self.ingest_requested = true;
    }

    /// If able, ingest changes, clear the ingestion request, and return true.
    /// Otherwise, return false.
    fn try_ingest_changes(&mut self) -> bool {
        // Do not try to ingest if we haven't requested.
        if !self.ingest_requested {
            return false;
        }
        let now = SystemTime::now();
        let Ok(last_diff) = now.duration_since(self.last_ingest.0) else {
            return false;
        };

        // Impose an arbitrary cap on requests within a time period.
        // This is so that immediate syncs -- such as those from a local server -- don't have to wait before getting synced.
        // But it also prevents spam of like a hundred slowing down the ingestion.
        if last_diff.as_millis() < 100 {
            if self.last_ingest.1 >= 3 {
                return false;
            }
        } else {
            // since we're past the duration with no other requests, the counter resets.
            self.last_ingest = (now, 0);
        }
        self.ingest_changes();
        self.ingest_requested = false;
        self.last_ingest.1 += 1;
        true
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
        let Some(driver) = &self.driver else {
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
        if self.driver.is_some() {
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
        self.driver = self
            .runtime
            .block_on(
                // I think it's correct to spawn this on a different task explicitly, because block_on runs the future on the current thread, not a worker thread.
                self.runtime.spawn(async move {
                    Driver::new(server_url, project_dir, storage_dir, metadata_id).await
                }),
            )
            .unwrap();

        if self.driver.is_none() {
            tracing::error!("Could not start the driver!");
            return;
        }
    }

    pub fn stop(&mut self) {
        self.driver.take();
    }

    pub fn process(&mut self, _delta: f64) -> (Vec<FileSystemEvent>, Vec<GodotProjectSignal>) {
        let Some(driver) = &self.driver else {
            return Default::default();
        };
        let driver = driver.clone();

        let mut signals: Vec<GodotProjectSignal> = Vec::new();
        if self.try_ingest_changes() {
            signals.push(GodotProjectSignal::ChangesIngested);
        }
		let safe_to_update = Self::safe_to_update_godot();
        // Run the main sync
        let (changed_files, checked_out_ref) =
            self.runtime
                .block_on(self.runtime.spawn(async move {
                    (driver.sync(safe_to_update).await, driver.get_checked_out_ref().await)
                }))
                .unwrap();

        let current_branch = checked_out_ref.and_then(|r| Some(r.branch));
        if self.last_known_branch != current_branch {
            signals.push(GodotProjectSignal::CheckedOutBranch);
			self.last_known_branch = current_branch;
        }

        // TODO (Lilith): VERY IMPORTANT, set the patchwork config branch ID here!!!
        // So that we save the branch ID for future checkouts.

        // TODO (Lilith): Don't request an ingestion every frame
        self.request_ingestion();

        (changed_files, signals)
    }
}
