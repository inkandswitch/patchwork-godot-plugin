use ::safer_ffi::prelude::*;
use automerge::op_tree::B;
use automerge::{Automerge, ObjId, Patch, PatchAction, Prop};
use autosurgeon::{Hydrate, Reconcile};
use futures::io::empty;
use safer_ffi::layout::OpaqueKind::T;
use std::collections::HashSet;
use std::{collections::HashMap, str::FromStr};

use automerge::{
    patches::TextRepresentation, transaction::Transactable, ChangeHash, ObjType, ReadDoc,
    TextEncoding, ROOT,
};
use automerge_repo::{DocHandle, DocumentId};
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender};
use godot::prelude::*;

use crate::godot_parser::{self, GodotScene};
use crate::godot_project_driver::{BranchState, DocHandleType};
use crate::patches::get_changed_files;
use crate::patches::get_changed_files_vec;
use crate::utils::{array_to_heads, heads_to_array, parse_automerge_url};
use crate::{
    doc_utils::SimpleDocReader,
    godot_project_driver::{GodotProjectDriver, InputEvent, OutputEvent},
};

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
                })
                .unwrap();

            files.insert(path, linked_file_content);
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
        match &self.get_checked_out_branch_state() {
            Some(branch_state) => branch_state_to_dict(branch_state).to_variant(),
            None => Variant::nil(),
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
    fn merge_branch(&mut self, branch_id: String) {
        let branch_doc_id = match DocumentId::from_str(&branch_id) {
            Ok(id) => id,
            Err(e) => {
                println!("invalid branch doc id: {:?}", e);
                return;
            }
        };

        self.driver_input_tx
            .unbounded_send(InputEvent::MergeBranch {
                branch_doc_handle: self.doc_handles.get(&branch_doc_id).unwrap().clone(),
            })
            .unwrap();

        let main_branch = self
            .branch_states
            .values()
            .find(|branch_state| branch_state.is_main)
            .unwrap();

        self.checked_out_branch_state =
            CheckedOutBranchState::CheckingOut(main_branch.doc_handle.document_id())
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

    #[func]
    fn get_branches(&self) -> Array<Dictionary> /* { name: String, id: String }[] */ {
        let mut branches = self
            .branch_states
            .values()
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
    fn process(&mut self) {
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
    ) -> Dictionary {
        let old_heads = array_to_heads(old_heads);
        let checked_out_branch_state = match self.get_checked_out_branch_state() {
            Some(branch_state) => branch_state,
            None => return Dictionary::new(),
        };

        let curr_heads = if curr_heads.len() == 0 {
            checked_out_branch_state.synced_heads.clone()
        } else {
            array_to_heads(curr_heads)
        };

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
        } else {
            "modified"
        };

        let mut result = Dictionary::new();
        let _ = result.insert("change_type", change_type);
        let _ = result.insert("old_content", old_content);
        let _ = result.insert("new_content", new_content);

        // If it's a scene file, add node changes
        if path.ends_with(".tscn") {
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
            }

            // Handle changed nodes
            for node_id in changed_node_ids {
                if !added_node_ids.contains(&node_id) && !deleted_node_ids.contains(&node_id) {
                    let mut node_info = Dictionary::new();
                    node_info.insert("type", "changed");

                    if let Some(scene) = &new_scene {
                        node_info.insert("node_path", scene.get_node_path(&node_id));
                    }

                    // Get old and new node content
                    if let Some(old_scene) = &old_scene {
                        if let Some(content) = old_scene.get_node_content(&node_id) {
                            node_info.insert("old_content", content);
                        }
                    }

                    if let Some(new_scene) = &new_scene {
                        if let Some(content) = new_scene.get_node_content(&node_id) {
                            node_info.insert("new_content", content);
                        }
                    }

                    changed_nodes.push(&node_info.to_variant());
                }
            }

            // Handle added nodes
            for node_id in added_node_ids {
                let mut node_info = Dictionary::new();
                node_info.insert("type", "added");

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
                node_info.insert("type", "deleted");

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
