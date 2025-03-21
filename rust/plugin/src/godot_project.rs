use ::safer_ffi::prelude::*;
use automerge::op_tree::B;
use automerge::{Automerge, ObjId, Patch, PatchAction, Prop};
use autosurgeon::{Hydrate, Reconcile};
use futures::io::empty;
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
pub struct Branch {
    pub name: String,
    pub id: String,
    pub is_merged: bool,
    pub forked_at: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FileContent {
    String(String),
    Binary(Vec<u8>),
    Scene(GodotScene),
}

#[derive(Debug, Clone)]
struct BranchUnionIds {
    primary_branch_doc_id: DocumentId,
    secondary_branch_doc_ids: Vec<DocumentId>,
}

struct BranchUnion {
    ids: BranchUnionIds,
    doc: Automerge,
    synced_heads: Vec<ChangeHash>,
    forked_at: Vec<ChangeHash>,
    primary_branch_state: BranchState,
}

#[derive(Debug, Clone, PartialEq)]
enum BranchUnionState {
    Invalid,
    Loading,
    Loaded,
}

#[derive(Debug, Clone)]
enum CheckedOutBranchState {
    NothingCheckedOut,
    CheckingOut(BranchUnionIds),
    CheckedOut(BranchUnionIds),
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
            Ok(doc_id) => CheckedOutBranchState::CheckingOut(BranchUnionIds {
                primary_branch_doc_id: doc_id,
                secondary_branch_doc_ids: Vec::new(),
            }),
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
        match &self.get_checked_out_branch_union() {
            Some(branch_union) => branch_union
                .doc
                .get_heads()
                .iter()
                .map(|h| GString::from(h.to_string()))
                .collect::<PackedStringArray>(),
            _ => PackedStringArray::new(),
        }
    }

    #[func]
    fn list_all_files(&self) -> PackedStringArray {
        let doc = match &self.get_checked_out_branch_union() {
            Some(branch_union) => branch_union.doc.clone(),
            _ => return PackedStringArray::new(),
        };

        let files = match doc.get_obj_id(ROOT, "files") {
            Some(files) => files,
            _ => {
                return PackedStringArray::new();
            }
        };
        doc.keys(files)
            .collect::<Vec<String>>()
            .iter()
            .map(|s| GString::from(s))
            .collect::<PackedStringArray>()
    }

    fn _get_file_at(&self, path: String, heads: Option<Vec<ChangeHash>>) -> Option<FileContent> {
        let branch_union = match self.get_checked_out_branch_union() {
            Some(branch_union) => branch_union,
            None => return None,
        };

        let heads = match heads {
            Some(heads) => heads,
            None => branch_union.synced_heads.clone(),
        };
        let mut doc = branch_union.doc.clone();

        let files = doc.get_at(ROOT, "files", &heads).unwrap().unwrap().1;
        // does the file exist?
        let file_entry = match doc.get_at(&files, &path, &heads) {
            Ok(Some((automerge::Value::Object(ObjType::Map), file_entry))) => file_entry,
            _ => return None,
        };

        // try to read file as scene
        let structured_content = doc
            .get_at(&file_entry, "structured_content", &heads)
            .unwrap()
            .map(|(value, _)| value);

        if structured_content.is_some() {
            return GodotScene::hydrate_at(&mut doc, &path, &heads)
                .ok()
                .map(|scene| FileContent::Scene(scene));
        }

        // try to read file as text
        let content = doc.get_at(&file_entry, "content", &heads);

        match content {
            Ok(Some((automerge::Value::Object(ObjType::Text), content))) => {
                match doc.text_at(content, &heads) {
                    Ok(text) => return Some(FileContent::String(text.to_string())),
                    Err(e) => println!("failed to read text file {:?}: {:?}", path, e),
                }
            }
            _ => match doc.get_string_at(&file_entry, "content", &heads) {
                Some(s) => return Some(FileContent::String(s)),
                _ => {}
            },
        }

        // ... otherwise try to read as linked binary doc
        doc.get_string_at(&file_entry, "url", &heads)
            .and_then(|url| parse_automerge_url(&url))
            .and_then(|doc_id| self.doc_handles.get(&doc_id))
            .and_then(|doc_handle| {
                doc_handle.with_doc(|d| match d.get(ROOT, "content") {
                    Ok(Some((value, _))) if value.is_bytes() => {
                        Some(FileContent::Binary(value.into_bytes().unwrap()))
                    }
                    Ok(Some((value, _))) if value.is_str() => {
                        Some(FileContent::String(value.into_string().unwrap()))
                    }
                    _ => {
                        println!(
                            "failed to read binary doc {:?} {:?} {:?}",
                            path,
                            doc_handle.document_id(),
                            doc_handle.with_doc(|d| d.get_heads())
                        );
                        None
                    }
                })
            })
    }

    fn _get_file(&self, path: String) -> Option<FileContent> {
        self._get_file_at(path, None)
    }

    #[func]
    fn get_file(&self, path: String) -> Variant {
        match self._get_file(path.clone()) {
            Some(FileContent::String(s)) => GString::from(s).to_variant(),
            Some(FileContent::Binary(bytes)) => PackedByteArray::from(bytes).to_variant(),
            Some(FileContent::Scene(scene)) => GString::from(scene.serialize()).to_variant(),
            None => Variant::nil(),
        }
    }

    #[func]
    fn get_node_changes(&self, path: String) -> Array<Variant> {
        let mut checked_out_branch_union = match self.get_checked_out_branch_union() {
            Some(branch_union) => branch_union,
            None => return Array::new(),
        };

        let heads = checked_out_branch_union.forked_at.clone();
        let curr_heads = checked_out_branch_union.synced_heads.clone();

        let patches = checked_out_branch_union.doc.diff(
            &heads,
            &curr_heads,
            TextRepresentation::String(TextEncoding::Utf8CodeUnit),
        );

        let scene =
            godot_parser::GodotScene::hydrate(&mut checked_out_branch_union.doc, &path).unwrap();

        let patch_path = Vec::from([
            Prop::Map(String::from("files")),
            Prop::Map(String::from(path)),
            Prop::Map(String::from("structured_content")),
            Prop::Map(String::from("nodes")),
        ]);

        let mut changed_node_ids: HashSet<String> = HashSet::new();
        let mut added_node_ids: HashSet<String> = HashSet::new();

        for patch in patches {
            match_path(&patch_path, &patch).inspect(|PathWithAction { path, action }| {
                match path.first() {
                    Some((_, Prop::Map(node_id))) => {
                        changed_node_ids.insert(node_id.clone());
                    }
                    None => {
                        if let PatchAction::PutMap {
                            key,
                            value: _,
                            conflict: _,
                        } = action
                        {
                            added_node_ids.insert(key.clone());
                        }
                    }
                    _ => {}
                }
            });
        }

        let mut result: Array<Variant> = Array::new();

        for node_id in changed_node_ids {
            // we need to filter out added nodes because they are already in the added_nodes array
            if !added_node_ids.contains(&node_id) {
                result.push(
                    &dict! {
                        "node_path": scene.get_node_path(&node_id),
                        "type": "changed"
                    }
                    .to_variant(),
                );
            }
        }

        for node_id in added_node_ids {
            result.push(
                &dict! {
                    "node_path": scene.get_node_path(&node_id),
                    "type": "added"
                }
                .to_variant(),
            );
        }

        result
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

        let checked_out_branch_union = match self.get_checked_out_branch_union() {
            Some(branch_union) => branch_union,
            None => return PackedStringArray::new(),
        };

        let curr_heads = if curr_heads.len() == 0 {
            checked_out_branch_union.synced_heads.clone()
        } else {
            array_to_heads(curr_heads)
        };

        let patches = checked_out_branch_union.doc.diff(
            &heads,
            &curr_heads,
            TextRepresentation::String(TextEncoding::Utf8CodeUnit),
        );
        get_changed_files(patches)
    }

    #[func]
    fn get_changed_file_content_between(
        &self,
        old_heads: PackedStringArray,
        curr_heads: PackedStringArray,
    ) -> Dictionary {
        // dict looks like:
        // {
        //     files: [
        //         {
        //             path: "path/to/file",
        //             change: "modified",
        //             old_content: "old content",
        //             new_content: "new content"
        //         },
        //         {
        //             path: "path/to/another/file",
        //             change: "added",
        //             old_content: null,
        //             new_content: "new content"
        //         },
        //         {
        //             path: "path/to/another/file",
        //             change: "deleted",
        //             old_content: "old content",
        //             new_content: null
        //         }
        //     ]
        // }
        let heads = array_to_heads(old_heads);
        // if curr_heads is empty, we're comparing against the current heads

        let checked_out_branch_union = match self.get_checked_out_branch_union() {
            Some(branch_union) => branch_union,
            None => return Dictionary::new(),
        };

        let curr_heads = if curr_heads.len() == 0 {
            checked_out_branch_union.synced_heads.clone()
        } else {
            array_to_heads(curr_heads)
        };

        let patches = checked_out_branch_union.doc.diff(
            &heads,
            &curr_heads,
            TextRepresentation::String(TextEncoding::Utf8CodeUnit),
        );
        let changed_files = get_changed_files_vec(patches);
        // get the file entry at the old_heads, then the current heads
        let mut changed_files_dict = Dictionary::new();
        let mut changed_files_dict_files_array = Array::new();
        for file in changed_files {
            let file_entry = match self._get_file_at(file.clone(), Some(heads.clone())) {
                Some(FileContent::String(s)) => GString::from(s).to_variant(),
                Some(FileContent::Binary(bytes)) => PackedByteArray::from(bytes).to_variant(),
                Some(FileContent::Scene(scene)) => scene.serialize().to_variant(),
                None => Variant::nil(),
            };
            let file_entry_current = match self._get_file_at(file.clone(), Some(curr_heads.clone()))
            {
                Some(FileContent::String(s)) => GString::from(s).to_variant(),
                Some(FileContent::Binary(bytes)) => PackedByteArray::from(bytes).to_variant(),
                Some(FileContent::Scene(scene)) => scene.serialize().to_variant(),
                None => Variant::nil(),
            };

            let change_type = if file_entry.is_nil() {
                "added"
            } else if file_entry_current.is_nil() {
                "deleted"
            } else {
                "modified"
            };
            changed_files_dict_files_array.push(&dict! {
                "path": file,
                "change": change_type,
                "old_content": file_entry,
                "new_content": file_entry_current,
            })
        }
        let _ = changed_files_dict.insert("files", changed_files_dict_files_array);
        // iterate over the changed_files, find

        changed_files_dict
    }

    #[func]
    fn get_changed_files_on_current_branch(&self) -> PackedStringArray {
        let checked_out_branch_union = match self.get_checked_out_branch_union() {
            Some(branch_union) => branch_union,
            None => return PackedStringArray::new(),
        };

        let patches = checked_out_branch_union.doc.diff(
            &checked_out_branch_union.forked_at,
            &checked_out_branch_union.synced_heads,
            TextRepresentation::String(TextEncoding::Utf8CodeUnit),
        );
        get_changed_files(patches)
    }

    #[func]
    fn get_changes(&self) -> Array<Dictionary> /* String[]  */ {
        let checked_out_branch_doc = match &self.get_checked_out_branch_union() {
            Some(branch_union) => branch_union.doc.clone(),
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
    fn save_files_at(
        &self,
        files: Dictionary, /*  Record<String, Variant> */
        heads: PackedStringArray,
    ) {
        let heads = array_to_heads(heads);

        match &self.get_checked_out_branch_union() {
            Some(branch_union) => self._save_files(
                branch_union.primary_branch_state.doc_handle.clone(),
                files,
                Some(heads),
            ),
            None => panic!("couldn't save files, no checked out branch"),
        }
    }

    #[func]
    fn save_files(&self, files: Dictionary) {
        match &self.get_checked_out_branch_union() {
            Some(branch_union) => self._save_files(
                branch_union.primary_branch_state.doc_handle.clone(),
                files,
                None,
            ),
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
        // we filter the files here because godot sends us indiscriminately all the files in the project
        // we only want to save the files that have actually changed
        let changed_files: Vec<(String, FileContent)> = files
            .iter_shared()
            .filter_map(|(path, content)| match content.get_type() {
                VariantType::STRING => {
                    let content = String::from(content.to::<GString>());

                    // save scene files as structured data
                    if path.to_string().ends_with(".tscn") {
                        let scene = match godot_parser::parse_scene(&content) {
                            Ok(scene) => scene,
                            Err(error) => {
                                println!("RUST: error parsing scene {}: {:?}", path, error);
                                return None;
                            }
                        };

                        if let Some(FileContent::Scene(stored_scene)) =
                            self._get_file(path.to_string())
                        {
                            if stored_scene == scene {
                                println!("file {:?} is already up to date", path.to_string());
                                return None;
                            }
                        }

                        return Some((path.to_string(), FileContent::Scene(scene)));
                    }

                    if let Some(FileContent::String(stored_content)) =
                        self._get_file(path.to_string())
                    {
                        if stored_content == content {
                            println!("file {:?} is already up to date", path.to_string());
                            return None;
                        }
                    }

                    Some((path.to_string(), FileContent::String(content)))
                }
                VariantType::PACKED_BYTE_ARRAY => {
                    let content = content.to::<PackedByteArray>().to_vec();

                    if let Some(FileContent::Binary(stored_content)) =
                        self._get_file(path.to_string())
                    {
                        if stored_content == content {
                            println!("file {:?} is already up to date", path.to_string());
                            return None;
                        }
                    }

                    Some((path.to_string(), FileContent::Binary(content)))
                }
                _ => panic!("invalid content type"),
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

        self.checked_out_branch_state = CheckedOutBranchState::CheckingOut(BranchUnionIds {
            primary_branch_doc_id: main_branch.doc_handle.document_id(),
            secondary_branch_doc_ids: Vec::new(),
        });
    }

    #[func]
    fn create_branch(&mut self, name: String) {
        self.driver_input_tx
            .unbounded_send(InputEvent::CreateBranch { name })
            .unwrap();

        self.checked_out_branch_state = CheckedOutBranchState::NothingCheckedOut;
    }

    #[func]
    fn checkout_branch(
        &mut self,
        primary_branch_doc_id: String,
        secondary_branch_doc_ids: Array<Variant>,
    ) {
        let seconday_doc_ids = secondary_branch_doc_ids
            .iter_shared()
            .map(|id| id.to::<GString>().to_string())
            .collect::<Vec<String>>();

        println!(
            "rust: checkout_branch {:?} {:?}",
            primary_branch_doc_id, seconday_doc_ids
        );

        let branch_union = BranchUnionIds {
            primary_branch_doc_id: DocumentId::from_str(&primary_branch_doc_id).unwrap(),
            secondary_branch_doc_ids: seconday_doc_ids
                .iter()
                .map(|id| DocumentId::from_str(&id).unwrap())
                .collect(),
        };

        match self.get_branch_union_state(&branch_union) {
            BranchUnionState::Invalid => {
                println!("couldn't checkout branch, some branch doc ids not found");
                return;
            }
            BranchUnionState::Loading => {
                self.checked_out_branch_state = CheckedOutBranchState::CheckingOut(branch_union);
            }
            BranchUnionState::Loaded => {
                self.checked_out_branch_state = CheckedOutBranchState::CheckedOut(branch_union);
                self.base_mut().emit_signal(
                    "checked_out_branch",
                    &[primary_branch_doc_id.to_string().to_variant()],
                );
            }
        }
    }

    #[func]
    fn get_branches(&self) -> Array<Dictionary> /* { name: String, id: String }[] */ {
        let mut branches = self
            .branch_states
            .values()
            .map(|branch_state| {
                dict! {
                    "name": branch_state.name.clone(),
                    "id": branch_state.doc_handle.document_id().to_string(),
                    "is_main": branch_state.is_main,
                    "forked_at": heads_to_array(branch_state.forked_at.clone()),

                    // we shouldn't have branches that don't have any changes but sometimes
                    // the branch docs are not synced correctly so this flag is used in the UI to
                    // indicate that the branch is not loaded and prevent users from checking it out
                    "is_not_loaded": branch_state.doc_handle.with_doc(|d| d.get_heads().len() == 0),
                }
            })
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
        match &self.get_checked_out_branch_union() {
            Some(branch_union) => dict! {
                "name": branch_union.primary_branch_state.name.clone(),
                "id": branch_union.primary_branch_state.doc_handle.document_id().to_string(),
                "is_main": branch_union.primary_branch_state.is_main,
                "forked_at": heads_to_array(branch_union.primary_branch_state.forked_at.clone()),
            }
            .to_variant(),
            None => Variant::nil(),
        }
    }

    // State api

    #[func]
    fn set_entity_state(&self, entity_id: String, prop: String, value: Variant) {
        let checked_out_doc_handle = match &self.get_checked_out_branch_union() {
            Some(branch_union) => branch_union.primary_branch_state.doc_handle.clone(),
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
        let checked_out_branch_union = match self.get_checked_out_branch_union() {
            Some(branch_union) => branch_union,
            None => {
                return Variant::nil();
            }
        };

        let checked_out_doc = checked_out_branch_union.doc;

        let state = match checked_out_doc.get_obj_id(ROOT, "state") {
            Some(id) => id,
            None => {
                println!("invalid document, no state property");
                return Variant::nil();
            }
        };

        let entity = match checked_out_doc.get_obj_id(state, entity_id) {
            Some(id) => id,
            None => {
                return Variant::nil();
            }
        };

        return match checked_out_doc.get_variant(entity, prop) {
            Some(value) => value,
            None => Variant::nil(),
        };
    }

    #[func]
    fn get_all_entity_ids(&self) -> PackedStringArray {
        let checked_out_branch_union = match self.get_checked_out_branch_union() {
            Some(branch_union) => branch_union,
            None => return PackedStringArray::new(),
        };

        let checked_out_doc = checked_out_branch_union.doc;

        let state = match checked_out_doc.get_obj_id(ROOT, "state") {
            Some(id) => id,
            None => return PackedStringArray::new(),
        };

        checked_out_doc
            .keys(&state)
            .map(|k| GString::from(k))
            .collect()
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

                    let mut active_branch_union: Option<BranchUnionIds> = None;
                    let mut checking_out_new_branch = false;

                    match &self.checked_out_branch_state {
                        CheckedOutBranchState::NothingCheckedOut => {
                            // check out main branch if we haven't checked out anything yet
                            if branch_state.is_main {
                                checking_out_new_branch = true;

                                let branch_union = BranchUnionIds {
                                    primary_branch_doc_id: branch_state.doc_handle.document_id(),
                                    secondary_branch_doc_ids: Vec::new(),
                                };

                                self.checked_out_branch_state =
                                    CheckedOutBranchState::CheckingOut(branch_union.clone());

                                active_branch_union = Some(branch_union);
                            }
                        }
                        CheckedOutBranchState::CheckingOut(branch_union) => {
                            active_branch_union = Some(branch_union.clone());
                            checking_out_new_branch = true;
                        }
                        CheckedOutBranchState::CheckedOut(branch_union) => {
                            active_branch_union = Some(branch_union.clone());
                        }
                    }

                    // only trigger update if checked out branch is fully synced
                    if let Some(active_branch_union) = active_branch_union {
                        if self.get_branch_union_state(&active_branch_union)
                            == BranchUnionState::Loaded
                        {
                            if checking_out_new_branch {
                                println!(
                                    "rust: TRIGGER checked out new branch: {}",
                                    branch_state.name
                                );

                                self.checked_out_branch_state =
                                    CheckedOutBranchState::CheckedOut(active_branch_union);

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
                        CheckedOutBranchState::CheckingOut(BranchUnionIds {
                            primary_branch_doc_id: branch_doc_id,
                            secondary_branch_doc_ids: Vec::new(),
                        });
                }

                OutputEvent::CompletedShutdown => {
                    println!("rust: CompletedShutdown event");
                    self.base_mut().emit_signal("shutdown_completed", &[]);
                }
            }
        }
    }

    // Helper functions

    fn get_checked_out_branch_union(&self) -> Option<BranchUnion> {
        match &self.checked_out_branch_state {
            CheckedOutBranchState::CheckedOut(branch_union) => {
                let primary_branch_state = self
                    .branch_states
                    .get(&branch_union.primary_branch_doc_id)
                    .unwrap()
                    .clone();

                let mut synced_heads = primary_branch_state.synced_heads.clone();
                let mut doc = primary_branch_state.doc_handle.with_doc(|d| d.clone());
                let mut forked_at = primary_branch_state.forked_at.clone();

                for secondary_branch_doc_id in branch_union.secondary_branch_doc_ids.iter() {
                    let secondary_branch_state = self
                        .branch_states
                        .get(secondary_branch_doc_id)
                        .unwrap()
                        .clone();

                    forked_at = secondary_branch_state.synced_heads.clone();

                    let mut secondary_doc =
                        secondary_branch_state.doc_handle.with_doc(|d| d.clone());
                    synced_heads.extend(secondary_branch_state.synced_heads.clone());
                    doc.merge(&mut secondary_doc);
                }

                Some(BranchUnion {
                    synced_heads,
                    ids: branch_union.clone(),
                    doc,
                    forked_at,
                    primary_branch_state: primary_branch_state.clone(),
                })
            }
            _ => {
                println!(
                    "tried to get checked out branch union but nothing is checked out: {:?}",
                    self.checked_out_branch_state
                );
                None
            }
        }
    }

    fn get_branch_union_state(&self, branch_union: &BranchUnionIds) -> BranchUnionState {
        let mut all_branch_doc_ids: Vec<DocumentId> = branch_union
            .secondary_branch_doc_ids
            .iter()
            .cloned()
            .collect();

        all_branch_doc_ids.push(branch_union.primary_branch_doc_id.clone());

        let all_branch_doc_states: Vec<BranchState> = all_branch_doc_ids
            .iter()
            .filter_map(|id| self.branch_states.get(id).cloned())
            .collect();

        if all_branch_doc_states.len() != all_branch_doc_ids.len() {
            println!("couldn't checkout branch, some branch doc ids not found");
            return BranchUnionState::Invalid;
        }

        let are_all_branch_doc_states_loaded = all_branch_doc_states.iter().all(|branch_state| {
            return branch_state.doc_handle.with_doc(|d| {
                d.get_heads().len() > 0 && d.get_heads() == branch_state.synced_heads
            });
        });

        if are_all_branch_doc_states_loaded {
            return BranchUnionState::Loaded;
        }

        BranchUnionState::Loading
    }

    #[func]
    fn get_scene_changes_between(
        &self,
        path: String,
        old_heads: PackedStringArray,
        curr_heads: PackedStringArray,
    ) -> Dictionary {
        let old_heads = array_to_heads(old_heads);
        let checked_out_branch_union = match self.get_checked_out_branch_union() {
            Some(branch_union) => branch_union,
            None => return Dictionary::new(),
        };

        let curr_heads = if curr_heads.len() == 0 {
            checked_out_branch_union.synced_heads.clone()
        } else {
            array_to_heads(curr_heads)
        };

        let patches = checked_out_branch_union.doc.diff(
            &old_heads,
            &curr_heads,
            TextRepresentation::String(TextEncoding::Utf8CodeUnit),
        );

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
            let mut old_doc = checked_out_branch_union.doc.clone();
            let mut new_doc = checked_out_branch_union.doc.clone();

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
                            changed_node_ids.insert(node_id.clone());
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
