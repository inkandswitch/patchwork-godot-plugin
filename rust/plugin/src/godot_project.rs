use crate::file_utils::FileContent;
use crate::godot_accessors::{EditorFilesystemAccessor, PatchworkConfigAccessor, PatchworkEditorAccessor};
use crate::godot_project_api::GodotProjectViewModel;
use crate::godot_project_impl::{GodotProjectImpl, GodotProjectSignal};
use automerge::ChangeHash;
use godot::classes::editor_plugin::DockSlot;
use ::safer_ffi::prelude::*;
use automerge_repo::{DocumentId, PeerConnectionInfo};
use godot::classes::resource_loader::CacheMode;
use godot::classes::{ConfirmationDialog, Control};
use godot::classes::EditorInterface;
use godot::classes::ProjectSettings;
use godot::classes::ResourceLoader;
use godot::classes::{EditorPlugin, Engine, IEditorPlugin};
use godot::classes::{DirAccess};
use godot::prelude::*;
use godot::prelude::Dictionary;
use tracing::instrument;
use std::collections::{HashSet};
use std::path::PathBuf;
use std::{collections::HashMap, str::FromStr};
use crate::godot_helpers::{ToGodotExt, ToVariantExt, are_valid_heads, array_to_heads};
use crate::file_system_driver::{FileSystemEvent};

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

/// Tracks updates that Godot may have made, to ensure we can track them and update the state accordingly
#[derive(Debug, Default)]
struct PendingEditorUpdate {
	added_files: HashSet<String>,
	deleted_files: HashSet<String>,
	scripts_to_reload: HashSet<String>,
	scenes_to_reload: HashMap<String, FileContent>,
	reimport_files: HashSet<String>,
	uids_to_add: HashMap<String, String>,
	reload_project_settings: bool
}

impl PendingEditorUpdate {
	/// Merges another PendingEditorUpdate into this one, combining their changes
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

	/// Returns true if there are any added or deleted files
	fn added_or_deleted_files(&self) -> bool {
		self.added_files.len() > 0 || self.deleted_files.len() > 0
	}

	/// Returns true if there are any file changes to process
	fn any_changes(&self) -> bool {
		self.scripts_to_reload.len() > 0 || self.scenes_to_reload.len() > 0 || self.reimport_files.len() > 0 || self.uids_to_add.len() > 0 || self.added_or_deleted_files()
	}

	/// Clears all pending updates
	fn clear(&mut self) {
		self.added_files.clear();
		self.deleted_files.clear();
		self.scripts_to_reload.clear();
		self.scenes_to_reload.clear();
		self.reimport_files.clear();
		self.uids_to_add.clear();
		self.reload_project_settings = false;
	}
}

/// GodotProject is the main interface between Godot's API and the Patchwork Rust core.
/// It is intended to be a gdscript-visible lightweight wrapper around the GodotProjectImpl, which contains the actual logic.
/// It also handles signals and communication with Godot.
#[derive(GodotClass, Debug)]
#[class(base=Node)]
pub struct GodotProject {
	base: Base<Node>,
	// todo (Lilith's PR): change this to GodotProjectViewModel trait ideally
	project: GodotProjectImpl,
	pending_editor_update: PendingEditorUpdate,
	reload_project_settings_callable: Option<Callable>,
	last_server_change_signal: std::time::SystemTime,
	pending_server_change_signal: Option<PeerConnectionInfo>
}

/// Macro to check if the GodotProject is started, and log an error if not
macro_rules! check_project_started {
	($self:ident) => {
		if !$self.project.is_started() {
			tracing::error!("GodotProject is not started, skipping...");
			// return the default value for the type
			return;
		}
	};
}

/// Macro to check if the GodotProject is started, and log an error if not, returning a default value
macro_rules! check_project_started_and_return_default {
	($self:ident, $default:expr) => {
		if !$self.project.is_started() {
			tracing::error!("GodotProject is not started, returning default value");
			return $default;
		}
	};
}

// new API
/// This implementation binds as closely as possible to [GodotProjectViewModel].
#[godot_api(secondary)]
impl GodotProject {
	#[func]
	fn clear_project(&mut self) {
		check_project_started!(self);
		self.project.clear_project();
	}

	#[func]
	fn get_user_name(&self) -> String {
		check_project_started_and_return_default!(self, String::new());
		self.project.get_user_name()
	}

	#[func]
	fn set_user_name(&self, name: String) {
		check_project_started!(self);
		self.project.set_user_name(name);
	}

	#[func]
    fn print_sync_debug(&self) {
		check_project_started!(self);
		self.project.print_sync_debug();
	}

	#[func]
	fn can_create_merge_preview_branch(&self) -> bool {
		check_project_started_and_return_default!(self, false);
		self.project.can_create_merge_preview_branch()
	}

	#[func]
	fn create_merge_preview_branch(&mut self) {
		check_project_started!(self);
		self.project.create_merge_preview_branch();
	}

	#[func]
	fn can_create_revert_preview_branch(&self, head: String) -> bool {
		check_project_started_and_return_default!(self, false);
		if let Ok(hash) = ChangeHash::from_str(&head) {
			return self.project.can_create_revert_preview_branch(hash);
		}
		false
	}

	#[func]
	fn create_revert_preview_branch(&mut self, head: String) {
		check_project_started!(self);
		if let Ok(hash) = ChangeHash::from_str(&head) {
			self.project.create_revert_preview_branch(hash);
		}
	}

	#[func]
	fn preview_branch_active(&self) -> bool {
		check_project_started_and_return_default!(self, false);
		self.project.preview_branch_active()
	}

	#[func]
	fn confirm_preview_branch(&mut self) {
		check_project_started!(self);
		self.project.confirm_preview_branch();
	}

	#[func]
	fn discard_preview_branch(&mut self) {
		check_project_started!(self);
		self.project.discard_preview_branch();
	}

	#[func]
	fn get_branch_history(&self) -> PackedStringArray {
		check_project_started_and_return_default!(self, PackedStringArray::new());
		self.project.get_branch_history().to_godot()
	}

	#[func]
	fn get_change_username(&self, hash: String) -> String {
		check_project_started_and_return_default!(self, String::new());
		if let Ok(hash) = ChangeHash::from_str(&hash) {
			return self.project.get_change_username(hash);
		}
		String::new()
	}

	#[func]
	fn is_change_synced(&self, hash: String) -> bool {
		check_project_started_and_return_default!(self, false);
		if let Ok(hash) = ChangeHash::from_str(&hash) {
			return self.project.is_change_synced(hash);
		}
		false
	}

	#[func]
	fn get_change_summary(&self, hash: String) -> String {
		check_project_started_and_return_default!(self, String::new());
		if let Ok(hash) = ChangeHash::from_str(&hash) {
			return self.project.get_change_summary(hash);
		}
		String::new()
	}

	#[func]
	fn is_change_merge(&self, hash: String) -> bool {
		check_project_started_and_return_default!(self, false);
		if let Ok(hash) = ChangeHash::from_str(&hash) {
			return self.project.is_change_merge(hash);
		}
		false
	}

	#[func]
	fn is_change_setup(&self, hash: String) -> bool {
		check_project_started_and_return_default!(self, false);
		if let Ok(hash) = ChangeHash::from_str(&hash) {
			return self.project.is_change_setup(hash);
		}
		false
	}

	#[func]
	fn get_change_exact_timestamp(&self, hash: String) -> String {
		check_project_started_and_return_default!(self, String::new());
		if let Ok(hash) = ChangeHash::from_str(&hash) {
			return self.project.get_change_exact_timestamp(hash);
		}
		String::new()
	}

	#[func]
	fn get_change_human_timestamp(&self, hash: String) -> String {
		check_project_started_and_return_default!(self, String::new());
		if let Ok(hash) = ChangeHash::from_str(&hash) {
			return self.project.get_change_human_timestamp(hash);
		}
		String::new()
	}

	#[func]
	fn get_change_merge_id(&self, hash: String) -> Variant {
		check_project_started_and_return_default!(self, Variant::nil());
		if let Ok(hash) = ChangeHash::from_str(&hash) {
			return self.project.get_change_merge_id(hash).to_variant();
		}
		Variant::nil()
	}
	
	#[func]
    fn get_sync_status(&self) -> Dictionary {
		check_project_started_and_return_default!(self, dict!{});
		self.project.get_sync_status().to_godot()
	}
}

// old API -- will be removed
#[godot_api]
impl GodotProject {
	#[signal]
	fn checked_out_branch(branch: Dictionary);

	#[signal]
	fn state_changed();

	#[func]
	fn revert_to_heads(&mut self, heads: PackedStringArray) {
		check_project_started!(self);
		self.project.revert_to_heads(array_to_heads(heads));
	}

	#[func]
	fn get_project_doc_id(&self) -> Variant {
		check_project_started_and_return_default!(self, Variant::nil());
		self.project.get_project_doc_id().to_variant()
	}

	#[func]
	fn get_heads(&self) -> PackedStringArray /* String[] */ {
		check_project_started_and_return_default!(self, PackedStringArray::new());
		self.project.get_heads().to_godot()
	}

    #[func]
    pub fn get_singleton() -> Gd<Self> {
        Engine::singleton()
            .get_singleton(&StringName::from("GodotProject"))
            .unwrap()
            .cast::<Self>()
    }

    #[func]
    fn get_main_branch(&self) -> Variant /* Branch? */ {
		check_project_started_and_return_default!(self, Variant::nil());
		self.project.get_main_branch().to_variant()
	}

    #[func]
    fn get_branch_by_id(&self, branch_id: String) -> Variant /* Branch? */ {
		check_project_started_and_return_default!(self, Variant::nil());
		self.project.get_branch_by_id(&DocumentId::from_str(&branch_id).unwrap()).to_variant()
	}
    #[func]
    fn merge_branch(&mut self, source_branch_doc_id: String, target_branch_doc_id: String) {
		check_project_started!(self);
		self.project.merge_branch(DocumentId::from_str(&source_branch_doc_id).unwrap(), DocumentId::from_str(&target_branch_doc_id).unwrap());
	}

    #[func]
    fn create_branch(&mut self, name: String) {
		check_project_started!(self);
		self.project.create_branch(name);
	}

    #[func]
    fn delete_branch(&mut self, branch_doc_id: String) {
		check_project_started!(self);
		self.project.delete_branch(DocumentId::from_str(&branch_doc_id).unwrap());
	}

    #[func]
    fn checkout_branch(&mut self, branch_doc_id: String) {
		check_project_started!(self);
		self.project.checkout_branch(DocumentId::from_str(&branch_doc_id).unwrap());
	}

    // filters out merge preview branches
    #[func]
    fn get_branches(&self) -> Array<Dictionary> /* { name: String, id: String }[] */ {
		check_project_started_and_return_default!(self, Array::new());
		self.project.get_branches().iter().map(|b| b.to_godot()).collect::<Array<Dictionary>>()
	}

    #[func]
    fn get_checked_out_branch(&self) -> Variant /* {name: String, id: String, is_main: bool}? */ {
		check_project_started_and_return_default!(self, Variant::nil());
		self.project.get_checked_out_branch_state().map(|b|b.to_godot().to_variant()).unwrap_or_default()
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
        self.project.get_changes_between(old_heads, new_heads)
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

	fn reload_project_settings(&self) {
		if let Some(reload_project_settings_callable) = &self.reload_project_settings_callable {
			reload_project_settings_callable.call(&[]);
		}
	}

	fn update_godot_after_source_change(&mut self) -> bool {
		if !self.pending_editor_update.any_changes() {
			return false;
		}
		if !GodotProjectImpl::safe_to_update_godot(false) {
			return false;
		}
		self.base_mut().set_process(false);
		PatchworkEditorAccessor::close_files_if_open(&self.pending_editor_update.deleted_files.iter().map(|path| path.clone()).collect::<Vec<String>>());
		if self.pending_editor_update.reload_project_settings {
			self.reload_project_settings();
		}
		PatchworkEditorAccessor::refresh_after_source_change();
		self.pending_editor_update.clear();
		self.base_mut().set_process(true);
		return true;
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
			reload_project_settings_callable: None,
			pending_server_change_signal: None,
			last_server_change_signal: std::time::SystemTime::UNIX_EPOCH
		}
    }

    fn enter_tree(&mut self) {
		let callables = steal_editor_node_private_reload_methods_from_dialog_signal_handlers();
		if let Some((_, reload_project_settings_callable)) = callables {
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
		let (updates, signals) = self.project.process(_delta);
		if updates.len() > 0 {
			self.pending_editor_update.merge(self.process_godot_updates(updates));
		}
		let mut refreshed = false;
		if self.pending_editor_update.any_changes() {
			refreshed = self.update_godot_after_source_change();
		}
		for signal in signals {
			match signal {
				GodotProjectSignal::CheckedOutBranch => {
					// TODO: This is a hack to clear the inspector item when the branch is changed to prevent crashes
					// Ideally, we'd figure out a way to keep the object in the inspector when the branch is changed
					if refreshed {
						EditorFilesystemAccessor::clear_inspector_item();
					}
					let branch = self.project.get_checked_out_branch_state().unwrap().to_godot();
					self.signals().checked_out_branch().emit(&branch);
				}
				GodotProjectSignal::FilesChanged => {
					self.signals().state_changed().emit();
				}
				GodotProjectSignal::SavedChanges => {
					self.signals().state_changed().emit();
				}
				GodotProjectSignal::BranchesChanged => {
					let branches = self.get_branches();
					self.signals().state_changed().emit();
				}
				GodotProjectSignal::SyncServerConnectionInfoChanged(_peer_connection_info) => {
					self.signals().state_changed().emit();
				}
				GodotProjectSignal::ConnectionThreadFailed => {
					self.signals().state_changed().emit();
				}
			}
		}
    }
}

/// An EditorPlugin to manage the GodotProject singleton and its UI. 
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
		self.sidebar_scene = Self
			::force_reload_resource("res://addons/patchwork/gdscript/sidebar.tscn")
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

	fn force_reload_resource(path: &str) -> Option<Gd<Resource>> {
		let scene = ResourceLoader::singleton()
			.load_ex(path)
			.cache_mode(CacheMode::REPLACE_DEEP)
			.done();
		scene
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