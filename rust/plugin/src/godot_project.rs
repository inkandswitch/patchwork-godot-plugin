use ::safer_ffi::prelude::*;
use automerge::PatchLog;
use automerge_repo::tokio::FsStorage;
use autosurgeon::{Hydrate, Reconcile};
use std::{collections::HashMap, str::FromStr};

use automerge::{
    patches::TextRepresentation, transaction::Transactable, ChangeHash, ObjType, ReadDoc,
    TextEncoding, ROOT,
};
use automerge_repo::{DocHandle, DocumentId, Repo, RepoHandle};
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender};
use godot::prelude::*;

use crate::godot_project_driver::{BranchState, DocHandleType};
use crate::godot_scene::{self};
use crate::patches::get_changed_files;
use crate::patches::get_changed_files_vec;
use crate::utils::{
    array_to_heads, commit_with_attribution_and_timestamp, heads_to_array, parse_automerge_url,
    print_branch_state,
};
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
    repo_handle: RepoHandle,
    user_name: String,
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
    fn create(maybe_branches_metadata_doc_id: String, user_name: String) -> Gd<Self> {
        let (driver_input_tx, driver_input_rx) = futures::channel::mpsc::unbounded();
        let (driver_output_tx, driver_output_rx) = futures::channel::mpsc::unbounded();

        let branches_metadata_doc_id = match DocumentId::from_str(&maybe_branches_metadata_doc_id) {
            Ok(doc_id) => Some(doc_id),
            Err(_) => None,
        };

        let storage = FsStorage::open("/tmp/automerge-godot-data").unwrap();
        let repo = Repo::new(None, Box::new(storage));
        let repo_handle = repo.run();

        let driver = GodotProjectDriver::create(repo_handle.clone());

        driver.spawn(
            driver_input_rx,
            driver_output_tx,
            branches_metadata_doc_id,
            if user_name == "" {
                None
            } else {
                Some(user_name.clone())
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
            repo_handle,
            user_name,
        })
    }

    // PUBLIC API

    #[func]
    fn set_user_name(&mut self, name: String) {
        self.user_name = name.clone();
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

    fn _get_file_at(
        &self,
        path: String,
        heads: Option<Vec<ChangeHash>>,
    ) -> Option<StringOrPackedByteArray> {
        let branch_state = self.get_checked_out_branch_state();
        if branch_state.is_none() {
            return None;
        }
        let branch_state = branch_state.unwrap();
        let heads = match heads {
            Some(heads) => heads,
            None => branch_state.synced_heads.clone(),
        };
        let doc = branch_state.doc_handle.with_doc(|d| d.clone());

        let files = doc.get_at(ROOT, "files", &heads).unwrap().unwrap().1;
        // does the file exist?
        let file_entry = match doc.get_at(files, &path, &heads) {
            Ok(Some((automerge::Value::Object(ObjType::Map), file_entry))) => file_entry,
            _ => return None,
        };

        // try to read file as text
        match doc.get_at(&file_entry, "content", &heads) {
            Ok(Some((automerge::Value::Object(ObjType::Text), content))) => {
                match doc.text_at(content, &heads) {
                    Ok(text) => return Some(StringOrPackedByteArray::String(text.to_string())),
                    Err(_) => {}
                }
            }
            _ => {}
        }

        // ... otherwise try to read as linked binary doc
        doc.get_string_at(&file_entry, "url", &heads)
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

        let checked_out_branch_state = match &self.get_checked_out_branch_state() {
            Some(branch_state) => branch_state.clone(),
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

        let checked_out_branch_state = match &self.get_checked_out_branch_state() {
            Some(branch_state) => branch_state.clone(),
            None => return Dictionary::new(),
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
        let changed_files = get_changed_files_vec(patches);
        // get the file entry at the old_heads, then the current heads
        let mut changed_files_dict = Dictionary::new();
        let mut changed_files_dict_files_array = Array::new();
        for file in changed_files {
            let file_entry = match self._get_file_at(file.clone(), Some(heads.clone())) {
                Some(StringOrPackedByteArray::String(s)) => GString::from(s).to_variant(),
                Some(StringOrPackedByteArray::Binary(bytes)) => {
                    PackedByteArray::from(bytes).to_variant()
                }
                None => Variant::nil(),
            };
            let file_entry_current = match self._get_file_at(file.clone(), Some(curr_heads.clone()))
            {
                Some(StringOrPackedByteArray::String(s)) => GString::from(s).to_variant(),
                Some(StringOrPackedByteArray::Binary(bytes)) => {
                    PackedByteArray::from(bytes).to_variant()
                }
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
        let checked_out_branch_state = match &self.get_checked_out_branch_state() {
            Some(branch_state) => branch_state.clone(),
            None => return PackedStringArray::new(),
        };

        let patches = checked_out_branch_state.doc_handle.with_doc(|d| {
            d.diff(
                &checked_out_branch_state.forked_at,
                &checked_out_branch_state.synced_heads,
                TextRepresentation::String(TextEncoding::Utf8CodeUnit),
            )
        });
        get_changed_files(patches)
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
    ) -> PackedStringArray {
        let heads = array_to_heads(heads);

        match &self.get_checked_out_branch_state() {
            Some(branch_state) => {
                self._save_files(branch_state.doc_handle.clone(), files, Some(heads))
            }
            None => panic!("couldn't save files, no checked out branch"),
        }
    }

    #[func]
    fn save_files(&self, files: Dictionary) -> PackedStringArray {
        match &self.get_checked_out_branch_state() {
            Some(branch_state) => self._save_files(branch_state.doc_handle.clone(), files, None),
            None => panic!("couldn't save files, no checked out branch"),
        }
    }

    #[func]
    fn save_file(&self, path: String, content: Variant) -> PackedStringArray {
        self.save_files(dict! { path: content })
    }

    #[func]
    fn save_file_at(
        &self,
        path: String,
        heads: PackedStringArray,
        content: Variant,
    ) -> PackedStringArray {
        self.save_files_at(dict! { path: content }, heads)
    }

    fn _save_files(
        &self,
        branch_doc_handle: DocHandle,
        files: Dictionary, /*  Record<String, Variant> */
        heads: Option<Vec<ChangeHash>>,
    ) -> PackedStringArray {
        let mut binary_entries: Vec<(String, DocHandle)> = Vec::new();
        let mut text_entries: Vec<(String, String)> = Vec::new();

        // we filter the files here because godot sends us indiscriminately all the files in the project
        // we only want to save the files that have actually changed
        for (path, content) in files.iter_shared() {
            match content.get_type() {
                VariantType::STRING => {
                    let content = String::from(content.to::<GString>());

                    if let Some(StringOrPackedByteArray::String(stored_content)) =
                        self._get_file(path.to_string())
                    {
                        if stored_content == content {
                            println!("file {:?} is already up to date", path.to_string());
                            continue;
                        }
                    }

                    text_entries.push((path.to_string(), content.clone()));
                }
                VariantType::PACKED_BYTE_ARRAY => {
                    let content = content.to::<PackedByteArray>().to_vec();

                    if let Some(StringOrPackedByteArray::Binary(stored_content)) =
                        self._get_file(path.to_string())
                    {
                        if stored_content == content {
                            println!("file {:?} is already up to date", path.to_string());
                            continue;
                        }
                    }

                    let binary_doc_handle = self.repo_handle.new_document();
                    binary_doc_handle.with_doc_mut(|d| {
                        let mut tx = d.transaction();
                        let _ = tx.put(ROOT, "content", content.clone());
                        commit_with_attribution_and_timestamp(tx, &Some(self.user_name.clone()));
                    });

                    binary_entries.push((path.to_string(), binary_doc_handle));
                }
                _ => panic!("invalid content type"),
            }
        }

        branch_doc_handle.with_doc_mut(|d| {
            let mut tx = match heads {
                Some(heads) => d.transaction_at(
                    PatchLog::inactive(TextRepresentation::String(TextEncoding::Utf8CodeUnit)),
                    &heads,
                ),
                None => d.transaction(),
            };

            let files = tx.get_obj_id(ROOT, "files").unwrap();

            // write text entries to doc
            for (path, content) in text_entries {
                // get existing file url or create new one
                let file_entry = match tx.get(&files, &path) {
                    Ok(Some((automerge::Value::Object(ObjType::Map), file_entry))) => file_entry,
                    _ => tx.put_object(&files, &path, ObjType::Map).unwrap(),
                };

                // delete url in file entry if it previously had one
                if let Ok(Some((_, _))) = tx.get(&file_entry, "url") {
                    let _ = tx.delete(&file_entry, "url");
                }
                // else if the path is tres or tscn, delete the content
                if path.ends_with(".tscn") || path.ends_with(".tres") {
                    let res = godot_scene::parse(&content);

                    match res {
                        Ok(scene) => {
                            scene.reconcile(&mut tx, path);
                        }
                        Err(e) => {
                            panic!("error parsing godot scene: {:?}", e);
                        }
                    }
                }
                // TODO: this is currently a hack to keep the text content available for everything
                // either get existing text or create new text
                let content_key = match tx.get(&file_entry, "content") {
                    Ok(Some((automerge::Value::Object(ObjType::Text), content))) => content,
                    _ => tx
                        .put_object(&file_entry, "content", ObjType::Text)
                        .unwrap(),
                };
                // TODO: we need to do line-by-line diffing here
                let _ = tx.update_text(&content_key, &content);
            }

            for (path, binary_doc_handle) in binary_entries {
                let file_entry = tx.put_object(&files, &path, ObjType::Map);
                let _ = tx.put(
                    file_entry.unwrap(),
                    "url",
                    format!("automerge:{}", &binary_doc_handle.document_id()),
                );
            }

            commit_with_attribution_and_timestamp(tx, &Some(self.user_name.clone()));
        });

        heads_to_array(branch_doc_handle.with_doc(|d| d.get_heads()))
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
    fn create_branch(&mut self, name: String) {
        self.driver_input_tx
            .unbounded_send(InputEvent::CreateBranch { name })
            .unwrap();

        self.checked_out_branch_state = CheckedOutBranchState::NothingCheckedOut;
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
                    let (is_new_branch, previous_heads) = match self
                        .branch_states
                        .get(&branch_state.doc_handle.document_id())
                    {
                        Some(previous_branch_state) => {
                            (false, previous_branch_state.synced_heads.clone())
                        }
                        None => (true, vec![]),
                    };

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
                                println!(
                                    "rust: TRIGGER checked out new branch: {}",
                                    branch_state.name
                                );

                                self.checked_out_branch_state = CheckedOutBranchState::CheckedOut(
                                    branch_state.doc_handle.document_id(),
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
                                self.base_mut().emit_signal("files_changed", &[]);
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

    // Helper functions

    fn get_checked_out_branch_state(&self) -> Option<BranchState> {
        match &self.checked_out_branch_state {
            CheckedOutBranchState::CheckedOut(document_id) => {
                self.branch_states.get(document_id).cloned()
            }
            _ => {
                println!(
                    "tried to get checked out branch state but nothing is checked out: {:?}",
                    self.checked_out_branch_state
                );
                None
            }
        }
    }
}
