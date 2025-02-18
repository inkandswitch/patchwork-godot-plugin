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

use crate::{
    godot_project::{BranchesMetadataDoc, GodotProjectDoc, StringOrPackedByteArray},
    utils::get_linked_docs_of_branch,
};
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

    CheckoutBranch {
        branch_doc_handle: DocHandle,
    },

    MergeBranch {
        branch_doc_handle: DocHandle,
    },

    SaveFile {
        path: String,
        content: StringOrPackedByteArray,
        heads: Option<Vec<ChangeHash>>,
    },
}

#[derive(Debug, Clone)]
pub enum OutputEvent {
    Initialized {
        checked_out_branch_doc_handle: DocHandle,
        branches_metadata_doc_handle: DocHandle,
    },
    NewDocHandle {
        doc_handle: DocHandle,
    },
    CheckedOutBranch {
        branch_doc_handle: DocHandle,
    },
    FilesChanged,
    BranchesChanged {
        branches: HashMap<String, Branch>,
    },
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


// todo: pull this into the driver state
pub struct DocHandleSubscriptions {
    subscribed_doc_handle_ids: HashSet<DocumentId>,
    futures: futures::stream::SelectAll<std::pin::Pin<Box<dyn Stream<Item = SubscriptionMessage> + Send>>>,
}

impl DocHandleSubscriptions {
    pub fn new() -> Self {
        return Self {
            subscribed_doc_handle_ids: HashSet::new(),
            futures: futures::stream::SelectAll::new(),
        };
    }

    pub fn add_doc_handle(&mut self, doc_handle: DocHandle) {

        println!("rust: subscribed to doc handle: {:?}", doc_handle.document_id());

        if self.subscribed_doc_handle_ids.contains(&doc_handle.document_id()) {
            return;
        }

        self.subscribed_doc_handle_ids.insert(doc_handle.document_id());
        self.futures.push(handle_changes(doc_handle.clone()).boxed());
        self.futures.push(futures::stream::once(async move {
            SubscriptionMessage::Added { doc_handle }
        }).boxed());
    }
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

#[derive(Debug, Clone)]
pub struct BinaryDocState {
    doc_handle: Option<DocHandle>, // is null if the binary doc is being requested but not loaded yet
    path: String,    
}

#[derive(Debug, Clone)]
pub struct BranchState {
    doc_handle: DocHandle,
    linked_doc_ids: HashSet<DocumentId>,
}

#[derive(Debug, Clone)]
enum CheckedOutBranchState {
    NothingCheckedOut,
    CheckingOut(BranchState),
    CheckedOut(BranchState),
}

struct DriverState {
    tx: UnboundedSender<OutputEvent>,
    repo_handle: RepoHandle,
    binary_doc_states: HashMap<DocumentId, BinaryDocState>,
    checked_out_branch_state: CheckedOutBranchState,
    main_branch_doc_handle: DocHandle,
    branches_metadata_doc_handle: DocHandle,
    is_initialized: bool,
    requesting_binary_docs: FuturesUnordered<Pin<Box<dyn Future<Output = (String, Result<DocHandle, RepoError>)> + Send>>>
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
    ) {
        // Spawn connection task
        self.spawn_connection_task();

        // Spawn sync task for all doc handles
        self.spawn_driver_task(rx, tx, maybe_branches_metadata_doc_id);
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
    ) {
        let repo_handle = self.repo_handle.clone();

        self.runtime.spawn(async move {
            let mut subscribed_doc_handles = DocHandleSubscriptions::new();        

            // destructure project doc handles
            let ProjectDocHandles { branches_metadata_doc_handle, main_branch_doc_handle } = init_project_doc_handles(&repo_handle, branches_metadata_doc_id).await;

            let mut state = DriverState {
                tx: tx.clone(),
                repo_handle: repo_handle.clone(),            
                binary_doc_states: HashMap::new(),
                checked_out_branch_state: CheckedOutBranchState::NothingCheckedOut,
                main_branch_doc_handle,
                branches_metadata_doc_handle,
                is_initialized: false,
                requesting_binary_docs : FuturesUnordered::new()
            };

            subscribed_doc_handles.add_doc_handle(state.branches_metadata_doc_handle.clone());
            subscribed_doc_handles.add_doc_handle(state.main_branch_doc_handle.clone());

            state.checkout_branch(state.main_branch_doc_handle.clone());        

            loop {
                futures::select! {
                    next = state.requesting_binary_docs.next() => {
                        if let Some((path, result)) = next {
                            match result {
                                Ok(doc_handle) => {
                                    state.add_binary_doc_handle(&path, &doc_handle);

                                    // we are trying to check out a branch ?
                                    if let CheckedOutBranchState::CheckingOut(branch_state) = state.checked_out_branch_state.clone() {
                                        if
                                            // check if the doc is linked to the branch
                                            branch_state.linked_doc_ids.contains(&doc_handle.document_id()) && 
                                                                            
                                            // and all linked docs are loaded
                                            branch_state.linked_doc_ids.iter().all(|doc_id| {                                        
                                            if let Some(binary_doc_state) =  state.binary_doc_states.get(doc_id) {
                                                binary_doc_state.doc_handle.is_some()
                                            } else {
                                                false
                                            }                                    
                                        }) {
                                            state.checked_out_branch_state = CheckedOutBranchState::CheckedOut(branch_state.clone());
                                        
                                            if state.is_initialized {
                                                tx.unbounded_send(OutputEvent::CheckedOutBranch { branch_doc_handle: branch_state.doc_handle.clone() }).unwrap();
                                            } else {
                                                state.is_initialized = true;
                                                tx.unbounded_send(OutputEvent::Initialized { checked_out_branch_doc_handle: branch_state.doc_handle.clone(), branches_metadata_doc_handle: state.branches_metadata_doc_handle.clone() }).unwrap();
                                            }
                                        }
                                    }

                                },
                                Err(e) => {
                                    println!("error requesting binary doc: {:?}", e);
                                }
                            }                        
                        }
                    },

                    message = subscribed_doc_handles.futures.select_next_some() => {

                       let doc_handle = match message {
                            SubscriptionMessage::Changed { doc_handle, diff: _ } => {
                                doc_handle
                            },
                            SubscriptionMessage::Added { doc_handle } => {
                                tx.unbounded_send(OutputEvent::NewDocHandle { doc_handle: doc_handle.clone() }).unwrap();
                                doc_handle
                            },
                        };      

                        let document_id = doc_handle.document_id();
                        

                        if document_id == state.branches_metadata_doc_handle.document_id() {
                            tx.unbounded_send(OutputEvent::BranchesChanged { branches: state.get_branches_metadata().branches }).unwrap();
                        }

                        if document_id == state.main_branch_doc_handle.document_id() {
                            tx.unbounded_send(OutputEvent::FilesChanged).unwrap();
                        }
                    },

                    message = rx.select_next_some() => {

                        match message {
                            InputEvent::CheckoutBranch { branch_doc_handle } => {
                                state.checkout_branch(branch_doc_handle);                                
                            },

                            InputEvent::CreateBranch {name} => {
//                                state.create_branch(&mut state, name);
                            },

                            InputEvent::MergeBranch { branch_doc_handle } => {
                                println!("rust: Merging branch: {:?}", branch_doc_handle.document_id());
                                // pending_tasks.push(Box::pin(state.merge_branch(branch_doc_handle)));
                            },

                            InputEvent::SaveFile { path, content, heads} => {
                                state.save_file(&path, heads, content);                           
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

async fn init_project_doc_handles (repo_handle: &RepoHandle, doc_id: Option<DocumentId>) -> ProjectDocHandles  {
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
                tx.commit();
            });

            let main_branch_doc_id = main_branch_doc_handle.document_id().to_string();
            let main_branch_doc_id_clone = main_branch_doc_id.clone();
            let branches = HashMap::from([(
                main_branch_doc_id,
                Branch {
                    name: String::from("main"),
                    id: main_branch_doc_handle.document_id().to_string(),
                    is_merged: true,
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
                tx.commit();
            });

            return ProjectDocHandles {
                branches_metadata_doc_handle: branches_metadata_doc_handle.clone(),
                main_branch_doc_handle: main_branch_doc_handle.clone(),
            }
        }
    }
}



impl DriverState {

    fn create_branch(&self, name: String)  {
        let new_branch_handle = clone_doc(&self.repo_handle, &self.main_branch_doc_handle);


        let branch = Branch { name, id: new_branch_handle.document_id().to_string(), is_merged: false};

        self.branches_metadata_doc_handle.with_doc_mut(|d| {
            let mut branches_metadata: BranchesMetadataDoc = hydrate(d).unwrap();
            let mut tx = d.transaction();
            branches_metadata
                .branches
                .insert(branch.id.clone(), branch);
            let _ = reconcile(&mut tx, branches_metadata);
            tx.commit();
        });


        // todo: checkout the new branch
    }

    fn checkout_branch(&mut self, branch_doc_handle: DocHandle) {
        let linked_docs = get_linked_docs_of_branch(&branch_doc_handle);
    
        let mut are_all_linked_docs_loaded = true;

        // request all linked docs that haven't been requested yet
        for (path, doc_id) in linked_docs.clone() {
            if self.binary_doc_states.contains_key(&doc_id) {
                continue;
            }

            are_all_linked_docs_loaded = false;
    
            self.binary_doc_states.insert(doc_id.clone(), BinaryDocState {
                doc_handle: None,
                path: path.clone(),
            });
    
            self.requesting_binary_docs.push(self.repo_handle.request_document(doc_id.clone()).map(|doc_handle| {
                (path, doc_handle)
            }).boxed());
        }
    
        let branch_state = BranchState {
            doc_handle: branch_doc_handle.clone(),
            linked_doc_ids: linked_docs.clone().iter().map(|(_, doc_id)| doc_id.clone()).collect(),
        };

        if are_all_linked_docs_loaded {            
            self.checked_out_branch_state = CheckedOutBranchState::CheckedOut(branch_state.clone());

            if (!self.is_initialized) {
                self.is_initialized = true;
                self.tx.unbounded_send(OutputEvent::Initialized { 
                    checked_out_branch_doc_handle: branch_state.doc_handle.clone(), 
                    branches_metadata_doc_handle: self.branches_metadata_doc_handle.clone() 
                }).unwrap();
            }    else {
                self.tx.unbounded_send(OutputEvent::CheckedOutBranch { branch_doc_handle: branch_doc_handle.clone() }).unwrap();
            }

        } else {
            println!("checkout branch: checking out");
            self.checked_out_branch_state = CheckedOutBranchState::CheckingOut(branch_state);                        
        }
    }
    
    fn save_file(
        &mut self,
        path: &String,
        heads: Option<Vec<ChangeHash>>,
        content: StringOrPackedByteArray,
    ) {
        let checked_out_branch_doc_handle = match &self.checked_out_branch_state {
            CheckedOutBranchState::CheckedOut(branch_state) => branch_state.doc_handle.clone(),
            _ => {
                println!(": save file called before branch is checked out");
                return;
            }
        };

        match content {
            StringOrPackedByteArray::String(content) => {
                println!("rust: save file: {:?}", path);
                checked_out_branch_doc_handle.with_doc_mut(|d| {
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

                    let _ = tx.put_object(ROOT, "fo", ObjType::Map);

                    // get existing file url or create new one
                    let file_entry = match tx.get(&files, path) {
                        Ok(Some((automerge::Value::Object(ObjType::Map), file_entry))) => {
                            file_entry
                        }
                        _ => tx.put_object(files, path, ObjType::Map).unwrap(),
                    };

                    // delete url in file entry if it previously had one
                    if let Ok(Some((_, _))) = tx.get(&file_entry, "url") {
                        let _ = tx.delete(&file_entry, "url");
                    }

                    // either get existing text or create new text
                    let content_key = match tx.get(&file_entry, "content") {
                        Ok(Some((automerge::Value::Object(ObjType::Text), content))) => content,
                        _ => tx
                            .put_object(&file_entry, "content", ObjType::Text)
                            .unwrap(),
                    };
                    let _ = tx.update_text(&content_key, &content);
                    tx.commit();
                });
            }
            StringOrPackedByteArray::Binary(content) => {
                // create binary doc
                let binary_doc_handle = self.repo_handle.new_document();
                binary_doc_handle.with_doc_mut(|d| {
                    let mut tx = d.transaction();
                    let _ = tx.put(ROOT, "content", content);
                    tx.commit();
                });

                // add to binary doc states
                self.add_binary_doc_handle(path, &binary_doc_handle);

                // write url to content doc into project doc
                checked_out_branch_doc_handle.with_doc_mut(|d| {
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

                    let file_entry = tx.put_object(files, path, ObjType::Map);
                    let _ = tx.put(
                        file_entry.unwrap(),
                        "url",
                        format!("automerge:{}", &binary_doc_handle.document_id()),
                    );
                    tx.commit();
                });
            }
        }
    }

    fn add_binary_doc_handle(&mut self, path: &String, binary_doc_handle: &DocHandle) {
        self.binary_doc_states.insert(binary_doc_handle.document_id().clone(), BinaryDocState {
            doc_handle: Some(binary_doc_handle.clone()),
            path: path.clone(),
        });
        let _ = &self.tx.unbounded_send(OutputEvent::NewDocHandle { doc_handle: binary_doc_handle.clone() }).unwrap();
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

/* 
impl ProjectState {
    fn add_branch(&mut self, branch: Branch) {
        let branch_clone = branch.clone();
        self.branches_metadata_doc_handle.with_doc_mut(|d| {
            let mut branches_metadata: BranchesMetadataDoc = hydrate(d).unwrap();
            let mut tx = d.transaction();
            branches_metadata
                .branches
                .insert(branch_clone.id.clone(), branch_clone);
            let _ = reconcile(&mut tx, branches_metadata);
            tx.commit();
        });
    }

 
}

*/


impl DriverState {

    /* 

    // return a vector of linked docs
    async fn handle_doc_change(&mut self, doc_handle: &DocHandle, loaded_doc_handle_ids: &HashSet<DocumentId>) -> (Vec<DocHandle>, Option<OutputEvent>) {
        let project = &self.project;

        let checked_out_branch_id = project.checked_out_branch_doc_handle.document_id();
        if checked_out_branch_id == doc_handle.document_id() {

            println!("rust: wait on all linked docs to be loaded {:?}", doc_handle.document_id());

            let linked_doc_ids: Vec<DocumentId> = get_linked_docs_of_branch(doc_handle);

            print_doc("rust: docs", doc_handle);


            let mut new_doc_handles = Vec::new();

            // make sure all linked docs are loaded

            let mut linked_doc_results = Vec::new();
            for doc_id in linked_doc_ids {
                if loaded_doc_handle_ids.contains(&doc_id) {
                    continue;
                }

                let result = self.repo_handle.request_document(doc_id).await;
                if let Ok(doc_handle) = &result {
                    new_doc_handles.push(doc_handle.clone());
                }
                linked_doc_results.push(result);
            }

            if linked_doc_results.iter().any(|result| result.is_err()) {
                println!("failed update doc handle, couldn't load all linked docs for ");
                return (vec![], None);
            }

            println!("rust: files changed");

            return (new_doc_handles, Some(OutputEvent::FilesChanged));
        }

        let branches_metadata_doc_id = project.branches_metadata_doc_handle.document_id();
        if branches_metadata_doc_id == doc_handle.document_id() {        

            println!("RUST: branches metadata doc changed {:?}", project.get_branches_metadata());

            return (vec![], Some(OutputEvent::BranchesChanged {
                branches: project.get_branches_metadata().branches
            }));
        }

        return (vec![], None);

    }

    fn create_branch(&mut self, name: String)  {
           let mut project = &self.project;
     

        let new_branch_handle = clone_doc(&self.repo_handle, &project.main_branch_doc_handle);

        project.add_branch(Branch {
            id: new_branch_handle.document_id().to_string().clone(),
            name: name.clone(),
            is_merged: false,
        });
        project.checked_out_branch_doc_handle = new_branch_handle.clone();

        self.project = project.clone();

        TaskResult {
            project: Some(project.clone()),
            new_doc_handles: vec![new_branch_handle.clone()],
            event: Some(OutputEvent::CheckedOutBranch {
                branch_doc_handle: new_branch_handle.clone(),
            }),
        }
    }

    fn merge_branch(&mut self, branch_doc_handle: DocHandle) -> impl Future<Output = TaskResult> + 'static {
        let project = match &self.project {
            Some(project) => project.clone(),
            None => {
                panic!("triggered merge branch '{}' before project was initialized", branch_doc_handle.document_id());
            }
        };

        branch_doc_handle.with_doc_mut(|branch_doc| {
            project.main_branch_doc_handle.with_doc_mut(|main_doc| {
                let _ = main_doc.merge(branch_doc);
            });
        });

        // todo: mark branch as merged
        let main_branch_doc_id = project.clone().main_branch_doc_handle.document_id();

        self.checkout_branch(main_branch_doc_id)
    }

    fn checkout_branch(&self, branch_doc_id: DocumentId) -> impl Future<Output = TaskResult> + 'static {
        let repo_handle = self.repo_handle.clone();
        let mut project = self.project.as_ref().unwrap_or_else(|| {
            panic!("triggered checkout branch '{}' before project was initialized", branch_doc_id);
        }).clone();       

        async move {
            let branch_doc_handle = repo_handle.request_document(branch_doc_id).await.unwrap_or_else(|e| {
                panic!("failed to checkout branch: {:?}", e);
            });

            let mut new_doc_handles = vec![branch_doc_handle.clone()];

            let linked_doc_ids = get_linked_docs_of_branch(&branch_doc_handle);

            // alex ?
            // todo: do this in parallel
            let mut linked_doc_results = Vec::new();
            for doc_id in linked_doc_ids {
                let result = repo_handle.request_document(doc_id).await;
                if let Ok(doc_handle) = &result {
                    new_doc_handles.push(doc_handle.clone());
                }
                linked_doc_results.push(result);
            }

            if linked_doc_results.iter().any(|result| result.is_err()) {
                panic!("failed to checkout branch, some linked docs are missing:");
            }

            project.checked_out_branch_doc_handle = branch_doc_handle.clone();

            TaskResult {
                project: Some(project),
                new_doc_handles,
                event: Some(OutputEvent::CheckedOutBranch {
                    branch_doc_handle: branch_doc_handle.clone(),
                }),
            }
        }
    }

    fn save_file(
        &self,
        path: &String,
        heads: Option<Vec<ChangeHash>>,
        content: StringOrPackedByteArray,
    ) -> impl Future<Output = TaskResult> + 'static {
        let project = match &self.project {
            Some(project) => project.clone(),
            None => {
                panic!("triggered save file '{}' before project was initialized", path);
            }
        };

        let path = path.clone();
        let repo_handle = self.repo_handle.clone();

        async move {
            match content {
                StringOrPackedByteArray::String(content) => {
                    println!("rust: save file: {:?}", path);
                    project.checked_out_branch_doc_handle.with_doc_mut(|d| {
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

                        let _ = tx.put_object(ROOT, "fo", ObjType::Map);

                        // get existing file url or create new one
                        let file_entry = match tx.get(&files, &path) {
                            Ok(Some((automerge::Value::Object(ObjType::Map), file_entry))) => {
                                file_entry
                            }
                            _ => tx.put_object(files, &path, ObjType::Map).unwrap(),
                        };

                        // delete url in file entry if it previously had one
                        if let Ok(Some((_, _))) = tx.get(&file_entry, "url") {
                            let _ = tx.delete(&file_entry, "url");
                        }

                        // either get existing text or create new text
                        let content_key = match tx.get(&file_entry, "content") {
                            Ok(Some((automerge::Value::Object(ObjType::Text), content))) => content,
                            _ => tx
                                .put_object(&file_entry, "content", ObjType::Text)
                                .unwrap(),
                        };
                        let _ = tx.update_text(&content_key, &content);
                        tx.commit();
                    });

                    TaskResult {
                        project: Some(project),
                        new_doc_handles: vec![],
                        event: None,
                    }
                }
                StringOrPackedByteArray::Binary(content) => {
                    // create binary doc
                    let binary_doc_handle = repo_handle.new_document();
                    binary_doc_handle.with_doc_mut(|d| {
                        let mut tx = d.transaction();
                        let _ = tx.put(ROOT, "content", content);
                        tx.commit();
                    });

                    // write url to content doc into project doc
                    project.checked_out_branch_doc_handle.with_doc_mut(|d| {
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

                        let file_entry = tx.put_object(files, &path, ObjType::Map);
                        let _ = tx.put(
                            file_entry.unwrap(),
                            "url",
                            format!("automerge:{}", &binary_doc_handle.document_id()),
                        );
                        tx.commit();
                    });

                    TaskResult {
                        project: Some(project),
                        new_doc_handles: vec![binary_doc_handle],
                        event: None,
                    }
                }
            }
        }
    } */
}

fn print_branch_doc (message: &str, doc_handle: &DocHandle) {
    doc_handle.with_doc(|d| {
        let files = d.get_obj_id(ROOT, "files").unwrap();

        let keys = d.keys( files).into_iter().collect::<Vec<_>>();

        println!("{:?}: {:?}", message, doc_handle.document_id());

        for key in keys {
            println!("  {:?}", key);
        }

    });
}


fn print_doc (message: &str, doc_handle: &DocHandle) {
    let checked_out_doc_json = doc_handle.with_doc(|d| serde_json::to_string(&automerge::AutoSerde::from(d)).unwrap());
    println!("rust: {:?}: {:?}", message, checked_out_doc_json);
}