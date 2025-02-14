use futures::stream::FuturesUnordered;
use futures::Stream;
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

//const SERVER_URL: &str = "104.131.179.247:8000"; // invalid server url
const SERVER_URL: &str = "104.131.179.247:8080";

#[derive(Debug, Clone)]
pub enum InputEvent {
    InitBranchesMetadataDoc {
        doc_id: Option<DocumentId>,
    },

    CreateBranch {
        name: String,
    },

    CheckoutBranch {
        branch_doc_id: DocumentId,
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
    ) {
        // Spawn connection task
        self.spawn_connection_task();

        // Spawn sync task for all doc handles
        self.spawn_driver_task(rx, tx);
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
    ) {
        let repo_handle = self.repo_handle.clone();


        self.runtime.spawn(async move {
            let mut state = DriverState {
                repo_handle,
                project: None,

                // todo: track state of branches

                // branch_deps_map: HashMap::new(),
                // doc_handles: HashMap::new(),
            };

            let mut subscribed_doc_handles = DocHandleSubscriptions::new();

    
            let mut requesting_doc_handles : FuturesUnordered<_> = Vec::new().into_iter().collect();

            requesting_doc_handles.push(state.repo_handle.request_document(DocumentId::from_str("123").unwrap()));

            let mut pending_tasks: FuturesUnordered<Pin<Box<dyn Future<Output = TaskResult> + Send>>> = FuturesUnordered::new();

            // Now, drive the SelectAll and also wait for any new documents to arrive and add
            // them to the selectall
            loop {
                futures::select! {
                    message = subscribed_doc_handles.futures.select_next_some() => {
                        let (new_doc_handles, event) = match message {
                            SubscriptionMessage::Changed { doc_handle, diff } => {
                                state.handle_doc_change(&doc_handle, &subscribed_doc_handles.subscribed_doc_handle_ids).await
                            },
                            SubscriptionMessage::Added { doc_handle } => {
                                tx.unbounded_send(OutputEvent::NewDocHandle { doc_handle: doc_handle.clone() }).unwrap();
                                state.handle_doc_change(&doc_handle, &subscribed_doc_handles.subscribed_doc_handle_ids).await
                            },
                        };            

                        // first emit events for new doc handles
                        for doc_handle in new_doc_handles {
                            subscribed_doc_handles.add_doc_handle(doc_handle);
                        }

                        // ... so that they are available when we trigger the event that depends on them
                        if let Some(event) = event {
                            tx.unbounded_send(event).unwrap();
                        }                    
                    },

                    /* 
                    message = requesting_doc_handles.select_next_some() => {


                    },*/

                    /*task_result = pending_tasks.select_next_some() => {                    
                        for doc_handle in task_result.new_doc_handles {
                            subscribed_doc_handles.add_doc_handle(doc_handle);
                        }

                        if let Some(project) = task_result.project {
                            state.project = Some(project);
                        }

                        if let Some(event) = task_result.event {
                            tx.unbounded_send(event).unwrap();
                        }
                    },*/                

                    message = rx.select_next_some() => {
                         match message {
                            InputEvent::InitBranchesMetadataDoc { doc_id } => {
                                println!("rust: Recieved init branches metadata doc: {:?}", doc_id);

                                pending_tasks.push(Box::pin(state.init_project(doc_id)));
                            }

                            InputEvent::CheckoutBranch { branch_doc_id } => {
                                println!("rust: Checking out branch: {:?}", branch_doc_id);
                                pending_tasks.push(Box::pin(state.checkout_branch(branch_doc_id)));
                            },

                            InputEvent::CreateBranch {name} => {
                                println!("rust: Creating new branch: {}", name);
                                pending_tasks.push(Box::pin(std::future::ready(state.create_branch(name))));
                            },

                            InputEvent::MergeBranch { branch_doc_handle } => {
                                println!("rust: Merging branch: {:?}", branch_doc_handle.document_id());
                                pending_tasks.push(Box::pin(state.merge_branch(branch_doc_handle)));
                            },

                            InputEvent::SaveFile { path, content, heads} => {
                                println!("rust: Saving file: {} (with {} heads)", path, heads.as_ref().map_or(0, |h| h.len()));
                                pending_tasks.push(Box::pin(state.save_file(&path, heads, content)));
                            }
                        };


                        // first emit events for new doc handles
                       /* for doc_handle in new_doc_handles {
                            subscribed_doc_handles.add_doc_handle(doc_handle);
                        } */ 
                    }
                }
            }    
        });
    }
}

#[derive(Clone)]
struct ProjectState {
    branches_metadata_doc_handle: DocHandle,
    main_branch_doc_handle: DocHandle,
    checked_out_branch_doc_handle: DocHandle,
}

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

    fn get_branches_metadata(&self) -> BranchesMetadataDoc {
        let branches_metadata : BranchesMetadataDoc = self
            .branches_metadata_doc_handle
            .with_doc(|d| hydrate(d).unwrap());

        return branches_metadata
    }
}

struct TaskResult {
    project: Option<ProjectState>,
    new_doc_handles: Vec<DocHandle>,
    event: Option<OutputEvent>,
}

struct DriverState {
    repo_handle: RepoHandle,
    project: Option<ProjectState>,
}

impl DriverState {

    // return a vector of linked docs
    async fn handle_doc_change(&mut self, doc_handle: &DocHandle, loaded_doc_handle_ids: &HashSet<DocumentId>) -> (Vec<DocHandle>, Option<OutputEvent>) {
        let project = match &self.project {
            Some(project) => project.clone(),
            None => {
                return (vec![], None);
            }
        };

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

    fn init_project(&self, doc_id: Option<DocumentId>) -> impl Future<Output = TaskResult> + 'static {
        let repo_handle = self.repo_handle.clone();


        println!("rust: CALL init project {:?}", doc_id);
        
        async move {
            match doc_id {

                // load existing project

                Some(doc_id) => {
                    let mut new_doc_handles = vec![];

                    let branches_metadata_doc_handle = repo_handle.request_document(doc_id.clone()).await.unwrap_or_else(|e| {
                        panic!("failed init, can't load branches metadata doc: {:?}", e);
                    });
                                
                    new_doc_handles.push(branches_metadata_doc_handle.clone());

                    let branches_metadata: BranchesMetadataDoc = branches_metadata_doc_handle.with_doc(|d| hydrate(d).unwrap_or_else(|_| {
                        panic!("failed init, can't hydrate metadata doc");
                    }));
                
                    let main_branch_doc_handle: DocHandle =
                        repo_handle.request_document(DocumentId::from_str(&branches_metadata.main_doc_id).unwrap()).await.unwrap_or_else(|_| {
                            panic!("failed init, can't load main branchs doc");
                        });

                    new_doc_handles.push(main_branch_doc_handle.clone());

                    let linked_doc_ids = get_linked_docs_of_branch(&main_branch_doc_handle);

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
                        panic!("failed init, couldn't load all binary docs for {:?}", doc_id);
                    }

                    println!("RUST: init project main branch doc: {:?}", main_branch_doc_handle.document_id());

                    return TaskResult {
                        project: Some(ProjectState {
                            branches_metadata_doc_handle: branches_metadata_doc_handle.clone(),
                            main_branch_doc_handle: main_branch_doc_handle.clone(),
                            checked_out_branch_doc_handle: main_branch_doc_handle.clone(),
                        }),
                        new_doc_handles,
                        event: Some(OutputEvent::Initialized {
                            checked_out_branch_doc_handle: main_branch_doc_handle.clone(),
                            branches_metadata_doc_handle: branches_metadata_doc_handle.clone(),
                        }),
                    }
                }

                // create new project

                None => {
                    let mut new_doc_handles = vec![];

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

                    new_doc_handles.push(main_branch_doc_handle.clone());

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
                    new_doc_handles.push(branches_metadata_doc_handle.clone());

                    println!(
                        "rust: branches metadata doc handle: {:?}",
                        branches_metadata_doc_handle.document_id()
                    );

                    return TaskResult {
                        project: Some(ProjectState {
                            branches_metadata_doc_handle: branches_metadata_doc_handle.clone(),
                            main_branch_doc_handle: main_branch_doc_handle.clone(),
                            checked_out_branch_doc_handle: main_branch_doc_handle.clone(),
                        }),
                        new_doc_handles,
                        event: Some(OutputEvent::Initialized {
                            checked_out_branch_doc_handle: main_branch_doc_handle.clone(),
                            branches_metadata_doc_handle: branches_metadata_doc_handle.clone(),
                        }),
                    }
                }
            };        
        }
    }

    fn create_branch(&mut self, name: String) -> TaskResult {
        let mut project = match &self.project {
            Some(project) => project.clone(),
            None => {
                panic!("triggered create branch '{}' before project was initialized", name);
            }
        };

        let new_branch_handle = clone_doc(&self.repo_handle, &project.main_branch_doc_handle);

        project.add_branch(Branch {
            id: new_branch_handle.document_id().to_string().clone(),
            name: name.clone(),
            is_merged: false,
        });
        project.checked_out_branch_doc_handle = new_branch_handle.clone();

        self.project = Some(project.clone());

        TaskResult {
            project: Some(project),
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
    }
}


fn clone_doc(repo_handle: &RepoHandle, doc_handle: &DocHandle) -> DocHandle {
    let new_doc_handle = repo_handle.new_document();

    let _ =
        doc_handle.with_doc_mut(|mut main_d| new_doc_handle.with_doc_mut(|d| d.merge(&mut main_d)));

    return new_doc_handle;
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