use std::{
  collections::HashMap, future::Future, str::FromStr, sync::{
      mpsc::{Receiver, Sender},
      Arc, Mutex,
  }
};
use std::collections::HashSet;
use std::env::var;
use ::safer_ffi::prelude::*;

use automerge::{patches::TextRepresentation, transaction::Transactable, Automerge, Change, ChangeHash, ObjType, PatchLog, ReadDoc, TextEncoding, ROOT};
use automerge_repo::{tokio::FsStorage, ConnDirection, DocHandle, DocumentId, Repo, RepoError, RepoHandle};
use autosurgeon::{bytes, hydrate, reconcile, Doc, Hydrate, HydrateError, Reconcile};
use futures::{channel::mpsc::{UnboundedReceiver, UnboundedSender}, executor::block_on, FutureExt, StreamExt};
use std::ffi::c_void;
use std::ops::Deref;
use std::os::raw::c_char;
use crate::{doc_handle_map::DocHandleMap, godot_project::{BranchesMetadataDoc, GodotProjectDoc}, utils::get_linked_docs_of_branch};

// use godot::prelude::*;
use tokio::{net::TcpStream, runtime::Runtime};

use crate::{ doc_utils::SimpleDocReader, godot_project::Branch};

const SERVER_URL: &str = "104.131.179.247:8080";


pub enum DriverInputEvent {
  InitBranchesMetadataDoc {
    doc_id: Option<DocumentId>,
  }, 

  CheckoutBranch {
    branch_doc_id: DocumentId,
  },

  CreateBranch {
    name: String
  },

  // only trigger this event internally in the driver
  NewDocHandle {
    doc_handle: DocHandle,
  }
}

pub enum DriverOutputEvent {
    Initialized,
    DocHandleChanged {
        doc_handle: DocHandle,
    },
    BranchesUpdated {
        branches: HashMap<String, Branch>,
    },
    CheckedOutBranch {
        branch_doc_id: DocumentId,
    },
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

    pub fn spawn(&self, rx: UnboundedReceiver<DriverInputEvent>, tx: UnboundedSender<DriverOutputEvent>) {
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

    fn spawn_driver_task(&self, mut rx: UnboundedReceiver<DriverInputEvent>, tx: UnboundedSender<DriverOutputEvent>) {
        let repo_handle = self.repo_handle.clone();

        self.runtime.spawn(async move {

            let mut state = DriverState {
              repo: RepoState {
                repo_handle,
                doc_handles: HashMap::new(),
                tx: tx.clone()
             },
              project: None,
              tx: tx.clone()
            };

            let mut all_doc_changes =  futures::stream::SelectAll::new();

            // Now, drive the SelectAll and also wait for any new documents to arrive and add
            // them to the selectall
            loop {
                futures::select! {
                    changed_doc_handle = all_doc_changes.select_next_some() => {


                      tx.unbounded_send(DriverOutputEvent::DocHandleChanged { doc_handle: changed_doc_handle }).unwrap();
                    },

                    message = rx.select_next_some() => {
                        match message {
                            DriverInputEvent::InitBranchesMetadataDoc { doc_id } => {                            
                                state.init_project(doc_id);
                            }

                            DriverInputEvent::CheckoutBranch { branch_doc_id } => {
                              state.checkout_branch(branch_doc_id);
                            },

                            DriverInputEvent::CreateBranch {name} => {
                                state.create_branch(name);
                            },

                            DriverInputEvent::NewDocHandle { doc_handle } => {
                                let change_stream = handle_changes(doc_handle.clone()).filter_map(move |diff| {

                                  // trigger the load all binary files

                                  let doc_handle = doc_handle.clone();
                                  async move {
                                      if diff.is_empty() {
                                          None
                                      } else {
                                          Some(
                                            doc_handle
                                          )
                                      }
                                  }
                              });                      

                              all_doc_changes.push(change_stream.boxed());
                            }
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

struct RepoState {
    repo_handle: RepoHandle,
    doc_handles: HashMap<DocumentId, DocHandle>,
    tx: UnboundedSender<DriverOutputEvent>
}

impl RepoState {

    fn clone_doc(&mut self, doc_handle: DocHandle) -> DocHandle {
        let new_doc_handle = self.new_document();

        let _ = doc_handle
            .with_doc_mut(|mut main_d| new_doc_handle.with_doc_mut(|d| d.merge(&mut main_d)));        


        return new_doc_handle
    }

    fn new_document(&mut self) -> DocHandle {
        let doc_handle = self.repo_handle.new_document();

        self._add_handle(doc_handle.clone());
        self.tx.unbounded_send(DriverOutputEvent::DocHandleChanged { doc_handle: doc_handle.clone() }).unwrap();

        doc_handle
    }

    async fn request_document(&mut self, doc_id: &DocumentId) -> Result<DocHandle, RepoError> {                
        if let Some(doc_handle) = self.doc_handles.get(doc_id) {
            return Ok(doc_handle.clone());
        }

        let doc_handle = match self.repo_handle.request_document(doc_id.clone()).await {
            Ok(handle) => handle,
            Err(e) => {
                return Err(e);
            }
        };

        self._add_handle(doc_handle.clone());
        self.tx.unbounded_send(DriverOutputEvent::DocHandleChanged { doc_handle: doc_handle.clone() }).unwrap();

        Ok(doc_handle)
    }

    fn _add_handle(&mut self, doc_handle: DocHandle) {
        let doc_id = doc_handle.document_id();
        if self.doc_handles.insert(doc_id.clone(), doc_handle.clone()).is_none() {

            // self.tx.unbounded_send(DriverOutputEvent::DocHandleChanged { doc_handle: doc_handle.clone() }).unwrap();
        }


        self.tx.unbounded_send(DriverOutputEvent::DocHandleChanged { doc_handle: doc_handle.clone() }).unwrap();      
    }    
}

#[derive(Clone)]
struct ProjectState {
    branches_metadata_doc_handle: DocHandle,
    main_branch_doc_handle: DocHandle,
    checked_out_branch_doc_handle: DocHandle,
    branches: HashMap<String, Branch>,
    tx: UnboundedSender<DriverOutputEvent>
}

impl ProjectState {

    fn add_branch(&mut self, branch: Branch) {    
        
        let branch_clone = branch.clone();
        self.branches_metadata_doc_handle.with_doc_mut(|d| {   
            let mut branches_metadata: BranchesMetadataDoc = hydrate(d).unwrap();                 
            let mut tx = d.transaction(); 
            branches_metadata.branches.insert(branch_clone.id.clone(), branch_clone);
            reconcile(&mut tx, branches_metadata);
            tx.commit();
        });


        self.branches.insert(branch.id.clone(), branch.clone());
        self.tx.unbounded_send(DriverOutputEvent::BranchesUpdated { branches: self.branches.clone() }).unwrap();
    }

    fn reconcile_branches (&self) {
        let branches_metadata:BranchesMetadataDoc = self.main_branch_doc_handle.with_doc(|d| hydrate(d).unwrap());

        self.tx.unbounded_send(DriverOutputEvent::BranchesUpdated { branches: branches_metadata.branches }).unwrap();
    }

}


struct DriverState {
    repo: RepoState,
    project: Option<ProjectState>,
    tx: UnboundedSender<DriverOutputEvent>,
}

impl DriverState {

    async fn init_project (&mut self, doc_id:Option<DocumentId>) {
        match doc_id {
            Some(doc_id) => {
                let branches_metadata_doc_handle = match self.repo.request_document(&doc_id).await {
                    Ok(doc_handle) => doc_handle,
                    Err(e) => {
                    println!("failed init, can't load branches metadata doc: {:?}", e);
                    return;
                    }
                };                                        

                let branches_metadata : BranchesMetadataDoc = match branches_metadata_doc_handle.with_doc(|d| hydrate(d)) {
                    Ok(branches_metadata) => branches_metadata,
                    Err(e) => {
                    println!("failed init, can't hydrate metadata doc: {:?}", e);
                    return;
                    }
                };

                let main_branch_doc_id: DocumentId = DocumentId::from_str(&branches_metadata.main_doc_id).unwrap();
                let main_branch_doc_handle = match self.repo.request_document(&main_branch_doc_id).await {
                    Ok(doc_handle) => doc_handle,
                    Err(err) => {
                        println!("failed init, can't load main branchs doc");
                        return
                    }
                };

                let linked_doc_ids = get_linked_docs_of_branch(&main_branch_doc_handle);

                // alex ?
                // todo: do this in parallel
                let mut linked_doc_results = Vec::new();
                for doc_id in linked_doc_ids {
                    let result = self.repo.request_document(&doc_id).await;
                    linked_doc_results.push(result);
                }                

                if linked_doc_results.iter().any(|result| result.is_err()) {
                    println!("failed init, couldn't load all binary docs for ");

                    for result in linked_doc_results {
                    if let Err(e) = result {
                        println!("{:?}", e);
                    }
                    }
                    return;
                }


                self.project = Some(ProjectState {
                    branches_metadata_doc_handle,
                    main_branch_doc_handle: main_branch_doc_handle.clone(),
                    checked_out_branch_doc_handle: main_branch_doc_handle.clone(),
                    branches: branches_metadata.branches,
                    tx: self.tx.clone()
                });
            }

            None => {                                    
                // Create new main branch doc
                let main_branch_doc_handle = self.repo.new_document();
                main_branch_doc_handle.with_doc_mut(|d| {
                    let mut tx = d.transaction();
                    let _ = reconcile(
                        &mut tx,
                        GodotProjectDoc {
                            files: HashMap::new(),
                            state: HashMap::new()
                        },
                    );
                    tx.commit();
                });
            
                let main_branch_doc_id = main_branch_doc_handle.document_id().to_string();
                let main_branch_doc_id_clone = main_branch_doc_id.clone();
                let branches =  HashMap::from([
                        (main_branch_doc_id,  Branch { name: String::from("main"), id: main_branch_doc_handle.document_id().to_string(), is_merged: true })                                                    
                    ]);
                let branches_clone = branches.clone();
                
                // create new branches metadata doc
                let branches_metadata_doc_handle = self.repo.new_document();
                branches_metadata_doc_handle.with_doc_mut(|d| {
                    let mut tx = d.transaction();
                    let _ = reconcile(
                        &mut tx,
                        BranchesMetadataDoc {
                            main_doc_id: main_branch_doc_id_clone,
                            branches: branches_clone
                        },
                    );
                    tx.commit();
                });                                        

                self.project = Some(ProjectState {
                    branches_metadata_doc_handle,
                    main_branch_doc_handle: main_branch_doc_handle.clone(),
                    checked_out_branch_doc_handle: main_branch_doc_handle.clone(),
                    branches,
                    tx: self.tx.clone()
                });
            }
        }

        self.tx.unbounded_send(DriverOutputEvent::Initialized).unwrap();
    }


    fn create_branch (&mut self, name: String) {       

        let mut project = match &self.project {
            Some(project) => project.clone(),
            None => {
                println!("warning: triggered create branch before project was initialized");
                return;
            },
        };

        let new_branch_handle = self.repo.clone_doc(project.main_branch_doc_handle.clone());
        
        project.add_branch(Branch { id: new_branch_handle.document_id().to_string().clone(), name: name.clone(), is_merged: false });
        project.checked_out_branch_doc_handle = new_branch_handle.clone();

        self.project = Some(project.clone());

        self.tx.unbounded_send(DriverOutputEvent::BranchesUpdated { branches: project.branches.clone() }).unwrap();
        self.tx.unbounded_send(DriverOutputEvent::CheckedOutBranch { branch_doc_id: new_branch_handle.document_id().clone() }).unwrap();
    }

    async fn checkout_branch (&mut self, branch_doc_id: DocumentId) {
        let mut project = match &self.project {
            Some(project) => project.clone(),
            None => {
                println!("warning: triggered create branch before project was initialized");
                return;
            },
        };

        let branch_doc_handle = match self.repo.request_document(&branch_doc_id).await {
            Ok(doc_handle) => doc_handle,
            Err(e) => {
              println!("failed to load branch doc: {:?}", e);
              return;
            },
        };                                        

        let linked_doc_ids = get_linked_docs_of_branch(&branch_doc_handle);

        // alex ?
        // todo: do this in parallel
        let mut linked_doc_results = Vec::new();
        for doc_id in linked_doc_ids {
            let result = self.repo.request_document(&doc_id).await;
            linked_doc_results.push(result);
        }                

        if linked_doc_results.iter().any(|result| result.is_err()) {
            println!("failed to checkout branch, some linked docs are missing:");

            for result in linked_doc_results {
              if let Err(e) = result {
                println!("{:?}", e);
              }
            }
            return;
        }

        project.checked_out_branch_doc_handle = branch_doc_handle.clone();

        self.project = Some(project);
        self.tx.unbounded_send(DriverOutputEvent::CheckedOutBranch { branch_doc_id: branch_doc_handle.document_id().clone() }).unwrap();

    }

}

fn handle_changes(handle: DocHandle) -> impl futures::Stream<Item = Vec<automerge::Patch>> + Send {
    futures::stream::unfold(handle, |doc_handle| async {
        let heads_before = doc_handle.with_doc(|d| d.get_heads().to_vec());
        let _ = doc_handle.changed().await;
        if crate::godot_project::is_branch_doc(&doc_handle) {}
        // todo: if this is a branch doc, check if all the binary files are loaded if not don't update the heads so the user sees an old version

        let heads_after = doc_handle.with_doc(|d| d.get_heads().to_vec());
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


