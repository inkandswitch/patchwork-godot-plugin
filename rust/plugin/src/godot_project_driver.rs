use automerge_repo::RepoError;
use futures::stream::FuturesUnordered;
use futures::{FutureExt, Stream};
use ::safer_ffi::prelude::*;
use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use std::{
    collections::HashMap,
    str::FromStr,
};

use crate::utils::{commit_with_attribution_and_timestamp, print_branch_state};
use crate::{godot_project::{BranchesMetadataDoc, GodotProjectDoc, StringOrPackedByteArray}, godot_scene, utils::get_linked_docs_of_branch};
use automerge::{
    patches::TextRepresentation, transaction::Transactable, ChangeHash, ObjType,
    PatchLog, ReadDoc, TextEncoding, ROOT,
};
use automerge_repo::{
    tokio::FsStorage, ConnDirection, DocHandle, DocumentId, Repo, RepoHandle,
};
use autosurgeon::{ hydrate, reconcile};
use futures::{
    channel::mpsc::{UnboundedReceiver, UnboundedSender},
     StreamExt,
};

// use godot::prelude::*;
use tokio::{net::TcpStream, runtime::Runtime};

use crate::{doc_utils::SimpleDocReader, godot_project::Branch};

const SERVER_URL: &str = "104.131.179.247:8080";

#[derive(Debug, Clone)]
pub enum InputEvent {
    CreateBranch {
        name: String,
    },

    MergeBranch {
        branch_doc_handle: DocHandle,
    },

    SaveFiles {
        branch_doc_handle: DocHandle,
        heads: Option<Vec<ChangeHash>>,
        files: Vec<(String, StringOrPackedByteArray)>,
    },

    SetUserName {
        name: String,
    },

    StartShutdown,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DocHandleType {
    Binary,
    Unknown,
}

#[derive(Debug, Clone)]
pub enum OutputEvent {
    Initialized {
        project_doc_id: DocumentId,
    },
    NewDocHandle {
        doc_handle: DocHandle,
        doc_handle_type: DocHandleType,
    },
    BranchStateChanged {
        branch_state: BranchState,
    },

    CompletedCreateBranch {
        branch_doc_id: DocumentId,
    },

    CompletedShutdown,
}

enum SubscriptionMessage {
    Changed {
        doc_handle: DocHandle,
        diff: Vec<automerge::Patch>,
    },
    Added {
        doc_handle: DocHandle,
    },
}

#[derive(Debug, Clone)]
pub struct BinaryDocState {
    doc_handle: Option<DocHandle>, // is null if the binary doc is being requested but not loaded yet
    path: String,    
}

#[derive(Debug, Clone)]
pub struct BranchState {    
    pub name: String,
    pub doc_handle: DocHandle,
    pub linked_doc_ids: HashSet<DocumentId>,
    pub synced_heads: Vec<ChangeHash>,
    pub forked_at: Vec<ChangeHash>,
    pub is_main: bool,
}

struct DriverState {
    tx: UnboundedSender<OutputEvent>,
    repo_handle: RepoHandle,

    user_name: Option<String>,

    main_branch_doc_handle: DocHandle,
    branches_metadata_doc_handle: DocHandle,
    
    binary_doc_states: HashMap<DocumentId, BinaryDocState>,
    branch_states: HashMap<DocumentId, BranchState>,

    pending_branch_doc_ids: HashSet<DocumentId>,
    pending_binary_doc_ids: HashSet<DocumentId>,

    requesting_binary_docs: FuturesUnordered<Pin<Box<dyn Future<Output = (String, Result<DocHandle, RepoError>)> + Send>>>,
    requesting_branch_docs: FuturesUnordered<Pin<Box<dyn Future<Output = (String, Result<DocHandle, RepoError>)> + Send>>>,

    subscribed_doc_ids: HashSet<DocumentId>,
    all_doc_changes: futures::stream::SelectAll<std::pin::Pin<Box<dyn Stream<Item = SubscriptionMessage> + Send>>>

}

pub struct GodotProjectDriver {
    runtime: Runtime,
    repo_handle: RepoHandle,
}

impl GodotProjectDriver {
    pub fn create() -> Self {
        let runtime: Runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();

        let _guard = runtime.enter();

        let storage = FsStorage::open("/tmp/automerge-godot-data").unwrap();
        let repo = Repo::new(None, Box::new(storage));
        let repo_handle = repo.run();

        return Self {            
            runtime,
            repo_handle,
        };
    }

    pub fn spawn(
        &self,
        rx: UnboundedReceiver<InputEvent>,
        tx: UnboundedSender<OutputEvent>,
        maybe_branches_metadata_doc_id: Option<DocumentId>,
        user_name: Option<String>,
    ) {
        // Spawn connection task
        self.spawn_connection_task();

        // Spawn sync task for all doc handles
        self.spawn_driver_task(rx, tx, maybe_branches_metadata_doc_id, &user_name);
    }

    fn spawn_connection_task(&self) {
        let repo_handle_clone = self.repo_handle.clone();

        self.runtime.spawn(async move {
            println!("start a client");

            // Start a client.
            let stream = loop {
                // Try to connect to a peer
                let res = TcpStream::connect(SERVER_URL).await;
                if let Err(e) = res {
                    println!("error connecting: {:?}", e);
                    continue;
                }
                break res.unwrap();
            };

            println!("connect repo");

            if let Err(e) = repo_handle_clone
                .connect_tokio_io(SERVER_URL, stream, ConnDirection::Outgoing)
                .await
            {
                println!("Failed to connect: {:?}", e);
                return;
            }

            println!("connected successfully!");
        });
    }

    fn spawn_driver_task(
        &self,
        mut rx: UnboundedReceiver<InputEvent>,
        tx: UnboundedSender<OutputEvent>,
        branches_metadata_doc_id: Option<DocumentId>,
        user_name: &Option<String>,
    ) {
        let repo_handle = self.repo_handle.clone();
        let user_name = user_name.clone();

        self.runtime.spawn(async move {
            // destructure project doc handles
            let ProjectDocHandles { branches_metadata_doc_handle, main_branch_doc_handle } = init_project_doc_handles(&repo_handle, branches_metadata_doc_id, &user_name).await;

            tx.unbounded_send(OutputEvent::Initialized { project_doc_id: branches_metadata_doc_handle.document_id() }).unwrap();

            let mut state = DriverState {                
                tx: tx.clone(),
                repo_handle: repo_handle.clone(),            
                user_name: user_name.clone(),
                main_branch_doc_handle: main_branch_doc_handle.clone(),
                binary_doc_states: HashMap::new(),
                branch_states: HashMap::new(),                                
                branches_metadata_doc_handle,
                pending_branch_doc_ids: HashSet::new(),
                pending_binary_doc_ids: HashSet::new(),
                requesting_binary_docs : FuturesUnordered::new(),
                requesting_branch_docs: FuturesUnordered::new(),
                subscribed_doc_ids: HashSet::new(),
                all_doc_changes: futures::stream::SelectAll::new(),
            };

            state.update_branch_doc_state(state.main_branch_doc_handle.clone());
            state.subscribe_to_doc_handle(state.branches_metadata_doc_handle.clone());
            state.subscribe_to_doc_handle(state.main_branch_doc_handle.clone());

            loop {
                let repo_handle_clone = repo_handle.clone();

                futures::select! {
                    next = state.requesting_binary_docs.next() => {
                        if let Some((path, result)) = next {
                            match result {
                                Ok(doc_handle) => {
                                    state.add_binary_doc_handle(&path, &doc_handle);
                                },
                                Err(e) => {
                                    println!("error requesting binary doc: {:?}", e);
                                }
                            }                        
                        }
                    },

                    next = state.requesting_branch_docs.next() => {
                        if let Some((branch_name, result)) = next {
                            match result {
                                Ok(doc_handle) => {
                                    state.pending_branch_doc_ids.remove(&doc_handle.document_id());                                
                                    state.update_branch_doc_state(doc_handle.clone());
                                    state.subscribe_to_doc_handle(doc_handle.clone());
                                    println!("rust: added branch doc: {:?}", branch_name);

                                }
                                Err(e) => {
                                    println!("error requesting branch doc: {:?}", e);
                                }
                            }
                        }
                    },

                    message = state.all_doc_changes.select_next_some() => {
                       let doc_handle = match message {
                            SubscriptionMessage::Changed { doc_handle, diff: _ } => {
                                doc_handle
                            },
                            SubscriptionMessage::Added { doc_handle } => {
                                tx.unbounded_send(OutputEvent::NewDocHandle { doc_handle: doc_handle.clone(), doc_handle_type: DocHandleType::Unknown }).unwrap();
                                doc_handle
                            },
                        };      

                        let document_id = doc_handle.document_id();                    

                        // branches metadata doc changed
                        if document_id == state.branches_metadata_doc_handle.document_id() {
                            let branches = state.get_branches_metadata().branches.clone();

                            // check if there are new branches that haven't loaded yet
                            for (branch_id_str, branch) in branches.iter() {
                                let branch_id = DocumentId::from_str(branch_id_str).unwrap();                            
                                let branch_name = branch.name.clone();

                                if !state.branch_states.contains_key(&branch_id) && !state.pending_branch_doc_ids.contains(&branch_id) {
                                    state.pending_branch_doc_ids.insert(branch_id.clone());
                                    state.requesting_branch_docs.push(repo_handle.request_document(branch_id.clone()).map(|doc_handle| {
                                        (branch_name, doc_handle)
                                    }).boxed());
                                }
                            }
                        }

                        // branch doc changed
                        if state.branch_states.contains_key(&document_id) {
                            state.update_branch_doc_state(doc_handle.clone());
                        }
                    },

                    message = rx.select_next_some() => {

                        match message {
                            InputEvent::CreateBranch { name } => {
                                state.create_branch(name.clone());
                            },

                            InputEvent::MergeBranch { branch_doc_handle } => {
                                state.merge_branch(branch_doc_handle);
                            },                        

                            InputEvent::SaveFiles { branch_doc_handle, files, heads} => {
                                state.save_files(branch_doc_handle, files, heads);                                                           
                            }

                            InputEvent::StartShutdown => {
                                println!("rust: shutting down");

                                let result = repo_handle_clone.stop();

                                println!("rust: shutdown result: {:?}", result);

                                tx.unbounded_send(OutputEvent::CompletedShutdown).unwrap();
                            }

                            InputEvent::SetUserName { name } => {
                                state.user_name = Some(name.clone());
                            }
                        };                    
                    }
                }
            }    
        });
    }
}



struct ProjectDocHandles {
    branches_metadata_doc_handle: DocHandle,
    main_branch_doc_handle: DocHandle,
}

async fn init_project_doc_handles (repo_handle: &RepoHandle, doc_id: Option<DocumentId>, user_name: &Option<String>) -> ProjectDocHandles  {
    match doc_id {

        // load existing project

        Some(doc_id) => {
            println!("rust: loading existing project: {:?}", doc_id);

            let branches_metadata_doc_handle = repo_handle.request_document(doc_id.clone()).await.unwrap_or_else(|e| {
                panic!("failed init, can't load branches metadata doc: {:?}", e);
            });

            let branches_metadata: BranchesMetadataDoc = branches_metadata_doc_handle.with_doc(|d| hydrate(d).unwrap_or_else(|_| {
                panic!("failed init, can't hydrate metadata doc");
            }));
        
            let main_branch_doc_handle: DocHandle =
                repo_handle.request_document(DocumentId::from_str(&branches_metadata.main_doc_id).unwrap()).await.unwrap_or_else(|_| {
                    panic!("failed init, can't load main branchs doc");
                });

            return ProjectDocHandles {
                branches_metadata_doc_handle: branches_metadata_doc_handle.clone(),
                main_branch_doc_handle: main_branch_doc_handle.clone(),
            }
        }

        // create new project

        None => {
            println!("rust: creating new project");

            // Create new main branch doc
            let main_branch_doc_handle = repo_handle.new_document();
            main_branch_doc_handle.with_doc_mut(|d| {
                let mut tx = d.transaction();
                let _ = reconcile(
                    &mut tx,
                    GodotProjectDoc {
                        files: HashMap::new(),
                        state: HashMap::new(),
                    },
                );
                commit_with_attribution_and_timestamp(tx, &user_name);
            });

            let main_branch_doc_id = main_branch_doc_handle.document_id().to_string();
            let main_branch_doc_id_clone = main_branch_doc_id.clone();
            let branches = HashMap::from([(
                main_branch_doc_id,
                Branch {
                    name: String::from("main"),
                    id: main_branch_doc_handle.document_id().to_string(),
                    is_merged: true,
                    forked_at: Vec::new(),
                },
            )]);
            let branches_clone = branches.clone();

            // create new branches metadata doc
            let branches_metadata_doc_handle = repo_handle.new_document();
            branches_metadata_doc_handle.with_doc_mut(|d| {
                let mut tx = d.transaction();
                let _ = reconcile(
                    &mut tx,
                    BranchesMetadataDoc {
                        main_doc_id: main_branch_doc_id_clone,
                        branches: branches_clone,
                    },
                );
                commit_with_attribution_and_timestamp(tx, &user_name);
            });

            return ProjectDocHandles {
                branches_metadata_doc_handle: branches_metadata_doc_handle.clone(),
                main_branch_doc_handle: main_branch_doc_handle.clone(),
            }
        }
    }
}



impl DriverState {

    fn create_branch(&mut self, name: String) {
        let new_branch_handle = clone_doc(&self.repo_handle, &self.main_branch_doc_handle);
        let main_heads = self.main_branch_doc_handle.with_doc(|d| d.get_heads()).iter().map(|h| h.to_string()).collect();
        let branch = Branch { name: name.clone(), id: new_branch_handle.document_id().to_string(), is_merged: false, forked_at: main_heads};

        self.tx.unbounded_send(OutputEvent::CompletedCreateBranch {
            branch_doc_id: new_branch_handle.document_id(),
        }).unwrap();  

        self.branches_metadata_doc_handle.with_doc_mut(|d| {
            let mut branches_metadata: BranchesMetadataDoc = hydrate(d).unwrap();
            let mut tx = d.transaction();
            branches_metadata
                .branches
                .insert(branch.id.clone(), branch);
            let _ = reconcile(&mut tx, branches_metadata);
            commit_with_attribution_and_timestamp(tx, &self.user_name);
        });
  
    }

    
    fn save_files(
        &mut self,
        branch_doc_handle: DocHandle,
        files: Vec<(String, StringOrPackedByteArray)>,
        heads: Option<Vec<ChangeHash>>,
    ) {    
        let binary_entries: Vec<(String, DocHandle)> = files.iter().filter_map(|(path, content)| {
            if let StringOrPackedByteArray::Binary(content) = content {
                let binary_doc_handle = self.repo_handle.new_document();
                binary_doc_handle.with_doc_mut(|d| {
                    let mut tx = d.transaction();
                    let _ = tx.put(ROOT, "content", content.clone());
                    commit_with_attribution_and_timestamp(tx, &self.user_name);
                });

                println!("create binary doc: {:?} size: {:?}", path, content.len());

                self.add_binary_doc_handle(path, &binary_doc_handle);

                Some((path.clone(), binary_doc_handle))
            } else {
                None
            }
        }).collect();

        let text_entries: Vec<(String, &String)> = files.iter().filter_map(|(path, content)| {
            if let StringOrPackedByteArray::String(content) = content {
                Some((path.clone(), content))
            } else {
                None
            }
        }).collect();

        branch_doc_handle.with_doc_mut(|d| {
            let mut tx = match heads {
                Some(heads) => d.transaction_at(
                    PatchLog::inactive(TextRepresentation::String(
                        TextEncoding::Utf8CodeUnit,
                    )),
                    &heads,
                ),
                None => d.transaction(),
            };

            let files = tx.get_obj_id(ROOT, "files").unwrap();


            // write text entries to doc
            for (path, content) in text_entries {

                // get existing file url or create new one
                let file_entry = match tx.get(&files, &path) {
                    Ok(Some((automerge::Value::Object(ObjType::Map), file_entry))) => {
                        file_entry
                    }
                    _ => tx.put_object(&files, &path, ObjType::Map).unwrap(),
                };

                 // delete url in file entry if it previously had one
                 if let Ok(Some((_, _))) = tx.get(&file_entry, "url") {
                    let _ = tx.delete(&file_entry, "url");
                }
                // else if the path is tres or tscn, delete the content
                /*if path.ends_with(".tscn") || path.ends_with(".tres") {
                    
                    let res = godot_scene::parse(&content);

                    match res {
                        Ok(scene) => {
                            scene.reconcile(&mut tx, path);
                        }
                        Err(e) => {
                            panic!("error parsing godot scene: {:?}", e);
                        }
                    }
                } else { */
                    // either get existing text or create new text
                    let content_key = match tx.get(&file_entry, "content") {
                        Ok(Some((automerge::Value::Object(ObjType::Text), content))) => content,
                        _ => tx
                            .put_object(&file_entry, "content", ObjType::Text)
                            .unwrap(),
                    };
                    let _ = tx.update_text(&content_key, &content);
                /* } */
            }

            for (path, binary_doc_handle) in binary_entries {
                let file_entry = tx.put_object(&files, &path, ObjType::Map);
                let _ = tx.put(
                    file_entry.unwrap(),
                    "url",
                    format!("automerge:{}", &binary_doc_handle.document_id()),
                );
            }

            commit_with_attribution_and_timestamp(tx, &self.user_name);
        });
    }

    fn merge_branch(&mut self, branch_doc_handle: DocHandle)  {    
        branch_doc_handle.with_doc_mut(|branch_doc| {
            self.main_branch_doc_handle.with_doc_mut(|main_doc| {
                let _ = main_doc.merge(branch_doc);
            });
        });

        // mark branch as merged
        self.branches_metadata_doc_handle.with_doc_mut(|d| {
            let mut branches_metadata: BranchesMetadataDoc = hydrate(d).unwrap();
            let mut tx = d.transaction();
            branches_metadata
                .branches.entry(branch_doc_handle.document_id().to_string()).and_modify(|branch| {
                    branch.is_merged = true;
                });

            let _ = reconcile(&mut tx, branches_metadata);
            commit_with_attribution_and_timestamp(tx, &self.user_name);
        });
    }

    fn update_branch_doc_state (&mut self, branch_doc_handle: DocHandle) {
        let branch_state = match self.branch_states.get_mut(&branch_doc_handle.document_id()) {
            Some(branch_state) => branch_state,
            None => {          
                let branch = self.get_branches_metadata().branches.get(&branch_doc_handle.document_id().to_string()).unwrap().clone();

                self.branch_states.insert(branch_doc_handle.document_id().clone(), BranchState {
                    name: branch.name.clone(),
                    doc_handle: branch_doc_handle.clone(),
                    linked_doc_ids: HashSet::new(),
                    synced_heads: Vec::new(),
                    forked_at: branch.forked_at.iter().map(|h| ChangeHash::from_str(h).unwrap()).collect(),
                    is_main: branch_doc_handle.document_id() == self.main_branch_doc_handle.document_id(),
                });
                self.branch_states.get_mut(&branch_doc_handle.document_id()).unwrap()
            }
        };

        print_branch_state("update_branch_doc_state", &branch_state);

        let linked_docs = get_linked_docs_of_branch(&branch_doc_handle);    

        // load binary docs if not already loaded
        for (path, doc_id) in linked_docs.iter() {
            if self.binary_doc_states.get(&doc_id).is_some() || self.pending_binary_doc_ids.contains(&doc_id) {
                continue;
            }

            self.pending_binary_doc_ids.insert(doc_id.clone());

            let path = path.clone();
            self.requesting_binary_docs.push(self.repo_handle.request_document(doc_id.clone()).map(|doc_handle| {
                (path, doc_handle)
            }).boxed());        
        }

        // update linked doc ids
        branch_state.linked_doc_ids = linked_docs.values().cloned().collect();


        // check if all linked docs have been loaded
        if branch_state.linked_doc_ids.iter().all(|doc_id| {                                                                                             
            if let Some(binary_doc_state) =  self.binary_doc_states.get(doc_id) {
                binary_doc_state.doc_handle.is_some()
            } else {
                false
            }                                    
        }) {
            branch_state.synced_heads = branch_doc_handle.with_doc(|d| d.get_heads());

            print_branch_state("branch doc state loaded", &branch_state);

            self.tx.unbounded_send(OutputEvent::BranchStateChanged {
                branch_state: branch_state.clone(),
            }).unwrap();            
        }
    }


    fn add_binary_doc_handle(&mut self, path: &String, binary_doc_handle: &DocHandle) {
        self.binary_doc_states.insert(binary_doc_handle.document_id().clone(), BinaryDocState {
            doc_handle: Some(binary_doc_handle.clone()),
            path: path.clone(),
        });


        let _ = &self.tx.unbounded_send(OutputEvent::NewDocHandle { doc_handle: binary_doc_handle.clone(), doc_handle_type: DocHandleType::Binary }).unwrap();
        
        println!("add_binary_doc_handle {:?} {:?}", path, binary_doc_handle.document_id());

        // check all branch states that link to this doc
        for branch_state in self.branch_states.values_mut() {
            if branch_state.linked_doc_ids.contains(&binary_doc_handle.document_id()) {
                // check if all linked docs have been loaded
                if branch_state.linked_doc_ids.iter().all(|doc_id| {                                                                                             
                    if let Some(binary_doc_state) =  self.binary_doc_states.get(doc_id) {
                        binary_doc_state.doc_handle.is_some()
                    } else {
                        false
                    }                                    
                }) {

                    print_branch_state("branch doc state loaded", &branch_state);
                    branch_state.synced_heads = branch_state.doc_handle.with_doc(|d| d.get_heads());
                    self.tx.unbounded_send(OutputEvent::BranchStateChanged {
                        branch_state: branch_state.clone(),
                    }).unwrap();
                }
            }
        }
    }


    pub fn subscribe_to_doc_handle(&mut self, doc_handle: DocHandle) {
        if self.subscribed_doc_ids.contains(&doc_handle.document_id()) {
            return;
        }

        self.subscribed_doc_ids.insert(doc_handle.document_id());
        self.all_doc_changes.push(handle_changes(doc_handle.clone()).boxed());
        self.all_doc_changes.push(futures::stream::once(async move {
            SubscriptionMessage::Added { doc_handle }
        }).boxed());
    }

    fn get_branches_metadata(&self) -> BranchesMetadataDoc {
        let branches_metadata : BranchesMetadataDoc = self
            .branches_metadata_doc_handle
            .with_doc(|d| hydrate(d).unwrap());

        return branches_metadata
    }
}


fn clone_doc(repo_handle: &RepoHandle, doc_handle: &DocHandle) -> DocHandle {
    let new_doc_handle = repo_handle.new_document();

    let _ =
        doc_handle.with_doc_mut(|mut main_d| new_doc_handle.with_doc_mut(|d| d.merge(&mut main_d)));

    return new_doc_handle;
}

fn handle_changes(handle: DocHandle) -> impl futures::Stream<Item = SubscriptionMessage> + Send {
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

        Some((
            SubscriptionMessage::Changed { doc_handle: doc_handle.clone(), diff },
            doc_handle,
        ))
    })
}
