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

use crate::godot_project::StringOrPackedByteArray::PackedByteArray;
use crate::utils::parse_automerge_url;
use crate::{
    doc_handle_map::DocHandleMap,
    doc_utils::SimpleDocReader,
    godot_project_driver::{DriverInputEvent, DriverOutputEvent, GodotProjectDriver},
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
    PackedByteArray(Vec<u8>),
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
    branches: HashMap<String, Branch>,
    doc_handles: HashMap<DocumentId, DocHandle>,
    checked_out_branch_doc_handle: Option<DocHandle>,
    driver: GodotProjectDriver,
    driver_input_tx: UnboundedSender<DriverInputEvent>,
    driver_output_rx: UnboundedReceiver<DriverOutputEvent>,
}

const SERVER_URL: &str = "104.131.179.247:8080";
static SIGNAL_BRANCHES_CHANGED: std::sync::LazyLock<std::ffi::CString> =
    std::sync::LazyLock::new(|| std::ffi::CString::new("branches_changed").unwrap());
static SIGNAL_FILES_CHANGED: std::sync::LazyLock<std::ffi::CString> =
    std::sync::LazyLock::new(|| std::ffi::CString::new("files_changed").unwrap());
static SIGNAL_CHECKED_OUT_BRANCH: std::sync::LazyLock<std::ffi::CString> =
    std::sync::LazyLock::new(|| std::ffi::CString::new("checked_out_branch").unwrap());
static SIGNAL_FILE_CHANGED: std::sync::LazyLock<std::ffi::CString> =
    std::sync::LazyLock::new(|| std::ffi::CString::new("file_changed").unwrap());
static SIGNAL_STARTED: std::sync::LazyLock<std::ffi::CString> =
    std::sync::LazyLock::new(|| std::ffi::CString::new("started").unwrap());
static SIGNAL_INITIALIZED: std::sync::LazyLock<std::ffi::CString> =
    std::sync::LazyLock::new(|| std::ffi::CString::new("initialized").unwrap());
// convert a slice of strings to a slice of char * strings (e.g. *const std::os::raw::c_char)
fn to_c_strs(strings: &[&str]) -> Vec<std::ffi::CString> {
    strings
        .iter()
        .map(|s| std::ffi::CString::new(*s).unwrap())
        .collect()
}
fn strings_to_c_strs(strings: &[String]) -> Vec<std::ffi::CString> {
    strings
        .iter()
        .map(|s| std::ffi::CString::new(s.as_str()).unwrap())
        .collect()
}

// convert a HashMap to a slice of char * strings; e.g. ["key1", "value1", "key2", "value2"]
fn to_c_strs_from_dict(dict: &HashMap<&str, String>) -> Vec<std::ffi::CString> {
    let mut c_strs = Vec::new();
    for (key, value) in dict.iter() {
        c_strs.push(std::ffi::CString::new(*key).unwrap());
        c_strs.push(std::ffi::CString::new(value.as_str()).unwrap());
    }
    c_strs
}

// convert a slice of std::ffi::CString to a slice of *const std::os::raw::c_char
fn to_char_stars(c_strs: &[std::ffi::CString]) -> Vec<*const std::os::raw::c_char> {
    c_strs.iter().map(|s| s.as_ptr()).collect()
}

#[godot_api]
impl GodotProject {
    #[signal]
    fn started();

    #[signal]
    fn initialized();

    #[signal]
    fn checked_out_branch(branch_id: String);

    #[signal]
    fn files_changed();

    #[signal]
    fn branches_changed();

    #[func]
    // hack: pass in empty string to create a new doc
    // godot rust doens't seem to support Option args
    fn create(maybe_branches_metadata_doc_id: String) -> Gd<Self> {
        let (driver_input_tx, driver_input_rx) = futures::channel::mpsc::unbounded();
        let (driver_output_tx, driver_output_rx) = futures::channel::mpsc::unbounded();

        let driver = GodotProjectDriver::create();

        driver.spawn(driver_input_rx, driver_output_tx);

        // @AI simplify
        let branches_metadata_doc_id = match DocumentId::from_str(&maybe_branches_metadata_doc_id) {
            Ok(doc_id) => Some(doc_id),
            Err(e) => None,
        };

        driver_input_tx
            .unbounded_send(DriverInputEvent::InitBranchesMetadataDoc {
                doc_id: branches_metadata_doc_id,
            })
            .unwrap();

        Gd::from_init_fn(|base| Self {
            base,
            branches: HashMap::new(),
            doc_handles: HashMap::new(),
            checked_out_branch_doc_handle: None,
            driver,
            driver_input_tx,
            driver_output_rx,
        })
    }

    fn get_checked_out_branch_handle(&self) -> Option<DocHandle> {
        match self.checked_out_branch_doc_handle.clone() {
            Some(doc_handle) => Some(doc_handle),
            None => {
                println!("warning: tried to access checked out doc when no branch was checked out");
                return None;
            }
        }
    }

    fn get_checked_out_branch_doc(&self) -> Option<Automerge> {
        self.get_checked_out_branch_handle()
            .map(|doc_handle| doc_handle.with_doc(|d| d.clone()))
    }

    // PUBLIC API

    #[func]
    fn stop(&self) {
        // todo
    }

    #[func]
    fn get_doc_id(&self) -> String {
        todo!("not implemented");
        // self.branches_metadata_doc_id.to_string()
    }

    #[func]
    fn get_heads(&self) -> PackedStringArray /* String[] */ {
        todo!("not implemented");
        // self.get_checked_out_doc_handle().with_doc(|d| {
        //     d.get_heads()
        //     .to_vec()
        //     .iter()
        //     .map(|h| h.to_string())
        //     .collect::<Vec<String>>()
        // })
    }

    #[func]
    fn list_all_files(&self) -> PackedStringArray {
        let doc = match self.get_checked_out_branch_doc() {
            Some(doc) => doc,
            None => return PackedStringArray::new(),
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
        let doc = match self.get_checked_out_branch_doc() {
            Some(doc) => doc,
            None => return None,
        };

        let files = doc.get(ROOT, "files").unwrap().unwrap().1;
        // does the file exist?
        let file_entry = match doc.get(files, path) {
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
                doc_handle.with_doc(|d| {
                    Some(StringOrPackedByteArray::PackedByteArray(
                        d.get_bytes(ROOT, "content").unwrap(),
                    ))
                })
            })
    }
    #[func]
    fn get_file(&self, path: String) -> Variant {
        match self._get_file(path) {
            Some(StringOrPackedByteArray::String(s)) => GString::from(s).to_variant(),
            Some(StringOrPackedByteArray::PackedByteArray(bytes)) => bytes.to_variant(),
            None => Variant::nil(),
        }
    }

    #[func]
    fn get_file_at(&self, path: String, heads: PackedStringArray /* String[] */) -> String /* String? */
    {
        todo!("not implemented");
        // self.get_checked_out_doc_handle().with_doc(|doc| {

        // let heads: Vec<ChangeHash> = heads
        //     .iter()
        //     .map(|h| ChangeHash::from_str(h.to_string().as_str()).unwrap())
        //     .collect();

        // let files = doc.get(ROOT, "files").unwrap().unwrap().1;

        // return match doc.get_at(files, path, &heads) {
        //     Ok(Some((value, _))) => Some(value.into_string().unwrap_or_default()),
        //     _ => None,
        // };

        // })
    }

    #[func]
    fn get_changes(&self) -> PackedStringArray /* String[]  */ {
        let checked_out_branch_doc = match self.get_checked_out_branch_doc() {
            Some(doc) => doc,
            None => return PackedStringArray::new(),
        };

        checked_out_branch_doc
            .get_changes(&[])
            .to_vec()
            .iter()
            .map(|c| GString::from(c.hash().to_string()))
            .collect::<PackedStringArray>()
    }

    fn _save_file(
        &self,
        path: String,
        heads: Option<Vec<ChangeHash>>,
        content: StringOrPackedByteArray,
    ) {
        // ignore if file is already up to date // ignore if file is already up to date
        if let Some(stored_content) = self._get_file(path.clone()) {
            if stored_content == content {
                println!("file {:?} is already up to date", path.clone());
                return;
            }
        }

        self.driver_input_tx
            .unbounded_send(DriverInputEvent::SaveFile {
                path,
                heads,
                content,
            })
            .unwrap();
        // todo: this
        // // ignore if file is already up to date
        // if let Some(stored_content) = self.get_file(path.clone()) {
        //     if stored_content == content {
        //         println!("file {:?} is already up to date", path.clone());
        //         return;
        //     }
        // }

        // self.get_checked_out_doc_handle()
        // .with_doc_mut(|d| {
        //         let mut tx = match heads {
        //             Some(heads) => {
        //                 d.transaction_at(PatchLog::inactive(TextRepresentation::String(TextEncoding::Utf8CodeUnit)), &heads)
        //             },
        //             None => {
        //                 d.transaction()
        //             }
        //         };

        //         let files = match tx.get(ROOT, "files") {
        //             Ok(Some((automerge::Value::Object(ObjType::Map), files))) => files,
        //             _ => panic!("Invalid project doc, doesn't have files map"),
        //         };

        //         match content {
        //             StringOrPackedByteArray::String(content) => {
        //                 println!("write string {:}", path);

        //                 // get existing file url or create new one
        //                 let file_entry = match tx.get(&files, &path) {
        //                     Ok(Some((automerge::Value::Object(ObjType::Map), file_entry))) => file_entry,
        //                     _ => tx.put_object(files, &path, ObjType::Map).unwrap()
        //                 };

        //                 // delete url in file entry if it previously had one
        //                 if let Ok(Some((_, _))) = tx.get(&file_entry, "url") {
        //                     let _ = tx.delete(&file_entry, "url");
        //                 }

        //                 // either get existing text or create new text
        //                 let content_key = match tx.get(&file_entry, "content") {
        //                     Ok(Some((automerge::Value::Object(ObjType::Text), content))) => content,
        //                     _ => tx.put_object(&file_entry, "content", ObjType::Text).unwrap(),
        //                 };
        //                 let _ = tx.update_text(&content_key, &content);
        //             },
        //             StringOrPackedByteArray::PackedByteArray(content) => {
        //                 println!("write binary {:}", path);

        //                 // create content doc
        //                 let content_doc_id = self.create_doc(|d| {
        //                     let mut tx = d.transaction();
        //                     let _ = tx.put(ROOT, "content", content.to_vec());
        //                     tx.commit();
        //                 });

        //                 // write url to content doc into project doc
        //                 let file_entry = tx.put_object(files, path, ObjType::Map);
        //                 let _ = tx.put(file_entry.unwrap(), "url", format!("automerge:{}", content_doc_id));
        //             },
        //         }

        //         tx.commit();
        //     });
    }

    #[func]
    fn save_file(&self, path: String, content: Variant) {
        let content = match content.get_type() {
            VariantType::STRING => StringOrPackedByteArray::String(content.to_string()),
            VariantType::PACKED_BYTE_ARRAY => StringOrPackedByteArray::PackedByteArray(
                content.to::<godot::builtin::PackedByteArray>().to_vec(),
            ),
            _ => {
                println!("invalid content type");
                return;
            }
        };

        self._save_file(path, None, content);
    }

    #[func]
    fn merge_branch(&self, branch_id: String) {
        let branch_doc_id = match DocumentId::from_str(&branch_id) {
            Ok(id) => id,
            Err(e) => {
                println!("invalid branch doc id: {:?}", e);
                return;
            }
        };

        self.driver_input_tx
            .unbounded_send(DriverInputEvent::MergeBranch {
                branch_doc_handle: self.doc_handles.get(&branch_doc_id).unwrap().clone(),
            })
            .unwrap();
    }

    #[func]
    fn create_branch(&self, name: String) {
        self.driver_input_tx
            .unbounded_send(DriverInputEvent::CreateBranch { name })
            .unwrap();
        // let mut branches_metadata = self.get_branches_metadata_doc();

        // let main_doc_id = DocumentId::from_str(&branches_metadata.main_doc_id).unwrap();
        // let new_doc_id = self.clone_doc(main_doc_id);

        // branches_metadata.branches.insert(
        //     new_doc_id.to_string(),
        //     Branch {
        //         is_merged: false,
        //         name,
        //         id: new_doc_id.to_string(),
        //     },
        // );

        // self.get_branches_metadata_doc_handle().with_doc_mut(|d| {
        //     let mut tx = d.transaction();
        //     reconcile(&mut tx, branches_metadata).unwrap();
        //     tx.commit();
        // });

        // new_doc_id.to_string()
    }

    // checkout branch in a separate thread
    // ensures that all linked docs are loaded before checking out the branch
    // todo: emit a signal when the branch is checked out
    //
    // current workaround is to call get_checked_out_branch_id every frame and check if has changed in GDScript

    #[func]
    fn checkout_branch(&mut self, branch_doc_id: String) {
        let branch_doc_id = match DocumentId::from_str(&branch_doc_id) {
            Ok(id) => id,
            Err(e) => {
                println!("invalid branch doc id: {:?}", e);
                return;
            }
        };

        self.driver_input_tx
            .unbounded_send(DriverInputEvent::CheckoutBranch {
                branch_doc_handle: self.doc_handles.get(&branch_doc_id).unwrap().clone(),
            })
            .unwrap();
    }

    #[func]
    fn get_branches(&self) -> Array<Dictionary> /* { name: String, id: String }[] */ {
        self.branches
            .values()
            .map(|branch| {
                dict! {
                    "name": branch.name.clone(),
                    "id": branch.id.clone()
                }
            })
            .collect::<Array<Dictionary>>()
    }

    #[func]
    fn get_checked_out_branch_id(&self) -> String {
        match self.get_checked_out_branch_handle() {
            Some(doc) => doc.document_id().to_string(),
            None => return String::new(),
        }
    }

    // State api

    fn set_state_int(&self, entity_id: String, prop: String, value: i64) {
        todo!("not implemented");
        // // let checked_out_doc_handle = self.get_checked_out_doc_handle();

        // checked_out_doc_handle.with_doc_mut(|d| {
        //     let mut tx = d.transaction();
        //     let state = match tx.get_obj_id(ROOT, "state") {
        //         Some(id) => id,
        //         _ => {
        //             println!("failed to load state");
        //             return
        //         }
        //     };

        //     match tx.get_obj_id(&state, &entity_id) {
        //         Some(id) => {
        //             let _ = tx.put(id, prop, value);
        //         },

        //         None => {
        //             match tx.put_object(state, &entity_id, ObjType::Map) {
        //                 Ok(id) => {
        //                     let _ = tx.put(id, prop, value);
        //                 },
        //                 Err(e) => {
        //                     println!("failed to create state object: {:?}", e);
        //                 }
        //             }
        //         }
        //     }

        //     tx.commit();
        // });
    }

    fn get_state_int(&self, entity_id: String, prop: String) -> Option<i64> {
        todo!("not implemented");

        //     self.get_checked_out_doc_handle().with_doc(|checked_out_doc| {

        //    let state  = match checked_out_doc.get_obj_id(ROOT, "state") {
        //         Some(id) => id,
        //         None => {
        //             println!("invalid document, no state property");
        //             return None
        //         }
        //     };

        //    let entity_id_clone = entity_id.clone();
        //    let entity  = match checked_out_doc.get_obj_id(state, entity_id) {
        //         Some(id) => id,
        //         None => {
        //             println!("entity {:?} does not exist", &entity_id_clone);
        //             return None
        //         }
        //     };

        //     return match checked_out_doc.get_int(entity, prop) {
        //         Some(value) => Some(value),
        //         None =>  None

        //     };

        // })
    }

    // these functions below should be extracted into a separate SyncRepo class

    // SYNC

    // needs to be called every frame to process the internal events
    #[func]
    fn process(&mut self) {
        while let Ok(Some(event)) = self.driver_output_rx.try_next() {
            match event {
                DriverOutputEvent::DocHandleChanged { doc_handle } => {
                    println!(
                        "rust: DocHandleChanged event for doc {}",
                        doc_handle.document_id()
                    );
                    self.doc_handles
                        .insert(doc_handle.document_id(), doc_handle.clone());
                }
                DriverOutputEvent::BranchesUpdated { branches } => {
                    self.branches = branches;
                    // (self.signal_callback)(self.signal_user_data, SIGNAL_BRANCHES_CHANGED.as_ptr(), std::ptr::null(), 0);
                    self.base_mut().emit_signal("branches_changed", &[]);
                }
                DriverOutputEvent::CheckedOutBranch { branch_doc_handle } => {
                    println!(
                        "rust: CheckedOutBranch event for doc {}",
                        branch_doc_handle.document_id()
                    );
                    self.checked_out_branch_doc_handle = Some(branch_doc_handle.clone());
                    let doc_id_c_str =
                        std::ffi::CString::new(format!("{}", &branch_doc_handle.document_id()))
                            .unwrap();
                    self.base_mut().emit_signal(
                        "checked_out_branch",
                        &[branch_doc_handle.document_id().to_string().to_variant()],
                    );
                }
                DriverOutputEvent::Initialized {
                    branches,
                    checked_out_branch_doc_handle,
                } => {
                    println!("rust: Initialized event");
                    self.branches = branches;
                    self.checked_out_branch_doc_handle =
                        Some(checked_out_branch_doc_handle.clone());
                    self.base_mut().emit_signal("initialized", &[]);
                }
            }
        }

        // // Process all pending sync events
        // while let Ok(event) = self.sync_event_receiver.try_recv() {
        //     match event {
        //         // this is internal, we don't pass this back to process
        //         SyncEvent::NewDoc { doc_id: _doc_id, doc_handle: _doc_handle } => {}
        //         SyncEvent::DocChanged { doc_id } => {
        //             println!("doc changed event {:?} {:?}", doc_id, self.checked_out_doc_id);
        //             // Check if branches metadata doc changed
        //             if doc_id == self.branches_metadata_doc_id  {
        //                 (self.signal_callback)(self.signal_user_data, BRANCHES_CHANGED.as_ptr(), std::ptr::null(), 0);
        //             } else if doc_id == self.checked_out_doc_id {
        //                 (self.signal_callback)(self.signal_user_data, SIGNAL_FILES_CHANGED.as_ptr(), std::ptr::null(), 0);
        //             }
        //         }

        //         SyncEvent::CheckedOutBranch { doc_id} => {
        //             self.checked_out_doc_id = doc_id.clone();

        //             let doc_id_c_str = std::ffi::CString::new(format!("{}", doc_id)).unwrap();
        //             (self.signal_callback)(self.signal_user_data, SIGNAL_CHECKED_OUT_BRANCH.as_ptr(), &doc_id_c_str.as_ptr(), 1);
        //         }
        //     }
        // }
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
                automerge::patches::TextRepresentation::String(TextEncoding::Utf8CodeUnit),
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
