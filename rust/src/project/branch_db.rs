use automerge::ChangeHash;
use samod::{DocHandle, DocumentId};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::SystemTime;
use crate::helpers::branch::BranchState;

/// Core database structure that can be merged across threads
/// Contains all automerge documents and derived branch state
#[derive(Clone, Debug)]
pub struct BranchDB {
    pub version: u64,

    pub last_merge: SystemTime,

    pub project_doc_id: Option<DocumentId>,

    pub doc_handles: HashMap<DocumentId, DocHandle>,

    pub binary_doc_handles: HashMap<DocumentId, DocHandle>,
    
    pub branch_states: HashMap<DocumentId, BranchState>,
    
    pub main_branch_doc_id: Option<DocumentId>,
    
    pub branches_metadata_doc_id: Option<DocumentId>,
}

impl BranchDB {
    pub fn new() -> Self {
        Self {
            version: 0,
            last_merge: SystemTime::now(),
            project_doc_id: None,
            doc_handles: HashMap::new(),
            binary_doc_handles: HashMap::new(),
            branch_states: HashMap::new(),
            main_branch_doc_id: None,
            branches_metadata_doc_id: None,
        }
    }
    
    pub fn merge(&mut self, other: &BranchDB) -> MergeResult {
        let mut result = MergeResult::default();
        
        // Merge Automerge documents
        for (doc_id, other_handle) in &other.doc_handles {
            match self.doc_handles.get_mut(doc_id) {
                Some(self_handle) => {
                    // Merge the automerge documents
                    // The pattern: mutable self_doc calls merge with immutable other_doc
                    self_handle.with_document(|mut self_doc| {
                        other_handle.with_document(|other_doc| {
                            if let Err(e) = self_doc.merge(other_doc) {
                                tracing::error!("Failed to merge document {:?}: {:?}", doc_id, e);
                                result.merge_errors.push((doc_id.clone(), e.to_string()));
                            } else {
                                result.merged_docs.insert(doc_id.clone());
                            }
                        });
                    });
                }
                None => {
                    // New document, just add it
                    self.doc_handles.insert(doc_id.clone(), other_handle.clone());
                    result.added_docs.insert(doc_id.clone());
                }
            }
        }
        
        // Merge binary documents
        // TODO: The idea is that we solve the issue of slow-loading binary docs by just having an empty doc handle
        // when it's not loaded yet.
        for (doc_id, other_handle) in &other.binary_doc_handles {
            match self.binary_doc_handles.get_mut(doc_id) {
                Some(self_handle) => {
                    self_handle.with_document(|mut self_doc| {
                        other_handle.with_document(|other_doc| {
                            if let Err(e) = self_doc.merge(other_doc) {
                                tracing::error!("Failed to merge binary doc {:?}: {:?}", doc_id, e);
                                result.merge_errors.push((doc_id.clone(), e.to_string()));
                            } else {
                                result.merged_binary_docs.insert(doc_id.clone());
                            }
                        });
                    });
                }
                None => {
                    self.binary_doc_handles.insert(doc_id.clone(), other_handle.clone());
                    result.added_binary_docs.insert(doc_id.clone());
                }
            }
        }
        
        // Merge branch states
        for (branch_id, other_state) in &other.branch_states {
            match self.branch_states.get(branch_id) {
                Some(_self_state) => {
                    // TODO: refactor BranchState into a mergable struct; right now we just use last-write-wins
                    if other.version > self.version {
                        self.branch_states.insert(branch_id.clone(), other_state.clone());
                        result.updated_branches.insert(branch_id.clone());
                    }
                }
                None => {
                    // New branch state
                    self.branch_states.insert(branch_id.clone(), other_state.clone());
                    result.added_branches.insert(branch_id.clone());
                }
            }
        }
        
        // Update metadata
        if other.version > self.version {
            if other.project_doc_id.is_some() {
                self.project_doc_id = other.project_doc_id.clone();
            }
            if other.main_branch_doc_id.is_some() {
                self.main_branch_doc_id = other.main_branch_doc_id.clone();
            }
            if other.branches_metadata_doc_id.is_some() {
                self.branches_metadata_doc_id = other.branches_metadata_doc_id.clone();
            }
        }
        
        // Increment version and update timestamp
        self.version += 1;
        self.last_merge = SystemTime::now();
        
        result
    }
    
    /// for staleness detection
    pub fn is_newer_than(&self, other: &BranchDB) -> bool {
        self.version > other.version
    }
    
    /// Get the version number
    pub fn version(&self) -> u64 {
        self.version
    }
}

/// Result of a merge operation, indicating what changed
#[derive(Debug, Default)]
pub struct MergeResult {
    pub merged_docs: HashSet<DocumentId>,
    pub added_docs: HashSet<DocumentId>,
    // TODO: figure out how to handle binary docs
    pub merged_binary_docs: HashSet<DocumentId>,
    pub added_binary_docs: HashSet<DocumentId>,
    pub updated_branches: HashSet<DocumentId>,
    pub added_branches: HashSet<DocumentId>,
    pub merge_errors: Vec<(DocumentId, String)>,
}

/// Shared BranchDB accessible from all threads
pub type SharedBranchDB = Arc<tokio::sync::RwLock<BranchDB>>;

/// Thread-local BranchDB copy with synchronization helpers
#[derive(Debug)]
pub struct ThreadLocalBranchDB {
    local: BranchDB,
    shared: SharedBranchDB,
    last_synced_version: u64,
}

impl ThreadLocalBranchDB {
    /// create a new ThreadLocalBranchDB by cloning the current shared state
    pub async fn new(shared: SharedBranchDB) -> Self {
        let local = shared.read().await.clone();
        let last_synced_version = local.version;
        Self {
            local,
            shared,
            last_synced_version,
        }
    }
    
    /// Get a read-only reference to local copy
    pub fn read(&self) -> &BranchDB {
        &self.local
    }
    
    /// get a writable reference to local copy
    pub fn write(&mut self) -> &mut BranchDB {
        &mut self.local
    }
    
    /// check if local copy is stale (shared has newer version)
    pub async fn is_stale(&self) -> bool {
        let shared_version = self.shared.read().await.version;
        shared_version > self.last_synced_version
    }
    
    /// pull latest changes from shared BranchDB into local copy
    pub async fn pull(&mut self) -> MergeResult {
        let shared = self.shared.read().await;
        let result = self.local.merge(&*shared);
        self.last_synced_version = self.local.version;
        result
    }
    
    /// push local changes to shared BranchDB
    pub async fn push(&mut self) -> MergeResult {
        let mut shared = self.shared.write().await;
        let result = shared.merge(&self.local);
        self.last_synced_version = shared.version;
        result
    }
    
    /// pull then push (sync both ways)
    pub async fn sync(&mut self) -> (MergeResult, MergeResult) {
        let pull_result = self.pull().await;
        let push_result = self.push().await;
        (pull_result, push_result)
    }
    
    /// get the last synced version
    pub fn last_synced_version(&self) -> u64 {
        self.last_synced_version
    }
    
    /// get the current local version
    pub fn local_version(&self) -> u64 {
        self.local.version
    }
}

impl Default for BranchDB {
    fn default() -> Self {
        Self::new()
    }
}
