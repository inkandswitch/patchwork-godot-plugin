use ::safer_ffi::prelude::*;
use automerge::{
    ChangeHash, ObjId, ObjType, ROOT, ReadDoc
};
use automerge::{Automerge, Patch, PatchAction, Prop};
use automerge_repo::{DocHandle, DocumentId, PeerConnectionInfo};
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender};
use godot::classes::resource_loader::CacheMode;
use godot::classes::{ClassDb, ResourceLoader};
use godot::global::str_to_var;
use godot::prelude::*;
use godot::prelude::Dictionary;
use tracing::instrument;
use std::{cell::RefCell, collections::HashSet};
use std::path::{PathBuf};
use std::time::SystemTime;
use std::{collections::HashMap, str::FromStr};

use crate::diff::differ::{ImportedDiff, TextDiffFile};
use crate::fs::file_system_driver::{FileSystemDriver, FileSystemEvent, FileSystemUpdateEvent};
use crate::fs::file_utils::FileContent;
use crate::helpers::branch::BranchState;
use crate::helpers::doc_utils::SimpleDocReader;
use crate::helpers::utils::{CommitInfo, ToShortForm, get_automerge_doc_diff, get_changed_files_vec, summarize_changes};
use crate::interop::godot_accessors::{EditorFilesystemAccessor, PatchworkConfigAccessor, PatchworkEditorAccessor};
use crate::parser::godot_parser::{GodotScene, TypeOrInstance};
use crate::project::project_driver::{ConnectionThreadError, DocHandleType, ProjectDriver, InputEvent, OutputEvent};
use crate::project::project_api::{BranchViewModel, ChangeViewModel, ProjectViewModel};
use crate::interop::godot_helpers::ToDict;
use crate::interop::godot_helpers::VariantTypeGetter;

/// Represents the state of the currently checked out branch.
#[derive(Debug, Clone)]
pub(super) enum CheckedOutBranchState {
	/// No branch is currently checked out.
    NothingCheckedOut(Option<DocumentId>),
	/// A branch is currently being checked out.
    CheckingOut(DocumentId, Option<DocumentId>),
	/// A branch is currently checked out.
    CheckedOut(DocumentId, Option<DocumentId>),
}

/// The different types of Godot-recognized string values that can be stored in a Variant.
enum VariantStrValue {
	/// A normal string that doesn't refer to a resource.
    Variant(String),
	/// A Godot resource path string.
    ResourcePath(String),
	/// A Godot sub-resource identifier string.
    SubResourceID(String),
	/// A Godot external resource identifier string.
    ExtResourceID(String),
}

// implement the to_string method for this enum
impl std::fmt::Display for VariantStrValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VariantStrValue::Variant(s) => write!(f, "{}", s),
            VariantStrValue::ResourcePath(s) => write!(f, "Resource({})", s),
            VariantStrValue::SubResourceID(s) => write!(f, "SubResource({})", s),
            VariantStrValue::ExtResourceID(s) => write!(f, "ExtResource({})", s),
        }
    }
}

/// A struct representing a path within an Automerge document, along with the associated patch action.
#[derive(Debug, Clone)]
struct PathWithAction {
    path: Vec<(ObjId, Prop)>,
    action: PatchAction,
}

fn match_path(path: &Vec<Prop>, patch: &Patch) -> Option<PathWithAction> {
    let mut remaining_path = patch.path.clone();

    for prop in path.iter() {
        if remaining_path.len() == 0 {
            return None;
        }

        let (_, part_prop) = remaining_path.remove(0);

        if part_prop != *prop {
            return None;
        }
    }

    Some(PathWithAction {
        path: remaining_path,
        action: patch.action.clone(),
    })
}

/// Manages the state and operations of a Patchwork project within Godot.
/// Its API is exposed to GDScript via the GodotProject struct.
#[derive(Debug)]
pub struct Project {
    doc_handles: HashMap<DocumentId, DocHandle>,
    pub(super) branch_states: HashMap<DocumentId, BranchState>,
    pub(super) checked_out_branch_state: CheckedOutBranchState,
    project_doc_id: Option<DocumentId>,
    new_project: bool,
	should_update_godot: bool,
	pub(super) just_checked_out_new_branch: bool,
	last_synced: Option<(DocumentId, Vec<ChangeHash>)>,
    driver: Option<ProjectDriver>,
    pub(super) driver_input_tx: UnboundedSender<InputEvent>,
    driver_output_rx: UnboundedReceiver<OutputEvent>,
    pub(super) sync_server_connection_info: Option<PeerConnectionInfo>,
    file_system_driver: Option<FileSystemDriver>,
	project_dir: String,
	is_started: bool,
	initial_load: bool,
    pub(super) history: Vec<ChangeHash>,
    pub(super) changes: HashMap<ChangeHash, CommitInfo>,
	// use RefCell for interior cache mutability
	pub(super) diff_cache: RefCell<HashMap<(Vec<ChangeHash>, Vec<ChangeHash>), Dictionary>>,
	last_ingest: (SystemTime, i32),
	ingest_requested: bool
}

impl Default for Project {
	fn default() -> Self {
		// TODO: Move driver input tx and output rx to the GodotProjectImpl struct, like in FileSystemDriver
		let (driver_input_tx, _) = futures::channel::mpsc::unbounded();
		let (_, driver_output_rx) = futures::channel::mpsc::unbounded();
		Self {
            sync_server_connection_info: None,
            doc_handles: HashMap::new(),
            branch_states: HashMap::new(),
            checked_out_branch_state: CheckedOutBranchState::NothingCheckedOut(None),
            project_doc_id: None,
            new_project: true,
			should_update_godot: false,
			just_checked_out_new_branch: false,
			last_synced: None,
            driver: None,
            driver_input_tx,
            driver_output_rx,
            file_system_driver: None,
			project_dir: "".to_string(),
			is_started: false,
			initial_load: true,
			history: Vec::new(),
			changes: HashMap::new(),
			diff_cache: RefCell::new(HashMap::new()),
			last_ingest: (SystemTime::UNIX_EPOCH, 0),
			ingest_requested: false
		}
	}
}

/// The default server URL used for syncing Patchwork projects. Can be overridden by user or project configuration.
const DEFAULT_SERVER_URL: &str = "24.199.97.236:8080";

/// Notifications that can be emitted via process and consumed by GodotProject, in order to trigger signals to GDScript.
pub enum GodotProjectSignal {
	CheckedOutBranch,
	ChangesIngested
}

impl Project {
	pub fn globalize_path(&self, path: &String) -> String {
		// trim the project_dir from the front of the path
		if path.starts_with("res://") {
			let thing = PathBuf::from(self.project_dir.clone()).join(PathBuf::from(&path["res://".len()..].to_string()));

			#[cfg(not(target_os = "windows"))]
			{
				return thing.to_string_lossy().to_string().replace("\\", "/");
			}
			#[cfg(target_os = "windows")]
			{
				return thing.to_string_lossy().to_string();
			}
		} else {
			path.to_string()
		}
	}

	// TODO: We need to test this on Windows
	pub fn localize_path(&self, path: &String) -> String {
		let path = path.replace("\\", "/");
		let project_dir = self.project_dir.replace("\\", "/");
		if path.starts_with(&project_dir) {
			let thing = PathBuf::from("res://".to_string()).join(PathBuf::from(&path[project_dir.len()..].to_string()));
			thing.to_string_lossy().to_string()
		} else {
			path.to_string()
		}
	}

    pub fn get_project_doc_id(&self) -> Option<DocumentId> {
		self.project_doc_id.clone()
	}

    /// Expensive operation to ingest all branch changes from automerge into the project data.
    /// Should be called when we think there are new changes to process.
    fn ingest_changes(&mut self) {
        let Some(branch_state) = self.get_checked_out_branch_state() else {
            return;
        };

		tracing::info!("Ingesting changes...");

		let last_acked_heads = self.sync_server_connection_info
			.as_ref()
			.and_then(|i| i.docs.get(&branch_state.doc_handle.document_id()))
			.and_then(|p| p.last_acked_heads.as_ref());

        let changes = branch_state.doc_handle.with_doc(|d|
            d.get_changes(&[])
            .to_vec()
            .iter()
            .map(|c| {
                CommitInfo::from(c)
            })
            .collect::<Vec<CommitInfo>>()
        );

        self.history.clear();
        self.changes.clear();

		// Check to see what the most recent ingested commit is.
		let mut synced_until_index = -1;
		for (i, change) in changes.iter().enumerate() {
			if last_acked_heads.as_ref().is_some_and(|f| f.contains(&change.hash)) {
				synced_until_index = i as i32;
			}
		}

		// Consume changes into self.changes
		let mut prev_change = None;
		for (i, mut change) in changes.into_iter().enumerate() {
            self.history.push(change.hash);
			// If we're after the most recent ingested commit, we're not synced!
			change.synced = (i as i32) <= synced_until_index;
			change.summary = self.get_change_summary(&change);
			change.prev_change = prev_change;
			prev_change = Some(change.hash);
            self.changes.insert(change.hash, change);
        }
    }

	fn get_change_summary(&self, change: &CommitInfo) -> String {
		(|| {
			let meta = change.metadata.as_ref();
			let author = meta?.username.as_ref()?;

			// merge commit
			if let Some(merge_info) = &meta?.merge_metadata {
				let merged_branch = &self.get_branch(&merge_info.merged_branch_id.clone())?.get_name();
				return Some(format!("↪️ {author} merged {merged_branch} branch"));
			}

			// revert commit
			if let Some(revert_info) = &meta?.reverted_to {
				let heads = revert_info.iter()
					.map(|s| &s[..7])
					.collect::<Vec<&str>>().join(", ");
				return Some(format!("↩️ {author} reverted to {heads}"));
			}

			// initial commit
			if change.is_setup() {
				return Some(format!("Initialized repository"));
			}

			return Some(summarize_changes(&author, meta?.changed_files.as_ref()?));
		})().unwrap_or("Invalid data".to_string())
	}

	pub fn get_branch_name(&self, branch_id: &DocumentId) -> String {
		self.branch_states.get(branch_id).map(|b| b.name.clone()).unwrap_or(branch_id.to_string())
	}

	#[instrument(skip_all, level = tracing::Level::INFO)]
	pub fn merge_branch(&mut self, source_branch_doc_id: DocumentId, target_branch_doc_id: DocumentId) {
		println!("");
		tracing::info!("******** MERGE BRANCH: {:?} into {:?}",
			self.get_branch_name(&source_branch_doc_id),
			self.get_branch_name(&target_branch_doc_id)
		);
		println!("");

        self.driver_input_tx
            .unbounded_send(InputEvent::MergeBranch {
                source_branch_doc_id: source_branch_doc_id,
                target_branch_doc_id: target_branch_doc_id.clone(),
            })
            .unwrap();

		// setting previous branch to None so that we don't delete any files when we checkout the new branch
        self.checked_out_branch_state = CheckedOutBranchState::CheckingOut(target_branch_doc_id, None);
    }

	pub fn create_merge_preview_branch_between(
		&mut self,
		source_branch_doc_id: DocumentId,
		target_branch_doc_id: DocumentId,
	) {
		println!("");
		tracing::info!("******** CREATE MERGE PREVIEW BRANCH: {:?} into {:?}",
			self.get_branch_name(&source_branch_doc_id),
			self.get_branch_name(&target_branch_doc_id)
		);
		println!("");

        self.driver_input_tx
            .unbounded_send(InputEvent::CreateMergePreviewBranch {
                source_branch_doc_id,
                target_branch_doc_id,
            })
            .unwrap();
    }

	pub fn create_revert_preview_branch_for(&mut self, branch_doc_id: DocumentId, revert_to: Vec<ChangeHash>) {
		println!("");
		tracing::info!("******** CREATE REVERT PREVIEW BRANCH: {:?} to {:?}",
			self.get_branch_name(&branch_doc_id),
			revert_to.to_short_form()
		);
		println!("");
		let branch_state = self.get_checked_out_branch_state().unwrap();
		let heads = branch_state.doc_handle.with_doc(|d| {
			d.get_heads()
		});
		let content = self.get_changed_file_content_between(Some(branch_state.doc_handle.document_id().clone()), branch_state.doc_handle.document_id().clone(), heads.clone(), revert_to.clone(), true);
		let files = content.into_iter().map(|event| {
			match event {
				FileSystemEvent::FileCreated(path, content) => (path.to_string_lossy().to_string(), content),
				FileSystemEvent::FileModified(path, content) => (path.to_string_lossy().to_string(), content),
				FileSystemEvent::FileDeleted(path) => (path.to_string_lossy().to_string(), FileContent::Deleted),
			}
		}).collect::<Vec<(String, FileContent)>>();


		self.driver_input_tx
			.unbounded_send(InputEvent::CreateRevertPreviewBranch {
				branch_doc_id,
				files,
				revert_to,
			})
			.unwrap();

	}


	pub fn delete_branch(&mut self, branch_doc_id: DocumentId) {
        self.driver_input_tx
            .unbounded_send(InputEvent::DeleteBranch { branch_doc_id })
            .unwrap();
    }

	pub fn get_descendent_document(
		&self,
		previous_branch_id: DocumentId,
		current_doc_id: DocumentId,
		previous_heads: Vec<ChangeHash>,
		current_heads: Vec<ChangeHash>,
	) -> Option<DocumentId> {
		let branch_state = match self.branch_states.get(&current_doc_id) {
			Some(branch_state) => branch_state,
			None => return None,
		};
		if current_heads.len() == 0 {
			panic!("_get_descendent_document: current_heads is empty");
		}
		if previous_heads.len() == 0 {
			panic!("_get_descendent_document: previous_heads is empty");
		}

		if branch_state.doc_handle.with_doc(|d| {
				d.get_obj_id_at(ROOT, "files", &previous_heads).is_some() &&
				d.get_obj_id_at(ROOT, "files", &current_heads).is_some()
		}) {
			return Some(current_doc_id);
		}
		// try it with the other doc_id
		let other_branch_state = match self.branch_states.get(&previous_branch_id) {
			Some(branch_state) => branch_state,
			None => {
				tracing::error!("previous branch id {} not found", previous_branch_id);
				return None;
			}
		};
		if other_branch_state.doc_handle.with_doc(|d| {
			d.get_obj_id_at(ROOT, "files", &previous_heads).is_some() &&
			d.get_obj_id_at(ROOT, "files", &current_heads).is_some()
		}) {
			return Some(previous_branch_id);
		}


		None

	}

	pub fn is_started(&self) -> bool {
		self.is_started
	}

	pub fn revert_to_heads(&mut self, to_revert_to: Vec<ChangeHash>) {
		let branch_state = self.get_checked_out_branch_state().unwrap();
		let heads = branch_state.doc_handle.with_doc(|d| {
			d.get_heads()
		});
		let content = self.get_changed_file_content_between(Some(branch_state.doc_handle.document_id().clone()), branch_state.doc_handle.document_id().clone(), heads.clone(), to_revert_to.clone(), true);
		let files = content.into_iter().map(|event| {
			match event {
				FileSystemEvent::FileCreated(path, content) => (path, content),
				FileSystemEvent::FileModified(path, content) => (path, content),
				FileSystemEvent::FileDeleted(path) => (path, FileContent::Deleted),
			}
		}).collect::<Vec<(PathBuf, FileContent)>>();
		self.sync_files_at(branch_state.doc_handle.clone(), files, Some(heads), Some(to_revert_to), false);
		self.checked_out_branch_state = CheckedOutBranchState::CheckingOut(branch_state.doc_handle.document_id().clone(), None);
	}

	// INTERNAL FUNCTIONS
	/// Gets the current file content on the current branch @ the current synced heads that changed
	/// between the previous branch @ the previous heads and the current branch @ the current heads
	#[instrument(skip_all, level = tracing::Level::DEBUG)]
	fn get_changed_file_content_between(
		&self,
		previous_branch_id: Option<DocumentId>,
		current_doc_id: DocumentId,
		previous_heads: Vec<ChangeHash>,
		current_heads: Vec<ChangeHash>,
		force_slow_diff: bool,
	) -> Vec<FileSystemEvent> {

        let current_branch_state = match self.branch_states.get(&current_doc_id) {
            Some(branch_state) => branch_state,
            None => return Vec::new(),
        };

        let curr_heads = if current_heads.len() == 0 {
			tracing::warn!("current heads is empty, using synced heads");
            current_branch_state.synced_heads.clone()
        } else {
            current_heads
        };
		if previous_heads.len() == 0 {
			tracing::debug!("No previous heads, getting all files on current branch {:?} between {} and {}", current_branch_state.name, previous_heads.to_short_form(), curr_heads.to_short_form());
			let files = self.get_files_on_branch_at(current_branch_state, Some(&curr_heads), None);
			return files.into_iter().map(|(path, content)| {
				match content {
					FileContent::Deleted => {
						FileSystemEvent::FileDeleted(PathBuf::from(path))
					}
					_ => {
						FileSystemEvent::FileCreated(PathBuf::from(path), content)
					}
				}
			}).collect::<Vec<FileSystemEvent>>();
		}

		let descendent_doc_id: Option<DocumentId> = if let Some(previous_branch_id) = previous_branch_id.clone() {
			if previous_branch_id == current_doc_id {
				Some(current_doc_id.clone())
			} else {
				self.get_descendent_document(previous_branch_id, current_doc_id.clone(), previous_heads.clone(), curr_heads.clone())
			}
		} else {
			Some(current_doc_id.clone())
		};
		if descendent_doc_id.is_none() || force_slow_diff {
			// neither document is the descendent of the other, we can't do a fast diff,
			// we need to do it the slow way; get the files from both docs
			// TODO: Is there a fast way to do this?
			let previous_branch_state = match self.branch_states.get(&previous_branch_id.unwrap()) {
				Some(branch_state) => branch_state,
				None => {
					tracing::warn!("_get_changed_file_content_between: previous branch id not found");
					return Vec::new();
				},
			};
			tracing::debug!("No descendent doc id, doing slow diff between previous {:?} @ {} and current {:?} @ {}", previous_branch_state.name, previous_heads.to_short_form(), current_branch_state.name, curr_heads.to_short_form());

			let previous_files = self.get_files_on_branch_at(previous_branch_state, Some(&previous_heads), None);
			let current_files = self.get_files_on_branch_at(current_branch_state, Some(&curr_heads), None);
			let mut events = Vec::new();
			for (path, _) in previous_files.iter() {
				if !current_files.contains_key(path) {
					events.push(FileSystemEvent::FileDeleted(PathBuf::from(path)));
				}
			}
			for (path, content) in current_files {
				match content {
					FileContent::Deleted => {
						events.push(FileSystemEvent::FileDeleted(PathBuf::from(path)));
						continue
					}
					_ => {}
				}
				if !previous_files.contains_key(&path) {
					events.push(FileSystemEvent::FileCreated(PathBuf::from(path), content));
				} else if &content != previous_files.get(&path).unwrap() {
					events.push(FileSystemEvent::FileModified(PathBuf::from(path), content));
				}
			}
			return events;
		}
		let descendent_doc_id = descendent_doc_id.unwrap();
		let branch_state = match self.branch_states.get(&descendent_doc_id) {
			Some(branch_state) => branch_state,
			None => panic!("_get_changed_file_content_between: descendent doc id not found"),
		};
		tracing::debug!("descendent branch: {:?}, getting changes between {:?} @ {} and {:?} @ {}",
			branch_state.name,
			if let Some(previous_branch_id) = previous_branch_id {
				self.get_branch_name(&previous_branch_id)
			} else {
				self.get_branch_name(&current_doc_id)
			},
			previous_heads.to_short_form(),
			self.get_branch_name(&current_doc_id),
			curr_heads.to_short_form()
		);
        let (patches, old_file_set, curr_file_set) =
		branch_state.doc_handle.with_doc(|d| {
			let old_files_id: Option<ObjId> = d.get_obj_id_at(ROOT, "files", &previous_heads);
			let curr_files_id = d.get_obj_id_at(ROOT, "files", &curr_heads);
			let old_file_set = if old_files_id.is_none(){
				HashSet::<String>::new()
			} else {
				d.keys_at(&old_files_id.unwrap(), &previous_heads).into_iter().collect::<HashSet<String>>()
			};
			let curr_file_set = if curr_files_id.is_none(){
				HashSet::<String>::new()
			} else {
				d.keys_at(&curr_files_id.unwrap(), &curr_heads).into_iter().collect::<HashSet<String>>()
			};
			let patches = get_automerge_doc_diff(
				d,
				&previous_heads,
				&curr_heads,
			);
			(patches, old_file_set, curr_file_set)
		});

		let deleted_files = old_file_set.difference(&curr_file_set).into_iter().cloned().collect::<HashSet<String>>();
		let added_files = curr_file_set.difference(&old_file_set).into_iter().cloned().collect::<HashSet<String>>();
		let mut modified_files = HashSet::new();

		// log all patches
		let changed_files = get_changed_files_vec(&patches);
		for file in changed_files {
			if added_files.contains(&file) || deleted_files.contains(&file) {
				continue;
			}
			modified_files.insert(file);
		}
		let make_event = |path: String, content: FileContent| {
			if added_files.contains(&path) {
				match content {
					FileContent::Deleted => {
						FileSystemEvent::FileDeleted(PathBuf::from(path))
					}
					_ => {
						FileSystemEvent::FileCreated(PathBuf::from(path), content)
					}
				}
			} else if deleted_files.contains(&path) {
				FileSystemEvent::FileDeleted(PathBuf::from(path))
			} else if modified_files.contains(&path) {
				match content {
					FileContent::Deleted => {
						FileSystemEvent::FileDeleted(PathBuf::from(path))
					}
					_ => {
						FileSystemEvent::FileModified(PathBuf::from(path), content)
					}
				}
			} else {
				tracing::debug!("file not found in added_files, deleted_files, or modified_files: {:?}", path);
				FileSystemEvent::FileModified(PathBuf::from(path), content)
			}
		};
		let mut changed_file_events = Vec::new();

		let mut linked_doc_ids = Vec::new();
		for path in deleted_files.iter() {
			changed_file_events.push(FileSystemEvent::FileDeleted(PathBuf::from(path)));
		}

		branch_state.doc_handle.with_doc(|doc|{
			let files_obj_id: ObjId = doc.get_at(ROOT, "files", &curr_heads).unwrap().unwrap().1;

			for path in doc.keys_at(&files_obj_id, &curr_heads) {
				if !added_files.contains(&path) && !modified_files.contains(&path) {
					continue;
				}

				let file_entry = match doc.get_at(&files_obj_id, &path, &curr_heads) {
					Ok(Some((automerge::Value::Object(ObjType::Map), file_entry))) => file_entry,
					_ => {
						tracing::error!("failed to get file entry for {:?}", path);
						continue;
					}
				};

				match FileContent::hydrate_content_at(file_entry, &doc, &path, &curr_heads) {
					Ok(content) => {
						changed_file_events.push(make_event(path, content));
					},
					Err(res) => {
						match res {
							Ok(id) => {
								linked_doc_ids.push((id, path));
							},
							Err(error_msg) => {
								tracing::error!("error: {:?}", error_msg);
							}
						}
					}
				};
			}
		});

		for (doc_id, path) in linked_doc_ids {
			let linked_file_content: Option<FileContent> = self.get_linked_file(&doc_id);
			if let Some(file_content) = linked_file_content {
				changed_file_events.push(make_event(path, file_content));
			}
		}

		changed_file_events
    }


    fn get_files_at(&self, heads: Option<&Vec<ChangeHash>>, filters: Option<&HashSet<String>>) -> HashMap<String, FileContent> {
		match &self.checked_out_branch_state {
			CheckedOutBranchState::CheckedOut(branch_doc_id, _) => {
				let branch_state = match self.branch_states.get(&branch_doc_id) {
					Some(branch_state) => branch_state,
					None => {
						tracing::error!("_get_files_at: branch doc id {:?} not found", branch_doc_id);
						return HashMap::new();
					},
				};
				self.get_files_on_branch_at(branch_state, heads, filters)
			}
			_ => panic!("_get_files_at: no checked out branch"),
		}
	}

	fn get_linked_file(&self, doc_id: &DocumentId) -> Option<FileContent> {
		self.doc_handles.get(&doc_id)
		.map(|doc_handle| {
			doc_handle.with_doc(|d| match d.get(ROOT, "content") {
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

	#[instrument(skip_all, level = tracing::Level::DEBUG)]
	fn get_files_on_branch_at(&self, branch_state: &BranchState, heads: Option<&Vec<ChangeHash>>, filters: Option<&HashSet<String>>) -> HashMap<String, FileContent> {

        let mut files = HashMap::new();

        let heads = match heads {
            Some(heads) => heads.clone(),
            None => branch_state.synced_heads.clone(),
        };
		tracing::debug!("Getting files on branch {:?} at {}", branch_state.name, heads.to_short_form());
		let mut linked_doc_ids = Vec::new();
		let filtered_paths = if let Some(filters) = filters {
			filters
		} else {
			&HashSet::new()
		};

        branch_state.doc_handle.with_doc(|doc|{
			let files_obj_id: ObjId = doc.get_at(ROOT, "files", &heads).unwrap().unwrap().1;
			for path in doc.keys_at(&files_obj_id, &heads) {
				if filtered_paths.len() > 0 && !filtered_paths.contains(&path) {
					continue;
				}
				let file_entry = match doc.get_at(&files_obj_id, &path, &heads) {
					Ok(Some((automerge::Value::Object(ObjType::Map), file_entry))) => file_entry,
					_ => panic!("failed to get file entry for {:?}", path),
				};

				match FileContent::hydrate_content_at(file_entry, &doc, &path, &heads) {
					Ok(content) => {
						files.insert(path, content);
					},
					Err(res) => {
						match res {
							Ok(id) => {
								linked_doc_ids.push((id, path));
							},
							Err(error_msg) => {
								tracing::error!("error: {:?}", error_msg);
							}
						}
					}
				};
			}
		});

		for (doc_id, path) in linked_doc_ids {
			let linked_file_content: Option<FileContent> = self.get_linked_file(&doc_id);
			if let Some(file_content) = linked_file_content {
				files.insert(path, file_content);
			} else {
				tracing::warn!("linked file {:?} not found", path);
			}
		}

        return files;

        // try to read file as scene
    }


	#[instrument(skip_all, level = tracing::Level::INFO)]
    fn sync_files_at(&self,
                      branch_doc_handle: DocHandle,
                      files: Vec<(PathBuf, FileContent)>, /*  Record<String, Variant> */
                      heads: Option<Vec<ChangeHash>>,
					  revert: Option<Vec<ChangeHash>>,
					  new_project: bool)
    {
		let filter = files.iter().map(|(path, _)| path.to_string_lossy().to_string()).collect::<HashSet<String>>();
		println!("");
		tracing::debug!("******** SYNC: branch {:?} at {:?}, num files: {}",
			self.branch_states.get(&branch_doc_handle.document_id()).map(|b| b.name.clone()).unwrap_or("unknown".to_string()),
			if let Some(heads) = heads.as_ref() {
					heads.to_short_form()
			} else {
				"<CURRENT>".to_string()
			},
			files.len()
		);
		println!("");
		tracing::trace!("files: [{}]",
			files.iter().map(|(path, content)|
			format!("{}: {}", path.to_string_lossy().to_string(), content.to_short_form())
		).collect::<Vec<String>>().join(", "));
        let stored_files = self.get_files_at(heads.as_ref(), Some(&filter));
		let files_len = files.len();
        let changed_files: Vec<(String, FileContent)> = files.into_iter().filter_map(|(path, content)| {
            let path = path.to_string_lossy().to_string();
            let stored_content = stored_files.get(&path);
			if let Some(stored_content) = stored_content {
				if stored_content == &content {
                    return None;
                }
            }
            Some((path, content))
        }).collect();
		tracing::debug!("syncing {}/{} files", changed_files.len(), files_len);
		tracing::trace!("syncing actually changed files: [{}]", changed_files.iter().map(|(path, content)|
			format!("{}: {}", path, content.to_short_form())
		).collect::<Vec<String>>().join(", "));
		if let Some(revert_heads) = revert {
			let _ = self.driver_input_tx
				.unbounded_send(InputEvent::RevertTo {
					branch_doc_handle,
					heads,
					files: changed_files,
					revert_to: revert_heads,
				});
		} else if new_project {
			let _ = self.driver_input_tx
				.unbounded_send(InputEvent::InitialCheckin {
					branch_doc_handle,
					heads,
					files: changed_files,
				});
		} else {
			let _ = self.driver_input_tx
				.unbounded_send(InputEvent::SaveFiles {
					branch_doc_handle,
					heads,
					files: changed_files,
				});
		}
    }


    fn get_file_at(&self, path: String, heads: Option<Vec<ChangeHash>>) -> Option<FileContent> {
		let mut ret: Option<FileContent> = None;
		{
			let files = self.get_files_at(heads.as_ref(),Some(&HashSet::from_iter(vec![path.clone()])));
			for file in files.into_iter() {
				if file.0 == path {
					ret = Some(file.1);
					break;
				} else {
					panic!("Returned a file that didn't match the path!?!??!?!?!?!?!?!!? {:?} != {:?}", file.0, path);
				}
			}
		}

		ret
    }

	pub fn get_checked_out_branch_state(&self) -> Option<&BranchState> {
        match &self.checked_out_branch_state {
            CheckedOutBranchState::CheckedOut(branch_doc_id, _) =>
				self.branch_states.get(&branch_doc_id),
            _ => None
        }
    }

    fn get_varstr_value(&self, prop_value: String) -> VariantStrValue {
        if prop_value.starts_with("Resource(") || prop_value.starts_with("SubResource(") || prop_value.starts_with("ExtResource(") {
            let id = prop_value
                .split("(\"")
                .nth(1)
                .unwrap()
                .split("\")")
                .nth(0)
                .unwrap()
                .trim()
                .to_string();
            if prop_value.contains("SubResource(") {
                return VariantStrValue::SubResourceID(id);
            } else if prop_value.contains("ExtResource(") {
                return VariantStrValue::ExtResourceID(id);
            } else {
                // Resource()
                return VariantStrValue::ResourcePath(id);
            }
        }
        // normal variant string
        return VariantStrValue::Variant(prop_value);
    }

    fn get_resource_at(
        &self,
        path: String,
		file_content: &FileContent,
        heads: Vec<ChangeHash>,
    ) -> Option<Variant> {
		let import_path = format!("{}.import", path);
        let mut import_file_content = self.get_file_at(import_path.clone(), Some(heads.clone()));
		if import_file_content.is_none() { // try at current heads
			import_file_content = self.get_file_at(import_path.clone(), None);
		}
        return self.create_temp_resource_from_content(&path, &file_content, &heads, import_file_content.as_ref());
    }

	fn create_temp_resource_from_content(
		&self,
		path: &str,
		content: &FileContent,
		heads: &Vec<ChangeHash>,
		import_file_content: Option<&FileContent>,
	) -> Option<Variant> {
        let temp_dir = format!(
            "res://.patchwork/temp_{}/",
            heads.first().to_short_form()
        );
        let temp_path = path.replace("res://", &temp_dir);
		if let Err(e) = FileContent::write_file_content(&PathBuf::from(self.globalize_path(&temp_path)), content) {
			tracing::error!("error writing file to temp path: {:?}", e);
			return None;
		}

		if let Some(import_file_content) = import_file_content {
            if let FileContent::String(import_file_content) = import_file_content {
                let import_file_content = import_file_content.replace("res://", &temp_dir);
                // regex to replace uid=uid://<...> and uid=uid://<invalid> with uid=uid://<...> and uid=uid://<invalid>
                let import_file_content =
                    import_file_content.replace(r#"uid=uid://[^\n]+"#, "uid=uid://<invalid>");
                // write the import file content to the temp path
                let import_file_path: String = format!("{}.import", temp_path);
                let _ = FileContent::write_file_content(
                    &PathBuf::from(self.globalize_path(&import_file_path)),
                    &FileContent::String(import_file_content),
                );

                let res = PatchworkEditorAccessor::import_and_load_resource(&temp_path);
                if res.is_nil() {
					tracing::error!("error importing resource: {:?}", temp_path);
                    return None;
                }
                return Some(res);
            }
        }
        let resource = ResourceLoader::singleton()
            .load_ex(&GString::from(temp_path))
            .cache_mode(CacheMode::IGNORE_DEEP)
            .done();
        if let Some(resource) = resource {
            return Some(resource.to_variant());
        }
		None
	}

    fn get_resource_diff(
        &self,
        path: &String,
        change_type: &str,
        old_content: Option<&FileContent>,
        new_content: Option<&FileContent>,
        old_heads: &Vec<ChangeHash>,
        curr_heads: &Vec<ChangeHash>,
    ) -> Dictionary {
        let imported_diff = self.get_imported_diff(path, old_content, new_content, old_heads, curr_heads);
        let result = self.imported_diff_to_dict(path, change_type, &imported_diff);
        result
    }

	fn imported_diff_to_dict(
		&self,
		path: &String,
		change_type: &str,
		imported_diff: &ImportedDiff,
	) -> Dictionary {
        let mut result = vdict! {
            "path" : path.to_variant(),
            "diff_type" : "resource_changed".to_variant(),
            "change_type" : change_type.to_variant(),
            "old_content" : imported_diff.old_content.as_ref().unwrap_or(&FileContent::Deleted).to_variant(),
            "new_content" : imported_diff.new_content.as_ref().unwrap_or(&FileContent::Deleted).to_variant(),
        };
		if let Some(old_content) = imported_diff.old_content.as_ref() {
            if let Some(old_resource) =
                self.create_temp_resource_from_content(&path, old_content, &imported_diff.old_heads, imported_diff.old_import_info.as_ref())
            {
                let _ = result.insert("old_resource", old_resource);
            }
        }
        if let Some(new_content) = imported_diff.new_content.as_ref() {
            if let Some(new_resource) =
                self.create_temp_resource_from_content(&path, new_content, &imported_diff.new_heads, imported_diff.new_import_info.as_ref())
            {
                let _ = result.insert("new_resource", new_resource);
            }
        }

		result
	}

	fn get_imported_diff(
		&self,
		path: &String,
		old_content: Option<&FileContent>,
		new_content: Option<&FileContent>,
		old_heads: &Vec<ChangeHash>,
		curr_heads: &Vec<ChangeHash>,
	) -> ImportedDiff {
		let import_path = format!("{}.import", path);
		let get_import_file_content = |heads: &Vec<ChangeHash>| -> Option<FileContent> {
			let mut import_file_content = self.get_file_at(import_path.clone(), Some(heads.clone()));
			if import_file_content.is_none() { // try at current heads
				import_file_content = self.get_file_at(import_path.clone(), None);
			}
			import_file_content
		};
		let old_import_file_content = if let Some(content) = old_content {
			if !matches!(content, FileContent::Deleted) {
				get_import_file_content(old_heads)
			} else {
				None
			}
		} else {
			None
		};
		let new_import_file_content = if let Some(content) = new_content {
			if !matches!(content, FileContent::Deleted) {
				get_import_file_content(curr_heads)
			} else {
				None
			}
		} else {
			None
		};
		ImportedDiff::create(
			old_heads.clone(),
			curr_heads.clone(),
			if let Some(old_content) = old_content {
				Some(old_content.clone())
			} else {
				None
			},
			if let Some(new_content) = new_content {
				Some(new_content.clone())
			} else {
				None
			},
			old_import_file_content,
			new_import_file_content,
		)
	}

    fn get_text_file_diff(
        &self,
        path: &String,
        change_type: &str,
        old_content: Option<&FileContent>,
        new_content: Option<&FileContent>,
    ) -> Dictionary {
        let empty_string = String::from("");
        let old_text = if let Some(FileContent::String(s)) = old_content {
            &s
        } else {
            &empty_string
        };
        let new_text = if let Some(FileContent::String(s)) = new_content {
            &s
        } else {
            &empty_string
        };
        let text_diff = TextDiffFile::create(path.clone(), path.clone(), old_text, new_text);
        let result = vdict! {
            "path" : path.to_variant(),
            "change_type" : change_type.to_variant(),
            "old_content" : old_content.unwrap_or(&FileContent::Deleted).to_variant(),
            "new_content" : new_content.unwrap_or(&FileContent::Deleted).to_variant(),
            "text_diff" : text_diff.to_godot(),
            "diff_type" : "text_changed".to_variant(),
        };
        result
    }

    fn get_non_scene_diff(
        &self,
        path: &String,
        change_type: &str,
        old_content: Option<&FileContent>,
        new_content: Option<&FileContent>,
        old_heads: &Vec<ChangeHash>,
        curr_heads: &Vec<ChangeHash>,
    ) -> Dictionary {
        let old_content_type = old_content.unwrap_or(&FileContent::Deleted).get_variant_type();
        let new_content_type = new_content.unwrap_or(&FileContent::Deleted).get_variant_type();
        if change_type == "unchanged" {
            return vdict! {
                "path" : path.to_variant(),
                "diff_type" : "file_unchanged".to_variant(),
                "change_type" : change_type.to_variant(),
                "old_content": old_content.unwrap_or(&FileContent::Deleted).to_variant(),
                "new_content": new_content.unwrap_or(&FileContent::Deleted).to_variant(),
            };
        }
        if old_content_type != VariantType::STRING && new_content_type != VariantType::STRING {
            return self.get_resource_diff(
                &path,
                &change_type,
                old_content,
                new_content,
                &old_heads,
                &curr_heads,
            );
        } else if old_content_type != VariantType::PACKED_BYTE_ARRAY
            && new_content_type != VariantType::PACKED_BYTE_ARRAY
        {
            return self.get_text_file_diff(&path, &change_type, old_content, new_content);
        } else {
            return vdict! {
                "path" : path.to_variant(),
                "diff_type" : "file_changed".to_variant(),
                "change_type" : change_type.to_variant(),
                "old_content" : old_content.unwrap_or(&FileContent::Deleted).to_variant(),
                "new_content" : new_content.unwrap_or(&FileContent::Deleted).to_variant(),
            };
        }
    }

	pub fn get_cached_diff(
		&self,
		heads_before: Vec<ChangeHash>,
		heads_after: Vec<ChangeHash>
	) -> Dictionary {
		(*self.diff_cache.borrow_mut())
			.entry((heads_before.clone(), heads_after.clone()))
			.or_insert_with(||
			self.get_changes_between(heads_before, heads_after))
			.clone()
	}

	#[instrument(skip_all, level = tracing::Level::DEBUG)]
    pub fn get_changes_between(
        &self,
        old_heads: Vec<ChangeHash>,
        curr_heads: Vec<ChangeHash>,
    ) -> Dictionary {
        let checked_out_branch_state = match self.get_checked_out_branch_state() {
            Some(branch_state) => branch_state,
            None => return Dictionary::new(),
        };

        let curr_heads = if curr_heads.len() == 0 {
            checked_out_branch_state.synced_heads.clone()
        } else {
            curr_heads
        };

		tracing::debug!("branch {:?}, getting changes between {} and {}", checked_out_branch_state.name, old_heads.to_short_form(), curr_heads.to_short_form());

		if old_heads == curr_heads{
			tracing::debug!("no changes");
			return Dictionary::new();
		}

        // only get the first 6 chars of the hash
        let patches: Vec<Patch> = checked_out_branch_state.doc_handle.with_doc(|d| {
            get_automerge_doc_diff(d, &old_heads, &curr_heads)
        });
        let mut changed_files_map = HashMap::new();
        let mut scene_files = Vec::new();

        let mut all_diff: HashMap<String, Dictionary> = HashMap::new();
        // Get old and new content
		let new_file_contents = self.get_changed_file_content_between(None, checked_out_branch_state.doc_handle.document_id().clone(), old_heads.clone(), curr_heads.clone(), false);
		let changed_files_set: HashSet<String> = new_file_contents.iter().map(|event|
			match event {
				FileSystemEvent::FileCreated(path, _) => path.to_string_lossy().to_string(),
				FileSystemEvent::FileModified(path, _) => path.to_string_lossy().to_string(),
				FileSystemEvent::FileDeleted(path) => path.to_string_lossy().to_string(),
			}
		).collect::<HashSet<String>>();
		let old_file_contents = self.get_files_on_branch_at(&checked_out_branch_state, Some(&old_heads), Some(&changed_files_set));

        for event in new_file_contents.iter() {
            let (path, new_file_content, change_type) = match event {
                FileSystemEvent::FileCreated(path, content) => (path.to_string_lossy().to_string(), content, "added"),
                FileSystemEvent::FileModified(path, content) => (path.to_string_lossy().to_string(), content, "modified"),
                FileSystemEvent::FileDeleted(path) => (path.to_string_lossy().to_string(), &FileContent::Deleted, "removed"),
            };
			let old_file_content = old_file_contents.get(&path).unwrap_or(&FileContent::Deleted);
            let old_content_type = old_file_content.get_variant_type();
            let new_content_type = new_file_content.get_variant_type();

            changed_files_map.insert(path.clone(), change_type.to_string());
            if old_content_type != VariantType::OBJECT && new_content_type != VariantType::OBJECT {
				let old_content = if old_content_type != VariantType::NIL { Some(old_file_content) } else { None };
				let new_content = if new_content_type != VariantType::NIL { Some(new_file_content) } else { None };
                // if both the old and new one are binary, or if one is none and the other is binary, then we can use the resource diff
                let _ = all_diff.insert(
                    path.clone(),
                    self.get_non_scene_diff(
                        &path,
                        &change_type,
                        old_content,
                        new_content,
                        &old_heads,
                        &curr_heads,
                    ),
                );
            } else {
                scene_files.push(path.clone());
            }
        }
        let mut loaded_ext_resources = HashMap::new();

        let mut get_scene_diff = |path: &String| -> Dictionary {
            let mut result = Dictionary::new();
            let _ = result.insert("path", path.to_variant());
            let _ = result.insert("diff_type", "scene_changed".to_variant());
            let _ = result.insert("change_type", "modified".to_variant());
            let _ = result.insert("old_content", Variant::nil());
            let _ = result.insert("new_content", Variant::nil());
            let mut changed_nodes = Array::new();
            let mut old_ext_resources = Dictionary::new();
            let mut new_ext_resources = Dictionary::new();
            // Get old and new scenes for content comparison
            let old_scene = match checked_out_branch_state
                .doc_handle
                .with_doc(|d: &Automerge| {
                    GodotScene::hydrate_at(d, &path, &old_heads)
                }) {
                Ok(scene) => Some(scene),
                Err(_) => None,
            };

            let new_scene = match checked_out_branch_state
                .doc_handle
                .with_doc(|d: &Automerge| {
                    GodotScene::hydrate_at(d, &path, &curr_heads)
                }) {
                Ok(scene) => Some(scene),
                Err(_) => None,
            };

            let patch_path = Vec::from([
                Prop::Map(String::from("files")),
                Prop::Map(String::from(path.clone())),
                Prop::Map(String::from("structured_content")),
                Prop::Map(String::from("nodes")),
            ]);

            let ext_resources_path = Vec::from([
                Prop::Map(String::from("files")),
                Prop::Map(String::from(path.clone())),
                Prop::Map(String::from("structured_content")),
                Prop::Map(String::from("ext_resources")),
            ]);

            let sub_resources_path = Vec::from([
                Prop::Map(String::from("files")),
                Prop::Map(String::from(path.clone())),
                Prop::Map(String::from("structured_content")),
                Prop::Map(String::from("sub_resources")),
            ]);
            let mut changed_ext_resources: HashSet<String> = HashSet::new();
            let mut all_changed_ext_resource_ids: HashSet<String> = HashSet::new();
            let all_changed_ext_resource_paths: HashSet<String> = HashSet::new();
            let mut added_ext_resources: HashSet<String> = HashSet::new();
            let mut deleted_ext_resources: HashSet<String> = HashSet::new();

            let mut changed_sub_resources: HashSet<String> = HashSet::new();
            let mut added_sub_resources: HashSet<String> = HashSet::new();
            let mut deleted_sub_resources: HashSet<String> = HashSet::new();
            let mut all_changed_sub_resource_ids: HashSet<String> = HashSet::new();

            let mut changed_node_ids: HashSet<i32> = HashSet::new();

            for patch in patches.iter() {
                match_path(&patch_path, &patch).inspect(
                    |PathWithAction { path, action: _ }| match path.first() {
                        Some((_, Prop::Map(node_id))) => {
                            // hack: only consider nodes where properties changed as changed
                            // this filters out all the parent nodes that don't really change only the child_node_ids change
                            // get second to last instead of last
                            if path.len() > 2 {
                                if let Some((_, Prop::Map(key))) = path.get(path.len() - 2) {
                                    if key == "properties" {
                                        changed_node_ids.insert(node_id.clone().parse::<i32>().unwrap());
                                        return;
                                    }
                                }
                            }
                            if let Some((_, Prop::Map(key))) = path.last() {
                                if key != "child_node_ids" {
                                    changed_node_ids.insert(node_id.clone().parse::<i32>().unwrap());
                                    return;
                                }
                            }
                        }
                        _ => {}
                    },
                );
                match_path(&ext_resources_path, &patch).inspect(
                    |PathWithAction { path, action: _action }| match path.first() {
                        Some((_, Prop::Map(ext_id))) => {
                            if let Some((_, Prop::Map(key))) = path.last() {
                                if key != "idx" {
                                    // ignore idx changes
                                    changed_ext_resources.insert(ext_id.clone());
                                    all_changed_ext_resource_ids.insert(ext_id.clone());
                                }
                            }
                        }
                        _ => {}
                    },
                );

                match_path(&sub_resources_path, &patch).inspect(
                    |PathWithAction { path, action: _ }| match path.first() {
                        Some((_, Prop::Map(sub_id))) => {
                            if path.len() > 2 {
                                if let Some((_, Prop::Map(key))) = path.get(path.len() - 2) {
                                    if key == "properties" {
                                        changed_sub_resources.insert(sub_id.clone());
                                        all_changed_sub_resource_ids.insert(sub_id.clone());
                                        return;
                                    }
                                }
                            }

                            if let Some((_, Prop::Map(key))) = path.last() {
                                if key != "idx" {
                                    // ignore idx changes
                                    changed_sub_resources.insert(sub_id.clone());
                                    all_changed_sub_resource_ids.insert(sub_id.clone());
                                }
                            }
                        }
                        _ => {}
                    },
                );
            }
            let mut all_node_ids = HashSet::new();
            let mut all_sub_resource_ids = HashSet::new();
            let mut all_ext_resource_ids = HashSet::new();
            let mut get_depsfn = |scene: Option<GodotScene>, ext_resources: &mut Dictionary| {
                if let Some(scene) = scene {
                    for (ext_id, ext_resource) in scene.ext_resources.iter() {
                        if changed_files_map.contains_key(&ext_resource.path) {
                            let change_type = changed_files_map.get(&ext_resource.path).unwrap();
                            if change_type == "modified" {
                                changed_ext_resources.insert(ext_id.clone());
                                all_changed_ext_resource_ids.insert(ext_id.clone());
                            }
                        }
                        all_ext_resource_ids.insert(ext_id.clone());
                        let _ = ext_resources.insert(ext_id.clone(), ext_resource.path.clone());
                    }
                    for (node_id, _node) in scene.nodes.iter() {
                        all_node_ids.insert(node_id.clone());
                    }
                    for (sub_id, _sub) in scene.sub_resources.iter() {
                        all_sub_resource_ids.insert(sub_id.clone());
                    }
                }
            };
            // now, we have to iterate through every ext_resource in the old and new scenes and compare their data by recursively calling this function
            if let Some(old_scene) = old_scene.clone() {
                get_depsfn(Some(old_scene), &mut old_ext_resources);
            }
            if let Some(new_scene) = new_scene.clone() {
                get_depsfn(Some(new_scene), &mut new_ext_resources);
            }

            for ext_id in all_ext_resource_ids.iter() {
                let old_has = old_scene
                    .as_ref()
                    .map(|scene| scene.ext_resources.get(ext_id).is_some())
                    .unwrap_or(false);
                let new_has = new_scene
                    .as_ref()
                    .map(|scene| scene.ext_resources.get(ext_id).is_some())
                    .unwrap_or(false);
                // check if the old_scene and the new_scene has the ext_resource to determine if it is added or deleted
                if old_has && !new_has {
                    deleted_ext_resources.insert(ext_id.clone());
                    all_changed_ext_resource_ids.insert(ext_id.clone());
                } else if !old_has && new_has {
                    added_ext_resources.insert(ext_id.clone());
                    all_changed_ext_resource_ids.insert(ext_id.clone());
                }
            }

            for sub_resource_id in all_sub_resource_ids.iter() {
                let old_has = old_scene
                    .as_ref()
                    .map(|scene| scene.sub_resources.get(sub_resource_id).is_some())
                    .unwrap_or(false);
                let new_has = new_scene
                    .as_ref()
                    .map(|scene| scene.sub_resources.get(sub_resource_id).is_some())
                    .unwrap_or(false);
                if old_has && !new_has {
                    deleted_sub_resources.insert(sub_resource_id.clone());
                    all_changed_sub_resource_ids.insert(sub_resource_id.clone());
                } else if !old_has && new_has {
                    added_sub_resources.insert(sub_resource_id.clone());
                    all_changed_sub_resource_ids.insert(sub_resource_id.clone());
                }
            }

            let _ = result.insert("old_ext_resources", old_ext_resources);
            let _ = result.insert("new_ext_resources", new_ext_resources);

            let fn_get_class_name = |type_or_instance: &TypeOrInstance,
                                     scene: &Option<GodotScene>| {
                match type_or_instance {
                    TypeOrInstance::Type(type_name) => type_name.clone(),
                    TypeOrInstance::Instance(instance_id) => {
                        if let Some(scene) = scene {
							// strip the "ExtResource(" and ")" from the instance_id
							let instance_id = instance_id.trim_start_matches("ExtResource(\"").trim_end_matches("\")");
                            if let Some(ext_resource) = scene.ext_resources.get(instance_id) {
                                return format!("Resource(\"{}\")", ext_resource.path);
                            }
                        }
                        String::new()
                    }
                }
            };

            let mut fn_get_prop_value = |prop_value: VariantStrValue,
                                         scene: &Option<GodotScene>,
                                         _is_old: bool|
                                         -> Variant {
                let path: Option<String>;
                match prop_value {
                    VariantStrValue::Variant(variant) => {
                        return str_to_var(&variant);
                    }
                    VariantStrValue::ResourcePath(resource_path) => {
                        path = Some(resource_path);
                    }
                    VariantStrValue::SubResourceID(sub_resource_id) => {
                        return format!("<SubResource {} changed>", sub_resource_id).to_variant();
                    }
                    VariantStrValue::ExtResourceID(ext_resource_id) => {
                        path = scene
                            .as_ref()
                            .map(|scene| {
                                scene
                                    .ext_resources
                                    .get(&ext_resource_id)
                                    .map(|ext_resource| ext_resource.path.clone())
                            })
                            .unwrap_or(None);
                    }
                }
                if let Some(path) = path {
                    // get old_resource or new_resource
                    let all_diff = &all_diff;
                    let diff = all_diff.get(&path);
                    if let Some(diff) = diff {
                        let resource = if _is_old {
                            diff.get("old_resource")
                        } else {
                            diff.get("new_resource")
                        };
                        if let Some(resource) = resource {
                            return resource;
                        }
                    }
                    if !loaded_ext_resources.contains_key(&path) {
                        // load it
                        let resource_content = self.get_file_at(
                            path.clone(),
                            if _is_old {
                                Some(old_heads.clone())
                            } else {
                                Some(curr_heads.clone())
                            },
                        );
                        if let Some(resource_content) = resource_content {
                            let resource = self.get_resource_at(
                                path.clone(),
                                &resource_content,
                                if _is_old {
                                    old_heads.clone()
                                } else {
                                    curr_heads.clone()
                                },
                            );
                            if let Some(resource) = resource {
                                let _ = loaded_ext_resources.insert(path.clone(), resource);
                            }
                        }
                    }
                    if let Some(resource) = loaded_ext_resources.get(&path) {
                        return resource.clone();
                    }
                }
                return format!("<ExtResource not found>").to_variant();
            };


            let mut get_changed_prop_dict =
                |prop: String, old_value: Option<VariantStrValue>, new_value: Option<VariantStrValue>| {
					// HACK: prevent loading script files during the diff and creating issues for the editor
					if prop == "script" {
						if old_value.is_some() && new_value.is_some() {
							return Some(vdict! {
								"name": prop.clone(),
								"change_type": "modified",
								"old_value": "<script_changed>",
								"new_value":"<script_changed>"
							});
						} else if old_value.is_some() {
							return Some(vdict! {
								"name": prop.clone(),
								"change_type": "deleted",
								"old_value": "<script_deleted>"
							});
						} else if new_value.is_some() {
							return Some(vdict! {
								"name": prop.clone(),
								"change_type": "added",
								"new_value": "<script_added>"
							});
						}
						return None;
					}
					if old_value.is_some() && new_value.is_some() {
						let mut changed = false;
						match (old_value.as_ref().unwrap(), new_value.as_ref().unwrap()) {
							(
								VariantStrValue::SubResourceID(sub_resource_id),
								VariantStrValue::SubResourceID(new_sub_resource_id),
							) => {
								if all_changed_sub_resource_ids.contains(sub_resource_id)
									|| all_changed_sub_resource_ids.contains(new_sub_resource_id)
								{
									changed = true;
								}
							}
							(
								VariantStrValue::ExtResourceID(ext_resource_id),
								VariantStrValue::ExtResourceID(new_ext_resource_id),
							) => {
								if ext_resource_id != new_ext_resource_id
									|| all_changed_ext_resource_ids.contains(ext_resource_id)
									|| all_changed_ext_resource_ids.contains(new_ext_resource_id)
								{
									changed = true;
								}
							}
							(
								VariantStrValue::ResourcePath(resource_path),
								VariantStrValue::ResourcePath(new_resource_path),
							) => {
								if resource_path!= new_resource_path ||
									all_changed_ext_resource_paths.contains(resource_path)
									|| all_changed_ext_resource_paths.contains(new_resource_path)
								{
									changed = true;
								} else if resource_path != new_resource_path {
									changed = true;
								}
							}
							(
								VariantStrValue::Variant(old_variant),
								VariantStrValue::Variant(new_variant),
							) => {
								if old_variant != new_variant {
									changed = true;
								}
							}
							_ => {
								// changed type
								changed = true;
							}
						}
						if changed {
							return Some(vdict! {
								"name": prop.clone(),
								"change_type": "modified",
								"old_value": fn_get_prop_value(old_value.unwrap(), &old_scene, true),
								"new_value": fn_get_prop_value(new_value.unwrap(), &new_scene, false)
							});
						}

                    return None;

				} else if old_value.is_some() {
					return Some(vdict! {
						"name": prop.clone(),
						"change_type": "deleted",
						"old_value": fn_get_prop_value(old_value.unwrap(), &old_scene, true)
					});
				} else if new_value.is_some() {
					return Some(vdict! {
						"name": prop.clone(),
						"change_type": "added",
						"new_value": fn_get_prop_value(new_value.unwrap(), &new_scene, false)
					});
				}
				return None;
			};
            // Handle changed sub resources
            // let mut changed_sub_resources_list: Array<Dictionary> = Array::new();
            // for sub_resource_id in changed_sub_resources.iter() {
            // 	let mut sub_resource_info = Dictionary::new();
            // 	sub_resource_info.insert("change_type", "modified");
            // 	sub_resource_info.insert("sub_resource_id", sub_resource_id.clone());
            // 	changed_sub_resources_list.push(&sub_resource_info);
            // }
            for node_id in all_node_ids.iter() {
                let old_has = old_scene
                    .as_ref()
                    .map(|scene| scene.nodes.get(node_id).is_some())
                    .unwrap_or(false);
                let new_has = new_scene
                    .as_ref()
                    .map(|scene| scene.nodes.get(node_id).is_some())
                    .unwrap_or(false);
                let mut changed_props: Dictionary = Dictionary::new();

				let removed = old_has && !new_has;
				let added = !old_has && new_has;
                if added || removed {
                    let mut node_info = Dictionary::new();
                    let _ = node_info.insert("change_type", if added { "added" } else { "removed" });
                    if let Some(scene) = if added { &new_scene } else { &old_scene } {
                        let _ = node_info.insert("node_path", scene.get_node_path(*node_id));
                        if let Some(node) = scene.nodes.get(&node_id.clone()) {
							let tp = fn_get_class_name(&node.type_or_instance, &new_scene);
							let _ = node_info.insert("type", tp);
							let mut changed_props = Dictionary::new();
							for (key, value) in node.properties.iter() {
								let val = value.get_value();
								if added {
									let changed_prop = get_changed_prop_dict(key.to_string(), None, Some(self.get_varstr_value(val)));
									_ = changed_props.insert(key.clone(), changed_prop.unwrap());
								} else {
									let changed_prop = get_changed_prop_dict(key.to_string(), Some(self.get_varstr_value(val)), None);
									_ = changed_props.insert(key.clone(), changed_prop.unwrap());
								}
							}
							let _ = node_info.insert("changed_props", changed_props);
                        }
                    }
                    changed_nodes.push(&node_info.to_variant());
                } else if old_has && new_has && changed_node_ids.contains(node_id) {
                    let mut node_info = Dictionary::new();
                    let _ = node_info.insert("change_type", "modified");

                    if let Some(scene) = &new_scene {
                        let _ = node_info.insert("node_path", scene.get_node_path(*node_id));
                    }
                    let mut old_props = Dictionary::new();
                    let mut new_props = Dictionary::new();
                    let mut old_type: TypeOrInstance = TypeOrInstance::Type(String::new());
                    let mut new_type: TypeOrInstance = TypeOrInstance::Type(String::new());
                    // Get old and new node content
                    if let Some(old_scene) = &old_scene {
                        if let Some(old_node) = old_scene.nodes.get(node_id) {
                            old_type = old_node.type_or_instance.clone();
                        }
                        if let Some(content) = old_scene.get_node(*node_id).map(|n| n.to_dict()) {
                            if let Some(props) = content.get("properties") {
                                old_props = props.to::<Dictionary>();
                            }
                            let _ = node_info.insert("old_content", content);
                        }
                    }

                    if let Some(new_scene) = &new_scene {
                        if let Some(new_node) = new_scene.nodes.get(node_id) {
                            new_type = new_node.type_or_instance.clone();
                        }
                        if let Some(content) = new_scene.get_node(*node_id).map(|n| n.to_dict()) {
                            if let Some(props) = content.get("properties") {
                                new_props = props.to::<Dictionary>();
                            }
                            let _ = node_info.insert("new_content", content);
                        }
                    }
                    // old_type and new_type
                    let old_class_name = fn_get_class_name(&old_type, &old_scene);
                    let new_class_name = fn_get_class_name(&new_type, &new_scene);

                    if old_class_name != new_class_name {
                        let _ = node_info.insert("change_type", "type_changed");
						let _ = node_info.insert("old_type", old_class_name);
						let _ = node_info.insert("new_type", new_class_name);
                    } else {
						let _ = node_info.insert("type", new_class_name);
                        let mut props: HashSet<String> = HashSet::new();
                        for (key, _) in old_props.iter_shared() {
                            let _ = props.insert(key.to_string());
                        }
                        for (key, _) in new_props.iter_shared() {
                            let _ = props.insert(key.to_string());
                        }
                        for prop in props {
							let changed_prop: Option<Dictionary>;
							{
								let prop = prop.clone();

								let sn_2: StringName = StringName::from(&prop);
								let default_value = if let TypeOrInstance::Type(class_name) = &new_type {
									ClassDb::singleton()
										.class_get_property_default_value(&StringName::from(class_name), &sn_2)
										.to_string()
								} else {
									"".to_string() // Instance properties are always set, regardless of the default value, so this is always empty
								};

								let old_prop = if let Some(old_prop) = old_props.get(prop.as_str()) {
									old_prop.to_string()
								} else {
									default_value.clone()
								};
								let new_prop = if let Some(new_prop) = new_props.get(prop.as_str()) {
									new_prop.to_string()
								} else {
									default_value.clone()
								};
								let old_value = self.get_varstr_value(old_prop.clone());
								let new_value: VariantStrValue = self.get_varstr_value(new_prop.clone());
								changed_prop = get_changed_prop_dict(prop, Some(old_value), Some(new_value));
							}

                            if let Some(changed_prop) = changed_prop
                            {
                                let _ = changed_props.insert(prop.clone(), changed_prop);
                            }
                        }
                        if changed_props.len() > 0 {
                            let _ = node_info.insert("changed_props", changed_props);
                        }
                        changed_nodes.push(&node_info.to_variant());
                    }
                }
            }
            let _ = result.insert("changed_nodes", changed_nodes);
            result
        };
        let mut scene_diffs: Vec<(String, Dictionary)> = Vec::new();
        for file in scene_files.iter() {
            scene_diffs.push((file.clone(), get_scene_diff(&file)));
        }
        for (file, diff) in scene_diffs {
            let _ = all_diff.insert(file, diff);
        }

        // If it's a scene file, add node changes
        let mut diff_result = Dictionary::new();
        for (path, diff) in all_diff {
            let _ = diff_result.insert(path.clone(), diff);
        }
        diff_result
    }

    fn start_driver(&mut self) {
        if self.driver.is_some() {
            return;
        }
        let (driver_input_tx, driver_input_rx) = futures::channel::mpsc::unbounded();
        let (driver_output_tx, driver_output_rx) = futures::channel::mpsc::unbounded();
        self.driver_input_tx = driver_input_tx;
        self.driver_output_rx = driver_output_rx;

        let storage_folder_path = self.globalize_path(&"res://.patchwork".to_string());
		let mut server_url = PatchworkConfigAccessor::get_project_value("server_url", "");
		if server_url.is_empty() {
			server_url = PatchworkConfigAccessor::get_user_value("server_url", "");
			if server_url.is_empty() {
				server_url = DEFAULT_SERVER_URL.to_string();
				tracing::info!("Using default server url: {:?}", server_url);
			} else {
				tracing::info!("Using user override for server url: {:?}", server_url);
			}
		} else {
			tracing::info!("Using project override for server url: {:?}", server_url);
		}

        let mut driver: ProjectDriver = ProjectDriver::create(storage_folder_path, server_url);
        let maybe_user_name: String = PatchworkConfigAccessor::get_user_value("user_name", "");
        driver.spawn(
            driver_input_rx,
            driver_output_tx,
            self.project_doc_id.clone(),
            if maybe_user_name == "" {
                None
            } else {
                Some(maybe_user_name)
            },
        );
        self.driver = Some(driver);
    }

    fn start_file_system_driver(&mut self) {
        let project_path: String = self.globalize_path(&"res://".to_string());
        let project_path = PathBuf::from(project_path);

		let mut ignore_globs = vec![
            "**/.DS_Store".to_string(),
            "**/thumbs.db".to_string(),
            "**/desktop.ini".to_string(),
            "**/patchwork.cfg".to_string(),
            "**/addons/patchwork*".to_string(),
			"**/target/*".to_string(),
			// "**/.godot".to_string(),
			"**/.*".to_string(),
			// "**/.patchwork*".to_string(),
			// "**/.patchwork/**/*".to_string(),
			// "res://addons/patchwork/**/*".to_string(),
        ];
		let mut parse_gitignore = |dir: PathBuf, file: &str| {
			let path = dir.join(file);
			let gitignore_content = if let Ok(content) = std::fs::read_to_string(path) {
				content
			} else {
				String::new()
			};

			for line in gitignore_content.lines() {
				// trim any comments and whitespace
				let line = line.trim().split('#').next().unwrap_or_default().trim();
				if line.is_empty() {
					continue;
				}
				let mut new_line = if line.starts_with("/") {
					line.to_string()
				} else {
					dir.join(line).to_string_lossy().to_string()
				};
				let new_line = if new_line.ends_with("/") {
					// just remove the trailing slash
					new_line.pop();
					new_line
				} else {
					new_line
				};
				ignore_globs.push(new_line);
			}
		};
		parse_gitignore(project_path.clone(), ".gitignore");
		parse_gitignore(project_path.clone(), ".patchworkignore");
		parse_gitignore(project_path.clone(), ".gdignore");


        self.file_system_driver = Some(FileSystemDriver::spawn(project_path, ignore_globs));
    }

    pub fn start(&mut self) {
        let project_doc_id: String = PatchworkConfigAccessor::get_project_value("project_doc_id", "");
        let checked_out_branch_doc_id = PatchworkConfigAccessor::get_project_value("checked_out_branch_doc_id", "");
        tracing::info!("Starting GodotProject with project doc id: {:?}", if project_doc_id == "" { "<NEW DOC>" } else { &project_doc_id });
		self.should_update_godot = false;
		self.just_checked_out_new_branch = false;
		self.last_synced = None;
        self.project_doc_id = match DocumentId::from_str(&project_doc_id) {
            Ok(doc_id) => Some(doc_id),
            Err(_e) => None,
        };
        self.new_project = match self.project_doc_id.is_none() {
            true => true,
            false => false,
        };

        self.checked_out_branch_state = match DocumentId::from_str(&checked_out_branch_doc_id) {
            Ok(doc_id) => CheckedOutBranchState::CheckingOut(doc_id, None),
            Err(_) => CheckedOutBranchState::NothingCheckedOut(None),
        };

        tracing::debug!(
            "initial checked out branch state: {:?}",
            self.checked_out_branch_state
        );

        self.start_driver();
        self.start_file_system_driver();
        self.is_started = true;
        // get the project path
    }

    fn stop_driver(&mut self) {
        if let Some(mut driver) = self.driver.take() {
            driver.teardown();
        }
    }

    pub fn stop(&mut self) {
		if !self.is_started {
			return;
		}
        self.stop_driver();
		if let Some(driver) = self.file_system_driver.take() {
			driver.stop();
		}
        self.checked_out_branch_state = CheckedOutBranchState::NothingCheckedOut(None);
        self.sync_server_connection_info = None;
        self.project_doc_id = None;
        self.doc_handles.clear();
        self.branch_states.clear();
        self.file_system_driver = None;
        self.is_started = false;
    }

	pub fn safe_to_update_godot(initial_load: bool) -> bool {
		return !(EditorFilesystemAccessor::is_scanning() ||
		PatchworkEditorAccessor::is_editor_importing() ||
		PatchworkEditorAccessor::is_changing_scene() ||
		(!initial_load && PatchworkEditorAccessor::unsaved_files_open())
	);
	}


	/// Syncs the local state of the patchwork project document(s) from the
	/// current local state to the current state at the current branch @ the current synced heads
	/// the current local state is defined by the given branch @ the given heads
	///
	/// from_branch_id is the branch that the current local state is on
	/// from_heads is the heads that the current local state is on
	#[instrument(skip_all, level = tracing::Level::INFO)]
    fn sync_patchwork_to_godot(&mut self, from_branch_id: Option<DocumentId>, from_heads: Vec<ChangeHash>) -> Vec<FileSystemEvent> {
		println!("");
		tracing::debug!("*** SYNC PATCHWORK TO GODOT");
		let current_branch_state = match self.get_checked_out_branch_state() {
			Some(branch_state) => branch_state,
			None => {
				tracing::error!("!!!!!!!no checked out branch!!!!!!");
				return Vec::new();
			}
		};
		let current_doc_id = current_branch_state.doc_handle.document_id();
		// TODO: Do we want synced heads or the current heads?
		let current_heads = current_branch_state.synced_heads.clone();
		let previous_heads = if from_heads.len() > 0 {
			from_heads
		} else {
			match &from_branch_id {
				Some(branch_id) => {
					match self.branch_states.get(branch_id) {
						Some(branch_state) => {
							tracing::warn!("no previous branch heads, using current branch heads on {:?}", branch_state.name);
							// TODO: Do we want synced heads or the current heads?
							branch_state.synced_heads.clone()
						}
						None => {
							tracing::error!("NO PREVIOUS BRANCH STATE?!?!?! Getting all changes from start to current_heads");
							Vec::new()
						}
					}
				}
				None => {
					tracing::info!("no previous branch id, getting all changes from start to current_heads");
					Vec::new()
				}
			}
		};
		if &current_doc_id == from_branch_id.as_ref().unwrap_or(&current_doc_id) && current_heads == previous_heads {
			tracing::debug!("heads are the same, no changes to sync");
			return Vec::new();
		}
		tracing::debug!("syncing branch {:?} from {}{} to {}", current_branch_state.name,
			if from_branch_id.as_ref().unwrap_or(&current_doc_id) != &current_doc_id {
				format!("{} @ ", self.get_branch_name(from_branch_id.as_ref().unwrap()))
			} else {
				"".to_string()
			}, previous_heads.to_short_form(), current_heads.to_short_form());
		let events = self.get_changed_file_content_between(from_branch_id, current_doc_id.clone(), previous_heads, current_heads, false);
		println!("");

        let mut updates = Vec::new();
        for event in events {
            match event {
                FileSystemEvent::FileDeleted(path) => {
                    updates.push(FileSystemUpdateEvent::FileDeleted(PathBuf::from(self.globalize_path(&path.to_string_lossy().to_string()).to_string())));
                }
                FileSystemEvent::FileCreated(path, content) => {
                    updates.push(FileSystemUpdateEvent::FileSaved(PathBuf::from(self.globalize_path(&path.to_string_lossy().to_string()).to_string()), content));
                }
                FileSystemEvent::FileModified(path, content) => {
                    updates.push(FileSystemUpdateEvent::FileSaved(PathBuf::from(self.globalize_path(&path.to_string_lossy().to_string()).to_string()), content));
                }
            }
        }
		if updates.len() == 0 {
			tracing::debug!("no updates to sync");
			return Vec::new();
		}
        if let Some(driver) = &mut self.file_system_driver {
            let events = driver.batch_update_blocking(updates);
			return events;
        }
		Vec::new()
    }

    fn sync_godot_to_patchwork(&mut self, new_project: bool) {
        match self.get_checked_out_branch_state() {
            Some(branch_state) => {
                // syncing the filesystem to patchwork
                // get_files_at returns patchwork stuff, we need to get the files from the filesystem
                if let Some(driver) = &self.file_system_driver {
                    let mut files = driver.get_all_files_blocking().into_iter().map(
                        |(path, content)| {
                            (self.localize_path(&path.to_string_lossy().to_string()).to_string(), content)
                        }
                    ).collect::<Vec<(String, FileContent)>>();
					if new_project {
						// Hack to prevent long reloads when opening a new project; we just resave all the scenes that need it
						let before_size: usize = files.len();
						files = files.into_iter().filter_map(
						|(path, content)|{
							if let FileContent::Scene(content) = content {
								return Some((path, FileContent::Scene(content)));
							}
							Some((path, content))
						}
						).collect::<Vec<_>>();
						let events: Vec<FileSystemEvent> = driver.batch_update_blocking(Vec::new());
						if before_size - files.len() != events.len() {
							tracing::error!("**** THIS SHOULD NOT HAPPEN: resaved {} files, but expected {} files back", before_size - files.len(), events.len());
							files = driver.get_all_files_blocking().into_iter().map(
								|(path, content)| {
									(self.localize_path(&path.to_string_lossy().to_string()).to_string(), content)
								}
							).collect::<Vec<(String, FileContent)>>();
						} else {
							files.extend(events.into_iter().map(|event| {
								match event {
									FileSystemEvent::FileCreated(path, content) => (self.localize_path(&path.to_string_lossy().to_string()), content),
									FileSystemEvent::FileModified(path, content) => (self.localize_path(&path.to_string_lossy().to_string()), content),
									FileSystemEvent::FileDeleted(path) => (self.localize_path(&path.to_string_lossy().to_string()), FileContent::Deleted)
								}
							}));
						}
					}
					self.sync_files_at(
						branch_state.doc_handle.clone(),
						files.into_iter().map(|(path, content)| (PathBuf::from(path), content)).collect::<Vec<(PathBuf, FileContent)>>(),
						Some(branch_state.synced_heads.clone()),
					None, true);
                }
            }
            None => panic!("couldn't save files, no checked out branch"),
        };
    }

	fn get_previous_branch_id(&self) -> Option<DocumentId> {
		match &self.checked_out_branch_state {
			CheckedOutBranchState::NothingCheckedOut(prev_branch_id) => prev_branch_id.clone(),
			CheckedOutBranchState::CheckingOut(_, prev_branch_id) => prev_branch_id.clone(),
			CheckedOutBranchState::CheckedOut(_, prev_branch_id) => prev_branch_id.clone(),
		}
	}

	pub fn new(project_dir: String) -> Self {
		Self {
			project_dir,
			..Default::default()
		}
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
		let Ok(last_diff) = now.duration_since(self.last_ingest.0) else { return false; };

		// Impose an arbitrary cap on requests within a time period.
		// This is so that immediate syncs -- such as those from a local server -- don't have to wait before getting synced.
		// But it also prevents spam of like a hundred slowing down the ingestion.
		if last_diff.as_millis() < 1 {
			if self.last_ingest.1 >= 3 {
				return false;
			}
		}
		else {
			// since we're past the duration with no other requests, the counter resets.
			self.last_ingest = (now, 0);
		}
		self.ingest_changes();
		self.ingest_requested = false;
		self.last_ingest.1 += 1;
		true
	}

	// TODO: this is a very long and complicated method. Ideally it could be factored out to be simpler.
	#[instrument(target = "patchwork_rust_core::godot_project::inner_process", level = tracing::Level::DEBUG, skip_all)]
	pub fn process(&mut self, _delta: f64) -> (Vec<FileSystemEvent>, Vec<GodotProjectSignal>) {
		let mut signals: Vec<GodotProjectSignal> = Vec::new();
		if self.try_ingest_changes() {
			signals.push(GodotProjectSignal::ChangesIngested);
		}

		if let Some(driver) = &mut self.driver {
			if let Some(error) = driver.connection_thread_get_last_error() {
				match error {
					ConnectionThreadError::ConnectionThreadDied(error) => {
						tracing::error!("automerge repo driver connection thread died, respawning: {}", error);
						if !driver.respawn_connection_thread() {
							tracing::error!("automerge repo driver connection thread failed too many times, aborting");
							// TODO: make the GUI do something with this
							self.request_ingestion();
						}
					}
					ConnectionThreadError::ConnectionThreadError(error) => {
						tracing::error!("automerge repo driver connection thread error: {}", error);
					}
				}
			}
		}

		let mut branches_changed = false;
        while let Ok(Some(event)) = self.driver_output_rx.try_next() {
            match event {
                OutputEvent::NewDocHandle {
                    doc_handle,
                    doc_handle_type,
                } => {
                    if doc_handle_type == DocHandleType::Binary {
                        tracing::trace!(
                            "NewBinaryDocHandle !!!! {} {} changes",
                            doc_handle.document_id(),
                            doc_handle.with_doc(|d| d.get_heads().len())
                        );
                    }

                    self.doc_handles
                        .insert(doc_handle.document_id(), doc_handle.clone());
                }
                OutputEvent::BranchStateChanged {
                    branch_state: new_branch_state,
                    trigger_reload,
                } => {
					let new_branch_state_doc_handle = new_branch_state.doc_handle.clone();
					let new_branch_state_doc_id = new_branch_state_doc_handle.document_id();
                    self.branch_states
                        .insert(new_branch_state_doc_id.clone(), new_branch_state);

					branches_changed = true;
                    let mut checking_out_new_branch = false;

                    let (active_branch_state, prev_branch_info) = match &self.checked_out_branch_state {
                        CheckedOutBranchState::NothingCheckedOut(prev_branch_id) => {
                            // check out main branch if we haven't checked out anything yet
							let cloned_prev_branch_id = prev_branch_id.clone();
							let branch_state = self.branch_states.get(&new_branch_state_doc_handle.document_id()).unwrap();
                            if branch_state.is_main {
                                checking_out_new_branch = true;

                                self.checked_out_branch_state = CheckedOutBranchState::CheckingOut(
                                    branch_state.doc_handle.document_id(),
                                    prev_branch_id.clone(),
                                );
                                (Some(branch_state), cloned_prev_branch_id)
                            } else {
								// we're still waiting for the project to be fully synced
								(None, None)
                            }
                        }
                        CheckedOutBranchState::CheckingOut(branch_doc_id, prev_branch_info) => {
							checking_out_new_branch = true;
                            (self.branch_states.get(branch_doc_id), prev_branch_info.clone())
                        }
                        CheckedOutBranchState::CheckedOut(branch_doc_id, prev_branch_info) => {
                            (self.branch_states.get(branch_doc_id), prev_branch_info.clone())
                        }
                    };

                    // only trigger update if checked out branch is fully synced
                    if let Some(active_branch_state) = active_branch_state {
                        if active_branch_state.is_synced() {
                            if checking_out_new_branch {
                                tracing::info!(
                                    "TRIGGER checked out new branch: {}",
                                    active_branch_state.name
                                );

                                self.checked_out_branch_state = CheckedOutBranchState::CheckedOut(
                                    active_branch_state.doc_handle.document_id(),
									prev_branch_info,
                                );

								self.just_checked_out_new_branch = true;
                            } else {
                                self.should_update_godot = self.should_update_godot || (new_branch_state_doc_id == active_branch_state.doc_handle.document_id() && trigger_reload);
                                if !trigger_reload {
                                    tracing::debug!("TRIGGER saved changes: {}", active_branch_state.name);
                                    self.request_ingestion();
                                }
                            }
                        }
                    }
                }
                OutputEvent::Initialized { project_doc_id } => {
                    self.project_doc_id = Some(project_doc_id);
                }

                OutputEvent::CompletedCreateBranch { branch_doc_id } => {
					// PLEASE NOTE: If we change the logic such that we don't check out a new branch when we create one,
					// we need to change _create_branch to not populate the previous branch id
                    self.checked_out_branch_state =
                        CheckedOutBranchState::CheckingOut(branch_doc_id, self.get_previous_branch_id());
                }

                OutputEvent::PeerConnectionInfoChanged {
                    peer_connection_info,
                } => {
                    let _info = match self
                        .sync_server_connection_info
                        .as_mut()
                    {
                        None => {
                            self.sync_server_connection_info = Some(peer_connection_info.clone());
                            peer_connection_info
                        }

                        Some(sync_server_connection_info) => {
                            sync_server_connection_info.last_received =
                                peer_connection_info.last_received;
                            sync_server_connection_info.last_sent = peer_connection_info.last_sent;

                            peer_connection_info
                                .docs
                                .iter()
                                .for_each(|(doc_id, doc_state)| {
                                    let had_previously_heads = sync_server_connection_info
                                        .docs
                                        .get(doc_id)
                                        .is_some_and(|doc_state| {
                                            doc_state
                                                .clone()
                                                .last_acked_heads
                                                .is_some_and(|heads| heads.len() > 0)
                                        });

                                    // don't overwrite the doc state if it had previously had heads
                                    // but now doesn't have any heads
                                    if had_previously_heads
                                        && doc_state
                                            .clone()
                                            .last_acked_heads
                                            .is_some_and(|heads| heads.len() == 0)
                                    {
                                        return;
                                    }

                                    sync_server_connection_info
                                        .docs
                                        .insert(doc_id.clone(), doc_state.clone());
                                });

                            peer_connection_info
                        }
                    };
					self.request_ingestion();
                }
            }
        }

		if branches_changed {
			self.request_ingestion();
		}

		let has_pending_updates = self.just_checked_out_new_branch || self.should_update_godot;
		let fs_driver_has_pending_updates = self.file_system_driver.as_ref().map(|driver| driver.has_events_pending()).unwrap_or(false);
		if !has_pending_updates && !fs_driver_has_pending_updates {
			return (Vec::new(), signals);
		}
		if !Self::safe_to_update_godot(self.initial_load) {
			if has_pending_updates {
				tracing::info!("Pending changes, but not safe to update godot, skipping...");
			}
			if fs_driver_has_pending_updates {
				tracing::info!("Pending editor changes to sync, but not safe to update godot, skipping...");
			}
			return (Vec::new(), signals);
		}
		let (has_branch_state, previous_branch_info) = match &self.checked_out_branch_state{
			CheckedOutBranchState::NothingCheckedOut(_) => (false, None),
			CheckedOutBranchState::CheckingOut(_, _) => (false, None),
			CheckedOutBranchState::CheckedOut(_, prev_branch_info) => (true, prev_branch_info.clone()),
		};
		if !has_branch_state {
			if has_pending_updates {
				tracing::info!("Pending changes, but we're not checked out on a branch, skipping...");
			}
			if fs_driver_has_pending_updates {
				tracing::info!("Pending editor changes to sync, but we're not checked out on a branch, skipping...");
			}
			return (Vec::new(), signals);
		}
		let mut updates = Vec::new();
		if self.just_checked_out_new_branch {
			self.just_checked_out_new_branch = false;
			self.should_update_godot = false;
			self.initial_load = false;
			let (branch_name, checked_out_branch_doc_id) = self.get_checked_out_branch_state().map(|branch_state|
				(branch_state.name.clone(), branch_state.doc_handle.document_id().clone())
			).unwrap();
			tracing::debug!("just checked out branch {:?}", branch_name);

			let (previous_branch_id, previous_branch_heads) =
				if self.new_project {
					(None, Vec::new())
				} else if previous_branch_info.is_some() {
					let heads = self.branch_states.get(previous_branch_info.as_ref().unwrap()).map(|branch_state| branch_state.synced_heads.clone()).unwrap_or_default();
					(previous_branch_info, heads)
				} else if self.last_synced.is_some() && self.get_checked_out_branch_state().unwrap().merge_info.is_none() && self.last_synced.as_ref().map(|(doc_id, _)| doc_id) == Some(&checked_out_branch_doc_id){
					// TODO: this doesn't handle the case where we're starting up the editor and we're syncing the current doc state to the editor,
					// the last_synced heads will be empty.
					// We need to think about how to handle this case; if changes happened while outside of the editor, we want to sync everything.
					// setting the from branch id to None to ensure it doesn't just sync the current heads
					self.last_synced.as_ref().map(|(_doc_id, synced_heads)| (None, synced_heads.clone())).unwrap_or_default()
				} else {
					(None, Vec::new())
				};

			if self.new_project {
				self.new_project = false;
				self.sync_godot_to_patchwork(true);
			} else {
				// Sync from the previous branch @ synced_heads to the current branch @ synced_heads
				updates = self.sync_patchwork_to_godot(previous_branch_id, previous_branch_heads);
			}
			self.last_synced = self.get_checked_out_branch_state().map(|branch_state| (branch_state.doc_handle.document_id().clone(), branch_state.synced_heads.clone()));
			// NOTE: it is VERY important that we save the project config AFTER we sync,
			// because this will trigger a file scan and then resave the current project files in the editor
			PatchworkConfigAccessor::set_project_value("project_doc_id", &match &self.get_project_doc_id() {
				Some(doc_id) => doc_id.to_string(),
				None => "".to_string(),
			});
			PatchworkConfigAccessor::set_project_value("checked_out_branch_doc_id", &checked_out_branch_doc_id.to_string());
			signals.push(GodotProjectSignal::CheckedOutBranch);
			self.request_ingestion();
		} else if self.should_update_godot {
			self.initial_load = false;
			// * Sync from the current branch @ previously synced_heads to the current branch @ synced_heads
			tracing::debug!("should update godot");
			self.should_update_godot = false;
			let current_branch_id = self.get_checked_out_branch_state().unwrap().doc_handle.document_id().clone();
			let last_synced_heads = self.last_synced.as_ref().map(|(branch_id, synced_heads)|
				if branch_id == &current_branch_id {
					synced_heads.clone()
				} else {
					Vec::new()
				}
			).unwrap_or_default();
			updates = self.sync_patchwork_to_godot(Some(current_branch_id), last_synced_heads);
			self.last_synced = self.get_checked_out_branch_state().map(|branch_state| (branch_state.doc_handle.document_id().clone(), branch_state.synced_heads.clone()));
			self.request_ingestion();
		} else if let Some(fs_driver) = self.file_system_driver.as_mut() {
			let mut new_files = Vec::new();
			while let Some(event) = fs_driver.try_next() {
				match event {
					FileSystemEvent::FileCreated(path, content) => {
						new_files.push((path, content));
					}
					FileSystemEvent::FileModified(path, content) => {
						new_files.push((path, content));
					}
					FileSystemEvent::FileDeleted(path) => {
						new_files.push((path, FileContent::Deleted));
					}
				}
			}
			if new_files.len() > 0 {
				let files: Vec<(PathBuf, FileContent)> = new_files.into_iter().map(
					|(path, content)| {
						tracing::debug!("godot editor updated file: {:?}", path);
						(PathBuf::from(self.localize_path(&path.to_string_lossy().to_string()).to_string()), content)
					}
				).collect::<Vec<(PathBuf, FileContent)>>();

				// TODO: Ask Paul about this tomorrow
				self.sync_files_at(self.get_checked_out_branch_state().unwrap().doc_handle.clone(), files, None, None, false);
				self.request_ingestion();
			}
        }

		(updates, signals)
	}
}
