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
use std::{collections::HashMap, str::FromStr};

use automerge::{
    patches::TextRepresentation, transaction::Transactable, ChangeHash, ObjType, ReadDoc,
    TextEncoding, ROOT,
};
use automerge_repo::{DocHandle, DocumentId};
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender};
use godot::classes::ClassDb;
use godot::classes::EditorFileSystem;
use godot::classes::EditorInterface;
use godot::classes::Image;
use godot::classes::ProjectSettings;
use godot::classes::ResourceLoader;
use godot::classes::{ConfigFile, DirAccess, FileAccess, ResourceImporter};
use godot::prelude::*;

use crate::godot_parser::{self, GodotScene, TypeOrInstance};
use crate::godot_project_driver::{BranchState, DocHandleType};
use crate::patches::get_changed_files;
use crate::utils::{array_to_heads, heads_to_array, parse_automerge_url};
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

enum VariantValue {
    Variant(Variant),
    ResourcePath(String),
    SubResourceID(String),
    ExtResourceID(String),
}

#[derive(Debug, Clone)]
struct GodotProjectState {
    checked_out_doc_id: DocumentId,
    branches_metadata_doc_id: DocumentId,
}

#[derive(GodotClass)]
#[class(no_init, base=Node)]
pub struct GodotProject {
    base: Base<Node>,
    doc_handles: HashMap<DocumentId, DocHandle>,
    branch_states: HashMap<DocumentId, BranchState>,
    checked_out_branch_state: CheckedOutBranchState,
    project_doc_id: Option<DocumentId>,
    driver: GodotProjectDriver,
    driver_input_tx: UnboundedSender<InputEvent>,
    driver_output_rx: UnboundedReceiver<OutputEvent>,
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

    #[func]
    fn create(
        storage_folder_path: String,
        branches_metadata_doc_id: String, // empty string to create a new project
        checked_out_branch_doc_id: String, // empty string to check out the main branch of the newly created project
        maybe_user_name: String,
    ) -> Gd<Self> {
        println!("rust: CREATE !!!! {:?}", storage_folder_path);

        let (driver_input_tx, driver_input_rx) = futures::channel::mpsc::unbounded();
        let (driver_output_tx, driver_output_rx) = futures::channel::mpsc::unbounded();

        let branches_metadata_doc_id = match DocumentId::from_str(&branches_metadata_doc_id) {
            Ok(doc_id) => Some(doc_id),
            Err(e) => None,
        };

        let driver = GodotProjectDriver::create(storage_folder_path);

        driver.spawn(
            driver_input_rx,
            driver_output_tx,
            branches_metadata_doc_id,
            if maybe_user_name == "" {
                None
            } else {
                Some(maybe_user_name)
            },
        );

        let checked_out_branch_state = match DocumentId::from_str(&checked_out_branch_doc_id) {
            Ok(doc_id) => CheckedOutBranchState::CheckingOut(doc_id),
            Err(_) => CheckedOutBranchState::NothingCheckedOut,
        };

        println!(
            "initial checked out branch state: {:?}",
            checked_out_branch_state
        );

        Gd::from_init_fn(|base| Self {
            base,
            doc_handles: HashMap::new(),
            branch_states: HashMap::new(),
            checked_out_branch_state,
            project_doc_id: None,
            driver,
            driver_input_tx,
            driver_output_rx,
        })
    }

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
        get_changed_files(patches)
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
                dict! {
                    "hash": GString::from(c.hash().to_string()).to_variant(),
                    "user_name": GString::from(c.message().unwrap_or(&String::new())).to_variant(),
                    "timestamp": c.timestamp(),
                }
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
        self.driver_input_tx
            .unbounded_send(InputEvent::CreateBranch { name })
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
                panic!("couldn't checkout branch, branch doc id not found");
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

    // needs to be called every frame to process the internal events
    #[func]
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
            }
        }
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
    fn get_scene_changes_between(
        &self,
        path: String,
        old_heads: PackedStringArray,
        curr_heads: PackedStringArray,
        resource_importer_getter: Callable, // TODO: This is a hack because our CI is not set up to build the bindings custom for our godot engine
    ) -> Dictionary {
        let old_heads = array_to_heads(old_heads);

        let new_heads = array_to_heads(curr_heads);
        let mut current_deps = HashMap::new();
        self._get_scene_changes_between(
            path,
            old_heads,
            new_heads,
            &mut current_deps,
            &resource_importer_getter,
            false,
        )
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

    fn get_varstr_value(&self, prop_value: String) -> VariantValue {
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
                return VariantValue::SubResourceID(id);
            } else if (prop_value.contains("ExtResource(")) {
                return VariantValue::ExtResourceID(id);
            } else {
                // Resource()
                return VariantValue::ResourcePath(id);
            }
        }
        // normal variant string
        return VariantValue::Variant(str_to_var(&prop_value));
    }

    fn get_diff_dict(
        old_path: String,
        new_path: String,
        old_text: String,
        new_text: String,
    ) -> Dictionary {
        let diff = TextDiff::from_lines(&old_text, &new_text);
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

    fn _get_scene_changes_between(
        &self,
        path: String,
        old_heads: Vec<ChangeHash>,
        curr_heads: Vec<ChangeHash>,
        current_deps: &mut HashMap<String, Dictionary>,
        resource_import_getter: &Callable,
        deps_only: bool,
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
        let temp_dir = format!(
            "res://.patchwork/temp_{}_{}",
            old_heads[0].to_string().chars().take(6).collect::<String>(),
            curr_heads[0]
                .to_string()
                .chars()
                .take(6)
                .collect::<String>()
        );
        let patches = checked_out_branch_state.doc_handle.with_doc(|d| {
            d.diff(
                &old_heads,
                &curr_heads,
                TextRepresentation::String(TextEncoding::Utf8CodeUnit),
            )
        });

        // Get old and new content
        let old_content = match self._get_file_at(path.clone(), Some(old_heads.clone())) {
            Some(FileContent::String(s)) => GString::from(s).to_variant(),
            Some(FileContent::Binary(bytes)) => PackedByteArray::from(bytes).to_variant(),
            Some(FileContent::Scene(scene)) => scene.serialize().to_variant(),
            None => Variant::nil(),
        };

        let new_content = match self._get_file_at(path.clone(), Some(curr_heads.clone())) {
            Some(FileContent::String(s)) => GString::from(s).to_variant(),
            Some(FileContent::Binary(bytes)) => PackedByteArray::from(bytes).to_variant(),
            Some(FileContent::Scene(scene)) => scene.serialize().to_variant(),
            None => Variant::nil(),
        };

        let change_type = if old_content.is_nil() {
            "added"
        } else if new_content.is_nil() {
            "deleted"
        } else if old_content == new_content {
            "unchanged"
        } else {
            "modified"
        };
        let has_old = change_type != "added";
        let has_new = change_type != "deleted";
        let old_content_type = old_content.get_type();
        let new_content_type = new_content.get_type();
        let mut result = Dictionary::new();
        let _ = result.insert("path", path.to_variant());
        let _ = result.insert("change_type", change_type);

        let _ = result.insert("old_content", old_content);
        let _ = result.insert("new_content", new_content);
        let import_path = format!("{}.import", path);
        // get the old import file and the new import file
        let old_import_file = self._get_file_at(import_path.clone(), Some(old_heads.clone()));
        let new_import_file = self._get_file_at(import_path.clone(), Some(curr_heads.clone()));
        if let Some(old_import_file) = old_import_file {
            if let FileContent::String(s) = old_import_file {
                let _ = result.insert("old_import_file_content", s);
            }
        }
        if let Some(new_import_file) = new_import_file {
            if let FileContent::String(s) = new_import_file {
                let _ = result.insert("new_import_file_content", s);
            }
        }

        let fn_get_resource = |path: String, result: &mut Dictionary, _is_old: bool| {
            let act_content = if _is_old {
                result.get("old_content")
            } else {
                result.get("new_content")
            };
            if let Some(content) = act_content {
                let new_temp_dir =
                    format!("{}/{}/", &temp_dir, if _is_old { "old" } else { "new" });
                let temp_path = path.replace("res://", &new_temp_dir);
                // append _old or _new to the temp path (i.e. res://thing.<EXT> -> user://temp_123_456/thing_old.<EXT>)
                self._write_variant_to_file(&temp_path, &content);
                // get the import file conetnt
                let import_path = format!("{}.import", path);
                let import_file_content = if _is_old {
                    self._get_file_at(import_path.clone(), Some(old_heads.clone()))
                } else {
                    self._get_file_at(import_path.clone(), Some(curr_heads.clone()))
                };
                if let Some(import_file_content) = import_file_content {
                    if let FileContent::String(import_file_content) = import_file_content {
                        let import_file_content =
                            import_file_content.replace("res://", &new_temp_dir);
                        // regex to replace uid=uid://<...> and uid=uid://<invalid> with uid=uid://<...> and uid=uid://<invalid>
                        let import_file_content = import_file_content
                            .replace(r#"uid=uid://[^\n]+"#, "uid=uid://<invalid>");
                        // write the import file content to the temp path
                        let import_file_path: String = format!("{}.import", temp_path);
                        self._write_variant_to_file(
                            &import_file_path,
                            &import_file_content.to_variant(),
                        );
                        let res = resource_import_getter.call(&[temp_path.to_variant()]);
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
            }
            None
        };
        if change_type != "unchanged" && !path.ends_with(".tscn") && !path.ends_with(".tres") {
            if (old_content_type != VariantType::STRING && new_content_type != VariantType::STRING)
            {
                let _ = result.insert("diff_type", "resource_changed");
                if (has_old) {
                    if let Some(old_resource) = fn_get_resource(path.clone(), &mut result, true) {
                        let _ = result.insert("old_resource", old_resource);
                    }
                }
                if (has_new) {
                    if let Some(new_resource) = fn_get_resource(path.clone(), &mut result, false) {
                        let _ = result.insert("new_resource", new_resource);
                    }
                }
            } else if (old_content_type == VariantType::STRING
                || new_content_type == VariantType::STRING)
                && (old_content_type != VariantType::PACKED_BYTE_ARRAY
                    && new_content_type != VariantType::PACKED_BYTE_ARRAY)
            {
                let old_text = if old_content_type == VariantType::STRING {
                    result.get("old_content").unwrap().to::<String>()
                } else {
                    String::from("")
                };
                let new_text = if new_content_type == VariantType::STRING {
                    result.get("new_content").unwrap().to::<String>()
                } else {
                    String::from("")
                };
                let diff =
                    GodotProject::get_diff_dict(path.clone(), path.clone(), old_text, new_text);
                let _ = result.insert("text_diff", diff);
                let _ = result.insert("diff_type", "text_changed");
            } else {
                let _ = result.insert("diff_type", "file_changed");
            }
        }

        // If it's a scene file, add node changes
        if change_type != "unchanged" && (path.ends_with(".tscn") || path.ends_with(".tres")) {
            let _ = result.insert("diff_type", "scene_changed");
            let mut changed_nodes = Array::new();

            // Get old and new scenes for content comparison
            let mut old_doc = checked_out_branch_state.doc_handle.with_doc(|d| d.clone());
            let mut new_doc = checked_out_branch_state.doc_handle.with_doc(|d| d.clone());

            let old_scene =
                match godot_parser::GodotScene::hydrate_at(&mut old_doc, &path, &old_heads) {
                    Ok(scene) => Some(scene),
                    Err(_) => None,
                };

            let new_scene =
                match godot_parser::GodotScene::hydrate_at(&mut new_doc, &path, &curr_heads) {
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
            let mut changed_ext_resource_paths: HashSet<String> = HashSet::new();
            let mut added_ext_resources: HashSet<String> = HashSet::new();
            let mut deleted_ext_resources: HashSet<String> = HashSet::new();

            let mut changed_sub_resources: HashSet<String> = HashSet::new();
            let mut added_sub_resources: HashSet<String> = HashSet::new();
            let mut deleted_sub_resources: HashSet<String> = HashSet::new();

            let mut changed_node_ids: HashSet<String> = HashSet::new();
            let mut added_node_ids: HashSet<String> = HashSet::new();
            let mut deleted_node_ids: HashSet<String> = HashSet::new();

            for patch in patches {
                match_path(&patch_path, &patch).inspect(
                    |PathWithAction { path, action }| match path.first() {
                        Some((_, Prop::Map(node_id))) => {
                            // hack: only consider nodes where properties changed as changed
                            // this filters out all the parent nodes that don't really change only the child_node_ids change

                            if let Some((_, Prop::Map(key))) = path.last() {
                                if key == "properties" {
                                    changed_node_ids.insert(node_id.clone());
                                }
                            }
                        }
                        None => match action {
                            PatchAction::PutMap {
                                key,
                                value: _,
                                conflict: _,
                            } => {
                                added_node_ids.insert(key.clone());
                            }
                            PatchAction::DeleteMap { key } => {
                                deleted_node_ids.insert(key.clone());
                            }
                            _ => {}
                        },
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
                                }
                            }
                        }
                        None => match action {
                            PatchAction::PutMap {
                                key,
                                value: _,
                                conflict: _,
                            } => {
                                added_ext_resources.insert(key.clone());
                            }
                            PatchAction::DeleteMap { key } => {
                                deleted_ext_resources.insert(key.clone());
                            }
                            _ => {}
                        },
                        _ => {}
                    },
                );

                match_path(&sub_resources_path, &patch).inspect(
                    |PathWithAction { path, action }| match path.first() {
                        Some((_, Prop::Map(sub_id))) => {
                            if let Some((_, Prop::Map(key))) = path.last() {
                                if key != "idx" {
                                    // ignore idx changes
                                    changed_sub_resources.insert(sub_id.clone());
                                }
                            }
                        }
                        None => match action {
                            PatchAction::PutMap {
                                key,
                                value: _,
                                conflict: _,
                            } => {
                                added_sub_resources.insert(key.clone());
                            }
                            PatchAction::DeleteMap { key } => {
                                deleted_sub_resources.insert(key.clone());
                            }
                            _ => {}
                        },
                        _ => {}
                    },
                );
            }
            let mut get_depsfn = |scene: Option<GodotScene>| {
                if let Some(scene) = scene {
                    for (ext_id, ext_resource) in scene.ext_resources.iter() {
                        if current_deps.contains_key(&ext_resource.path) {
                            continue;
                        }
                        let mut ext_resource_content: Dictionary = self._get_scene_changes_between(
                            ext_resource.path.clone(),
                            old_heads.clone(),
                            curr_heads.clone(),
                            current_deps,
                            resource_import_getter,
                            true,
                        );

                        if let Some(change_type) = ext_resource_content.get("change_type") {
                            if change_type.to_string() != "unchanged" {
                                let path = ext_resource.path.clone();
                                changed_ext_resource_paths.insert(path.clone());
                            }
                            if change_type.to_string() == "added" {
                                added_ext_resources.insert(ext_id.clone());
                            } else if change_type.to_string() == "deleted" {
                                deleted_ext_resources.insert(ext_id.clone());
                            } else if change_type.to_string() == "modified" {
                                changed_ext_resources.insert(ext_id.clone());
                            }
                        }
                        current_deps.insert(ext_resource.path.clone(), ext_resource_content);
                    }
                }
            };
            // now, we have to iterate through every ext_resource in the old and new scenes and compare their data by recursively calling this function
            if let Some(old_scene) = old_scene.clone() {
                get_depsfn(Some(old_scene));
            }
            if let Some(new_scene) = new_scene.clone() {
                get_depsfn(Some(new_scene));
            }

            if deps_only {
                return result;
            }
            let fn_get_class_name = |type_or_instance: TypeOrInstance,
                                     scene: &Option<GodotScene>,
                                     content_key: &str| {
                match type_or_instance {
                    TypeOrInstance::Type(type_name) => type_name,
                    TypeOrInstance::Instance(instance_id) => {
                        if let Some(scene) = scene {
                            if let Some(ext_resource) = scene.ext_resources.get(&instance_id) {
                                if let Some(content) = current_deps.get(&ext_resource.path) {
                                    if let Some(old_content) = content.get(content_key) {
                                        return self.get_class_name(old_content.to::<String>());
                                    }
                                }
                            }
                        }
                        String::new()
                    }
                }
            };
            let fn_get_ext_resource_path = |ext_resource_id: String, scene: &Option<GodotScene>| {
                if let Some(scene) = &scene {
                    if let Some(ext_resource) = scene.ext_resources.get(&ext_resource_id) {
                        return Some(ext_resource.path.clone());
                    }
                }
                None
            };

            let fn_get_prop_value =
                |prop_value: VariantValue, scene: &Option<GodotScene>, _is_old: bool| -> Variant {
                    let mut path: Option<String> = None;
                    match prop_value {
                        VariantValue::Variant(variant) => {
                            return variant;
                        }
                        VariantValue::ResourcePath(resource_path) => {
                            path = Some(resource_path);
                        }
                        VariantValue::SubResourceID(sub_resource_id) => {
                            return format!("<SubResource {} changed>", sub_resource_id)
                                .to_variant();
                        }
                        VariantValue::ExtResourceID(ext_resource_id) => {
                            path = fn_get_ext_resource_path(ext_resource_id, scene);
                        }
                    }
                    if let Some(path) = path {
                        // get old_resource or new_resource
                        let diff = current_deps.get(&path);
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
                    }
                    return format!("<ExtResource not found>").to_variant();
                };
            let mut get_changed_prop_dict =
                |prop: String, old_value: VariantValue, new_value: VariantValue| {
                    return dict! {
                        "name": prop.clone(),
                        "change_type": "modified",
                        "old_value": fn_get_prop_value(old_value, &old_scene, true),
                        "new_value": fn_get_prop_value(new_value, &new_scene, false)
                    };
                };
            let mut detect_changed_prop =
                |prop: String,
                 class_name: String,
                 old_prop: Option<String>,
                 new_prop: Option<String>| {
                    let sn_1 = StringName::from(&class_name);
                    let sn_2: StringName = StringName::from(&prop);
                    let default_value = ClassDb::singleton()
                        .class_get_property_default_value(&sn_1, &sn_2)
                        .to_string();
                    let old_prop = old_prop.unwrap_or(default_value.clone());
                    let new_prop = new_prop.unwrap_or(default_value.clone());
                    let old_value = self.get_varstr_value(old_prop.clone());
                    let new_value: VariantValue = self.get_varstr_value(new_prop.clone());
                    match (&old_value, &new_value) {
                        (
                            VariantValue::SubResourceID(sub_resource_id),
                            VariantValue::SubResourceID(new_sub_resource_id),
                        ) => {
                            if changed_sub_resources.contains(sub_resource_id)
                                || changed_sub_resources.contains(new_sub_resource_id)
                            {
                                return Some(get_changed_prop_dict(prop, old_value, new_value));
                            }
                        }
                        (
                            VariantValue::ExtResourceID(ext_resource_id),
                            VariantValue::ExtResourceID(new_ext_resource_id),
                        ) => {
                            if changed_ext_resources.contains(ext_resource_id)
                                || changed_ext_resources.contains(new_ext_resource_id)
                                || added_ext_resources.contains(ext_resource_id)
                                || added_ext_resources.contains(new_ext_resource_id)
                                || deleted_ext_resources.contains(ext_resource_id)
                                || deleted_ext_resources.contains(new_ext_resource_id)
                            {
                                return Some(get_changed_prop_dict(prop, old_value, new_value));
                            } else if (ext_resource_id != new_ext_resource_id) {
                                return Some(get_changed_prop_dict(prop, old_value, new_value));
                            }
                        }
                        (
                            VariantValue::ResourcePath(resource_path),
                            VariantValue::ResourcePath(new_resource_path),
                        ) => {
                            if changed_ext_resource_paths.contains(resource_path)
                                || changed_ext_resource_paths.contains(new_resource_path)
                            {
                                return Some(get_changed_prop_dict(prop, old_value, new_value));
                            } else if (resource_path != new_resource_path) {
                                return Some(get_changed_prop_dict(prop, old_value, new_value));
                            }
                        }
                        (
                            VariantValue::Variant(old_variant),
                            VariantValue::Variant(new_variant),
                        ) => {
                            if old_variant != new_variant {
                                return Some(get_changed_prop_dict(prop, old_value, new_value));
                            }
                        }
                        _ => {
                            return Some(get_changed_prop_dict(prop, old_value, new_value));
                        }
                    }
                    None
                };

            // Handle changed nodes
            for node_id in changed_node_ids {
                let mut changed_props: Dictionary = Dictionary::new();
                if !added_node_ids.contains(&node_id) && !deleted_node_ids.contains(&node_id) {
                    let mut node_info = Dictionary::new();
                    node_info.insert("change_type", "modified");

                    if let Some(scene) = &new_scene {
                        node_info.insert("node_path", scene.get_node_path(&node_id));
                    }
                    let mut old_props = Dictionary::new();
                    let mut new_props = Dictionary::new();
                    let mut old_type: TypeOrInstance = TypeOrInstance::Type(String::new());
                    let mut new_type: TypeOrInstance = TypeOrInstance::Type(String::new());
                    // Get old and new node content
                    if let Some(old_scene) = &old_scene {
                        if let Some(old_node) = old_scene.nodes.get(&node_id) {
                            old_type = old_node.type_or_instance.clone();
                        }
                        if let Some(content) = old_scene.get_node_content(&node_id) {
                            if let Some(props) = content.get("properties") {
                                old_props = props.to::<Dictionary>();
                            }
                            node_info.insert("old_content", content);
                        }
                    }

                    if let Some(new_scene) = &new_scene {
                        if let Some(new_node) = new_scene.nodes.get(&node_id) {
                            new_type = new_node.type_or_instance.clone();
                        }
                        if let Some(content) = new_scene.get_node_content(&node_id) {
                            if let Some(props) = content.get("properties") {
                                new_props = props.to::<Dictionary>();
                            }
                            node_info.insert("new_content", content);
                        }
                    }
                    // old_type and new_type
                    let old_class_name = fn_get_class_name(old_type, &old_scene, "old_content");
                    let new_class_name = fn_get_class_name(new_type, &new_scene, "new_content");

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
                            if let Some(changed_prop) = detect_changed_prop(
                                prop.clone(),
                                new_class_name.clone(),
                                old_prop,
                                new_prop,
                            ) {
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
            // Handle added nodes
            for node_id in added_node_ids {
                let mut node_info = Dictionary::new();
                node_info.insert("change_type", "added");

                if let Some(scene) = &new_scene {
                    node_info.insert("node_path", scene.get_node_path(&node_id));
                    if let Some(content) = scene.get_node_content(&node_id) {
                        node_info.insert("new_content", content);
                    }
                }

                changed_nodes.push(&node_info.to_variant());
            }

            // Handle deleted nodes
            for node_id in deleted_node_ids {
                let mut node_info = Dictionary::new();
                node_info.insert("change_type", "removed");

                if let Some(scene) = &old_scene {
                    node_info.insert("node_path", scene.get_node_path(&node_id));
                    if let Some(content) = scene.get_node_content(&node_id) {
                        node_info.insert("old_content", content);
                    }
                }

                changed_nodes.push(&node_info.to_variant());
            }

            result.insert("changed_nodes", changed_nodes);
        }
        result
    }
}

#[godot_api]
impl INode for GodotProject {
    fn init(_base: Base<Node>) -> Self {
        let storage_folder_path =
            String::from(ProjectSettings::singleton().globalize_path("res://.patchwork"));
        let mut project_config_file = ConfigFile::new_gd();
        project_config_file.load("res://patchwork.cfg");
        let mut user_config_file = ConfigFile::new_gd();
        user_config_file.load("user://patchwork.cfg");
        let branches_metadata_doc_id = project_config_file
            .get_value("patchwork", "branches_metadata_doc_id")
            .to_string();
        let checked_out_branch_doc_id = project_config_file
            .get_value("patchwork", "checked_out_branch_doc_id")
            .to_string();
        let maybe_user_name = user_config_file
            .get_value("patchwork", "user_name")
            .to_string();
        println!("rust: INIT !!!! {:?}", storage_folder_path);

        let (driver_input_tx, driver_input_rx) = futures::channel::mpsc::unbounded();
        let (driver_output_tx, driver_output_rx) = futures::channel::mpsc::unbounded();

        let branches_metadata_doc_id = match DocumentId::from_str(&branches_metadata_doc_id) {
            Ok(doc_id) => Some(doc_id),
            Err(e) => None,
        };

        let driver = GodotProjectDriver::create(storage_folder_path);

        driver.spawn(
            driver_input_rx,
            driver_output_tx,
            branches_metadata_doc_id,
            if maybe_user_name == "" {
                None
            } else {
                Some(maybe_user_name)
            },
        );

        let checked_out_branch_state = match DocumentId::from_str(&checked_out_branch_doc_id) {
            Ok(doc_id) => CheckedOutBranchState::CheckingOut(doc_id),
            Err(_) => CheckedOutBranchState::NothingCheckedOut,
        };

        println!(
            "initial checked out branch state: {:?}",
            checked_out_branch_state
        );

        Self {
            base: _base,
            doc_handles: HashMap::new(),
            branch_states: HashMap::new(),
            checked_out_branch_state,
            project_doc_id: None,
            driver,
            driver_input_tx,
            driver_output_rx,
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
