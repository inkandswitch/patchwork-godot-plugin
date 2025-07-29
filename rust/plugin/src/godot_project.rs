use crate::file_utils::{FileContent};
use godot::classes::editor_plugin::DockSlot;
use ::safer_ffi::prelude::*;
use automerge::{
    patches::TextRepresentation, ChangeHash, ObjType, ReadDoc,
    TextEncoding, ROOT,
};
use automerge::{Automerge, ObjId, Patch, PatchAction, Prop};
use automerge_repo::{DocHandle, DocumentId, PeerConnectionInfo};
use autosurgeon::{Hydrate, Reconcile};
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender};
use godot::classes::file_access::ModeFlags;
use godot::classes::resource_loader::CacheMode;
use godot::classes::{ConfirmationDialog, Control, Script};
use godot::classes::EditorInterface;
use godot::classes::ProjectSettings;
use godot::classes::ResourceLoader;
use godot::classes::{ClassDb, EditorPlugin, Engine, IEditorPlugin};
use godot::global::str_to_var;
use godot::classes::{ResourceUid, DirAccess, FileAccess};
use godot::prelude::*;
use godot::prelude::Dictionary;
use tracing::instrument;
use std::any::Any;
use std::collections::{HashSet};
use std::path::PathBuf;
use std::{collections::HashMap, str::FromStr};
use crate::godot_helpers::{get_resource_or_scene_path_for_object, ToGodotExt, ToVariantExt};
use crate::file_system_driver::{FileSystemDriver, FileSystemEvent, FileSystemUpdateEvent};
use crate::godot_parser::{self, GodotScene, TypeOrInstance};
use crate::godot_project_driver::{BranchState, ConnectionThreadError, DocHandleType};
use crate::patches::{get_changed_files_vec};
use crate::patchwork_config::PatchworkConfig;
use crate::utils::{are_valid_heads, array_to_heads, CommitInfo, ToShortForm};
use crate::{
    doc_utils::SimpleDocReader,
    godot_project_driver::{GodotProjectDriver, InputEvent, OutputEvent},
};
use similar::{ChangeTag, DiffOp, TextDiff};

#[derive(Debug, Clone, Reconcile, Hydrate, PartialEq)]
struct BinaryFile {
    content: Vec<u8>,
}

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
}

#[derive(Debug, Clone)]
enum CheckedOutBranchState {
    NothingCheckedOut(Option<DocumentId>),
    CheckingOut(DocumentId, Option<DocumentId>),
    CheckedOut(DocumentId, Option<DocumentId>),
}

enum VariantStrValue {
    Variant(String),
    ResourcePath(String),
    SubResourceID(String),
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

#[derive(Debug)]
pub struct GodotProjectImpl {
    doc_handles: HashMap<DocumentId, DocHandle>,
    branch_states: HashMap<DocumentId, BranchState>,
    checked_out_branch_state: CheckedOutBranchState,
    project_doc_id: Option<DocumentId>,
    new_project: bool,
	should_update_godot: bool,
	just_checked_out_new_branch: bool,
	last_synced: Option<(DocumentId, Vec<ChangeHash>)>,
    driver: Option<GodotProjectDriver>,
    driver_input_tx: UnboundedSender<InputEvent>,
    driver_output_rx: UnboundedReceiver<OutputEvent>,
    sync_server_connection_info: Option<PeerConnectionInfo>,
    file_system_driver: Option<FileSystemDriver>,
	project_dir: String,
	is_started: bool,
	initial_load: bool,
}

impl Default for GodotProjectImpl {
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
		}
	}
}


const DEFAULT_SERVER_URL: &str = "24.199.97.236:8080";

// This is the worst thing I've ever done
// Get the file system
// get the parent of the file system, that's the editor node
// look for the first Panel child of the editor node, that's the gui base
// look for ConfirmationDialog children of the gui base
// it's unique in that it has a vbox container with a tree child; just look for that
// if we find it, get the signals
// find the signals connected to the confirmed signal
// the first is the _reload_modified_scenes callable
// the second is the _reload_project_settings callable
// steal those, call _reload_modified_scenes
fn steal_editor_node_private_reload_methods_from_dialog_signal_handlers() -> Option<(Callable, Callable)> {
		// get the editor node
	let editor_file_system = EditorInterface::singleton().get_resource_filesystem();
	let editor_node = if let Some(editor_file_system) = editor_file_system {
		// get the parent of the editor file system, that's the editor node
		editor_file_system.get_parent()
	} else {
		return None;
	};
	if let Some(editor_node) = editor_node {
			// get the first Panel child of the editor node, that's the gui base
		let children = editor_node.get_children();
		// it should be the first panel
		if let Some(gui_base) = children.iter_shared().find(|c| c.get_class().to_string() == "Panel") {
			// find the disk_changed dialog child of the gui base
			let children = gui_base.get_children();
			if let Some(disk_changed_dialog_node) = children.iter_shared().find(|c|{
				if c.get_class().to_string() == "ConfirmationDialog" {
					// check that one of the children is a VBoxContainer
					let children = c.get_children();
					if let Some(vbox_container) = children.iter_shared().find(|c| c.get_class().to_string() == "VBoxContainer") {
						// check that one of the children is a Tree
						let children = vbox_container.get_children();
						if let Some(_) = children.iter_shared().find(|c| c.get_class().to_string() == "Tree") {
							return true;
						}
					}
				}
				false
			}) {
				let disk_changed_dialog = match disk_changed_dialog_node.try_cast::<ConfirmationDialog>() {
					Ok(dialog) => dialog,
					Err(_) => return None,
				};
				let signals = disk_changed_dialog.get_signal_connection_list("confirmed");
				if signals.len() >= 2 {
					// the first two should be the _reload_modified_scenes and _reload_project_settings signals
					let reload_modified_scenes_callable = signals.get(0).unwrap().get("callable").unwrap().to::<Callable>();
					let reload_project_settings_callable = signals.get(1).unwrap().get("callable").unwrap().to::<Callable>();
					return Some((reload_modified_scenes_callable, reload_project_settings_callable));
				} else {
					return None;
				}
			} else {
				return None;
			}
		} else {
			return None;
		}
	}

	None
}



// PatchworkEditor accessor functions
struct PatchworkEditorAccessor{
}

impl PatchworkEditorAccessor {
	fn import_and_load_resource(path: &str) -> Variant {
		ClassDb::singleton().class_call_static(
			"PatchworkEditor",
			"import_and_load_resource",
			&[path.to_variant()],
		)
	}

	fn is_editor_importing() -> bool {
		return ClassDb::singleton().class_call_static(
			"PatchworkEditor",
			"is_editor_importing",
			&[],
		).to::<bool>()
	}

	fn is_changing_scene() -> bool {
		return ClassDb::singleton().class_call_static(
			"PatchworkEditor",
			"is_changing_scene",
			&[],
		).to::<bool>()
	}

	fn reload_scripts(scripts: &Vec<String>) {
		ClassDb::singleton().class_call_static(
			"PatchworkEditor",
			"reload_scripts",
			&[scripts.to_variant()],
		);
	}

	fn force_refresh_editor_inspector() {
		ClassDb::singleton().class_call_static(
			"PatchworkEditor",
			"force_refresh_editor_inspector",
			&[],
		);
	}

	fn progress_add_task(task: &str, label: &str, steps: i32, can_cancel: bool) {
		ClassDb::singleton().class_call_static(
			"PatchworkEditor",
			"progress_add_task",
			&[task.to_variant(), label.to_variant(), steps.to_variant(), can_cancel.to_variant()],
		);
	}
	fn progress_task_step(task: &str, state: &str, step: i32, force_refresh: bool) {
		ClassDb::singleton().class_call_static(
			"PatchworkEditor",
			"progress_task_step",
			&[task.to_variant(), state.to_variant(), step.to_variant(), force_refresh.to_variant()],
		);
	}
	fn progress_end_task(task: &str) {
		ClassDb::singleton().class_call_static(
			"PatchworkEditor",
			"progress_end_task",
			&[task.to_variant()],
		);
	}
	fn unsaved_files_open() -> bool {
		ClassDb::singleton().class_call_static(
			"PatchworkEditor",
			"unsaved_files_open",
			&[],
		).to::<bool>()
	}

	fn clear_editor_selection() {
		ClassDb::singleton().class_call_static(
			"PatchworkEditor",
			"clear_editor_selection",
			&[],
		);
	}
}

struct EditorFilesystemAccessor{
}

impl EditorFilesystemAccessor {
	fn is_scanning() -> bool {
		EditorInterface::singleton().get_resource_filesystem().map(|fs| return fs.is_scanning()).unwrap_or(false)
	}

	fn reimport_files(files: &Vec<String>) {
		let files_packed = files.iter().map(|f| GString::from(f.clone())).collect::<PackedStringArray>();
		EditorInterface::singleton().get_resource_filesystem().unwrap().reimport_files(&files_packed);
	}

	fn reload_scene_from_path(path: &str) {
		EditorInterface::singleton().reload_scene_from_path(&GString::from(path));
	}

	fn scan() {
		EditorInterface::singleton().get_resource_filesystem().unwrap().scan();
	}

	fn get_inspector_edited_object() -> Option<Gd<Object>> {
		EditorInterface::singleton().get_inspector().unwrap().get_edited_object()
	}
}

struct PatchworkConfigAccessor{
}

impl PatchworkConfigAccessor {
	fn get_project_value(name: &str, default: &str) -> String {
		PatchworkConfig::singleton().bind().get_project_value(GString::from(name), default.to_variant()).to::<String>()
	}

	fn get_project_doc_id() -> String {
		PatchworkConfigAccessor::get_project_value("project_doc_id", "")
	}

	fn get_user_value(name: &str, default: &str) -> String {
		PatchworkConfig::singleton().bind().get_user_value(GString::from(name), default.to_variant()).to::<String>()
	}

	fn set_project_value(name: &str, value: &str) {
		PatchworkConfig::singleton().bind_mut().set_project_value(GString::from(name), value.to_variant());
	}

	fn set_user_value(name: &str, value: &str) {
		PatchworkConfig::singleton().bind_mut().set_user_value(GString::from(name), value.to_variant());
	}
}

enum GodotProjectSignal {
	Started,
	CheckedOutBranch,
	FilesChanged,
	SavedChanges,
	BranchesChanged,
	ShutdownCompleted,
	SyncServerConnectionInfoChanged(PeerConnectionInfo),
	ConnectionThreadFailed,
}

impl GodotProjectImpl {
	fn globalize_path(&self, path: &String) -> String {
		// trim the project_dir from the front of the path
		if path.starts_with("res://") {
			let thing = PathBuf::from(self.project_dir.clone()).join(PathBuf::from(&path["res://".len()..].to_string()));
			thing.to_string_lossy().to_string()
		} else {
			path.to_string()
		}
	}

	// TODO: We need to test this on Windows
	fn localize_path(&self, path: &String) -> String {
		if path.starts_with(&self.project_dir) {
			let thing = PathBuf::from("res://".to_string()).join(PathBuf::from(&path[self.project_dir.len()..].to_string()));
			thing.to_string_lossy().to_string()
		} else {
			path.to_string()
		}
	}


    fn _get_project_doc_id(&self) -> Option<DocumentId> {
		self.project_doc_id.clone()
	}


	fn _get_heads(&self) -> Vec<ChangeHash> {
		match self.get_checked_out_branch_state() {
            Some(branch_state) => branch_state
                .doc_handle
                .with_doc(|d| d.get_heads()),
				_ => Vec::new(),
		}
	}


    fn _get_files(&self) -> Vec<String> {
        let files = self._get_files_at(None, None);

        // let mut result = Dictionary::new();
		let mut result: Vec<String> = Vec::new();

        for (path, _) in files {
            let _ = result.push(path);
        }

        result
    }

	fn _get_changes(&self) -> Vec<CommitInfo> {
        match self.get_checked_out_branch_state() {
            Some(branch_state) => branch_state.doc_handle.with_doc(|d|
				d.get_changes(&[])
				.to_vec()
				.iter()
				.map(|c| {
					CommitInfo::from(c)
				})
				.collect::<Vec<CommitInfo>>()
			),
            _ => Vec::new(),
        }
    }

	fn _get_main_branch(&self) -> Option<&BranchState> {
		self
            .branch_states
            .values()
            .find(|branch_state| branch_state.is_main)
    }


	fn _get_branch_by_id(&self, branch_id: &String) -> Option<&BranchState> {
        match DocumentId::from_str(branch_id) {
            Ok(id) => self
                .branch_states
                .get(&id),
            Err(_) => None,
        }
    }

	fn _get_branch_name(&self, branch_id: &DocumentId) -> String {
		self.branch_states.get(branch_id).map(|b| b.name.clone()).unwrap_or(branch_id.to_string())
	}

	#[instrument(skip_all, level = tracing::Level::INFO)]
	fn _merge_branch(&mut self, source_branch_doc_id: DocumentId, target_branch_doc_id: DocumentId) {
		println!("");
		tracing::info!("******** MERGE BRANCH: {:?} into {:?}",
			self._get_branch_name(&source_branch_doc_id),
			self._get_branch_name(&target_branch_doc_id)
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


	#[instrument(skip(self), fields(name = ?name), level = tracing::Level::INFO)]
	fn _create_branch(&mut self, name: String) {
		println!("");
		tracing::info!("******** CREATE BRANCH");
		println!("");
        let source_branch_doc_id = match &self.get_checked_out_branch_state() {
            Some(branch_state) => branch_state.doc_handle.document_id(),
            None => {
                panic!("couldn't create branch, no checked out branch");
            }
        };

        self.driver_input_tx
            .unbounded_send(InputEvent::CreateBranch {
                name,
                source_branch_doc_id: source_branch_doc_id.clone(),
            })
            .unwrap();

		// TODO: do we want to set this? or let _process set it?
        self.checked_out_branch_state = CheckedOutBranchState::NothingCheckedOut(Some(source_branch_doc_id));
		// self.checked_out_branch_state = CheckedOutBranchState::NothingCheckedOut(None);
    }


	fn _create_merge_preview_branch(
		&mut self,
		source_branch_doc_id: DocumentId,
		target_branch_doc_id: DocumentId,
	) {
		println!("");
		tracing::info!("******** CREATE MERGE PREVIEW BRANCH: {:?} into {:?}",
			self._get_branch_name(&source_branch_doc_id),
			self._get_branch_name(&target_branch_doc_id)
		);
		println!("");

        self.driver_input_tx
            .unbounded_send(InputEvent::CreateMergePreviewBranch {
                source_branch_doc_id,
                target_branch_doc_id,
            })
            .unwrap();
    }


	fn _delete_branch(&mut self, branch_doc_id: DocumentId) {
        self.driver_input_tx
            .unbounded_send(InputEvent::DeleteBranch { branch_doc_id })
            .unwrap();
    }


	fn _checkout_branch(&mut self, branch_doc_id: DocumentId) {
		let current_branch = match &self.checked_out_branch_state {
			CheckedOutBranchState::CheckedOut(doc_id, _) => Some(doc_id.clone()),
			CheckedOutBranchState::CheckingOut(doc_id, _) => {
				tracing::error!("**@#%@#%!@#%#@!*** CHECKING OUT BRANCH WHILE STILL CHECKING OUT?!?!?! {:?}", doc_id);
				Some(doc_id.clone())
			},
			CheckedOutBranchState::NothingCheckedOut(current_branch_id) => {
				tracing::warn!("Checking out a branch while not checked out on any branch????");
				current_branch_id.clone()
			}
		};
        let target_branch_state = match self.branch_states.get(&branch_doc_id) {
            Some(branch_state) => branch_state,
            None => panic!("couldn't checkout branch, branch doc id not found")
        };
		println!("");
		tracing::debug!("******** CHECKOUT: {:?}\n", target_branch_state.name);
		println!("");

        if target_branch_state.synced_heads == target_branch_state.doc_handle.with_doc(|d| d.get_heads()) {
            self.checked_out_branch_state =
                CheckedOutBranchState::CheckedOut(
					branch_doc_id.clone(),
					current_branch.clone());
			self.just_checked_out_new_branch = true;
        } else {
			tracing::debug!("checked out branch {:?} has unsynced heads", target_branch_state.name);
            self.checked_out_branch_state =
				CheckedOutBranchState::CheckingOut(
					branch_doc_id.clone(),
					current_branch.clone()
				);
        }
    }


	fn _get_branches(&self) -> Vec<&BranchState> {
        let mut branches = self
            .branch_states
            .values()
            .filter(|branch_state| branch_state.merge_info.is_none())
            .collect::<Vec<&BranchState>>();

        branches.sort_by(|a, b| {
            let a_is_main = a.is_main;
            let b_is_main = b.is_main;

            if a_is_main && !b_is_main {
                return std::cmp::Ordering::Less;
            }
            if !a_is_main && b_is_main {
                return std::cmp::Ordering::Greater;
            }

            let name_a = a.name.clone().to_lowercase();
            let name_b = b.name.clone().to_lowercase();
            name_a.cmp(&name_b)
        });

        branches
    }


	fn _get_sync_server_connection_info(&self) -> Option<&PeerConnectionInfo> {
		self.sync_server_connection_info.as_ref()
	}


	fn _get_descendent_document(
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


	// INTERNAL FUNCTIONS
	/// Gets the current file content on the current branch @ the current synced heads that changed
	/// between the previous branch @ the previous heads and the current branch @ the current heads
	#[instrument(skip_all, level = tracing::Level::DEBUG)]
	fn _get_changed_file_content_between(
		&self,
		previous_branch_id: Option<DocumentId>,
		current_doc_id: DocumentId,
		previous_heads: Vec<ChangeHash>,
		current_heads: Vec<ChangeHash>,
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
			let files = self._get_files_on_branch_at(current_branch_state, Some(&curr_heads), None);
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
				self._get_descendent_document(previous_branch_id, current_doc_id.clone(), previous_heads.clone(), curr_heads.clone())
			}
		} else {
			Some(current_doc_id.clone())
		};
		if descendent_doc_id.is_none() {
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

			let previous_files = self._get_files_on_branch_at(previous_branch_state, Some(&previous_heads), None);
			let current_files = self._get_files_on_branch_at(current_branch_state, Some(&curr_heads), None);
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
				self._get_branch_name(&previous_branch_id)
			} else {
				self._get_branch_name(&current_doc_id)
			},
			previous_heads.to_short_form(),
			self._get_branch_name(&current_doc_id),
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
			let patches = d.diff(
				&previous_heads,
				&curr_heads,
				TextRepresentation::String(TextEncoding::Utf8CodeUnit),
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
			let linked_file_content: Option<FileContent> = self._get_linked_file(&doc_id);
			if let Some(file_content) = linked_file_content {
				changed_file_events.push(make_event(path, file_content));
			}
		}

		changed_file_events
    }


    fn _get_files_at(&self, heads: Option<&Vec<ChangeHash>>, filters: Option<&HashSet<String>>) -> HashMap<String, FileContent> {
		match &self.checked_out_branch_state {
			CheckedOutBranchState::CheckedOut(branch_doc_id, _) => {
				let branch_state = match self.branch_states.get(&branch_doc_id) {
					Some(branch_state) => branch_state,
					None => {
						tracing::error!("_get_files_at: branch doc id {:?} not found", branch_doc_id);
						return HashMap::new();
					},
				};
				self._get_files_on_branch_at(branch_state, heads, filters)
			}
			_ => panic!("_get_files_at: no checked out branch"),
		}
	}

	fn _get_linked_file(&self, doc_id: &DocumentId) -> Option<FileContent> {
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
	fn _get_files_on_branch_at(&self, branch_state: &BranchState, heads: Option<&Vec<ChangeHash>>, filters: Option<&HashSet<String>>) -> HashMap<String, FileContent> {

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
			let linked_file_content: Option<FileContent> = self._get_linked_file(&doc_id);
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
    fn _sync_files_at(&self,
                      branch_doc_handle: DocHandle,
                      files: Vec<(PathBuf, FileContent)>, /*  Record<String, Variant> */
                      heads: Option<Vec<ChangeHash>>)
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
        let stored_files = self._get_files_at(heads.as_ref(), Some(&filter));
		let files_len = files.len();
		let mut requires_resave = false;
        let changed_files: Vec<(String, FileContent)> = files.into_iter().filter_map(|(path, content)| {
            let path = path.to_string_lossy().to_string();
            let stored_content = stored_files.get(&path);
			if let FileContent::Scene(scene) = &content {
				if scene.requires_resave {
					requires_resave = true;
				}
			} else if let Some(stored_content) = stored_content {
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
		if requires_resave {
			tracing::debug!("updates require resave");
			// TODO: rethink this system; how do we handle resaves? SHOULD we even have nodes with IDs?
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


    fn _get_file_at(&self, path: String, heads: Option<Vec<ChangeHash>>) -> Option<FileContent> {
		let mut ret: Option<FileContent> = None;
		{
			let files = self._get_files_at(heads.as_ref(),Some(&HashSet::from_iter(vec![path.clone()])));
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

	fn get_checked_out_branch_state(&self) -> Option<&BranchState> {
        match &self.checked_out_branch_state {
            CheckedOutBranchState::CheckedOut(branch_doc_id, _) => {
				self.branch_states.get(&branch_doc_id)
            }
            _ => {
                tracing::info!(
                    "Tried to get checked out branch state when nothing is checked out"
                );
                None
            }
        }
    }


    fn _write_variant_to_file(&self, path: &String, variant: &Variant) {
        // mkdir -p everything
        let dir = PathBuf::from(path)
            .parent()
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        // do the mkdir
        // get the first part "e.g. res:// or user://"
        let root = path.split("//").nth(0).unwrap_or("").to_string() + "//";
        let dir_access = DirAccess::open(&root);
        if let Some(mut dir_access) = dir_access {
            let _ = dir_access.make_dir_recursive(&GString::from(dir));
        }

        let file = FileAccess::open(path, ModeFlags::WRITE);
        if let None = file {
            tracing::error!("error opening file: {}", path);
            return;
        }
        let mut file = file.unwrap();
        // if it's a packedbytearray, write the bytes
        if let Ok(packed_byte_array) = variant.try_to::<PackedByteArray>() {
            file.store_buffer(&packed_byte_array);
        } else if let Ok(string) = variant.try_to::<String>() {
            file.store_line(&GString::from(string));
        } else {
            tracing::error!("unsupported variant type!! {:?}", variant.type_id());
        }
        file.close();
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

    fn get_diff_dict(
        old_path: String,
        new_path: String,
        old_text: &String,
        new_text: &String,
    ) -> Dictionary {
        let diff = TextDiff::from_lines(old_text, new_text);
        let mut unified = diff.unified_diff();
        unified.header(old_path.as_str(), new_path.as_str());
        // The diff of a file is a list of hunks, each hunk is a list of lines
        // the diff viewer expects the following data, but in Dictionary form
        // struct DiffLine {
        //     int new_line_no;
        //     int old_line_no;
        //     String content;
        //     String status;
        // These are manipulated by the diff viewer, no need to include them
        //     String old_text;
        //     String new_text;
        // };

        // struct DiffHunk {
        //     int new_start;
        //     int old_start;
        //     int new_lines;
        //     int old_lines;
        //     List<DiffLine> diff_lines;
        // };

        // struct DiffFile {
        //     String new_file;
        //     String old_file;
        //     List<DiffHunk> diff_hunks;
        // };

        fn get_range(ops: &[DiffOp]) -> (usize, usize, usize, usize) {
            let first = ops[0];
            let last = ops[ops.len() - 1];
            let old_start = first.old_range().start;
            let new_start = first.new_range().start;
            let old_end = last.old_range().end;
            let new_end = last.new_range().end;
            (
                old_start + 1,
                new_start + 1,
                old_end - old_start,
                new_end - new_start,
            )
        }
        let mut diff_file = Dictionary::new();
        let _ = diff_file.insert("new_file", new_path);
        let _ = diff_file.insert("old_file", old_path);
        let mut diff_hunks = Array::new();
        for (i, hunk) in unified.iter_hunks().enumerate() {
            let mut diff_hunk = Dictionary::new();
            let header = hunk.header();
            let (old_start, new_start, old_lines, new_lines) = get_range(&hunk.ops());
            let _ = diff_hunk.insert("old_start", old_start as i64);
            let _ = diff_hunk.insert("new_start", new_start as i64);
            let _ = diff_hunk.insert("old_lines", old_lines as i64);
            let _ = diff_hunk.insert("new_lines", new_lines as i64);
            let mut diff_lines = Array::new();
            for (idx, change) in hunk.iter_changes().enumerate() {
                let mut diff_line = Dictionary::new();
                // get the tag
                let status = match change.tag() {
                    ChangeTag::Equal => " ",
                    ChangeTag::Delete => "-",
                    ChangeTag::Insert => "+",
                };
                if let Some(old_index) = change.old_index() {
                    let _ = diff_line.insert("old_line_no", old_index as i64 + 1);
                } else {
                    let _ = diff_line.insert("old_line_no", -1);
                }
                if let Some(new_index) = change.new_index() {
                    let _ = diff_line.insert("new_line_no", new_index as i64 + 1);
                } else {
                    let _ = diff_line.insert("new_line_no", -1);
                }
                let content = change.as_str().unwrap();
                let _ = diff_line.insert("content", content);
                let _ = diff_line.insert("status", status);
                diff_lines.push(&diff_line);
            }
            let _ = diff_hunk.insert("diff_lines", diff_lines);
            diff_hunks.push(&diff_hunk);
        }
        let _ = diff_file.insert("diff_hunks", diff_hunks);
        diff_file
    }

    fn _get_resource_at(
        &self,
        path: String,
        content: &FileContent,
        heads: Vec<ChangeHash>,
    ) -> Option<Variant> {
        let temp_dir = format!(
            "res://.patchwork/temp_{}/",
            heads.first().to_short_form()
        );
        let temp_path = path.replace("res://", &temp_dir);
        // append _old or _new to the temp path (i.e. res://thing.<EXT> -> user://temp_123_456/thing_old.<EXT>)
        let _ = FileContent::write_file_content(&PathBuf::from(self.globalize_path(&temp_path)), content);
        // get the import file content
        let import_path = format!("{}.import", path);
        let import_file_content = self._get_file_at(import_path.clone(), Some(heads.clone()));
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


    fn _get_resource_diff(
        &self,
        path: &String,
        change_type: &str,
        old_content: Option<&FileContent>,
        new_content: Option<&FileContent>,
        old_heads: &Vec<ChangeHash>,
        curr_heads: &Vec<ChangeHash>,
    ) -> Dictionary {
        let mut result = dict! {
            "path" : path.to_variant(),
            "diff_type" : "resource_changed".to_variant(),
            "change_type" : change_type.to_variant(),
            "old_content" : old_content.unwrap_or(&FileContent::Deleted).to_variant(),
            "new_content" : new_content.unwrap_or(&FileContent::Deleted).to_variant(),
        };
        if let Some(old_content) = old_content {
            if let Some(old_resource) =
                self._get_resource_at(path.clone(), old_content, old_heads.clone())
            {
                let _ = result.insert("old_resource", old_resource);
            }
        }
        if let Some(new_content) = new_content {
            if let Some(new_resource) =
                self._get_resource_at(path.clone(), new_content, curr_heads.clone())
            {
                let _ = result.insert("new_resource", new_resource);
            }
        }
        result
    }

    fn _get_text_file_diff(
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
        let diff = Self::get_diff_dict(path.clone(), path.clone(), old_text, new_text);
        let result = dict! {
            "path" : path.to_variant(),
            "change_type" : change_type.to_variant(),
            "old_content" : old_content.unwrap_or(&FileContent::Deleted).to_variant(),
            "new_content" : new_content.unwrap_or(&FileContent::Deleted).to_variant(),
            "text_diff" : diff,
            "diff_type" : "text_changed".to_variant(),
        };
        result
    }

    fn _get_non_scene_diff(
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
            return dict! {
                "path" : path.to_variant(),
                "diff_type" : "file_unchanged".to_variant(),
                "change_type" : change_type.to_variant(),
                "old_content": old_content.unwrap_or(&FileContent::Deleted).to_variant(),
                "new_content": new_content.unwrap_or(&FileContent::Deleted).to_variant(),
            };
        }
        if old_content_type != VariantType::STRING && new_content_type != VariantType::STRING {
            return self._get_resource_diff(
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
            return self._get_text_file_diff(&path, &change_type, old_content, new_content);
        } else {
            return dict! {
                "path" : path.to_variant(),
                "diff_type" : "file_changed".to_variant(),
                "change_type" : change_type.to_variant(),
                "old_content" : old_content.unwrap_or(&FileContent::Deleted).to_variant(),
                "new_content" : new_content.unwrap_or(&FileContent::Deleted).to_variant(),
            };
        }
    }

	#[instrument(skip_all, level = tracing::Level::DEBUG)]
    fn _get_changes_between(
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
            d.diff(
                &old_heads,
                &curr_heads,
                TextRepresentation::String(TextEncoding::Utf8CodeUnit),
            )
        });
        let mut changed_files_map = HashMap::new();
        let mut scene_files = Vec::new();

        let mut all_diff: HashMap<String, Dictionary> = HashMap::new();
        // Get old and new content
		let new_file_contents = self._get_changed_file_content_between(None, checked_out_branch_state.doc_handle.document_id().clone(), old_heads.clone(), curr_heads.clone());
		let changed_files_set: HashSet<String> = new_file_contents.iter().map(|event|
			match event {
				FileSystemEvent::FileCreated(path, _) => path.to_string_lossy().to_string(),
				FileSystemEvent::FileModified(path, _) => path.to_string_lossy().to_string(),
				FileSystemEvent::FileDeleted(path) => path.to_string_lossy().to_string(),
			}
		).collect::<HashSet<String>>();
		let old_file_contents = self._get_files_on_branch_at(&checked_out_branch_state, Some(&old_heads), Some(&changed_files_set));

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
                // if both the old and new one are binary, or if one is none and the other is binary, then we can use the resource diff
                let _ = all_diff.insert(
                    path.clone(),
                    self._get_non_scene_diff(
                        &path,
                        &change_type,
                        Some(old_file_content),
                        Some(new_file_content),
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
                    godot_parser::GodotScene::hydrate_at(d, &path, &old_heads)
                }) {
                Ok(scene) => Some(scene),
                Err(_) => None,
            };

            let new_scene = match checked_out_branch_state
                .doc_handle
                .with_doc(|d: &Automerge| {
                    godot_parser::GodotScene::hydrate_at(d, &path, &curr_heads)
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
            let mut all_changed_ext_resource_paths: HashSet<String> = HashSet::new();
            let mut added_ext_resources: HashSet<String> = HashSet::new();
            let mut deleted_ext_resources: HashSet<String> = HashSet::new();

            let mut changed_sub_resources: HashSet<String> = HashSet::new();
            let mut added_sub_resources: HashSet<String> = HashSet::new();
            let mut deleted_sub_resources: HashSet<String> = HashSet::new();
            let mut all_changed_sub_resource_ids: HashSet<String> = HashSet::new();

            let mut changed_node_ids: HashSet<String> = HashSet::new();

            for patch in patches.iter() {
                match_path(&patch_path, &patch).inspect(
                    |PathWithAction { path, action }| match path.first() {
                        Some((_, Prop::Map(node_id))) => {
                            // hack: only consider nodes where properties changed as changed
                            // this filters out all the parent nodes that don't really change only the child_node_ids change
                            // get second to last instead of last
                            if path.len() > 2 {
                                if let Some((_, Prop::Map(key))) = path.get(path.len() - 2) {
                                    if key == "properties" {
                                        changed_node_ids.insert(node_id.clone());
                                        return;
                                    }
                                }
                            }
                            if let Some((_, Prop::Map(key))) = path.last() {
                                if key != "child_node_ids" {
                                    changed_node_ids.insert(node_id.clone());
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
                    |PathWithAction { path, action }| match path.first() {
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
                                return format!("Resource({})", ext_resource.path);
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
                let mut path: Option<String> = None;
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
                        let resource_content = self._get_file_at(
                            path.clone(),
                            if _is_old {
                                Some(old_heads.clone())
                            } else {
                                Some(curr_heads.clone())
                            },
                        );
                        if let Some(resource_content) = resource_content {
                            let resource = self._get_resource_at(
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
					if old_value.is_some() && new_value.is_some() {
                    return dict! {
                        "name": prop.clone(),
                        "change_type": "modified",
                        "old_value": fn_get_prop_value(old_value.unwrap(), &old_scene, true),
                        "new_value": fn_get_prop_value(new_value.unwrap(), &new_scene, false)
                    };

				} else if old_value.is_some() {
					return dict! {
						"name": prop.clone(),
						"change_type": "deleted",
						"old_value": fn_get_prop_value(old_value.unwrap(), &old_scene, true)
					};
				} else if new_value.is_some() {
					return dict! {
						"name": prop.clone(),
						"change_type": "added",
						"new_value": fn_get_prop_value(new_value.unwrap(), &new_scene, false)
					};
				}
				return dict!{};
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
                        let _ = node_info.insert("node_path", scene.get_node_path(&node_id));
                        if let Some(node) = scene.nodes.get(&node_id.clone()) {
							let tp = fn_get_class_name(&node.type_or_instance, &new_scene);
							let _ = node_info.insert("type", tp);
							let mut changed_props = Dictionary::new();
							for (key, value) in node.properties.iter() {
								let val = value.get_value();
								if added {
									let changed_prop = get_changed_prop_dict(key.to_string(), None, Some(self.get_varstr_value(val)));
									_ = changed_props.insert(key.clone(), changed_prop);
								} else {
									let changed_prop = get_changed_prop_dict(key.to_string(), Some(self.get_varstr_value(val)), None);
									_ = changed_props.insert(key.clone(), changed_prop);
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
                        let _ = node_info.insert("node_path", scene.get_node_path(node_id));
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
                        if let Some(content) = old_scene.get_node_content(node_id) {
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
                        if let Some(content) = new_scene.get_node_content(node_id) {
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
							let mut changed_prop: Option<Dictionary> = None;
							{
								let prop = prop.clone();
								let old_prop = if let Some(old_prop) = old_props.get(prop.as_str()) {
									Some(old_prop.to_string())
								} else {
									None
								};
								let new_prop = if let Some(new_prop) = new_props.get(prop.as_str()) {
									Some(new_prop.to_string())
								} else {
									None
								};

								let sn_2: StringName = StringName::from(&prop);
								let default_value = if let TypeOrInstance::Type(class_name) = &new_type {
									ClassDb::singleton()
										.class_get_property_default_value(&StringName::from(class_name), &sn_2)
										.to_string()
								} else {
									"".to_string() // Instance properties are always set, regardless of the default value, so this is always empty
								};
								let old_prop = old_prop.unwrap_or(default_value.clone());
								let new_prop = new_prop.unwrap_or(default_value.clone());
								let old_value = self.get_varstr_value(old_prop.clone());
								let new_value: VariantStrValue = self.get_varstr_value(new_prop.clone());
								match (&old_value, &new_value) {
									(
										VariantStrValue::SubResourceID(sub_resource_id),
										VariantStrValue::SubResourceID(new_sub_resource_id),
									) => {
										if all_changed_sub_resource_ids.contains(sub_resource_id)
											|| all_changed_sub_resource_ids.contains(new_sub_resource_id)
										{
											changed_prop = Some(get_changed_prop_dict(prop, Some(old_value), Some(new_value)));
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
											changed_prop = Some(get_changed_prop_dict(prop, Some(old_value), Some(new_value)));
										}
									}
									(
										VariantStrValue::ResourcePath(resource_path),
										VariantStrValue::ResourcePath(new_resource_path),
									) => {
										if all_changed_ext_resource_paths.contains(resource_path)
											|| all_changed_ext_resource_paths.contains(new_resource_path)
										{
											changed_prop = Some(get_changed_prop_dict(prop, Some(old_value), Some(new_value)));
										} else if resource_path != new_resource_path {
											changed_prop = Some(get_changed_prop_dict(prop, Some(old_value), Some(new_value)));
										}
									}
									(
										VariantStrValue::Variant(old_variant),
										VariantStrValue::Variant(new_variant),
									) => {
										if old_variant != new_variant {
											changed_prop = Some(get_changed_prop_dict(prop, Some(old_value), Some(new_value)));
										}
									}
									_ => {
										// changed type
										changed_prop = Some(get_changed_prop_dict(prop, Some(old_value), Some(new_value)));
									}
								}
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

    fn _start_driver(&mut self) {
        if self.driver.is_some() {
            return;
        }
        let (driver_input_tx, driver_input_rx) = futures::channel::mpsc::unbounded();
        let (driver_output_tx, driver_output_rx) = futures::channel::mpsc::unbounded();
        self.driver_input_tx = driver_input_tx;
        self.driver_output_rx = driver_output_rx;

        let storage_folder_path = self.globalize_path(&"res://.patchwork".to_string());
		let server_url = PatchworkConfigAccessor::get_project_value("server_url", DEFAULT_SERVER_URL);
        let mut driver: GodotProjectDriver = GodotProjectDriver::create(storage_folder_path, server_url);
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

    fn _start_file_system_driver(&mut self) {
        let project_path: String = self.globalize_path(&"res://".to_string());
        let project_path = PathBuf::from(project_path);

		// read in .gitignore from the project path
		let gitignore_path = project_path.join(".gitignore");




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

    fn start(&mut self) {
        let project_doc_id: String = PatchworkConfigAccessor::get_project_value("project_doc_id", "");
        let checked_out_branch_doc_id = PatchworkConfigAccessor::get_project_value("checked_out_branch_doc_id", "");
        tracing::info!("Starting GodotProject with project doc id: {:?}", if project_doc_id == "" { "<NEW DOC>" } else { &project_doc_id });
		self.should_update_godot = false;
		self.just_checked_out_new_branch = false;
		self.last_synced = None;
        self.project_doc_id = match DocumentId::from_str(&project_doc_id) {
            Ok(doc_id) => Some(doc_id),
            Err(e) => None,
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

        self._start_driver();
        self._start_file_system_driver();
        self.is_started = true;
        // get the project path
    }

    fn _stop_driver(&mut self) {
        if let Some(mut driver) = self.driver.take() {
            driver.teardown();
        }
    }

    fn stop(&mut self) {
		if !self.is_started {
			return;
		}
        self._stop_driver();
		if let Some(mut driver) = self.file_system_driver.take() {
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

	fn safe_to_update_godot(initial_load: bool) -> bool {
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
				format!("{} @ ", self._get_branch_name(from_branch_id.as_ref().unwrap()))
			} else {
				"".to_string()
			}, previous_heads.to_short_form(), current_heads.to_short_form());
		let events = self._get_changed_file_content_between(from_branch_id, current_doc_id.clone(), previous_heads, current_heads);
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
						let mut driver_updates: Vec<FileSystemUpdateEvent> = Vec::new();
						let before_size: usize = files.len();
						files = files.into_iter().filter_map(
						|(path, content)|{
							if let FileContent::Scene(content) = content {
								if content.requires_resave {
									driver_updates.push(FileSystemUpdateEvent::FileSaved(PathBuf::from(self.globalize_path(&path)), FileContent::Scene(content)));
									return None;
								}
								return Some((path, FileContent::Scene(content)));
							}
							Some((path, content))
						}
						).collect::<Vec<_>>();
						let events: Vec<FileSystemEvent> = driver.batch_update_blocking(driver_updates);
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
					self._sync_files_at(
						branch_state.doc_handle.clone(),
						files.into_iter().map(|(path, content)| (PathBuf::from(path), content)).collect::<Vec<(PathBuf, FileContent)>>(),
						Some(branch_state.synced_heads.clone()));
                }
            }
            None => panic!("couldn't save files, no checked out branch"),
        };
    }

	fn _get_previous_branch_id(&self) -> Option<DocumentId> {
		match &self.checked_out_branch_state {
			CheckedOutBranchState::NothingCheckedOut(prev_branch_id) => prev_branch_id.clone(),
			CheckedOutBranchState::CheckingOut(_, prev_branch_id) => prev_branch_id.clone(),
			CheckedOutBranchState::CheckedOut(_, prev_branch_id) => prev_branch_id.clone(),
		}
	}

	fn new(project_dir: String) -> Self {
		Self {
			project_dir,
			..Default::default()
		}
	}

	#[instrument(target = "patchwork_rust_core::godot_project::inner_process", level = tracing::Level::DEBUG, skip_all)]
	fn _process(&mut self, _delta: f64) -> (Vec<FileSystemEvent>, Vec<GodotProjectSignal>) {
		let mut signals: Vec<GodotProjectSignal> = Vec::new();

		if let Some(driver) = &mut self.driver {
			if let Some(error) = driver.connection_thread_get_last_error() {
				match error {
					ConnectionThreadError::ConnectionThreadDied(error) => {
						tracing::error!("automerge repo driver connection thread died, respawning: {}", error);
						if !driver.respawn_connection_thread() {
							tracing::error!("automerge repo driver connection thread failed too many times, aborting");
							// TODO: make the GUI do something with this
							signals.push(GodotProjectSignal::ConnectionThreadFailed);
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
                                    signals.push(GodotProjectSignal::SavedChanges);
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
                        CheckedOutBranchState::CheckingOut(branch_doc_id, self._get_previous_branch_id());
                }

                OutputEvent::CompletedShutdown => {
                    tracing::debug!("CompletedShutdown event");
                    signals.push(GodotProjectSignal::ShutdownCompleted);
                }

                OutputEvent::PeerConnectionInfoChanged {
                    peer_connection_info,
                } => {
                    let new_sync_server_connection_info = match self
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

                    signals.push(GodotProjectSignal::SyncServerConnectionInfoChanged(new_sync_server_connection_info));
                }
            }
        }

		if branches_changed {
			signals.push(GodotProjectSignal::BranchesChanged);
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
			PatchworkConfigAccessor::set_project_value("project_doc_id", &match &self._get_project_doc_id() {
				Some(doc_id) => doc_id.to_string(),
				None => "".to_string(),
			});
			PatchworkConfigAccessor::set_project_value("checked_out_branch_doc_id", &checked_out_branch_doc_id.to_string());
			signals.push(GodotProjectSignal::CheckedOutBranch);
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
				self._sync_files_at(self.get_checked_out_branch_state().unwrap().doc_handle.clone(), files, None);
			}
        }

		(updates, signals)
	}

	fn is_started(&self) -> bool {
		self.is_started
	}
}


const MODAL_TASK_NAME: &str = "Reloading scene";
#[derive(Debug, Default)]
pub struct PendingEditorUpdate {
	added_files: HashSet<String>,
	deleted_files: HashSet<String>,
	scripts_to_reload: HashSet<String>,
	scenes_to_reload: HashMap<String, FileContent>,
	reimport_files: HashSet<String>,
	uids_to_add: HashMap<String, String>,
	reload_project_settings: bool,
	inspector_refresh_queue_time: u128,
	changing_scene_cooldown: i64,
	modal_shown: bool,
}

impl PendingEditorUpdate {
	fn merge(&mut self, other: PendingEditorUpdate) {
		self.added_files.extend(other.added_files);
		self.deleted_files.extend(other.deleted_files);
		self.scripts_to_reload.extend(other.scripts_to_reload);
		for (path, content) in other.scenes_to_reload.into_iter() {
			self.scenes_to_reload.insert(path, content);
		}
		self.reimport_files.extend(other.reimport_files);
		for (path, uid) in other.uids_to_add.into_iter() {
			self.uids_to_add.insert(path, uid);
		}
		self.reload_project_settings = self.reload_project_settings || other.reload_project_settings;
	}
	fn added_or_deleted_files(&self) -> bool {
		self.added_files.len() > 0 || self.deleted_files.len() > 0
	}
	fn any_changes(&self) -> bool {
		self.any_file_changes() || self.has_inspector_refresh_queued() || self.modal_shown
	}

	fn any_file_changes(&self) -> bool {
		self.scripts_to_reload.len() > 0 || self.scenes_to_reload.len() > 0 || self.reimport_files.len() > 0 || self.uids_to_add.len() > 0 || self.added_or_deleted_files()
	}

	fn has_inspector_refresh_queued(&self) -> bool {
		self.inspector_refresh_queue_time > 0
	}

	fn queue_inspector_dock_refresh(&mut self) {
		// don't use Godot classes for this
		self.inspector_refresh_queue_time = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis();
	}


	fn refresh_inspector_dock(&mut self) {
		let current_time = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis();
		if current_time - self.inspector_refresh_queue_time < 500 {
			return;
		}
		PatchworkEditorAccessor::force_refresh_editor_inspector();
		self.inspector_refresh_queue_time = 0;
	}
}

#[derive(GodotClass, Debug)]
#[class(base=Node)]
pub struct GodotProject {
	base: Base<Node>,
	project: GodotProjectImpl,
	pending_editor_update: PendingEditorUpdate,
	reload_modified_scenes_callable: Option<Callable>,
	reload_project_settings_callable: Option<Callable>,
}



// macro for handling when the project is not started
macro_rules! check_project_started {
	($self:ident) => {
		if !$self.project.is_started() {
			tracing::error!("GodotProject is not started, skipping...");
			// return the default value for the type
			return;
		}
	};
}

macro_rules! check_project_started_and_return_default {
	($self:ident, $default:expr) => {
		if !$self.project.is_started() {
			tracing::error!("GodotProject is not started, returning default value");
			return $default;
		}
	};
}

#[godot_api]
impl GodotProject {
	#[signal]
	fn started();

	#[signal]
	fn checked_out_branch(branch: Dictionary);

	#[signal]
	fn files_changed();

	#[signal]
	fn saved_changes();

	#[signal]
	fn branches_changed(branches: Array<Dictionary>);

	#[signal]
	fn shutdown_completed();

	#[signal]
	fn sync_server_connection_info_changed(peer_connection_info: Dictionary);

	#[signal]
	fn connection_thread_failed();


	// PUBLIC API

	#[func]
	fn set_user_name(&self, name: String) {
		check_project_started!(self);
		self.project.driver_input_tx
			.unbounded_send(InputEvent::SetUserName { name })
			.unwrap();
	}

	#[func]
	fn shutdown(&self) {
		check_project_started!(self);
		self.project.driver_input_tx
			.unbounded_send(InputEvent::StartShutdown)
			.unwrap();
	}

	#[func]
	fn get_project_doc_id(&self) -> Variant {
		check_project_started_and_return_default!(self, Variant::nil());
		self.project._get_project_doc_id().to_variant()
	}

	#[func]
	fn get_heads(&self) -> PackedStringArray /* String[] */ {
		check_project_started_and_return_default!(self, PackedStringArray::new());
		self.project._get_heads().to_godot()
	}


	#[func]
	fn get_files(&self) -> PackedStringArray {
		check_project_started_and_return_default!(self, PackedStringArray::new());
		self.project._get_files().to_godot()
	}

    #[func]
    pub fn get_singleton() -> Gd<Self> {
        Engine::singleton()
            .get_singleton(&StringName::from("GodotProject"))
            .unwrap()
            .cast::<Self>()
    }

    #[func]
    fn get_changes(&self) -> Array<Dictionary> /* String[]  */ {
		check_project_started_and_return_default!(self, Array::new());
		let changes = self.project._get_changes();
		changes.iter().map(|c| c.to_godot()).collect::<Array<Dictionary>>()
	}

    #[func]
    fn get_main_branch(&self) -> Variant /* Branch? */ {
		check_project_started_and_return_default!(self, Variant::nil());
		self.project._get_main_branch().to_variant()
	}

    #[func]
    fn get_branch_by_id(&self, branch_id: String) -> Variant /* Branch? */ {
		check_project_started_and_return_default!(self, Variant::nil());
		self.project._get_branch_by_id(&branch_id).to_variant()
	}
    #[func]
    fn merge_branch(&mut self, source_branch_doc_id: String, target_branch_doc_id: String) {
		check_project_started!(self);
		self.project._merge_branch(DocumentId::from_str(&source_branch_doc_id).unwrap(), DocumentId::from_str(&target_branch_doc_id).unwrap());
	}

    #[func]
    fn create_branch(&mut self, name: String) {
		check_project_started!(self);
		self.project._create_branch(name);
	}
    #[func]
    fn create_merge_preview_branch(
        &mut self,
        source_branch_doc_id: String,
        target_branch_doc_id: String,
    ) {
		check_project_started!(self);
		let source_branch_doc_id = DocumentId::from_str(&source_branch_doc_id).unwrap();
        let target_branch_doc_id = DocumentId::from_str(&target_branch_doc_id).unwrap();
		self.project._create_merge_preview_branch(source_branch_doc_id, target_branch_doc_id);
	}
    #[func]
    fn delete_branch(&mut self, branch_doc_id: String) {
		check_project_started!(self);
		self.project._delete_branch(DocumentId::from_str(&branch_doc_id).unwrap());
	}
    #[func]
    fn checkout_branch(&mut self, branch_doc_id: String) {
		check_project_started!(self);
		self.project._checkout_branch(DocumentId::from_str(&branch_doc_id).unwrap());
	}
    // filters out merge preview branches
    #[func]
    fn get_branches(&self) -> Array<Dictionary> /* { name: String, id: String }[] */ {
		check_project_started_and_return_default!(self, Array::new());
		self.project._get_branches().iter().map(|b| b.to_godot()).collect::<Array<Dictionary>>()
	}
    #[func]
    fn get_checked_out_branch(&self) -> Variant /* {name: String, id: String, is_main: bool}? */ {
		check_project_started_and_return_default!(self, Variant::nil());
		self.project.get_checked_out_branch_state().map(|b|b.to_godot().to_variant()).unwrap_or_default()
	}

    #[func]
    fn get_sync_server_connection_info(&self) -> Variant {
		check_project_started_and_return_default!(self, Variant::nil());
        match self.project._get_sync_server_connection_info() {
            Some(peer_connection_info) => {
                peer_connection_info.to_variant()
            }
            None => Variant::nil(),
        }
    }

    #[func]
    fn get_all_changes_between(
        &self,
        old_heads: PackedStringArray,
        curr_heads: PackedStringArray,
    ) -> Dictionary {
		check_project_started_and_return_default!(self, Dictionary::new());
		if !are_valid_heads(&old_heads) || !are_valid_heads(&curr_heads) {
			tracing::error!("invalid heads: {:?}, {:?}", old_heads, curr_heads);
			return Dictionary::new();
		}
        let old_heads = array_to_heads(old_heads);
        let new_heads = array_to_heads(curr_heads);
        self.project._get_changes_between(old_heads, new_heads)
    }

	fn add_new_uid(path: &str, uid: &str) {
        let id = ResourceUid::singleton().text_to_id(uid);
        if id == ResourceUid::INVALID_ID as i64 {
            return;
        }
		let path = GString::from(path);
        if !ResourceUid::singleton().has_id(id) {
            ResourceUid::singleton().add_id(id, &path);
        } else if ResourceUid::singleton().get_id_path(id) != path {
            ResourceUid::singleton().set_id(id, &path);
        }
    }

	fn process_godot_updates(&self, events: Vec<FileSystemEvent>) -> PendingEditorUpdate {
		let mut pending_editor_update = PendingEditorUpdate::default();
		let mut files_changed = Vec::new();
        for event in events {
			let mut file_created = false;
            let (abs_path, content) = match event {
                FileSystemEvent::FileCreated(path, content) => {
					pending_editor_update.added_files.insert(self.project.localize_path(&path.to_string_lossy().to_string()));
					file_created = true;
					(path, content)
				},
                FileSystemEvent::FileModified(path, content) => (path, content),
                FileSystemEvent::FileDeleted(path) => {
					pending_editor_update.deleted_files.insert(self.project.localize_path(&path.to_string_lossy().to_string()));
					continue;
				},
            };
			files_changed.push(abs_path.to_string_lossy().to_string());
            let res_path = self.project.localize_path(&abs_path.to_string_lossy().to_string());
            let extension = abs_path.extension().unwrap_or_default().to_string_lossy().to_string().to_ascii_lowercase();
            if extension == "gd" {
				pending_editor_update.scripts_to_reload.insert(res_path);
            } else if extension == "tscn" {
                pending_editor_update.scenes_to_reload.insert(res_path, content);
            } else if extension == "import" {
				let mut pb = PathBuf::from(res_path);
				pb.set_extension("");
				let base = pb.to_string_lossy().to_string();
				if !file_created {
					pending_editor_update.reimport_files.insert(base.clone());
				}
                if let FileContent::String(string) = content {
                    // go line by line, find the line that begins with "uid="
                    for line in string.lines() {
                        if line.starts_with("uid=") {
                            let uid = line.split("=").nth(1).unwrap_or_default().to_string();
                            pending_editor_update.uids_to_add.insert(base, uid);
                            break;
                        }
                    }
                }
            } else if extension == "uid" {
                if let FileContent::String(string) = content {
                    pending_editor_update.uids_to_add.insert(res_path.to_string(), string);
                }
			} else if extension == "godot" {
				pending_editor_update.reload_project_settings = true;
            // check if a file with .import added exists
            } else  {
                let mut import_path = abs_path.clone();
				import_path.set_extension(abs_path.extension().unwrap_or_default().to_string_lossy().to_string() + ".import");
                if import_path.exists() {
					if !file_created {
						pending_editor_update.reimport_files.insert(res_path.to_string());
					}
                }
            }
        }
		tracing::info!("---------- files_changed: {:?}", files_changed);
		return pending_editor_update;
	}

	fn reload_modified_scenes(&self) -> bool {
		if PatchworkEditorAccessor::is_changing_scene() {
			return false;
		}
		if let Some(reload_modified_scenes_callable) = &self.reload_modified_scenes_callable {
			reload_modified_scenes_callable.call(&[]);
			return true;
		}
		false
	}

	fn reload_project_settings(&self) {
		if let Some(reload_project_settings_callable) = &self.reload_project_settings_callable {
			reload_project_settings_callable.call(&[]);
		}
	}

	fn update_godot_after_sync(&mut self) {
		if !self.pending_editor_update.any_changes() {
			return;
		}
		if !GodotProjectImpl::safe_to_update_godot(false) {
			return;
		}
		if !self.pending_editor_update.any_file_changes() {
			// refresh the editor inspector AFTER all the file changes have been applied
			if self.pending_editor_update.has_inspector_refresh_queued() {
				self.pending_editor_update.refresh_inspector_dock();
			}
			// if self.pending_editor_update.modal_shown {
			// 	PatchworkEditorAccessor::progress_end_task(MODAL_TASK_NAME);
			// }
			return;
		}


		let obj = EditorFilesystemAccessor::get_inspector_edited_object();
		let inspector_dock_needs_refresh = if let Some(obj) = obj {
			let obj_path = get_resource_or_scene_path_for_object(&obj);
			if obj_path == "" {
				false
			} else if self.pending_editor_update.scenes_to_reload.contains_key(&obj_path) {
				true
			} else if self.pending_editor_update.scripts_to_reload.contains(&obj_path) {
				true
			} else if self.pending_editor_update.reimport_files.contains(&obj_path) {
				true
			} else {
				// get the script from the object
				let var = obj.get_script();
				let res_obj: Result<Gd<Object>, _> = var.try_to::<Gd<Object>>();
				if let Ok(o) = res_obj {
					if let Ok(script) = o.try_cast::<Script>() {
						self.pending_editor_update.scripts_to_reload.contains(&script.get_path().to_string())
					} else {
						false
					}
				} else {
					false
				}
			}
		} else {
			false
		};
		if inspector_dock_needs_refresh {
			self.pending_editor_update.queue_inspector_dock_refresh();
		}
		// We have to turn off process here because:
		// * This was probably called from `process()`
		// * Any of these functions we're about to call could result in popping up and stepping the ProgressDialog modal
		// * ProgressDialog::step() will call `Main::iteration()`, which calls `process()` on all the scene tree nodes
		// * calling `process()` on us again will cause gd_ext to attempt to re-bind_mut() the GodotProject singleton
		// * This will cause a panic because we're already in the middle of `process()` with a bound mut ref to base
		self.base_mut().set_process(false);

		// To prevent crashes when switching branches after selecting a node or resource
		// TODO: This can lead to a bad user experience because we lose the selection history and the user can't go back to any previous selection(s)
		// we may not need to do this if we can properly handle the state for the TileMap editor and other editors
		PatchworkEditorAccessor::clear_editor_selection();

		// let reload_scene_func = |scene_path: &str| {
		// 	if PatchworkEditorAccessor::is_changing_scene() {
		// 		tracing::debug!("Editor is changing scene BEFORE RELOADING SCENE, skipping reload of {}", scene_path);
		// 		return false;
		// 	}
		// 	// we don't need to do this and it may cause issues with the scripts
		// 	// let scene = force_reload_resource(scene_path);
		// 	if PatchworkEditorAccessor::is_changing_scene() {
		// 		tracing::debug!("Editor is changing scene AFTER RELOADING SCENE, skipping reload of {}", scene_path);
		// 		return false;
		// 	} else {
		// 		EditorFilesystemAccessor::reload_scene_from_path(&scene_path);
		// 	}
		// 	true
		// };

		if self.pending_editor_update.uids_to_add.len() > 0 {
			tracing::debug!("adding uids");
			for (path, uid) in self.pending_editor_update.uids_to_add.iter() {
				Self::add_new_uid(path, uid);
			}
			self.pending_editor_update.uids_to_add.clear();
		}
		// if there are scripts to reload, we need to reload them first and let it run `process()` at least once
		// before we start reloading anything else because the ScriptEditor forces the reload to run deferred
		// (i.e. AFTER the current process() call)
		// TODO: remove this after PR lands
		if self.pending_editor_update.scripts_to_reload.len() > 0 {
			PatchworkEditorAccessor::reload_scripts(&self.pending_editor_update.scripts_to_reload.iter().map(|path| path.clone()).collect::<Vec<String>>());
			self.pending_editor_update.scripts_to_reload.clear();
			self.base_mut().set_process(true);
			return;
		}

		// scene instances require scripts to be reloaded first
		if self.pending_editor_update.scenes_to_reload.len() > 0 {
			// TODO: NO longer needed, but keeping it around because not needing this depends on upstream patches.
			// let scene_root = EditorInterface::singleton().get_edited_scene_root();
			// let scene_root_path = if let Some(scene_root) = scene_root {
			// 	 scene_root.get_scene_file_path().to_string()
			// } else {
			// 	"".to_string()
			// };
			// let mut updating_current_scene = self.pending_editor_update.scenes_to_reload.contains_key(&scene_root_path);
			// let open_scene_paths = EditorInterface::singleton().get_open_scenes().to_vec().iter().map(|scene_path| scene_path.to_string()).collect::<HashSet<String>>();

			// // if the current edited scene is in the list of scenes to reload, reload ONLY that scene,
			// // then scan and wait until it's done to reload the rest of the scenes.
			// // Otherwise, the editor will completely fuck up and screw up the user's viewport.
			// let updating_scenes_not_in_open_scenes = self.pending_editor_update.scenes_to_reload.iter().find(|(path, _)| !open_scene_paths.contains(*path)).is_some();
			// // this is just to keep a reference to the main scene resource so it stays cached if needed
			// let mut _main_scene_resource = None;
			// if updating_scenes_not_in_open_scenes && !updating_current_scene {
			// 	let current_scene_content = if scene_root_path == "" {
			// 		None
			// 	} else if let Some(content) = self.pending_editor_update.scenes_to_reload.remove(&scene_root_path) {
			// 		Some(content)
			// 	} else if !updating_current_scene { // the scene we're updating may be a dependency of another scene, so we need to get the content
			// 		self.project._get_file_at(scene_root_path.clone(), None)
			// 	} else {
			// 		None
			// 	};
			// 	if let Some(FileContent::Scene(scene)) = &current_scene_content {
			// 			// check if any of the external dependencies are in the list of scenes to reload
			// 		for (path, _) in scene.ext_resources.iter() {
			// 			if self.pending_editor_update.scenes_to_reload.contains_key(path) {
			// 				// force update the main scene
			// 				updating_current_scene = true;
			// 				_main_scene_resource = force_reload_resource(&scene_root_path);
			// 				break;
			// 				// tracing::debug!("scene {} depends on scene {}, popping up modal", scene_root_path, path);
			// 				// self.pending_editor_update.modal_shown = true;
			// 				// PatchworkEditorAccessor::progress_add_task(MODAL_TASK_NAME, "Reloading scenes", 2, false);
			// 			}
			// 		}
			// 	}
			// }
			// if self.pending_editor_update.scenes_to_reload.contains_key(&scene_root_path) {
			// 	// reload the current scene manually so it retains the state
			// 	reload_scene_func(&scene_root_path);
			// }
			if !self.reload_modified_scenes() {
				self.pending_editor_update.changing_scene_cooldown = 6;
			}


			self.pending_editor_update.scenes_to_reload.clear();
			// let mut reloaded_scenes = HashSet::new();
			// let mut updating_current_scene = self.pending_editor_update.scenes_to_reload.contains_key(&scene_root_path);
			// let open_scene_paths = EditorInterface::singleton().get_open_scenes().to_vec().iter().map(|scene_path| scene_path.to_string()).collect::<HashSet<String>>();

        }
		if self.pending_editor_update.reimport_files.len() > 0 {
			EditorFilesystemAccessor::reimport_files(&self.pending_editor_update.reimport_files.iter().map(|path| path.clone()).collect::<Vec<String>>());
			self.pending_editor_update.reimport_files.clear();
        }

		if self.pending_editor_update.reload_project_settings {
			self.reload_project_settings();
			self.pending_editor_update.reload_project_settings = false;
		}

		if self.pending_editor_update.changing_scene_cooldown > 0 {
			self.pending_editor_update.changing_scene_cooldown -= 1;
		}
		if self.pending_editor_update.changing_scene_cooldown == 0 {
			self.pending_editor_update.changing_scene_cooldown = 0;
			EditorFilesystemAccessor::scan();
			self.pending_editor_update.added_files.clear();
			self.pending_editor_update.deleted_files.clear();
		} else {
			tracing::debug!("waiting to scan until after main scene is reloaded, scenes pending: {:?}", self.pending_editor_update.scenes_to_reload.len());
		}
		self.base_mut().set_process(true);
    }

	#[func]
	fn start(&mut self) {
		if !self.project.is_started() {
			self.project.start();
		} else {
			tracing::info!("GodotProject is already started, skipping...");
		}
	}

	#[func]
	fn is_started(&self) -> bool {
		self.project.is_started()
	}

	#[func]
	fn stop(&mut self) {
		if self.project.is_started() {
			self.project.stop();
		}
	}
}


#[godot_api]
impl INode for GodotProject {
    fn init(_base: Base<Node>) -> Self {
        GodotProject {
			base: _base,
			project: GodotProjectImpl::new(ProjectSettings::singleton().globalize_path("res://").to_string()),
			pending_editor_update: PendingEditorUpdate::default(),
			reload_modified_scenes_callable: None,
			reload_project_settings_callable: None,
		}
    }

    fn enter_tree(&mut self) {
		let callables = steal_editor_node_private_reload_methods_from_dialog_signal_handlers();
		if let Some((reload_modified_scenes_callable, reload_project_settings_callable)) = callables {
			self.reload_modified_scenes_callable = Some(reload_modified_scenes_callable);
			self.reload_project_settings_callable = Some(reload_project_settings_callable);
		} else {
			// if we rebase and this fails, we're going to have to do something else
			panic!("Failed to steal reload methods from dialog signal handlers");
		}
		let project_id = PatchworkConfigAccessor::get_project_doc_id();
		if project_id == "" {
			tracing::info!("Patchwork config has no project id, not autostarting...");
			return;
		}
		self.project.start();
    }

    fn exit_tree(&mut self) {
		if self.project.is_started() {
			self.project.stop();
		}
        // Perform typical plugin operations here.
    }

	#[instrument(target = "patchwork_rust_core::godot_project::outer_process", level = tracing::Level::DEBUG, skip_all)]
    fn process(&mut self, _delta: f64) {
		if !self.project.is_started() {
			return;
		}
		let (updates, signals) = self.project._process(_delta);
		if updates.len() > 0 {
			self.pending_editor_update.merge(self.process_godot_updates(updates));
		}
		if self.pending_editor_update.any_changes() {
			self.update_godot_after_sync();
		}
		for signal in signals {
			match signal {
				GodotProjectSignal::CheckedOutBranch => {
					let branch = self.project.get_checked_out_branch_state().unwrap().to_godot();
					self.signals().checked_out_branch().emit(&branch);
				}
				GodotProjectSignal::FilesChanged => {
					self.signals().files_changed().emit();
				}
				GodotProjectSignal::SavedChanges => {
					self.signals().saved_changes().emit();
				}
				GodotProjectSignal::BranchesChanged => {
					let branches = self.get_branches();
					self.signals().branches_changed().emit(&branches);
				}
				GodotProjectSignal::Started => {
					self.signals().started().emit();
				}
				GodotProjectSignal::ShutdownCompleted => {
					self.signals().shutdown_completed().emit();
				}
				GodotProjectSignal::SyncServerConnectionInfoChanged(peer_connection_info) => {
					self.signals().sync_server_connection_info_changed().emit(&peer_connection_info.to_godot());
				}
				GodotProjectSignal::ConnectionThreadFailed => {
					self.signals().connection_thread_failed().emit();
				}
			}
		}
    }
}


#[derive(GodotClass)]
#[class(init, base=EditorPlugin, tool)]
pub struct GodotProjectPlugin {
    base: Base<EditorPlugin>,
	sidebar_scene: Option<Gd<PackedScene>>,
	sidebar: Option<Gd<Control>>,
	initialized: bool,
	ui_needs_update: bool,
}


#[godot_api]
impl GodotProjectPlugin {
	#[func]
	fn _on_reload_ui(&mut self) {
		self.ui_needs_update = true;
	}

	fn add_sidebar(&mut self) {
		self.sidebar_scene = force_reload_resource("res://addons/patchwork/gdscript/sidebar.tscn")
			.map(|scene| scene.try_cast::<PackedScene>().ok())
			.flatten();
		self.sidebar = if let Some(Some(sidebar)) = self.sidebar_scene.as_ref().map(|scene| scene.instantiate()) {
			if let Ok(mut sidebar) = sidebar.try_cast::<Control>() {
				let _ = sidebar.connect("reload_ui", &Callable::from_object_method(&self.to_gd(), "_on_reload_ui"));
				Some(sidebar)
			} else {
				None
			}
		} else {
			None
		};
		if let Some(sidebar) = self.sidebar.as_ref() {
			self.to_gd().add_control_to_dock(DockSlot::RIGHT_UL, sidebar);
		} else {
			panic!("Failed to instantiate sidebar");
		};
	}

	fn remove_sidebar(&mut self) {
		if let Some(sidebar) = self.sidebar.as_ref() {
			self.to_gd().remove_control_from_docks(sidebar);
			let mut sidebar = self.sidebar.take().unwrap();
			sidebar.queue_free();
		} else {
			tracing::warn!("no sidebar to remove");
		}
		self.sidebar_scene = None;
	}
}

#[godot_api]
impl IEditorPlugin for GodotProjectPlugin {
    fn enter_tree(&mut self) {
        tracing::debug!("** GodotProjectPlugin: enter_tree");
    }

	fn ready(&mut self) {
		self.process(0.0);
	}

	fn process(&mut self, _delta: f64) {
		// Don't initialize until the project is fully loaded and the editor is not importing
		if !self.initialized
			&& !EditorFilesystemAccessor::is_scanning()
			&& !PatchworkEditorAccessor::is_editor_importing()
			&& DirAccess::dir_exists_absolute("res://.godot") // This is at the end because DirAccess::dir_exists_absolute locks a global mutex
			{
			let godot_project_singleton: Gd<GodotProject> = GodotProject::get_singleton();
			self.base_mut().add_child(&godot_project_singleton);
			self.add_sidebar();
			self.initialized = true;
		}
		if self.ui_needs_update {
			self.ui_needs_update = false;
			self.remove_sidebar();
			self.add_sidebar();
		}
	}
    fn exit_tree(&mut self) {
        tracing::debug!("** GodotProjectPlugin: exit_tree");
		if self.initialized {
			self.remove_sidebar();
			self.base_mut().remove_child(&GodotProject::get_singleton());
		} else {
			tracing::error!("*************** DID NOT INITIALIZE!!!!!!");
		}
    }
}


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

fn force_reload_resource(path: &str) -> Option<Gd<Resource>> {
	let scene = ResourceLoader::singleton()
	.load_ex(path)
	.cache_mode(CacheMode::REPLACE_DEEP)
	.done();
	scene
}
