use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
};

use automerge::{Automerge, ChangeHash};
use samod::{DocHandle, DocumentId, Repo};
use tokio::sync::{Mutex, RwLock};

use crate::{
    helpers::branch::{BranchState, BranchesMetadataDoc},
    project::branch_db::{branch_sync::BranchSyncState, history_ref::HistoryRef},
};

mod branch;
mod branch_sync;
mod commit;
mod file;
pub mod history_ref;
mod merge_revert;
mod util;
use ignore::gitignore::Gitignore;

/// [BranchDb] is the primary data source for project data.
/// It stores the project state, and provides a handful of convenient state-manipulation methods for controllers to use.
#[derive(Clone, Debug)]
pub struct BranchDb {
    // Path is immutable, so it can be outside the inner
    project_dir: PathBuf,
    gitignore: Arc<Gitignore>,
    repo: Repo,

    username: Arc<Mutex<Option<String>>>,

    // TODO (Lilith): I think we can possibly combine binary_states and branch_sync_states' mutexes to reduce complexity
    binary_states: Arc<Mutex<HashMap<DocumentId, Option<DocHandle>>>>,
    branch_states: Arc<Mutex<HashMap<DocumentId, Arc<Mutex<BranchState>>>>>,
    branch_sync_states: Arc<Mutex<HashMap<DocumentId, Arc<Mutex<BranchSyncState>>>>>,

    metadata_state: Arc<Mutex<Option<(DocHandle, BranchesMetadataDoc)>>>,

    // The checked out ref is the ref that the filesystem is currently synced with.
    // Has a separate lock because of its importance; it needs to be locked while we're prepping a commit or checking out stuff
    checked_out_ref: Arc<RwLock<Option<HistoryRef>>>,
}

impl BranchDb {
    pub fn new(repo: Repo, project_dir: PathBuf, gitignore: Gitignore) -> Self {
        Self {
            project_dir,
            repo,
            gitignore: Arc::new(gitignore),
            username: Default::default(),
            binary_states: Default::default(),
            branch_states: Default::default(),
            metadata_state: Default::default(),
            checked_out_ref: Default::default(),
            branch_sync_states: Default::default(),
        }
    }

    pub fn get_project_dir(&self) -> PathBuf {
        self.project_dir.clone()
    }

    pub async fn set_username(&self, username: Option<String>) {
        let mut user = self.username.lock().await;
        *user = username;
    }
}
