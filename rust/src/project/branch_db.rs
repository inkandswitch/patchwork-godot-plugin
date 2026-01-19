use automerge::{Automerge, ChangeHash, ROOT};
use samod::{DocHandle, DocumentId, Repo};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::SystemTime;
use std::str::FromStr;
use crate::fs::file_utils::FileContent;
use crate::project::branch_doc_wrapper::BranchDocWrapper;
use crate::helpers::branch::{Branch, BranchState, BranchStateForkInfo, ForkInfo};
use crate::helpers::utils::{CommitMetadata, commit_with_attribution_and_timestamp, heads_to_vec_string};
use automerge::ReadDoc;
use autosurgeon::{hydrate, reconcile};
use tokio::spawn;
/// Core database structure that can be merged across threads
/// Contains all automerge documents and derived branch state
#[derive(Clone, Debug)]
pub struct BranchDB {
    pub version: u64,

    pub last_merge: SystemTime,

    pub project_doc_id: Option<DocumentId>,

    pub branches_metadata_doc_handle: Option<DocHandle>,
    
    pub branch_wrappers: HashMap<DocumentId, BranchDocWrapper>,
    
    pub binary_doc_handles: HashMap<DocumentId, DocHandle>,
    
    pub main_branch_doc_id: Option<DocumentId>,
}

impl BranchDB {
    pub fn new() -> Self {
        Self {
            version: 0,
            last_merge: SystemTime::now(),
            project_doc_id: None,
            branches_metadata_doc_handle: None,
            branch_wrappers: HashMap::new(),
            binary_doc_handles: HashMap::new(),
            main_branch_doc_id: None,
        }
    }
    
    pub fn merge(&mut self, other: &BranchDB) -> MergeResult {
        let mut result = MergeResult::default();
        
        // Merge branches metadata document
        match (&mut self.branches_metadata_doc_handle, &other.branches_metadata_doc_handle) {
            (Some(self_handle), Some(other_handle)) => {
                // Merge the branches metadata document (CRDT merge)
                self_handle.with_document(|mut self_doc| {
                    other_handle.with_document(|other_doc| {
                        if let Err(e) = self_doc.merge(other_doc) {
                            tracing::error!("Failed to merge branches metadata doc: {:?}", e);
                            if let Some(doc_id) = &self.project_doc_id {
                                result.merge_errors.push((doc_id.clone(), e.to_string()));
                            }
                        } else {
                            if let Some(doc_id) = &self.project_doc_id {
                                result.merged_docs.insert(doc_id.clone());
                            }
                        }
                    });
                });
            }
            (None, Some(other_handle)) => {
                // New branches metadata document
                self.branches_metadata_doc_handle = Some(other_handle.clone());
                if let Some(doc_id) = &other.project_doc_id {
                    result.added_docs.insert(doc_id.clone());
                }
            }
            (Some(_), None) => {
                // Keep existing, nothing to merge
            }
            (None, None) => {
                // Neither has it, nothing to do
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
        
        // Merge branch wrappers
        for (branch_id, other_wrapper) in &other.branch_wrappers {
            match self.branch_wrappers.get_mut(branch_id) {
                Some(self_wrapper) => {
                    // Merge the branch document (CRDT merge)
                    self_wrapper.doc_handle.with_document(|mut self_doc| {
                        other_wrapper.doc_handle.with_document(|other_doc| {
                            if let Err(e) = self_doc.merge(other_doc) {
                                tracing::error!("Failed to merge branch doc {:?}: {:?}", branch_id, e);
                                result.merge_errors.push((branch_id.clone(), e.to_string()));
                            } else {
                                result.merged_docs.insert(branch_id.clone());
                            }
                        });
                    });
                    
                    // TODO: refactor BranchState into a mergable struct; right now we just use last-write-wins
                    if other.version > self.version {
                        // Update branch state (last-write-wins)
                        self_wrapper.branch_state = other_wrapper.branch_state.clone();
                        result.updated_branches.insert(branch_id.clone());
                    }
                }
                None => {
                    // New branch wrapper
                    self.branch_wrappers.insert(branch_id.clone(), other_wrapper.clone());
                    result.added_branches.insert(branch_id.clone());
                }
            }
        }
        
        // Update metadata (last-write-wins)
        if other.version > self.version {
            if other.project_doc_id.is_some() {
                self.project_doc_id = other.project_doc_id.clone();
            }
            if other.main_branch_doc_id.is_some() {
                self.main_branch_doc_id = other.main_branch_doc_id.clone();
            }
        }
        
        // Increment version and update timestamp
        self.version += 1;
        self.last_merge = SystemTime::now();
        
        result
    }


    pub fn get_files_on_branch_at(&self, branch_id: &DocumentId, heads: Option<&Vec<ChangeHash>>, filters: Option<&HashSet<String>>) -> (HashMap<String, FileContent>) {
        let (mut files, linked_doc_ids) = self.branch_wrappers.get(branch_id).unwrap().get_files_at(heads, filters);
        for (doc_id, path) in linked_doc_ids {
            match self.get_linked_file(&doc_id) {
                Some(file_content) => {
                    files.insert(path, file_content);
                }
                None => {
                    tracing::error!("linked file {:?} not found", path);
                }
            }
        }
        files
    }   

    fn get_linked_file(&self, doc_id: &DocumentId) -> Option<FileContent> {
		self.binary_doc_handles.get(&doc_id)
		.map(|doc_handle| {
			doc_handle.with_document(|d| match d.get(ROOT, "content") {
				Ok(Some((value, _))) if value.is_bytes() => {
					Some(FileContent::Binary(value.into_bytes().unwrap()))
				}
				Ok(Some((value, _))) if value.is_str() => {
					Some(FileContent::String(value.into_string().unwrap()))
				}
				_ => {
					None
				}
			})
		}).unwrap_or(None)
	}

    
    /// for staleness detection
    pub fn is_newer_than(&self, other: &BranchDB) -> bool {
        self.version > other.version
    }
    
    /// Get the version number
    pub fn version(&self) -> u64 {
        self.version
    }
    
    /// Get a branch wrapper by ID
    pub fn get_branch_wrapper(&self, branch_id: &DocumentId) -> Option<&BranchDocWrapper> {
        self.branch_wrappers.get(branch_id)
    }
    
    /// Get a mutable branch wrapper by ID
    pub fn get_branch_wrapper_mut(&mut self, branch_id: &DocumentId) -> Option<&mut BranchDocWrapper> {
        self.branch_wrappers.get_mut(branch_id)
    }
    
    /// Get all branch wrappers
    pub fn branch_wrappers(&self) -> &HashMap<DocumentId, BranchDocWrapper> {
        &self.branch_wrappers
    }
    
    /// Get all branch wrappers mutably
    pub fn branch_wrappers_mut(&mut self) -> &mut HashMap<DocumentId, BranchDocWrapper> {
        &mut self.branch_wrappers
    }
    
    /// Insert or update a branch wrapper
    pub fn insert_branch_wrapper(&mut self, branch_id: DocumentId, wrapper: BranchDocWrapper) {
        self.branch_wrappers.insert(branch_id, wrapper);
    }
    
    /// Check if a branch wrapper exists
    pub fn has_branch_wrapper(&self, branch_id: &DocumentId) -> bool {
        self.branch_wrappers.contains_key(branch_id)
    }
    
    /// Get the branches metadata document handle
    pub fn branches_metadata_doc_handle(&self) -> Option<&DocHandle> {
        self.branches_metadata_doc_handle.as_ref()
    }
    
    /// Set the branches metadata document handle
    pub fn set_branches_metadata_doc_handle(&mut self, handle: DocHandle) {
        self.branches_metadata_doc_handle = Some(handle);
        // Update project_doc_id to match
        self.project_doc_id = Some(self.branches_metadata_doc_handle.as_ref().unwrap().document_id().clone());
    }
    
    /// Get a binary document handle by ID
    pub fn get_binary_doc_handle(&self, doc_id: &DocumentId) -> Option<&DocHandle> {
        self.binary_doc_handles.get(doc_id)
    }
    
    /// Insert or update a binary document handle
    pub fn insert_binary_doc_handle(&mut self, doc_id: DocumentId, handle: DocHandle) {
        self.binary_doc_handles.insert(doc_id, handle);
    }
    
    pub async fn create_branch(
        &mut self,
        repo_handle: &Repo,
        name: String,
        source_branch_id: &DocumentId,
        user_name: Option<String>,
    ) -> Result<DocumentId, String> {
        // Get the source branch wrapper
        let source_wrapper = self.branch_wrappers.get(source_branch_id)
            .ok_or_else(|| format!("Source branch {:?} not found", source_branch_id))?;
        
        let source_doc_handle = &source_wrapper.doc_handle;

        // Clone the branch document
        let new_branch_handle = Self::clone_doc(repo_handle, source_doc_handle).await;
        let new_branch_id = new_branch_handle.document_id().clone();
        
        // Get the source branch heads
        let forked_at_heads: Vec<String> = source_doc_handle
            .with_document(|d| d.get_heads())
            .iter()
            .map(|h| h.to_string())
            .collect();
        
        // Create branch metadata
        let branch = Branch {
            name: name.clone(),
            id: new_branch_id.to_string(),
            fork_info: Some(ForkInfo {
                forked_from: source_branch_id.to_string(),
                forked_at: forked_at_heads,
            }),
            merge_info: None,
            created_by: user_name.clone(),
            merged_into: None,
            reverted_to: None,
        };
        
        // Update branches metadata document
        let branches_metadata_doc_handle = self.branches_metadata_doc_handle.as_mut()
            .ok_or_else(|| "Branches metadata document not found".to_string())?;
        
        branches_metadata_doc_handle.with_document(|d| {
            let mut branches_metadata: crate::helpers::branch::BranchesMetadataDoc = match hydrate(d) {
                Ok(metadata) => metadata,
                Err(e) => {
                    tracing::error!("Failed to hydrate branches metadata: {:?}", e);
                    return;
                }
            };
            let mut tx = d.transaction();
            branches_metadata.branches.insert(branch.id.clone(), branch);
            let _ = reconcile(&mut tx, branches_metadata);
            commit_with_attribution_and_timestamp(
                tx,
                &CommitMetadata {
                    username: user_name.clone(),
                    branch_id: None,
                    merge_metadata: None,
                    reverted_to: None,
                    changed_files: None,
                    is_setup: Some(true),
                },
            );
        });
        
        // Create BranchState for the new branch
        let new_branch_state = BranchState {
            name: name.clone(),
            doc_handle: new_branch_handle.clone(),
            linked_doc_ids: HashSet::new(), // Will be populated when update_branch_doc_state is called
            synced_heads: new_branch_handle.with_document(|d| d.get_heads()),
            fork_info: Some(BranchStateForkInfo {
                forked_from: source_branch_id.clone(),
                forked_at: new_branch_handle.with_document(|d| d.get_heads()),
            }),
            merge_info: None,
            is_main: self.main_branch_doc_id.as_ref() == Some(&new_branch_id),
            created_by: user_name.clone(),
            merged_into: None,
            revert_info: None,
        };
        
        // Create and insert the new branch wrapper
        let new_wrapper = BranchDocWrapper::new(new_branch_handle, new_branch_state);
        self.branch_wrappers.insert(new_branch_id.clone(), new_wrapper);
        
        tracing::debug!("BranchDB: created new branch: {:?}", new_branch_id);
        
        Ok(new_branch_id)
    }
    
    /// Clone a document by creating a new document and merging the source into it
    async fn clone_doc(repo_handle: &Repo, doc_handle: &DocHandle) -> DocHandle {
        let new_doc_handle = repo_handle.create(Automerge::new()).await.unwrap();
        let new_doc_handle_clone = new_doc_handle.clone();
        let doc_handle_clone = doc_handle.clone();
        tokio::task::spawn_blocking(move || {
            let _ = doc_handle_clone.with_document(|source_doc| {
                new_doc_handle_clone.with_document(|mut new_doc| {
                    new_doc.merge(source_doc)
                })
            });
        }).await.unwrap();
        
        new_doc_handle
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
