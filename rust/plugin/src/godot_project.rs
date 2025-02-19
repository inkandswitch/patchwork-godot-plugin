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

use crate::utils::parse_automerge_url;
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
struct GodotProjectState {
    checked_out_doc_id: DocumentId,
    branches_metadata_doc_id: DocumentId,
}

#[derive(GodotClass)]
#[class(no_init, base=Node)]
pub struct GodotProject {
    base: Base<Node>,
    doc_handles: HashMap<DocumentId, DocHandle>,
    checked_out_branch_doc_handle: Option<DocHandle>,
    branches_metadata_doc_handle: Option<DocHandle>,
    driver: GodotProjectDriver,
    driver_input_tx: UnboundedSender<InputEvent>,
    driver_output_rx: UnboundedReceiver<OutputEvent>,
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
    fn branches_changed(branches: Array<Dictionary>);

    #[signal]
    fn shutdown_completed();

    #[func]
    // hack: pass in empty string to create a new doc
    // godot rust doens't seem to support Option args
    fn create(maybe_branches_metadata_doc_id: String) -> Gd<Self> {
        let (driver_input_tx, driver_input_rx) = futures::channel::mpsc::unbounded();
        let (driver_output_tx, driver_output_rx) = futures::channel::mpsc::unbounded();

        let branches_metadata_doc_id = match DocumentId::from_str(&maybe_branches_metadata_doc_id) {
            Ok(doc_id) => Some(doc_id),
            Err(e) => None,
        };

        let driver = GodotProjectDriver::create();

        driver.spawn(driver_input_rx, driver_output_tx, branches_metadata_doc_id);

        Gd::from_init_fn(|base| Self {
            base,
            doc_handles: HashMap::new(),
            checked_out_branch_doc_handle: None,
            branches_metadata_doc_handle: None,
            driver,
            driver_input_tx,
            driver_output_rx,
        })
    }

    fn get_branches_metadata_doc_handle(&self) -> Option<DocHandle> {
        match self.branches_metadata_doc_handle.clone() {
            Some(doc_handle) => Some(doc_handle),
            None => {
                println!("warning: tried to access branches metadata doc handle when no branch was checked out");
                return None;
            }
        }
    }

    fn get_branches_metadata(&self) -> Option<BranchesMetadataDoc> {
        self.get_branches_metadata_doc_handle()
            .map(|doc_handle| doc_handle.with_doc(|d| hydrate(d).unwrap()))
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
    fn shutdown(&self) {
        self.driver_input_tx
            .unbounded_send(InputEvent::StartShutdown)
            .unwrap();
    }

    #[func]
    fn get_doc_id(&self) -> String {
        match self.get_branches_metadata_doc_handle() {
            Some(doc_handle) => doc_handle.document_id().to_string(),
            None => String::new(),
        }
    }

    #[func]
    fn get_heads(&self) -> PackedStringArray /* String[] */ {
        match self.get_checked_out_branch_handle() {
            Some(doc_handle) => doc_handle.with_doc(|d| {
                d.get_heads()
                    .iter()
                    .map(|h| GString::from(h.to_string()))
                    .collect::<PackedStringArray>()
            }),
            None => PackedStringArray::new(),
        }
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
    fn get_diff(&self) -> PackedStringArray {
        let branch_doc_handle = match self.get_checked_out_branch_handle() {
            Some(doc_handle) => doc_handle,
            None => return PackedStringArray::new(),
        };

        let branch = match self.get_branches_metadata() {
            Some(branches) => match branches
                .branches
                .get(&branch_doc_handle.document_id().to_string())
            {
                Some(branch) => branch.clone(),
                None => return PackedStringArray::new(),
            },
            None => return PackedStringArray::new(),
        };

        // ignore main, doesn't have a diff
        if branch.forked_at.is_empty() {
            return PackedStringArray::new();
        }

        let forked_at: Vec<ChangeHash> = branch
            .forked_at
            .iter()
            .map(|h| ChangeHash::from_str(h.as_str()).unwrap())
            .collect();

        let branch_heads = branch_doc_handle.with_doc(|d| d.get_heads());

        let patches = branch_doc_handle.with_doc(|d| {
            d.diff(
                &forked_at,
                &branch_heads,
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

        self._save_files(files, Some(heads));
    }

    #[func]
    fn save_files(&self, files: Dictionary) {
        self._save_files(files, None);
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
                heads,
                files: changed_files,
            })
            .unwrap();
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
            .unbounded_send(InputEvent::MergeBranch {
                branch_doc_handle: self.doc_handles.get(&branch_doc_id).unwrap().clone(),
            })
            .unwrap();
    }

    #[func]
    fn create_branch(&self, name: String) {
        self.driver_input_tx
            .unbounded_send(InputEvent::CreateBranch { name })
            .unwrap();
    }

    // checkout branch in a separate thread
    // ensures that all linked docs are loaded before checking out the branch
    // todo: emit a signal when the branch is checked out
    //
    // current workaround is to call get_checked_out_branch_id every frame and check if has changed in GDScript

    #[func]
    fn checkout_branch(&mut self, branch_doc_id: String) {
        let maybe_branch_doc_handle =
            DocumentId::from_str(&branch_doc_id).map(|id| self.doc_handles.get(&id));

        let branch_doc_handle = match maybe_branch_doc_handle {
            Ok(Some(doc_handle)) => doc_handle.clone(),
            _ => {
                println!("invalid branch doc id: {:?}", maybe_branch_doc_handle);
                return;
            }
        };

        self.driver_input_tx
            .unbounded_send(InputEvent::CheckoutBranch {
                branch_doc_handle: branch_doc_handle.clone(),
            })
            .unwrap();
    }

    #[func]
    fn get_branches(&self) -> Array<Dictionary> /* { name: String, id: String }[] */ {
        match self.get_branches_metadata() {
            Some(branches_metadata) => {
                println!("get branches {:?}", branches_metadata.branches);
                branches_to_gd(&branches_metadata.branches)
            }
            None => Array::new(),
        }
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
                OutputEvent::FilesChanged => {
                    println!("rust: FilesChanged event");
                    self.base_mut().emit_signal("files_changed", &[]);
                }

                OutputEvent::NewDocHandle { doc_handle } => {
                    println!(
                        "rust: DocHandleChanged event for doc {}",
                        doc_handle.document_id()
                    );
                    self.doc_handles
                        .insert(doc_handle.document_id(), doc_handle.clone());
                }
                OutputEvent::BranchesChanged { branches } => {
                    let branches_gd = branches_to_gd(&branches);

                    println!("RUST: receive branches changed {:?}", branches.len());

                    self.base_mut()
                        .emit_signal("branches_changed", &[branches_gd.to_variant()]);
                }
                OutputEvent::CheckedOutBranch { branch_doc_handle } => {
                    println!(
                        "rust: CheckedOutBranch event for doc {}",
                        branch_doc_handle.document_id()
                    );
                    self.checked_out_branch_doc_handle = Some(branch_doc_handle.clone());
                    self.base_mut().emit_signal(
                        "checked_out_branch",
                        &[branch_doc_handle.document_id().to_string().to_variant()],
                    );
                }
                OutputEvent::Initialized {
                    checked_out_branch_doc_handle,
                    branches_metadata_doc_handle,
                } => {
                    println!("rust: Initialized event");
                    self.checked_out_branch_doc_handle =
                        Some(checked_out_branch_doc_handle.clone());
                    self.branches_metadata_doc_handle = Some(branches_metadata_doc_handle.clone());
                    self.base_mut().emit_signal("initialized", &[]);
                }

                OutputEvent::CompletedShutdown => {
                    println!("rust: CompletedShutdown event");
                    self.base_mut().emit_signal("shutdown_completed", &[]);
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
