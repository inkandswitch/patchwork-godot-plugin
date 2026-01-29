use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
};

use automerge::ChangeHash;
use samod::{DocHandle, DocumentId, Repo};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, RwLock};

use crate::
    helpers::
        branch::{BinaryDocState, BranchState, BranchesMetadataDoc}
    
;

mod branch;
mod commit;
mod file;
mod util;
mod merge_revert;

// TODO (Lilith): Move this to utils
/// Represents a location anywhere in Patchwork's history.
/// Associates a branch with heads on that branch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryRef {
    /// The branch the ref is on.
    pub branch: DocumentId,
    // todo: it would be very nice to have a Heads struct
    /// The Automerge heads for the history location
    pub heads: Vec<ChangeHash>,
}

impl HistoryRef {
    pub fn is_valid(&self) -> bool {
        return !self.heads.is_empty();
    }
}

impl Eq for HistoryRef {}

impl std::hash::Hash for HistoryRef {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.branch.hash(state);
        self.heads.hash(state);
    }
}

impl PartialEq for HistoryRef {
    fn eq(&self, other: &Self) -> bool {
        self.branch == other.branch && self.heads == other.heads
    }
}

/// [BranchDb] is the primary data source for project data.
/// It stores the project state, and provides a handful of convenient state-manipulation methods for controllers to use.
#[derive(Clone, Debug)]
pub struct BranchDb {
    // Path is immutable, so it can be outside the inner
    project_dir: PathBuf,
    ignore_globs: Arc<Vec<glob::Pattern>>,
    repo: Repo,
    
    username: Arc<Mutex<Option<String>>>,
    binary_states: Arc<Mutex<HashMap<DocumentId, BinaryDocState>>>,
    branch_states: Arc<Mutex<HashMap<DocumentId, Arc<Mutex<BranchState>>>>>, // might be too much locking
    metadata_state: Arc<Mutex<Option<(DocHandle, BranchesMetadataDoc)>>>,

    // The checked out ref is the ref that the filesystem is currently synced with.
    // Has a separate lock because of its importance; it needs to be locked while we're prepping a commit or checking out stuff
    checked_out_ref: Arc<RwLock<Option<HistoryRef>>>,
}

impl BranchDb {
    pub fn new(repo: Repo, project_dir: PathBuf, ignore_globs: Vec<glob::Pattern>) -> Self {
        Self {
            project_dir,
            repo,
            ignore_globs: Arc::new(ignore_globs),
            username: Arc::new(Mutex::new(None)),
            binary_states: Arc::new(Mutex::new(HashMap::new())),
            branch_states: Arc::new(Mutex::new(HashMap::new())),
            metadata_state: Arc::new(Mutex::new(None)),
            checked_out_ref: Arc::new(RwLock::new(None)),
        }
    }


    pub fn get_ignore_globs(&self) -> Vec<glob::Pattern> {
        (*self.ignore_globs).clone()
    }

    pub fn get_project_dir(&self) -> PathBuf {
        self.project_dir.clone()
    }

    pub async fn set_username(&self, username: Option<String>) {
        let mut user = self.username.lock().await;
        *user = username;
    }

    /// Get the mutable checked out ref for locking.
    /// TODO (Lilith): This smells kind of nasty, maybe don't expose this... but how else to ensure we don't step on toes?
    pub fn get_checked_out_ref_mut(&self) -> Arc<RwLock<Option<HistoryRef>>> {
        return self.checked_out_ref.clone();
    }

    pub async fn get_metadata_state(&self) -> Option<(DocHandle, BranchesMetadataDoc)> {
        // This is a needlessly expensive operation; we should consider allowing reference introspection via external lockers.
        // And/or improve clone perf by reducing string usage in BranchesMetadataDoc.
        self.metadata_state.lock().await.clone()
    }

    pub async fn set_metadata_state(&self, handle: DocHandle, state: BranchesMetadataDoc) {
        let mut st = self.metadata_state.lock().await;
        *st = Some((handle, state));
    }

    pub async fn has_branch(&self, id: &DocumentId) -> bool {
        let st = self.branch_states.lock().await;
        return st.contains_key(id);
    }

    pub async fn insert_branch_state_if_not_exists<F>(&self, id: DocumentId, f: F)
    where
        F: FnOnce() -> BranchState,
    {
        let mut st = self.branch_states.lock().await;
        st
            .entry(id.clone())
            .or_insert_with(|| Arc::new(Mutex::new(f())));
    }

    pub async fn set_linked_docs_for_branch(&self, id: &DocumentId, linked_docs: HashSet<DocumentId>) {
        let states = self.branch_states.lock().await;
        let Some(state) = states.get(id) else {
            return;
        };
        let mut state = state.lock().await;
        state.linked_doc_ids = linked_docs;
    }
}
