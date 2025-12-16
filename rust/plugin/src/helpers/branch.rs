use automerge::ChangeHash;
use samod::{DocHandle, DocumentId};
use autosurgeon::{Hydrate, Reconcile};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Reconcile, Hydrate, PartialEq)]
pub struct FileEntry {
    pub content: Option<String>,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Reconcile, Hydrate, PartialEq)]
pub struct GodotProjectDoc {
    pub files: HashMap<String, FileEntry>,
    pub state: HashMap<String, HashMap<String, String>>,
}

// type AutoMergeSignalCallback = extern "C" fn(*mut c_void, *const std::os::raw::c_char, *const *const std::os::raw::c_char, usize) -> ();

#[derive(Debug, Clone, Reconcile, Hydrate, PartialEq)]
pub struct BranchesMetadataDoc {
    pub main_doc_id: String,
    pub branches: HashMap<String, Branch>,
}

#[derive(Debug, Clone, Reconcile, Hydrate, PartialEq)]
pub struct ForkInfo {
    pub forked_from: String,
    pub forked_at: Vec<String>,
}

#[derive(Debug, Clone, Reconcile, Hydrate, PartialEq)]
pub struct MergeInfo {
    pub merge_into: String,
    pub merge_at: Vec<String>,
}

#[derive(Debug, Clone, Reconcile, Hydrate, PartialEq)]
pub struct Branch {
    pub name: String,
    pub id: String,
    pub fork_info: Option<ForkInfo>,
    pub merge_info: Option<MergeInfo>,
	pub created_by: Option<String>,
	pub merged_into: Option<String>,
	pub reverted_to: Option<Vec<String>>,
}

#[derive(Debug, Clone)]
pub struct BinaryDocState {
    pub doc_handle: Option<DocHandle>, // is null if the binary doc is being requested but not loaded yet
}

#[derive(Debug, Clone)]
pub struct BranchStateForkInfo {
    pub forked_from: DocumentId,
    pub forked_at: Vec<ChangeHash>,
}

#[derive(Debug, Clone)]
pub struct BranchStateMergeInfo {
    pub merge_into: DocumentId,
    pub merge_at: Vec<ChangeHash>,
}

#[derive(Debug, Clone)]
pub struct BranchStateRevertInfo {
    pub reverted_to: Vec<ChangeHash>,
}

#[derive(Debug, Clone)]
pub struct BranchState {
    pub name: String,
    pub doc_handle: DocHandle,
    pub linked_doc_ids: HashSet<DocumentId>,
    pub synced_heads: Vec<ChangeHash>,
    pub fork_info: Option<BranchStateForkInfo>,
    pub merge_info: Option<BranchStateMergeInfo>,
	pub revert_info: Option<BranchStateRevertInfo>,
    pub is_main: bool,
	// These are currently not used by anything, but we want to keep them for later use.
	#[allow(dead_code)]
	pub created_by: Option<String>,
	#[allow(dead_code)]
	pub merged_into: Option<DocumentId>,
}
