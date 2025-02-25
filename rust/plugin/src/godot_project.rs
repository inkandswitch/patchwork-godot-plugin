use ::safer_ffi::prelude::*;
use std::collections::HashSet;
use std::env::var;
use std::{
    collections::HashMap,
    future::Future,
    str::FromStr,
    sync::{
        mpsc::{Receiver, Sender},
        Arc, Mutex,
    },
};

use automerge::{
    patches::TextRepresentation, transaction::Transactable, Automerge, Change, ChangeHash, ObjType,
    PatchLog, ReadDoc, TextEncoding, ROOT,
};
use automerge_repo::{tokio::FsStorage, ConnDirection, DocHandle, DocumentId, Repo, RepoHandle};
use autosurgeon::{bytes, hydrate, reconcile, Hydrate, Reconcile};
use futures::{
    channel::mpsc::{UnboundedReceiver, UnboundedSender},
    executor::block_on,
    FutureExt, StreamExt,
};
use godot::sys::interface_fn;
use godot::{obj, prelude::*};
use std::ffi::c_void;
use std::ops::Deref;
use std::os::raw::c_char;
use tokio::{net::TcpStream, runtime::Runtime};

use crate::godot_project_driver::{BranchState, DocHandleType};
use crate::utils::{parse_automerge_url, print_branch_state};
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

#[derive(Clone)]
enum SyncEvent {
    NewDoc {
        doc_id: DocumentId,
        doc_handle: DocHandle,
    },
    DocChanged {
        doc_id: DocumentId,
    },
    CheckedOutBranch {
        doc_id: DocumentId,
    },
}

#[derive(Debug, Clone, Reconcile, Hydrate, PartialEq)]
pub enum StringOrPackedByteArray {
    String(String),
    Binary(Vec<u8>),
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
    fn branches_changed(branches: Array<Dictionary>);

    #[signal]
    fn shutdown_completed();

    #[func]
    // hack: pass in empty string to create a new doc
    // godot rust doens't seem to support Option args
    fn create(maybe_branches_metadata_doc_id: String, maybe_user_name: String) -> Gd<Self> {
        let (driver_input_tx, driver_input_rx) = futures::channel::mpsc::unbounded();
        let (driver_output_tx, driver_output_rx) = futures::channel::mpsc::unbounded();

        let branches_metadata_doc_id = match DocumentId::from_str(&maybe_branches_metadata_doc_id) {
            Ok(doc_id) => Some(doc_id),
            Err(e) => None,
        };

        let driver = GodotProjectDriver::create();

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

        Gd::from_init_fn(|base| Self {
            base,
            doc_handles: HashMap::new(),
            branch_states: HashMap::new(),
            checked_out_branch_state: CheckedOutBranchState::NothingCheckedOut,
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
                .synced_heads
                .iter()
                .map(|h| GString::from(h.to_string()))
                .collect::<PackedStringArray>(),
            _ => PackedStringArray::new(),
        }
    }

    #[func]
    fn list_all_files(&self) -> PackedStringArray {
        let doc = match &self.get_checked_out_branch_state() {
            Some(branch_state) => branch_state.doc_handle.with_doc(|d| d.clone()),
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

    fn _get_file(&self, path: String) -> Option<StringOrPackedByteArray> {
        let doc = match &self.get_checked_out_branch_state() {
            Some(branch_state) => branch_state.doc_handle.with_doc(|d| d.clone()),
            _ => return None,
        };

        let files = doc.get(ROOT, "files").unwrap().unwrap().1;
        // does the file exist?
        let file_entry = match doc.get(files, &path) {
            Ok(Some((automerge::Value::Object(ObjType::Map), file_entry))) => file_entry,
            _ => return None,
        };

        // try to read file as text
        match doc.get(&file_entry, "content") {
            Ok(Some((automerge::Value::Object(ObjType::Text), content))) => {
                match doc.text(content) {
                    Ok(text) => return Some(StringOrPackedByteArray::String(text.to_string())),
                    Err(_) => {}
                }
            }
            _ => {}
        }

        // ... otherwise try to read as linked binary doc
        doc.get_string(&file_entry, "url")
            .and_then(|url| parse_automerge_url(&url))
            .and_then(|doc_id| self.doc_handles.get(&doc_id))
            .and_then(|doc_handle| {
                doc_handle.with_doc(|d| match d.get(ROOT, "content") {
                    Ok(Some((value, _))) if value.is_bytes() => {
                        Some(StringOrPackedByteArray::Binary(value.into_bytes().unwrap()))
                    }
                    Ok(Some((value, _))) if value.is_str() => Some(
                        StringOrPackedByteArray::String(value.into_string().unwrap()),
                    ),
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
    #[func]
    fn get_file(&self, path: String) -> Variant {
        match self._get_file(path) {
            Some(StringOrPackedByteArray::String(s)) => GString::from(s).to_variant(),
            Some(StringOrPackedByteArray::Binary(bytes)) => {
                PackedByteArray::from(bytes).to_variant()
            }
            None => Variant::nil(),
        }
    }

    #[func]
    fn get_changed_files(&self) -> PackedStringArray {
        let checked_out_branch_state = match &self.get_checked_out_branch_state() {
            Some(branch_state) => branch_state.clone(),
            None => return PackedStringArray::new(),
        };

        // ignore main, doesn't have changed files
        if checked_out_branch_state.is_main {
            return PackedStringArray::new();
        }

        let patches = checked_out_branch_state.doc_handle.with_doc(|d| {
            d.diff(
                &checked_out_branch_state.forked_at,
                &checked_out_branch_state.synced_heads,
                TextRepresentation::String(TextEncoding::Utf8CodeUnit),
            )
        });

        let mut changed_files = HashSet::new();

        // log all patches
        for patch in patches.clone() {
            let first_key = match patch.path.get(0) {
                Some((_, prop)) => match prop {
                    automerge::Prop::Map(string) => string,
                    _ => continue,
                },
                _ => continue,
            };

            // get second key
            let second_key = match patch.path.get(1) {
                Some((_, prop)) => match prop {
                    automerge::Prop::Map(string) => string,
                    _ => continue,
                },
                _ => continue,
            };

            if first_key == "files" {
                changed_files.insert(second_key.to_string());
            }

            println!("changed files: {:?}", changed_files);
        }

        return changed_files
            .iter()
            .map(|s| GString::from(s))
            .collect::<PackedStringArray>();
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
    fn save_files_at(
        &self,
        files: Dictionary, /*  Record<String, Variant> */
        heads: PackedStringArray,
    ) {
        let heads: Vec<ChangeHash> = heads
            .to_vec()
            .iter()
            .filter_map(|h| ChangeHash::from_str(h.to_string().as_str()).ok())
            .collect();

        match &self.get_checked_out_branch_state() {
            Some(branch_state) => {
                self._save_files(branch_state.doc_handle.clone(), files, Some(heads))
            }
            None => println!("couldn't save files, no checked out branch"),
        }
    }

    #[func]
    fn save_files(&self, files: Dictionary) {
        match &self.get_checked_out_branch_state() {
            Some(branch_state) => self._save_files(branch_state.doc_handle.clone(), files, None),
            None => println!("couldn't save files, no checked out branch"),
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
        let changed_files: Vec<(String, StringOrPackedByteArray)> = files
            .iter_shared()
            .filter_map(|(path, content)| match content.get_type() {
                VariantType::STRING => {
                    let content = String::from(content.to::<GString>());

                    if let Some(StringOrPackedByteArray::String(stored_content)) =
                        self._get_file(path.to_string())
                    {
                        if stored_content == content {
                            println!("file {:?} is already up to date", path.to_string());
                            return None;
                        }
                    }

                    Some((path.to_string(), StringOrPackedByteArray::String(content)))
                }
                VariantType::PACKED_BYTE_ARRAY => {
                    let content = content.to::<PackedByteArray>().to_vec();

                    if let Some(StringOrPackedByteArray::Binary(stored_content)) =
                        self._get_file(path.to_string())
                    {
                        if stored_content == content {
                            println!("file {:?} is already up to date", path.to_string());
                            return None;
                        }
                    }

                    Some((path.to_string(), StringOrPackedByteArray::Binary(content)))
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

        self.checked_out_branch_state =
            CheckedOutBranchState::CheckingOut(main_branch.doc_handle.document_id());
    }

    #[func]
    fn create_branch(&self, name: String) {
        self.driver_input_tx
            .unbounded_send(InputEvent::CreateBranch { name })
            .unwrap();
    }

    #[func]
    fn checkout_branch(&mut self, branch_doc_id: String) {
        let branch_doc_state = self
            .branch_states
            .get(&DocumentId::from_str(&branch_doc_id).unwrap())
            .cloned();

        match branch_doc_state {
            Some(branch_state) => {
                // if it's loaded check out immediately
                if branch_state.synced_heads == branch_state.doc_handle.with_doc(|d| d.get_heads())
                {
                    self.checked_out_branch_state =
                        CheckedOutBranchState::CheckedOut(branch_state.doc_handle.document_id());

                    self.base_mut().emit_signal(
                        "checked_out_branch",
                        &[branch_state
                            .doc_handle
                            .document_id()
                            .to_string()
                            .to_variant()],
                    );

                // ... otherwise wait in checking out state
                } else {
                    self.checked_out_branch_state =
                        CheckedOutBranchState::CheckingOut(branch_state.doc_handle.document_id());
                }
            }
            None => {
                println!("couldn't checkout branch, no branch state found");
            }
        }
    }

    #[func]
    fn get_branches(&self) -> Array<Dictionary> /* { name: String, id: String }[] */ {
        self.branch_states
            .values()
            .map(|branch_state| {
                dict! {
                    "name": branch_state.name.clone(),
                    "id": branch_state.doc_handle.document_id().to_string(),
                    "is_main": branch_state.is_main,
                }
            })
            .collect()
    }

    #[func]
    fn get_checked_out_branch(&self) -> Variant /* {name: String, id: String, is_main: bool}? */ {
        match &self.get_checked_out_branch_state() {
            Some(branch_state) => dict! {
                "name": branch_state.name.clone(),
                "id": branch_state.doc_handle.document_id().to_string(),
                "is_main": branch_state.is_main,
            }
            .to_variant(),
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
        let checked_out_branch_doc_handle = match &self.get_checked_out_branch_state() {
            Some(branch_state) => branch_state.doc_handle.clone(),
            None => {
                return Variant::nil();
            }
        };

        checked_out_branch_doc_handle.with_doc(|checked_out_doc| {
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
        })
    }

    #[func]
    fn get_all_entity_ids(&self) -> PackedStringArray {
        let checked_out_branch_doc_handle = match &self.get_checked_out_branch_state() {
            Some(branch_state) => branch_state.doc_handle.clone(),
            None => return PackedStringArray::new(),
        };

        checked_out_branch_doc_handle.with_doc(|d| {
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
                            "rust: NewBinaryDocHandle {} {} changes",
                            doc_handle.document_id(),
                            doc_handle.with_doc(|d| d.get_heads().len())
                        );
                    }

                    self.doc_handles
                        .insert(doc_handle.document_id(), doc_handle.clone());
                }
                OutputEvent::BranchStateChanged { branch_state } => {
                    let is_new_branch = self
                        .branch_states
                        .get(&branch_state.doc_handle.document_id())
                        .is_none();

                    self.branch_states
                        .insert(branch_state.doc_handle.document_id(), branch_state.clone());

                    if is_new_branch {
                        let branches = self
                            .get_branches()
                            .iter_shared()
                            .map(|branch| branch.to_variant())
                            .collect::<Array<Variant>>()
                            .to_variant();

                        self.base_mut().emit_signal("branches_changed", &[branches]);
                    }

                    print_branch_state("BranchStateChanged", &branch_state);
                    println!(
                        "checked_out_branch_state: {:?}",
                        self.checked_out_branch_state
                    );

                    let mut active_branch_doc_id: Option<DocumentId> = None;
                    let mut checking_out_new_branch = false;

                    match &self.checked_out_branch_state {
                        CheckedOutBranchState::NothingCheckedOut => {
                            // check out main branch if we haven't checked out anything yet
                            if branch_state.is_main {
                                checking_out_new_branch = true;
                                self.checked_out_branch_state = CheckedOutBranchState::CheckingOut(
                                    branch_state.doc_handle.document_id(),
                                );
                                active_branch_doc_id = Some(branch_state.doc_handle.document_id());
                            }
                        }
                        CheckedOutBranchState::CheckingOut(branch_doc_id) => {
                            active_branch_doc_id = Some(branch_doc_id.clone());
                            checking_out_new_branch = true;
                        }
                        CheckedOutBranchState::CheckedOut(branch_doc_id) => {
                            active_branch_doc_id = Some(branch_doc_id.clone());
                        }
                    }

                    // only trigger update if checked out branch is fully synced
                    if let Some(active_branch_doc_id) = active_branch_doc_id {
                        if branch_state.doc_handle.document_id() == active_branch_doc_id
                            && branch_state.synced_heads
                                == branch_state.doc_handle.with_doc(|d| d.get_heads())
                        {
                            if checking_out_new_branch {
                                println!("rust: checked out new branch");
                                self.base_mut().emit_signal(
                                    "checked_out_branch",
                                    &[branch_state
                                        .doc_handle
                                        .document_id()
                                        .to_string()
                                        .to_variant()
                                        .to_variant()],
                                );

                                self.checked_out_branch_state = CheckedOutBranchState::CheckedOut(
                                    branch_state.doc_handle.document_id(),
                                );
                            } else {
                                println!("rust: files changed");
                                self.base_mut().emit_signal("files_changed", &[]);
                            }
                        }
                    }
                }
                OutputEvent::Initialized { project_doc_id } => {
                    self.project_doc_id = Some(project_doc_id);
                }

                OutputEvent::CompletedShutdown => {
                    println!("rust: CompletedShutdown event");
                    self.base_mut().emit_signal("shutdown_completed", &[]);
                }
            }
        }
    }

    // Helper functions

    fn get_checked_out_branch_state(&self) -> Option<BranchState> {
        match &self.checked_out_branch_state {
            CheckedOutBranchState::CheckedOut(document_id) => {
                self.branch_states.get(document_id).cloned()
            }
            _ => None,
        }
    }
}

fn handle_changes(handle: DocHandle) -> impl futures::Stream<Item = Vec<automerge::Patch>> + Send {
    futures::stream::unfold(handle, |doc_handle| async {
        let heads_before = doc_handle.with_doc(|d| d.get_heads());
        let _ = doc_handle.changed().await;
        let heads_after = doc_handle.with_doc(|d| d.get_heads());
        let diff = doc_handle.with_doc(|d| {
            d.diff(
                &heads_before,
                &heads_after,
                TextRepresentation::String(TextEncoding::Utf8CodeUnit),
            )
        });

        Some((diff, doc_handle))
    })
}

pub(crate) fn is_branch_doc(branch_doc_handle: &DocHandle) -> bool {
    branch_doc_handle.with_doc(|d| match d.get_obj_id(ROOT, "files") {
        Some(_) => true,
        None => false,
    })
}

pub(crate) fn vec_string_to_packed_string_array(vec: &Vec<String>) -> PackedStringArray {
    vec.iter()
        .map(|s| GString::from(s))
        .collect::<PackedStringArray>()
}

pub(crate) fn packed_string_array_to_vec_string(array: &PackedStringArray) -> Vec<String> {
    array.to_vec().iter().map(|s| String::from(s)).collect()
}

fn branches_to_gd(branches: &HashMap<String, Branch>) -> Array<Dictionary> {
    branches
        .iter()
        .map(|(_, branch)| {
            dict! {
                "name": branch.name.clone(),
                "id": branch.id.clone(),
            }
        })
        .collect::<Array<Dictionary>>()
}
