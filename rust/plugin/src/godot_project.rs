use ::safer_ffi::prelude::*;
use automerge::op_tree::B;
use automerge::{Automerge, ObjId, Patch, PatchAction, Prop};
use autosurgeon::{Hydrate, Reconcile};
use futures::io::empty;
use godot::classes::file_access::ModeFlags;
use godot::classes::resource_loader::CacheMode;
use godot::global::str_to_var;
use godot::meta::AsArg;
use safer_ffi::layout::OpaqueKind::T;
use std::any::Any;
use std::collections::HashSet;
use std::io::BufWriter;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use std::{collections::HashMap, str::FromStr};

use automerge::{
    patches::TextRepresentation, transaction::Transactable, ChangeHash, ObjType, ReadDoc,
    TextEncoding, ROOT,
};
use automerge_repo::{DocHandle, DocumentId, PeerConnectionInfo};
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender};
use godot::classes::EditorFileSystem;
use godot::classes::EditorInterface;
use godot::classes::Image;
use godot::classes::ProjectSettings;
use godot::classes::ResourceLoader;
use godot::classes::{ClassDb, EditorPlugin, Engine, IEditorPlugin};
use godot::classes::{ConfigFile, DirAccess, FileAccess, ResourceImporter};
use godot::prelude::*;

use crate::godot_parser::{self, GodotScene, TypeOrInstance};
use crate::godot_project_driver::{BranchState, DocHandleType};
use crate::patches::{get_changed_files, get_changed_files_vec};
use crate::patchwork_config::PatchworkConfig;
use crate::utils::{array_to_heads, heads_to_array, parse_automerge_url, CommitMetadata};
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
}

#[derive(Debug, Clone, PartialEq)]
pub enum FileContent {
    String(String),
    Binary(Vec<u8>),
    Scene(GodotScene),
}

#[derive(Debug, Clone)]
enum CheckedOutBranchState {
    NothingCheckedOut,
    CheckingOut(DocumentId),
    CheckedOut(DocumentId),
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

#[derive(Debug, Clone)]
struct GodotProjectState {
    checked_out_doc_id: DocumentId,
    branches_metadata_doc_id: DocumentId,
}

#[derive(GodotClass)]
#[class(base=Node, tool)]
pub struct GodotProject {
    base: Base<Node>,
    doc_handles: HashMap<DocumentId, DocHandle>,
    branch_states: HashMap<DocumentId, BranchState>,
    checked_out_branch_state: CheckedOutBranchState,
    project_doc_id: Option<DocumentId>,
    driver: Option<GodotProjectDriver>,
    driver_input_tx: UnboundedSender<InputEvent>,
    driver_output_rx: UnboundedReceiver<OutputEvent>,
    sync_server_connection_info: Option<PeerConnectionInfo>,
}

enum ChangeOp {
    Added,
    Removed,
    Modified,
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

    // PUBLIC API

    #[func]
    fn set_user_name(&self, name: String) {
        self.driver_input_tx
            .unbounded_send(InputEvent::SetUserName { name })
            .unwrap();
    }

    #[func]
    fn shutdown(&self) {
        self.driver_input_tx
            .unbounded_send(InputEvent::StartShutdown)
            .unwrap();
    }

    #[func]
    fn get_project_doc_id(&self) -> Variant {
        match &self.project_doc_id {
            Some(doc_id) => GString::from(doc_id.to_string()).to_variant(),
            None => Variant::nil(),
        }
    }

    #[func]
    fn get_heads(&self) -> PackedStringArray /* String[] */ {
        match &self.get_checked_out_branch_state() {
            Some(branch_state) => branch_state
                .doc_handle
                .with_doc(|d| d.get_heads())
                .iter()
                .map(|h| GString::from(h.to_string()))
                .collect::<PackedStringArray>(),
            _ => PackedStringArray::new(),
        }
    }

    #[func]
    fn get_files(&self) -> Dictionary {
        let files = self._get_files_at(&None);

        let mut result = Dictionary::new();

        for (path, content) in files {
            match content {
                FileContent::String(text) => {
                    let _ = result.insert(path, text.to_variant());
                }
                FileContent::Binary(binary) => {
                    let _ = result.insert(path, PackedByteArray::from(binary).to_variant());
                }
                FileContent::Scene(godot_scene) => {
                    let _ = result.insert(path, godot_scene.serialize().to_variant());
                }
            }
        }

        result
    }

    fn _get_files_at(&self, heads: &Option<Vec<ChangeHash>>) -> HashMap<String, FileContent> {
        let mut files = HashMap::new();

        let branch_state = match self.get_checked_out_branch_state() {
            Some(branch_state) => branch_state,
            None => return files,
        };

        let heads = match heads {
            Some(heads) => heads.clone(),
            None => branch_state.synced_heads.clone(),
        };

        let doc = branch_state.doc_handle.with_doc(|d| d.clone());

        let files_obj_id = doc.get_at(ROOT, "files", &heads).unwrap().unwrap().1;

        for path in doc.keys_at(&files_obj_id, &heads) {
            let file_entry = match doc.get_at(&files_obj_id, &path, &heads) {
                Ok(Some((automerge::Value::Object(ObjType::Map), file_entry))) => file_entry,
                _ => panic!("failed to get file entry for {:?}", path),
            };

            let structured_content = doc
                .get_at(&file_entry, "structured_content", &heads)
                .unwrap()
                .map(|(value, _)| value);

            if structured_content.is_some() {
                let scene: GodotScene = GodotScene::hydrate_at(&doc, &path, &heads).ok().unwrap();
                files.insert(path, FileContent::Scene(scene));
                continue;
            }

            // try to read file as text
            let content = doc.get_at(&file_entry, "content", &heads);

            match content {
                Ok(Some((automerge::Value::Object(ObjType::Text), content))) => {
                    match doc.text_at(content, &heads) {
                        Ok(text) => {
                            files.insert(path, FileContent::String(text.to_string()));
                            continue;
                        }
                        Err(e) => println!("failed to read text file {:?}: {:?}", path, e),
                    }
                }
                _ => match doc.get_string_at(&file_entry, "content", &heads) {
                    Some(s) => {
                        files.insert(path, FileContent::String(s.to_string()));
                        continue;
                    }
                    _ => {}
                },
            }

            // ... otherwise try to read as linked binary doc
            let linked_file_content = doc
                .get_string_at(&file_entry, "url", &heads)
                .and_then(|url| parse_automerge_url(&url))
                .and_then(|doc_id| self.doc_handles.get(&doc_id))
                .map(|doc_handle| {
                    doc_handle.with_doc(|d| match d.get(ROOT, "content") {
                        Ok(Some((value, _))) if value.is_bytes() => {
                            FileContent::Binary(value.into_bytes().unwrap())
                        }
                        Ok(Some((value, _))) if value.is_str() => {
                            FileContent::String(value.into_string().unwrap())
                        }
                        _ => {
                            panic!(
                                "failed to read binary doc {:?} {:?} {:?}",
                                path,
                                doc_handle.document_id(),
                                doc_handle.with_doc(|d| d.get_heads())
                            );
                        }
                    })
                });
            if let Some(file_content) = linked_file_content {
                files.insert(path, file_content);
            }
        }

        return files;

        // try to read file as scene
    }

    #[func]
    pub fn get_singleton() -> Gd<Self> {
        Engine::singleton()
            .get_singleton(&StringName::from("GodotProject"))
            .unwrap()
            .cast::<Self>()
    }

    #[func]
    fn get_changed_files(&self, heads: PackedStringArray) -> PackedStringArray {
        self.get_changed_files_between(heads, PackedStringArray::new())
    }

    #[func]
    fn get_changed_files_between(
        &self,
        heads: PackedStringArray,
        curr_heads: PackedStringArray,
    ) -> PackedStringArray {
        let heads = array_to_heads(heads);
        // if curr_heads is empty, we're comparing against the current heads

        let checked_out_branch_state = match self.get_checked_out_branch_state() {
            Some(branch_state) => branch_state,
            None => return PackedStringArray::new(),
        };

        let curr_heads = if curr_heads.len() == 0 {
            checked_out_branch_state.synced_heads.clone()
        } else {
            array_to_heads(curr_heads)
        };

        let patches = checked_out_branch_state.doc_handle.with_doc(|d| {
            d.diff(
                &heads,
                &curr_heads,
                TextRepresentation::String(TextEncoding::Utf8CodeUnit),
            )
        });
        get_changed_files(&patches)
    }

    #[func]
    fn get_changed_file_content_between(
        &self,
        old_heads: PackedStringArray,
        curr_heads: PackedStringArray,
    ) -> Dictionary {
        return Dictionary::new();
    }

    #[func]
    fn get_changes(&self) -> Array<Dictionary> /* String[]  */ {
        let checked_out_branch_doc = match &self.get_checked_out_branch_state() {
            Some(branch_state) => branch_state.doc_handle.with_doc(|d| d.clone()),
            _ => return Array::new(),
        };

        checked_out_branch_doc
            .get_changes(&[])
            .to_vec()
            .iter()
            .map(|c| {
                let a = c.message();

                let mut commit_dict = dict! {
                    "hash": GString::from(c.hash().to_string()).to_variant(),
                    "timestamp": c.timestamp(),
                };

                if let Some(metadata) = c
                    .message()
                    .and_then(|m| serde_json::from_str::<CommitMetadata>(&m).ok())
                {
                    if let Some(username) = metadata.username {
                        let _ =
                            commit_dict.insert("username", GString::from(username).to_variant());
                    }

                    if let Some(branch_id) = metadata.branch_id {
                        let _ =
                            commit_dict.insert("branch_id", GString::from(branch_id).to_variant());
                    }

                    if let Some(merge_metadata) = metadata.merge_metadata {
                        let merge_metadata_dict = dict! {
                            "merged_branch_id": GString::from(merge_metadata.merged_branch_id).to_variant(),
                            "merged_at_heads": merge_metadata.merged_at_heads.iter().map(|h| GString::from(h.to_string())).collect::<PackedStringArray>().to_variant(),
                            "forked_at_heads": merge_metadata.forked_at_heads.iter().map(|h| GString::from(h.to_string())).collect::<PackedStringArray>().to_variant(),
                        };

                        let _ = commit_dict.insert("merge_metadata", merge_metadata_dict);
                    }
                }

                commit_dict
            })
            .collect::<Array<Dictionary>>()
    }

    #[func]
    fn get_main_branch(&self) -> Variant /* Branch? */ {
        match &self
            .branch_states
            .values()
            .find(|branch_state| branch_state.is_main)
        {
            Some(branch_state) => branch_state_to_dict(branch_state).to_variant(),
            None => Variant::nil(),
        }
    }

    #[func]
    fn get_branch_by_id(&self, branch_id: String) -> Variant /* Branch? */ {
        match DocumentId::from_str(&branch_id) {
            Ok(id) => self
                .branch_states
                .get(&id)
                .map(|branch_state| branch_state_to_dict(branch_state).to_variant())
                .unwrap_or(Variant::nil()),
            Err(_) => Variant::nil(),
        }
    }

    #[func]
    fn save_files_at(
        &self,
        files: Dictionary, /*  Record<String, Variant> */
        heads: PackedStringArray,
    ) {
        let heads = array_to_heads(heads);

        match &self.get_checked_out_branch_state() {
            Some(branch_state) => {
                self._save_files(branch_state.doc_handle.clone(), files, Some(heads))
            }
            None => panic!("couldn't save files, no checked out branch"),
        }
    }

    #[func]
    fn save_files(&self, files: Dictionary) {
        match &self.get_checked_out_branch_state() {
            Some(branch_state) => self._save_files(branch_state.doc_handle.clone(), files, None),
            None => panic!("couldn't save files, no checked out branch"),
        }
    }

    #[func]
    fn save_file(&self, path: String, content: Variant) {
        self.save_files(dict! { path: content });
    }

    #[func]
    fn save_file_at(&self, path: String, heads: PackedStringArray, content: Variant) {
        self.save_files_at(dict! { path: content }, heads);
    }

    fn _save_files(
        &self,
        branch_doc_handle: DocHandle,
        files: Dictionary, /*  Record<String, Variant> */
        heads: Option<Vec<ChangeHash>>,
    ) {
        let stored_files = self._get_files_at(&heads);

        // we filter the files here because godot sends us indiscriminately all the files in the project
        // we only want to save the files that have actually changed
        let changed_files: Vec<(String, FileContent)> = files
            .iter_shared()
            .filter_map(|(path, content)| {
                let stored_content = stored_files.get(&path.to_string());

                match content.get_type() {
                    VariantType::STRING => {
                        let new_content = String::from(content.to::<GString>());

                        // save scene files as structured data
                        if path.to_string().ends_with(".tscn") {
                            let new_scene = match godot_parser::parse_scene(&new_content) {
                                Ok(scene) => scene,
                                Err(error) => {
                                    println!("RUST: error parsing scene {}: {:?}", path, error);
                                    return None;
                                }
                            };

                            if let Some(FileContent::Scene(stored_scene)) = stored_content {
                                if stored_scene == &new_scene {
                                    println!("file {:?} is already up to date", path.to_string());
                                    return None;
                                }
                            }

                            return Some((path.to_string(), FileContent::Scene(new_scene)));
                        }

                        if let Some(FileContent::String(stored_text)) = stored_content {
                            if stored_text == &new_content {
                                println!("file {:?} is already up to date", path.to_string());
                                return None;
                            }
                        }

                        Some((path.to_string(), FileContent::String(new_content)))
                    }
                    VariantType::PACKED_BYTE_ARRAY => {
                        let new_content = content.to::<PackedByteArray>().to_vec();

                        if let Some(FileContent::Binary(stored_binary_content)) = stored_content {
                            if stored_binary_content == &new_content {
                                println!("file {:?} is already up to date", path.to_string());
                                return None;
                            }
                        }

                        Some((path.to_string(), FileContent::Binary(new_content)))
                    }
                    _ => panic!("invalid content type"),
                }
            })
            .collect();

        self.driver_input_tx
            .unbounded_send(InputEvent::SaveFiles {
                branch_doc_handle,
                heads,
                files: changed_files,
            })
            .unwrap();
    }

    fn _get_file_at(&self, path: String, heads: Option<Vec<ChangeHash>>) -> Option<FileContent> {
        let files = self._get_files_at(&heads);
        files.get(&path).cloned()
    }

    #[func]
    fn merge_branch(&mut self, source_branch_doc_id: String, target_branch_doc_id: String) {
        let source_branch_doc_id = DocumentId::from_str(&source_branch_doc_id).unwrap();
        let target_branch_doc_id = DocumentId::from_str(&target_branch_doc_id).unwrap();

        self.driver_input_tx
            .unbounded_send(InputEvent::MergeBranch {
                source_branch_doc_id: source_branch_doc_id,
                target_branch_doc_id: target_branch_doc_id.clone(),
            })
            .unwrap();

        self.checked_out_branch_state = CheckedOutBranchState::CheckingOut(target_branch_doc_id);
    }

    #[func]
    fn create_branch(&mut self, name: String) {
        let source_branch_doc_id = match &self.get_checked_out_branch_state() {
            Some(branch_state) => branch_state.doc_handle.document_id(),
            None => {
                panic!("couldn't create branch, no checked out branch");
            }
        };

        self.driver_input_tx
            .unbounded_send(InputEvent::CreateBranch {
                name,
                source_branch_doc_id,
            })
            .unwrap();

        self.checked_out_branch_state = CheckedOutBranchState::NothingCheckedOut;
    }

    #[func]
    fn create_merge_preview_branch(
        &mut self,
        source_branch_doc_id: String,
        target_branch_doc_id: String,
    ) {
        let source_branch_doc_id = DocumentId::from_str(&source_branch_doc_id).unwrap();
        let target_branch_doc_id = DocumentId::from_str(&target_branch_doc_id).unwrap();

        self.driver_input_tx
            .unbounded_send(InputEvent::CreateMergePreviewBranch {
                source_branch_doc_id,
                target_branch_doc_id,
            })
            .unwrap();
    }

    #[func]
    fn delete_branch(&mut self, branch_doc_id: String) {
        let branch_doc_id = DocumentId::from_str(&branch_doc_id).unwrap();

        self.driver_input_tx
            .unbounded_send(InputEvent::DeleteBranch { branch_doc_id })
            .unwrap();
    }

    #[func]
    fn checkout_branch(&mut self, branch_doc_id: String) {
        let branch_doc_id = DocumentId::from_str(&branch_doc_id).unwrap();

        let branch_state = match self.branch_states.get(&branch_doc_id) {
            Some(branch_state) => branch_state,
            None => {
                panic!("couldn't checkout basdfasdfrdfsfsdanch, branch doc id not found");
            }
        };

        if branch_state.synced_heads == branch_state.doc_handle.with_doc(|d| d.get_heads()) {
            self.checked_out_branch_state =
                CheckedOutBranchState::CheckedOut(branch_doc_id.clone());
            self.base_mut().emit_signal(
                "checked_out_branch",
                &[branch_doc_id.to_string().to_variant()],
            );
        } else {
            self.checked_out_branch_state =
                CheckedOutBranchState::CheckingOut(branch_doc_id.clone());
        }
    }

    // filters out merge preview branches
    #[func]
    fn get_branches(&self) -> Array<Dictionary> /* { name: String, id: String }[] */ {
        let mut branches = self
            .branch_states
            .values()
            .filter(|branch_state| branch_state.merge_info.is_none())
            .map(branch_state_to_dict)
            .collect::<Vec<Dictionary>>();

        branches.sort_by(|a, b| {
            let a_is_main = a.get("is_main").unwrap().to::<bool>();
            let b_is_main = b.get("is_main").unwrap().to::<bool>();

            if a_is_main && !b_is_main {
                return std::cmp::Ordering::Less;
            }
            if !a_is_main && b_is_main {
                return std::cmp::Ordering::Greater;
            }

            let name_a = a.get("name").unwrap().to_string().to_lowercase();
            let name_b = b.get("name").unwrap().to_string().to_lowercase();
            name_a.cmp(&name_b)
        });

        Array::from_iter(branches)
    }

    #[func]
    fn get_checked_out_branch(&self) -> Variant /* {name: String, id: String, is_main: bool}? */ {
        match &self.get_checked_out_branch_state() {
            Some(branch_state) => branch_state_to_dict(branch_state).to_variant(),
            None => Variant::nil(),
        }
    }

    #[func]
    fn get_sync_server_connection_info(&self) -> Variant {
        match &self.sync_server_connection_info {
            Some(peer_connection_info) => {
                peer_connection_info_to_dict(peer_connection_info).to_variant()
            }
            None => Variant::nil(),
        }
    }

    // State api

    #[func]
    fn set_entity_state(&self, entity_id: String, prop: String, value: Variant) {
        let checked_out_doc_handle = match &self.get_checked_out_branch_state() {
            Some(branch_state) => branch_state.doc_handle.clone(),
            None => {
                return;
            }
        };

        checked_out_doc_handle.with_doc_mut(|d| {
            let mut tx = d.transaction();
            let state = match tx.get_obj_id(ROOT, "state") {
                Some(id) => id,
                _ => {
                    println!("failed to load state");
                    return;
                }
            };

            let entity_id = match tx.get_obj_id(&state, &entity_id) {
                Some(id) => id,
                None => match tx.put_object(state, &entity_id, ObjType::Map) {
                    Ok(id) => id,
                    Err(e) => {
                        println!("failed to create state object: {:?}", e);
                        return;
                    }
                },
            };

            match value.get_type() {
                VariantType::INT => {
                    let _ = tx.put(entity_id, prop, value.to::<i64>());
                }
                VariantType::FLOAT => {
                    let _ = tx.put(entity_id, prop, value.to::<f64>());
                }
                VariantType::STRING => {
                    let _ = tx.put(entity_id, prop, value.to::<GString>().to_string());
                }
                VariantType::STRING_NAME => {
                    let _ = tx.put(entity_id, prop, value.to::<StringName>().to_string());
                }
                VariantType::BOOL => {
                    let _ = tx.put(entity_id, prop, value.to::<bool>());
                }
                _ => println!(
                    "failed to store {}.{} unsupported value type: {:?}",
                    entity_id,
                    prop,
                    value.get_type()
                ),
            }

            tx.commit();
        });
    }

    #[func]
    fn get_entity_state(&self, entity_id: String, prop: String) -> Variant /* Option<int | float | string | bool */
    {
        let checked_out_branch_state = match self.get_checked_out_branch_state() {
            Some(branch_state) => branch_state,
            None => {
                return Variant::nil();
            }
        };

        checked_out_branch_state.doc_handle.with_doc(|d| {
            let state = match d.get_obj_id(ROOT, "state") {
                Some(id) => id,
                None => {
                    println!("invalid document, no state property");
                    return Variant::nil();
                }
            };

            let entity = match d.get_obj_id(state, entity_id) {
                Some(id) => id,
                None => {
                    return Variant::nil();
                }
            };

            return match d.get_variant(entity, prop) {
                Some(value) => value,
                None => Variant::nil(),
            };
        })
    }

    #[func]
    fn get_all_entity_ids(&self) -> PackedStringArray {
        let checked_out_branch_state = match self.get_checked_out_branch_state() {
            Some(branch_state) => branch_state,
            None => {
                return PackedStringArray::new();
            }
        };

        checked_out_branch_state.doc_handle.with_doc(|d| {
            let state = match d.get_obj_id(ROOT, "state") {
                Some(id) => id,
                None => return PackedStringArray::new(),
            };

            d.keys(&state).map(|k| GString::from(k)).collect()
        })
    }

    fn get_checked_out_branch_state(&self) -> Option<BranchState> {
        match &self.checked_out_branch_state {
            CheckedOutBranchState::CheckedOut(branch_doc_id) => {
                Some(self.branch_states.get(&branch_doc_id).unwrap().clone())
            }
            _ => {
                println!(
                    "warning: tried to get checked out branch state when nothing is checked out"
                );
                None
            }
        }
    }

    #[func]
    fn get_all_changes_between(
        &self,
        old_heads: PackedStringArray,
        curr_heads: PackedStringArray,
    ) -> Dictionary {
        let old_heads = array_to_heads(old_heads);
        let new_heads = array_to_heads(curr_heads);
        self._get_changes_between(old_heads, new_heads)
    }

    fn get_class_name(&self, script_content: String) -> String {
        // just keep going until we find `class_name <something>`
        for line in script_content.lines() {
            if line.trim().starts_with("class_name") {
                return line.trim().split(" ").nth(1).unwrap().trim().to_string();
            }
        }
        String::new()
    }

    fn _write_file_content_to_file(&self, path: &String, file_content: &FileContent) {
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
        let mut dir_access = DirAccess::open(&root);
        if let Some(mut dir_access) = dir_access {
            let _ = dir_access.make_dir_recursive(&GString::from(dir));
        }

        let file = FileAccess::open(path, ModeFlags::WRITE);
        if let None = file {
            println!("error opening file: {}", path);
            return;
        }
        let mut file = file.unwrap();
        // if it's a packedbytearray, write the bytes
        if let FileContent::Binary(bytes) = file_content {
            file.store_buffer(&PackedByteArray::from(bytes.as_slice()));
        } else if let FileContent::String(s) = file_content {
            file.store_line(&GString::from(s));
        } else {
            println!("unsupported variant type!! {:?}", file_content.type_id());
        }
        file.close();
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
        let mut dir_access = DirAccess::open(&root);
        if let Some(mut dir_access) = dir_access {
            let _ = dir_access.make_dir_recursive(&GString::from(dir));
        }

        let file = FileAccess::open(path, ModeFlags::WRITE);
        if let None = file {
            println!("error opening file: {}", path);
            return;
        }
        let mut file = file.unwrap();
        // if it's a packedbytearray, write the bytes
        if let Ok(packed_byte_array) = variant.try_to::<PackedByteArray>() {
            file.store_buffer(&packed_byte_array);
        } else if let Ok(string) = variant.try_to::<String>() {
            file.store_line(&GString::from(string));
        } else {
            println!("unsupported variant type!! {:?}", variant.type_id());
        }
        file.close();
    }

    fn get_varstr_value(&self, prop_value: String) -> VariantStrValue {
        if prop_value.contains("Resource(") {
            let id = prop_value
                .split("(\"")
                .nth(1)
                .unwrap()
                .split("\")")
                .nth(0)
                .unwrap()
                .trim()
                .to_string();
            if (prop_value.contains("SubResource(")) {
                return VariantStrValue::SubResourceID(id);
            } else if (prop_value.contains("ExtResource(")) {
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
            heads[0].to_string().chars().take(6).collect::<String>()
        );
        let temp_path = path.replace("res://", &temp_dir);
        // append _old or _new to the temp path (i.e. res://thing.<EXT> -> user://temp_123_456/thing_old.<EXT>)
        self._write_file_content_to_file(&temp_path, content);
        // get the import file conetnt
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
                self._write_file_content_to_file(
                    &import_file_path,
                    &FileContent::String(import_file_content),
                );

                let res = ClassDb::singleton().class_call_static(
                    "PatchworkEditor",
                    "import_and_load_resource",
                    &[temp_path.to_variant()],
                );
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

    fn _file_content_to_variant(content: &Option<FileContent>) -> Variant {
        match content {
            Some(FileContent::String(s)) => GString::from(s).to_variant(),
            Some(FileContent::Binary(bytes)) => {
                PackedByteArray::from(bytes.as_slice()).to_variant()
            }
            Some(FileContent::Scene(scene)) => scene.serialize().to_variant(),
            None => Variant::nil(),
        }
    }

    fn _get_resource_diff(
        &self,
        path: &String,
        change_type: &str,
        old_content: &Option<FileContent>,
        new_content: &Option<FileContent>,
        old_heads: &Vec<ChangeHash>,
        curr_heads: &Vec<ChangeHash>,
    ) -> Dictionary {
        let mut result = dict! {
            "path" : path.to_variant(),
            "diff_type" : "resource_changed".to_variant(),
            "change_type" : change_type.to_variant(),
            "old_content" : GodotProject::_file_content_to_variant(old_content),
            "new_content" : GodotProject::_file_content_to_variant(new_content),
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
        old_content: &Option<FileContent>,
        new_content: &Option<FileContent>,
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
        let diff = GodotProject::get_diff_dict(path.clone(), path.clone(), old_text, new_text);
        let result = dict! {
            "path" : path.to_variant(),
            "change_type" : change_type.to_variant(),
            "old_content" : if old_text.is_empty() { Variant::nil() } else { old_text.to_variant() },
            "new_content" : if new_text.is_empty() { Variant::nil() } else { new_text.to_variant() },
            "text_diff" : diff,
            "diff_type" : "text_changed".to_variant(),
        };
        result
    }

    fn _get_file_content_variant_type(content: &Option<FileContent>) -> VariantType {
        match content {
            Some(FileContent::String(_)) => VariantType::STRING,
            Some(FileContent::Binary(_)) => VariantType::PACKED_BYTE_ARRAY,
            Some(FileContent::Scene(_)) => VariantType::OBJECT,
            None => VariantType::NIL,
        }
    }

    fn _get_non_scene_diff(
        &self,
        path: &String,
        change_type: &str,
        old_content: &Option<FileContent>,
        new_content: &Option<FileContent>,
        old_heads: &Vec<ChangeHash>,
        curr_heads: &Vec<ChangeHash>,
    ) -> Dictionary {
        let old_content_type = GodotProject::_get_file_content_variant_type(old_content);
        let new_content_type = GodotProject::_get_file_content_variant_type(new_content);
        if (change_type == "unchanged") {
            return dict! {
                "path" : path.to_variant(),
                "diff_type" : "file_unchanged".to_variant(),
                "change_type" : change_type.to_variant(),
                "old_content": GodotProject::_file_content_to_variant(old_content),
                "new_content": GodotProject::_file_content_to_variant(new_content),
            };
        }
        if (old_content_type != VariantType::STRING && new_content_type != VariantType::STRING) {
            return self._get_resource_diff(
                &path,
                &change_type,
                &old_content,
                &new_content,
                &old_heads,
                &curr_heads,
            );
        } else if (old_content_type != VariantType::PACKED_BYTE_ARRAY
            && new_content_type != VariantType::PACKED_BYTE_ARRAY)
        {
            return self._get_text_file_diff(&path, &change_type, &old_content, &new_content);
        } else {
            return dict! {
                "path" : path.to_variant(),
                "diff_type" : "file_changed".to_variant(),
                "change_type" : change_type.to_variant(),
                "old_content" : GodotProject::_file_content_to_variant(&old_content),
                "new_content" : GodotProject::_file_content_to_variant(&new_content),
            };
        }
    }

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

        // only get the first 6 chars of the hash
        let patches = checked_out_branch_state.doc_handle.with_doc(|d| {
            d.diff(
                &old_heads,
                &curr_heads,
                TextRepresentation::String(TextEncoding::Utf8CodeUnit),
            )
        });
        let mut changed_files = get_changed_files_vec(&patches);
        let mut changed_files_set = HashMap::new();
        let mut scene_files = Vec::new();

        let mut all_diff: HashMap<String, Dictionary> = HashMap::new();
        // Get old and new content
        for path in changed_files.iter() {
            let old_file_content = self._get_file_at(path.clone(), Some(old_heads.clone()));
            let new_file_content = self._get_file_at(path.clone(), Some(curr_heads.clone()));
            let old_content_type = GodotProject::_get_file_content_variant_type(&old_file_content);
            let new_content_type = GodotProject::_get_file_content_variant_type(&new_file_content);
            let change_type = if old_file_content.is_none() {
                "added"
            } else if new_file_content.is_none() {
                "deleted"
            } else {
                "modified"
            };
            changed_files_set.insert(path.clone(), change_type.to_string());
            if old_content_type != VariantType::OBJECT && new_content_type != VariantType::OBJECT {
                // if both the old and new one are binary, or if one is none and the other is binary, then we can use the resource diff
                let _ = all_diff.insert(
                    path.clone(),
                    self._get_non_scene_diff(
                        &path,
                        &change_type,
                        &old_file_content,
                        &new_file_content,
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
            let mut added_node_ids: HashSet<String> = HashSet::new();
            let mut deleted_node_ids: HashSet<String> = HashSet::new();

            let mut node_modifications: HashMap<String, Vec<ChangeOp>> = HashMap::new();

            let mut insert_node_modification = |node_id: &String, change_op: ChangeOp| {
                let entry = node_modifications
                    .entry(node_id.clone())
                    .or_insert(Vec::new());
                // if the last change_op is add and the current one is deleted, remove the last one
                if (matches!(entry.last(), Some(&ChangeOp::Added))
                    && matches!(change_op, ChangeOp::Removed))
                    || (matches!(entry.last(), Some(&ChangeOp::Removed))
                        && matches!(change_op, ChangeOp::Added))
                {
                    entry.pop();
                } else {
                    entry.push(change_op);
                }
            };

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
                    |PathWithAction { path, action }| match path.first() {
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
                        if changed_files_set.contains_key(&ext_resource.path) {
                            let change_type = changed_files_set.get(&ext_resource.path).unwrap();
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
                                     scene: &Option<GodotScene>,
                                     content_key: &str| {
                match type_or_instance {
                    TypeOrInstance::Type(type_name) => type_name.clone(),
                    TypeOrInstance::Instance(instance_id) => {
                        if let Some(scene) = scene {
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
                    if (!loaded_ext_resources.contains_key(&path)) {
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
                |prop: String, old_value: VariantStrValue, new_value: VariantStrValue| {
                    return dict! {
                        "name": prop.clone(),
                        "change_type": "modified",
                        "old_value": fn_get_prop_value(old_value, &old_scene, true),
                        "new_value": fn_get_prop_value(new_value, &new_scene, false)
                    };
                };
            let mut detect_changed_prop =
                |prop: String,
                 class_name: &TypeOrInstance,
                 old_prop: Option<String>,
                 new_prop: Option<String>| {
                    let sn_2: StringName = StringName::from(&prop);
                    let default_value = if let TypeOrInstance::Type(class_name) = class_name {
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
                                return Some(get_changed_prop_dict(prop, old_value, new_value));
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
                                return Some(get_changed_prop_dict(prop, old_value, new_value));
                            }
                        }
                        (
                            VariantStrValue::ResourcePath(resource_path),
                            VariantStrValue::ResourcePath(new_resource_path),
                        ) => {
                            if all_changed_ext_resource_paths.contains(resource_path)
                                || all_changed_ext_resource_paths.contains(new_resource_path)
                            {
                                return Some(get_changed_prop_dict(prop, old_value, new_value));
                            } else if (resource_path != new_resource_path) {
                                return Some(get_changed_prop_dict(prop, old_value, new_value));
                            }
                        }
                        (
                            VariantStrValue::Variant(old_variant),
                            VariantStrValue::Variant(new_variant),
                        ) => {
                            if old_variant != new_variant {
                                return Some(get_changed_prop_dict(prop, old_value, new_value));
                            }
                        }
                        _ => {
                            // changed type
                            return Some(get_changed_prop_dict(prop, old_value, new_value));
                        }
                    }
                    None
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

                if old_has && !new_has {
                    let mut node_info = Dictionary::new();
                    node_info.insert("change_type", "removed");
                    if let Some(scene) = &old_scene {
                        node_info.insert("node_path", scene.get_node_path(&node_id));
                        if let Some(content) = scene.get_node_content(&node_id) {
                            node_info.insert("old_content", content);
                        }
                    }
                    changed_nodes.push(&node_info.to_variant());
                } else if !old_has && new_has {
                    let mut node_info = Dictionary::new();
                    node_info.insert("change_type", "added");
                    if let Some(scene) = &new_scene {
                        node_info.insert("node_path", scene.get_node_path(&node_id));
                        if let Some(content) = scene.get_node_content(&node_id) {
                            node_info.insert("new_content", content);
                        }
                    }
                    changed_nodes.push(&node_info.to_variant());
                } else if old_has && new_has && changed_node_ids.contains(node_id) {
                    let mut node_info = Dictionary::new();
                    node_info.insert("change_type", "modified");

                    if let Some(scene) = &new_scene {
                        node_info.insert("node_path", scene.get_node_path(node_id));
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
                            node_info.insert("old_content", content);
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
                            node_info.insert("new_content", content);
                        }
                    }
                    // old_type and new_type
                    let old_class_name = fn_get_class_name(&old_type, &old_scene, "old_content");
                    let new_class_name = fn_get_class_name(&new_type, &new_scene, "new_content");

                    if old_class_name != new_class_name {
                        node_info.insert("change_type", "type_changed");
                    } else {
                        let mut props: HashSet<String> = HashSet::new();
                        for (key, _) in old_props.iter_shared() {
                            props.insert(key.to_string());
                        }
                        for (key, _) in new_props.iter_shared() {
                            props.insert(key.to_string());
                        }
                        for prop in props {
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
                            if let Some(changed_prop) =
                                detect_changed_prop(prop.clone(), &new_type, old_prop, new_prop)
                            {
                                let _ = changed_props.insert(prop.clone(), changed_prop);
                            }
                        }
                        if changed_props.len() > 0 {
                            node_info.insert("changed_props", changed_props);
                        }
                        changed_nodes.push(&node_info.to_variant());
                    }
                }
            }
            result.insert("changed_nodes", changed_nodes);
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

        let storage_folder_path =
            String::from(ProjectSettings::singleton().globalize_path("res://.patchwork"));
        let mut driver: GodotProjectDriver = GodotProjectDriver::create(storage_folder_path);
        let maybe_user_name: String = PatchworkConfig::singleton()
            .bind()
            .get_user_value(GString::from("user_name"), "".to_variant())
            .to_string();
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

    fn start(&mut self) {
        let project_doc_id: String = PatchworkConfig::singleton()
            .bind()
            .get_project_value(GString::from("project_doc_id"), "".to_variant())
            .to_string();
        let checked_out_branch_doc_id = PatchworkConfig::singleton()
            .bind()
            .get_project_value(GString::from("checked_out_branch_doc_id"), "".to_variant())
            .to_string();
        println!("rust: START {:?}", project_doc_id);
        self.project_doc_id = match DocumentId::from_str(&project_doc_id) {
            Ok(doc_id) => Some(doc_id),
            Err(e) => None,
        };

        self.checked_out_branch_state = match DocumentId::from_str(&checked_out_branch_doc_id) {
            Ok(doc_id) => CheckedOutBranchState::CheckingOut(doc_id),
            Err(_) => CheckedOutBranchState::NothingCheckedOut,
        };

        println!(
            "initial checked out branch state: {:?}",
            self.checked_out_branch_state
        );

        self._start_driver();
    }

    fn _stop_driver(&mut self) {
        if let Some(mut driver) = self.driver.take() {
            driver.teardown();
        }
    }

    fn stop(&mut self) {
        self._stop_driver();
        self.checked_out_branch_state = CheckedOutBranchState::NothingCheckedOut;
        self.sync_server_connection_info = None;
        self.project_doc_id = None;
        self.doc_handles.clear();
        self.branch_states.clear();
    }
}

// static singleton variable
static mut GODOT_PROJECT: Option<GodotProject> = None;

#[godot_api]
impl INode for GodotProject {
    fn init(_base: Base<Node>) -> Self {
        let (driver_input_tx, driver_input_rx) = futures::channel::mpsc::unbounded();
        let (driver_output_tx, driver_output_rx) = futures::channel::mpsc::unbounded();

        let mut ret = Self {
            base: _base,
            sync_server_connection_info: None,
            doc_handles: HashMap::new(),
            branch_states: HashMap::new(),
            checked_out_branch_state: CheckedOutBranchState::NothingCheckedOut,
            project_doc_id: None,
            driver: None,
            driver_input_tx,
            driver_output_rx,
        };
        // process it a few times to get it to check out the branch
        ret
    }

    fn enter_tree(&mut self) {
        println!("** GodotProject: enter_tree");
        self.start();
        // Perform typical plugin operations here.
    }

    fn exit_tree(&mut self) {
        println!("** GodotProject: exit_tree");
        self.stop();
        // Perform typical plugin operations here.
    }

    fn process(&mut self, _delta: f64) {
        while let Ok(Some(event)) = self.driver_output_rx.try_next() {
            match event {
                OutputEvent::NewDocHandle {
                    doc_handle,
                    doc_handle_type,
                } => {
                    if doc_handle_type == DocHandleType::Binary {
                        println!(
                            "rust: NewBinaryDocHandle !!!! {} {} changes",
                            doc_handle.document_id(),
                            doc_handle.with_doc(|d| d.get_heads().len())
                        );
                    }

                    self.doc_handles
                        .insert(doc_handle.document_id(), doc_handle.clone());
                }
                OutputEvent::BranchStateChanged {
                    branch_state,
                    trigger_reload,
                } => {
                    self.branch_states
                        .insert(branch_state.doc_handle.document_id(), branch_state.clone());

                    let branches = self
                        .get_branches()
                        .iter_shared()
                        .map(|branch| branch.to_variant())
                        .collect::<Array<Variant>>()
                        .to_variant();

                    self.base_mut().emit_signal("branches_changed", &[branches]);

                    let mut active_branch_state: Option<BranchState> = None;
                    let mut checking_out_new_branch = false;

                    match &self.checked_out_branch_state {
                        CheckedOutBranchState::NothingCheckedOut => {
                            // check out main branch if we haven't checked out anything yet
                            if branch_state.is_main {
                                checking_out_new_branch = true;

                                self.checked_out_branch_state = CheckedOutBranchState::CheckingOut(
                                    branch_state.doc_handle.document_id(),
                                );
                                active_branch_state = Some(branch_state.clone());
                            }
                        }
                        CheckedOutBranchState::CheckingOut(branch_doc_id) => {
                            active_branch_state = self.branch_states.get(&branch_doc_id).cloned();
                            checking_out_new_branch = true;
                        }
                        CheckedOutBranchState::CheckedOut(branch_doc_id) => {
                            active_branch_state = self.branch_states.get(&branch_doc_id).cloned();
                        }
                    }

                    // only trigger update if checked out branch is fully synced
                    if let Some(active_branch_state) = active_branch_state {
                        if active_branch_state.is_synced() {
                            if checking_out_new_branch {
                                println!(
                                    "rust: TRIGGER checked out new branch: {}",
                                    active_branch_state.name
                                );

                                self.checked_out_branch_state = CheckedOutBranchState::CheckedOut(
                                    active_branch_state.doc_handle.document_id(),
                                );

                                self.base_mut().emit_signal(
                                    "checked_out_branch",
                                    &[branch_state
                                        .doc_handle
                                        .document_id()
                                        .to_string()
                                        .to_variant()
                                        .to_variant()],
                                );
                            } else {
                                if trigger_reload {
                                    println!("rust: TRIGGER files changed: {}", branch_state.name);
                                    self.base_mut().emit_signal("files_changed", &[]);
                                } else {
                                    println!("rust: TRIGGER saved changes: {}", branch_state.name);
                                    self.base_mut().emit_signal("saved_changes", &[]);
                                }
                            }
                        }
                    }
                }
                OutputEvent::Initialized { project_doc_id } => {
                    self.project_doc_id = Some(project_doc_id);
                }

                OutputEvent::CompletedCreateBranch { branch_doc_id } => {
                    self.checked_out_branch_state =
                        CheckedOutBranchState::CheckingOut(branch_doc_id);
                }

                OutputEvent::CompletedShutdown => {
                    println!("rust: CompletedShutdown event");
                    self.base_mut().emit_signal("shutdown_completed", &[]);
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
                                    sync_server_connection_info
                                        .docs
                                        .insert(doc_id.clone(), doc_state.clone());
                                });

                            peer_connection_info
                        }
                    };

                    self.base_mut().emit_signal(
                        "sync_server_connection_info_changed",
                        &[
                            peer_connection_info_to_dict(&new_sync_server_connection_info)
                                .to_variant(),
                        ],
                    );
                }
            }
        }
    }
}

#[derive(GodotClass)]
#[class(init, base=EditorPlugin, tool)]
pub struct GodotProjectPlugin {
    base: Base<EditorPlugin>,
}

#[godot_api]
impl IEditorPlugin for GodotProjectPlugin {
    fn enter_tree(&mut self) {
        println!("** GodotProjectPlugin: enter_tree");
        let godot_project_singleton: Gd<GodotProject> = GodotProject::get_singleton();
        self.base_mut().add_child(&godot_project_singleton);
    }

    fn exit_tree(&mut self) {
        println!("** GodotProjectPlugin: exit_tree");
        self.base_mut().remove_child(&GodotProject::get_singleton());
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

fn branch_state_to_dict(branch_state: &BranchState) -> Dictionary {
    let mut branch = dict! {
        "name": branch_state.name.clone(),
        "id": branch_state.doc_handle.document_id().to_string(),
        "is_main": branch_state.is_main,

        // we shouldn't have branches that don't have any changes but sometimes
        // the branch docs are not synced correctly so this flag is used in the UI to
        // indicate that the branch is not loaded and prevent users from checking it out
        "is_not_loaded": branch_state.doc_handle.with_doc(|d| d.get_heads().len() == 0),
        "heads": heads_to_array(branch_state.synced_heads.clone()),
        "is_merge_preview": branch_state.merge_info.is_some(),
    };

    if let Some(fork_info) = &branch_state.fork_info {
        let _ = branch.insert("forked_from", fork_info.forked_from.to_string());
        let _ = branch.insert("forked_at", heads_to_array(fork_info.forked_at.clone()));
    }

    if let Some(merge_info) = &branch_state.merge_info {
        let _ = branch.insert("merge_into", merge_info.merge_into.to_string());
        let _ = branch.insert("merge_at", heads_to_array(merge_info.merge_at.clone()));
    }

    branch
}

fn peer_connection_info_to_dict(peer_connection_info: &PeerConnectionInfo) -> Dictionary {
    let mut doc_sync_states = Dictionary::new();

    for (doc_id, doc_state) in peer_connection_info.docs.iter() {
        let last_received = doc_state
            .last_received
            .map(system_time_to_variant)
            .unwrap_or(Variant::nil());

        let last_sent = doc_state
            .last_sent
            .map(system_time_to_variant)
            .unwrap_or(Variant::nil());

        let last_sent_heads = doc_state
            .last_sent_heads
            .as_ref()
            .map(|heads| heads_to_array(heads.clone()).to_variant())
            .unwrap_or(Variant::nil());

        let last_acked_heads = doc_state
            .last_acked_heads
            .as_ref()
            .map(|heads| heads_to_array(heads.clone()).to_variant())
            .unwrap_or(Variant::nil());

        let _ = doc_sync_states.insert(
            doc_id.to_string(),
            dict! {
                "last_received": last_received,
                "last_sent": last_sent,
                "last_sent_heads": last_sent_heads,
                "last_acked_heads": last_acked_heads,
            },
        );
    }

    let last_received = peer_connection_info
        .last_received
        .map(system_time_to_variant)
        .unwrap_or(Variant::nil());

    let last_sent = peer_connection_info
        .last_sent
        .map(system_time_to_variant)
        .unwrap_or(Variant::nil());

    let is_connected = !last_received.is_nil();

    dict! {
        "doc_sync_states": doc_sync_states,
        "last_received": last_received,
        "last_sent": last_sent,
        "is_connected": is_connected,
    }
}

fn system_time_to_variant(time: SystemTime) -> Variant {
    time.duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs().to_variant())
        .unwrap_or(Variant::nil())
}
