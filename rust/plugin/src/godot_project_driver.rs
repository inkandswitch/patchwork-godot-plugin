use futures::Stream;
use ::safer_ffi::prelude::*;
use std::collections::HashSet;
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

    SaveFiles {
        files: HashMap<String,StringOrPackedByteArray>,
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
    FilesChanged {
        files: HashMap<String, StringOrPackedByteArray>,
    },
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
                tx: tx.clone(),
            };

            let mut subscribed_doc_handles = DocHandleSubscriptions::new();

    
            // Now, drive the SelectAll and also wait for any new documents to arrive and add
            // them to the selectall
            loop {
                futures::select! {
                    message = subscribed_doc_handles.futures.select_next_some() => {
                        let (new_doc_handles, event) = match message {
                            SubscriptionMessage::Changed { doc_handle, diff } => {                    
                                state.handle_doc_change(&doc_handle, &diff, &subscribed_doc_handles.subscribed_doc_handle_ids).await
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

                    message = rx.select_next_some() => {
                        let new_doc_handles : Vec<DocHandle> = match message {
                            InputEvent::InitBranchesMetadataDoc { doc_id } => {
                                println!("rust: Initializing project with metadata doc: {:?}", doc_id);
                                state.init_project(doc_id).await
                            }

                            InputEvent::CheckoutBranch { branch_doc_id } => {
                                println!("rust: Checking out branch: {:?}", branch_doc_id);
                                state.checkout_branch(branch_doc_id).await
                            },

                            InputEvent::CreateBranch {name} => {
                                println!("rust: Creating new branch: {}", name);
                                state.create_branch(name)
                            },

                            InputEvent::MergeBranch { branch_doc_handle } => {
                                println!("rust: Merging branch: {:?}", branch_doc_handle.document_id());
                                state.merge_branch(branch_doc_handle).await
                            },

                            InputEvent::SaveFiles { files, heads } => {
                                println!("rust: Saving {} files", files.len());
                                state.save_files(files, heads)
                            },
                        };


                        // first emit events for new doc handles
                        for doc_handle in new_doc_handles {
                            subscribed_doc_handles.add_doc_handle(doc_handle);
                        }  
                    }
                }
            }    
        });
    }
}

enum DocHandleType {
    BranchDoc,
    BinaryDoc,
    BranchesMetadataDoc,
}

// what should happen if you receive an update doc handle for each type
// BranchDoc -> check if all the binary files are loaded if not don't update the heads so the user sees an old version
// BinaryDoc -> check the checked out branch if this new file is the last missing binary file then update the heads

struct DocHandleWithType {
    doc_handle: DocHandle,
    doc_handle_type: DocHandleType,
    heads: Vec<ChangeHash>,
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

struct DriverState {
    repo_handle: RepoHandle,
    project: Option<ProjectState>,
    tx: UnboundedSender<OutputEvent>,
}

impl DriverState {

    async fn handle_doc_change(&mut self, doc_handle: &DocHandle, diff: &Vec<automerge::Patch>, loaded_doc_handle_ids: &HashSet<DocumentId>) -> (Vec<DocHandle>, Option<OutputEvent>) {
        let project = match &self.project {
            Some(project) => project.clone(),
            None => {
                return (vec![], None);
            }
        };


        let checked_out_branch_id = project.checked_out_branch_doc_handle.document_id();
        if checked_out_branch_id == doc_handle.document_id() {

            let linked_doc_ids: Vec<DocumentId> = project.checked_out_branch_doc_handle.with_doc(|d| {
                return get_linked_docs_of_branch(doc_handle);
            });

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


            let files_changed: HashMap<String, StringOrPackedByteArray> = diff.iter().filter_map(|patch| {
                let path = patch.path;

                if path.len() == 2 {
                    println!("rust: path: {:?}", path);
                }

                return None;

            }).collect::<HashMap<String, StringOrPackedByteArray>>();


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

    async fn init_project(&mut self, doc_id: Option<DocumentId>) -> Vec<DocHandle> {
        let mut new_doc_handles = vec![];

        match doc_id {

            // load existing project

            Some(doc_id) => {
                let branches_metadata_doc_handle =
                    match self.repo_handle.request_document(doc_id).await {
                        Ok(doc_handle) => doc_handle,
                        Err(e) => {
                            println!("failed init, can't load branches metadata doc: {:?}", e);
                            return vec![];
                        }
                    };

                new_doc_handles.push(branches_metadata_doc_handle.clone());

                let branches_metadata: BranchesMetadataDoc =
                    match branches_metadata_doc_handle.with_doc(|d| hydrate(d)) {
                        Ok(branches_metadata) => branches_metadata,
                        Err(e) => {
                            println!("failed init, can't hydrate metadata doc: {:?}", e);
                            return vec![];
                        }
                    };

                let main_branch_doc_id: DocumentId =
                    DocumentId::from_str(&branches_metadata.main_doc_id).unwrap();
                let main_branch_doc_handle =
                    match self.repo_handle.request_document(main_branch_doc_id).await {
                        Ok(doc_handle) => doc_handle,
                        Err(err) => {
                            println!("failed init, can't load main branchs doc: {:?}", err);
                            return vec![];
                        }
                    };

                new_doc_handles.push(main_branch_doc_handle.clone());

                let linked_doc_ids = get_linked_docs_of_branch(&main_branch_doc_handle);

                // alex ?
                // todo: do this in parallel
                let mut linked_doc_results = Vec::new();
                for doc_id in linked_doc_ids {
                    let result = self.repo_handle.request_document(doc_id).await;
                    if let Ok(doc_handle) = &result {
                        new_doc_handles.push(doc_handle.clone());
                    }
                    linked_doc_results.push(result);
                }

                if linked_doc_results.iter().any(|result| result.is_err()) {
                    println!("failed init, couldn't load all binary docs for ");
                    return vec![];
                }

                println!("RUST: init project main branch doc: {:?}", main_branch_doc_handle.document_id());

                self.project = Some(ProjectState {
                    branches_metadata_doc_handle,
                    main_branch_doc_handle: main_branch_doc_handle.clone(),
                    checked_out_branch_doc_handle: main_branch_doc_handle.clone(),
                });
            }

            // create new project

            None => {
                // Create new main branch doc
                let main_branch_doc_handle = self.repo_handle.new_document();
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

                println!(
                    "rust: main branch doc handle: {:?}",
                    main_branch_doc_handle.document_id()
                );

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
                let branches_metadata_doc_handle = self.repo_handle.new_document();
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

                self.project = Some(ProjectState {
                    branches_metadata_doc_handle,
                    main_branch_doc_handle: main_branch_doc_handle.clone(),
                    checked_out_branch_doc_handle: main_branch_doc_handle.clone(),
                });
            }
        }

        self.tx
            .unbounded_send(OutputEvent::Initialized {
                checked_out_branch_doc_handle: self
                    .project
                    .as_ref()
                    .unwrap()
                    .checked_out_branch_doc_handle
                    .clone(),
                branches_metadata_doc_handle: self
                    .project
                    .as_ref()
                    .unwrap()
                    .branches_metadata_doc_handle
                    .clone(),
            })
            .unwrap();

        return new_doc_handles;
    }

    fn create_branch(&mut self, name: String) -> Vec<DocHandle> {
        let mut project = match &self.project {
            Some(project) => project.clone(),
            None => {
                println!("warning: triggered create branch before project was initialized");
                return vec![];
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

        self.tx
            .unbounded_send(OutputEvent::CheckedOutBranch {
                branch_doc_handle: new_branch_handle.clone(),
            })
            .unwrap();

        return vec![new_branch_handle];
    }

    async fn merge_branch(&mut self, branch_doc_handle: DocHandle) -> Vec<DocHandle> {
        let project = match &self.project {
            Some(project) => project.clone(),
            None => {
                println!("warning: triggered merge branch before project was initialized");
                return vec![];
            }
        };

        branch_doc_handle.with_doc_mut(|branch_doc| {
            project.main_branch_doc_handle.with_doc_mut(|main_doc| {
                let _ = main_doc.merge(branch_doc);
            });
        });

        // todo: mark branch as merged
        ///project.mark_branch_as_merged(branch_doc_handle.document_id());
        let main_branch_doc_id = project.clone().main_branch_doc_handle.document_id();

        return self.checkout_branch(main_branch_doc_id).await;
    }

    async fn checkout_branch(&mut self, branch_doc_id: DocumentId) -> Vec<DocHandle> {
        let branch_doc_handle = match self.repo_handle.request_document(branch_doc_id).await {
            Ok(doc_handle) => doc_handle,
            Err(e) => {
                println!("failed to checkout branch: {:?}", e);
                return vec![];
            }
        };
        let mut new_doc_handles = vec![branch_doc_handle.clone()];

        let mut project = match &self.project {
            Some(project) => project.clone(),
            None => {
                println!("warning: triggered create branch before project was initialized");
                return vec![];
            }
        };

        let linked_doc_ids = get_linked_docs_of_branch(&branch_doc_handle);

        // alex ?
        // todo: do this in parallel
        let mut linked_doc_results = Vec::new();
        for doc_id in linked_doc_ids {
            let result = self.repo_handle.request_document(doc_id).await;
            if let Ok(doc_handle) = &result {
                new_doc_handles.push(doc_handle.clone());
            }
            linked_doc_results.push(result);
        }

        if linked_doc_results.iter().any(|result| result.is_err()) {
            println!("failed to checkout branch, some linked docs are missing:");

            for result in linked_doc_results {
                if let Err(e) = result {
                    println!("{:?}", e);
                }
            }
            return vec![];
        }

        project.checked_out_branch_doc_handle = branch_doc_handle.clone();

        self.project = Some(project);
        self.tx
            .unbounded_send(OutputEvent::CheckedOutBranch {
                branch_doc_handle: branch_doc_handle.clone(),
            })
            .unwrap();
        return new_doc_handles;
    }

    fn save_files(
        &mut self,
        files: HashMap<String, StringOrPackedByteArray>,
        heads: Option<Vec<ChangeHash>>,
    ) -> Vec<DocHandle> {
        let project = match &self.project {
            Some(project) => project.clone(),
            None => {
                println!("warning: triggered save files before project was initialized");
                return vec![];
            }
        };

        let mut new_doc_handles = Vec::new();

        project.checked_out_branch_doc_handle.with_doc_mut(|d| {
            let mut tx = match heads {
                Some(heads) => d.transaction_at(
                    PatchLog::inactive(TextRepresentation::String(TextEncoding::Utf8CodeUnit)),
                    &heads,
                ),
                None => d.transaction(),
            };

            let files_obj = tx.get_obj_id(ROOT, "files").unwrap();

            for (path, content) in files {
                match content {
                    StringOrPackedByteArray::String(content) => {
                        // Handle text content
                        let file_entry = match tx.get(&files_obj, &path) {
                            Ok(Some((automerge::Value::Object(ObjType::Map), file_entry))) => file_entry,
                            _ => tx.put_object(&files_obj, &path, ObjType::Map).unwrap(),
                        };

                        // Remove existing url if present
                        if let Ok(Some((_, _))) = tx.get(&file_entry, "url") {
                            let _ = tx.delete(&file_entry, "url");
                        }

                        // Update or create text content
                        let content_key = match tx.get(&file_entry, "content") {
                            Ok(Some((automerge::Value::Object(ObjType::Text), content))) => content,
                            _ => tx.put_object(&file_entry, "content", ObjType::Text).unwrap(),
                        };
                        let _ = tx.update_text(&content_key, &content);
                    }
                    StringOrPackedByteArray::Binary(content) => {
                        // Create binary doc
                        let binary_doc_handle = self.repo_handle.new_document();
                        binary_doc_handle.with_doc_mut(|binary_d| {
                            let mut binary_tx = binary_d.transaction();
                            let _ = binary_tx.put(ROOT, "content", content);
                            binary_tx.commit();
                        });
                        new_doc_handles.push(binary_doc_handle.clone());

                        // Store reference in project doc
                        let file_entry = tx.put_object(&files_obj, &path, ObjType::Map).unwrap();
                        let _ = tx.put(
                            file_entry,
                            "url",
                            format!("automerge:{}", &binary_doc_handle.document_id()),
                        );
                    }
                }
            }

            tx.commit();
        });

        return new_doc_handles;
    }
}


fn clone_doc(repo_handle: &RepoHandle, doc_handle: &DocHandle) -> DocHandle {
    let new_doc_handle = repo_handle.new_document();

    let _ =
        doc_handle.with_doc_mut(|mut main_d| new_doc_handle.with_doc_mut(|d| d.merge(&mut main_d)));

    return new_doc_handle;
}
