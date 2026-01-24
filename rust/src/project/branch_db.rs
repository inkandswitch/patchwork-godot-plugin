use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
};

use automerge::{Automerge, ChangeHash, ObjId, ObjType, ROOT, ReadDoc};
use autosurgeon::Doc;
use samod::{DocHandle, DocumentId, Repo};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, RwLock};

use crate::{
    fs::file_utils::FileContent,
    helpers::{
        branch::{BinaryDocState, BranchState, BranchesMetadataDoc},
        utils::{CommitMetadata, commit_with_attribution_and_timestamp},
    },
};

mod branch;
mod commit;
mod file;
mod util;

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
    // TODO (Lilith): consider swapping to RwLock
    inner: Arc<Mutex<BranchDbInner>>,
    // These are immutable, so they can be outside the mutex
    project_dir: PathBuf,
    ignore_globs: Arc<Vec<glob::Pattern>>,
}

#[derive(Debug)]
pub struct BranchDbInner {
    // TODO (Lilith): We need to figure out a way to populate binary docs.
    // The old system is hugely flawed and would be best to avoid.
    // Instead, consider a helper shim class to track each binary doc as they come in, and update the fs as needed.
    // But what's the intended user behavior? Consult with Nikita and Paul.

    // Does this need to be in inner? Probably not, right?
    repo: Repo,
    username: Option<String>,

    binary_states: HashMap<DocumentId, BinaryDocState>,
    branch_states: HashMap<DocumentId, Arc<Mutex<BranchState>>>,
    metadata_state: Option<(DocumentId, BranchesMetadataDoc)>,

    // The checked out ref is the ref that the filesystem is currently synced with.
    // Has a separate lock because of its importance; it needs to be locked while we're prepping a commit or checking out stuff
    checked_out_ref: Arc<RwLock<Option<HistoryRef>>>,
}

impl BranchDb {
    pub fn new(repo: Repo, project_dir: PathBuf, ignore_globs: Vec<glob::Pattern>) -> Self {
        Self {
            project_dir,
            ignore_globs: Arc::new(ignore_globs),
            inner: Arc::new(Mutex::new(BranchDbInner {
                repo,
                username: None,
                binary_states: HashMap::new(),
                branch_states: HashMap::new(),
                metadata_state: None,
                checked_out_ref: Arc::new(RwLock::new(None)),
            })),
        }
    }


    pub fn get_ignore_globs(&self) -> Vec<glob::Pattern> {
        (*self.ignore_globs).clone()
    }

    pub fn get_project_dir(&self) -> PathBuf {
        self.project_dir.clone()
    }

    pub async fn set_username(&self, username: Option<String>) {
        let mut inner = self.inner.lock().await;
        inner.username = username;
    }

    /// Get the mutable checked out ref for locking.
    pub async fn get_checked_out_ref_mut(&self) -> Arc<RwLock<Option<HistoryRef>>> {
        return self.inner.lock().await.checked_out_ref.clone();
    }

    pub async fn get_metadata_state(&self) -> Option<(DocumentId, BranchesMetadataDoc)> {
        let inner = self.inner.lock().await;
        // This is a needlessly expensive operation; we should consider allowing reference introspection via external lockers.
        // And/or improve clone perf by reducing string usage in BranchesMetadataDoc.
        inner.metadata_state.clone()
    }

    pub async fn set_metadata_state(&self, id: DocumentId, state: BranchesMetadataDoc) {
        let mut inner = self.inner.lock().await;
        inner.metadata_state = Some((id, state));
    }

    pub async fn has_branch(&self, id: &DocumentId) -> bool {
        let inner = self.inner.lock().await;
        return inner.branch_states.contains_key(id);
    }

    pub async fn insert_branch_state_if_not_exists<F>(&self, id: DocumentId, f: F)
    where
        F: FnOnce() -> BranchState,
    {
        let mut inner = self.inner.lock().await;
        inner
            .branch_states
            .entry(id.clone())
            .or_insert_with(|| Arc::new(Mutex::new(f())));
    }

    // This exposes inner BranchState objects via Arc. This is important because we use branch states all over the place.
    // Alternatively we could provide a read-only view in a closure, or clone them.
    // We do need to be a little careful about locks though.
    pub async fn get_branch_state(&self, id: &DocumentId) -> Option<Arc<Mutex<BranchState>>> {
        let inner = self.inner.lock().await;
        inner.branch_states.get(id).cloned()
    }

    pub async fn get_branch_handle(&self, id: &DocumentId) -> Option<DocHandle> {
        let inner = self.inner.lock().await;
        let Some(state) = inner.branch_states.get(id) else {
            return None;
        };
        Some(state.lock().await.doc_handle.clone())
    }
}
