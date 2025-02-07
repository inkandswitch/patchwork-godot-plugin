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

use crate::{
    godot_project::{BranchesMetadataDoc, GodotProjectDoc, StringOrPackedByteArray},
    godot_scene::PackedGodotScene,
    utils::get_linked_docs_of_branch,
};
use automerge::{
    patches::TextRepresentation, transaction::Transactable, Automerge, Change, ChangeHash, ObjType,
    PatchLog, ReadDoc, TextEncoding, ROOT,
};
use automerge_repo::{
    tokio::FsStorage, ConnDirection, DocHandle, DocumentId, Repo, RepoError, RepoHandle,
};
use autosurgeon::{bytes, hydrate, reconcile, Hydrate, HydrateError, Reconcile};
use futures::{
    channel::mpsc::{UnboundedReceiver, UnboundedSender},
    executor::block_on,
    FutureExt, StreamExt,
};
use std::ffi::c_void;
use std::ops::Deref;
use std::os::raw::c_char;

// use godot::prelude::*;
use tokio::{net::TcpStream, runtime::Runtime};

use crate::{doc_utils::SimpleDocReader, godot_project::Branch};

const SERVER_URL: &str = "104.131.179.247:8080";

pub enum DriverInputEvent {
    InitBranchesMetadataDoc {
        doc_id: Option<DocumentId>,
    },

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

pub enum DriverOutputEvent {
    Initialized {
        branches: HashMap<String, Branch>,
        checked_out_branch_doc_handle: DocHandle,
        branches_metadata_doc_handle: DocHandle,
    },
    DocHandleChanged {
        doc_handle: DocHandle,
    },
    BranchesUpdated {
        branches: HashMap<String, Branch>,
    },
    CheckedOutBranch {
        branch_doc_handle: DocHandle,
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

    pub fn spawn(
        &self,
        rx: UnboundedReceiver<DriverInputEvent>,
        tx: UnboundedSender<DriverOutputEvent>,
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
        mut rx: UnboundedReceiver<DriverInputEvent>,
        tx: UnboundedSender<DriverOutputEvent>,
    ) {
        let repo_handle = self.repo_handle.clone();

        self.runtime.spawn(async move {
            let mut state = DriverState {
                repo_handle,
                project: None,
                tx: tx.clone()
            };

            let mut tracked_doc_handle_ids: HashSet<DocumentId> = HashSet::new();
            let mut all_doc_changes =  futures::stream::SelectAll::new();

            // Now, drive the SelectAll and also wait for any new documents to arrive and add
            // them to the selectall
            loop {
                futures::select! {
                    changed_doc_handle = all_doc_changes.select_next_some() => {


                     println!("rust: Changed doc handle");

                      tx.unbounded_send(DriverOutputEvent::DocHandleChanged { doc_handle: changed_doc_handle }).unwrap();
                    },

                    message = rx.select_next_some() => {
                        let new_doc_handles : Vec<DocHandle> = match message {
                            DriverInputEvent::InitBranchesMetadataDoc { doc_id } => {
                                println!("rust: Initializing project with metadata doc: {:?}", doc_id);
                                state.init_project(doc_id).await
                            }

                            DriverInputEvent::CheckoutBranch { branch_doc_handle } => {
                                println!("rust: Checking out branch: {:?}", branch_doc_handle.document_id());
                                state.checkout_branch(branch_doc_handle).await
                            },

                            DriverInputEvent::CreateBranch {name} => {
                                println!("rust: Creating new branch: {}", name);
                                state.create_branch(name)
                            },

                            DriverInputEvent::MergeBranch { branch_doc_handle } => {
                                println!("rust: Merging branch: {:?}", branch_doc_handle.document_id());
                                state.merge_branch(branch_doc_handle)
                            },

                            DriverInputEvent::SaveFile { path, content, heads} => {
                                println!("rust: Saving file: {} (with {} heads)", path, heads.as_ref().map_or(0, |h| h.len()));
                                state.save_file(path, heads, content)
                            },                        
                        };

                        for doc_handle in new_doc_handles {
                            tx.unbounded_send(DriverOutputEvent::DocHandleChanged { doc_handle: doc_handle.clone() }).unwrap();

                            let doc_handle_id = doc_handle.document_id().clone();

                            // make sure we don't add the same doc handle twice
                            if tracked_doc_handle_ids.contains(&doc_handle_id) {
                                continue;
                            }
                            tracked_doc_handle_ids.insert(doc_handle_id);                        
                            let change_stream = handle_changes(doc_handle.clone()).filter_map(move |diff| {
                                let doc_handle = doc_handle.clone();
                                async move {
                                    if diff.is_empty() {
                                        None
                                    } else {
                                        Some(doc_handle.clone())
                                    }
                                }
                            });
            
                            all_doc_changes.push(change_stream.boxed());
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
    branches: HashMap<String, Branch>,
    tx: UnboundedSender<DriverOutputEvent>,
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
            reconcile(&mut tx, branches_metadata);
            tx.commit();
        });

        self.branches.insert(branch.id.clone(), branch.clone());
        self.tx
            .unbounded_send(DriverOutputEvent::BranchesUpdated {
                branches: self.branches.clone(),
            })
            .unwrap();
    }

    fn mark_branch_as_merged(&mut self, branch_doc_id: DocumentId) {
        self.branches_metadata_doc_handle.with_doc_mut(|d| {
            let mut branches_metadata: BranchesMetadataDoc = hydrate(d).unwrap();
            let mut tx = d.transaction();

            branches_metadata
                .branches
                .get_mut(&branch_doc_id.to_string())
                .unwrap()
                .is_merged = true;

            self.branches = branches_metadata.branches.clone();

            reconcile(&mut tx, branches_metadata);
            tx.commit();
        });

        self.tx
            .unbounded_send(DriverOutputEvent::BranchesUpdated {
                branches: self.branches.clone(),
            })
            .unwrap();
    }

    fn reconcile_branches(&self) {
        let branches_metadata: BranchesMetadataDoc = self
            .main_branch_doc_handle
            .with_doc(|d| hydrate(d).unwrap());

        self.tx
            .unbounded_send(DriverOutputEvent::BranchesUpdated {
                branches: branches_metadata.branches,
            })
            .unwrap();
    }
}

struct DriverState {
    repo_handle: RepoHandle,
    project: Option<ProjectState>,
    tx: UnboundedSender<DriverOutputEvent>,
}

impl DriverState {
    async fn init_project(&mut self, doc_id: Option<DocumentId>) -> Vec<DocHandle> {
        let mut new_doc_handles = vec![];

        match doc_id {
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

                new_doc_handles.push(branches_metadata_doc_handle.clone());

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

                self.project = Some(ProjectState {
                    branches_metadata_doc_handle,
                    main_branch_doc_handle: main_branch_doc_handle.clone(),
                    checked_out_branch_doc_handle: main_branch_doc_handle.clone(),
                    branches: branches_metadata.branches,
                    tx: self.tx.clone(),
                });
            }

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
                    branches,
                    tx: self.tx.clone(),
                });
            }
        }

        self.tx
            .unbounded_send(DriverOutputEvent::Initialized {
                branches: self.project.as_ref().unwrap().branches.clone(),
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
            .unbounded_send(DriverOutputEvent::BranchesUpdated {
                branches: project.branches.clone(),
            })
            .unwrap();
        self.tx
            .unbounded_send(DriverOutputEvent::CheckedOutBranch {
                branch_doc_handle: new_branch_handle.clone(),
            })
            .unwrap();

        return vec![new_branch_handle];
    }

    fn merge_branch(&mut self, branch_doc_handle: DocHandle) -> Vec<DocHandle> {
        let mut project = match &self.project {
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

        // mark branch as merged
        project.mark_branch_as_merged(branch_doc_handle.document_id());
        self.project = Some(project);

        return vec![];
    }

    async fn checkout_branch(&mut self, branch_doc_handle: DocHandle) -> Vec<DocHandle> {
        let mut new_doc_handles = vec![];

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
            .unbounded_send(DriverOutputEvent::CheckedOutBranch {
                branch_doc_handle: branch_doc_handle.clone(),
            })
            .unwrap();

        return new_doc_handles;
    }

    fn save_file(
        &mut self,
        path: String,
        heads: Option<Vec<ChangeHash>>,
        content: StringOrPackedByteArray,
    ) -> Vec<DocHandle> {
        let project = match &self.project {
            Some(project) => project.clone(),
            None => {
                println!("warning: triggered save file before project was initialized");
                return vec![];
            }
        };

        match content {
            StringOrPackedByteArray::String(content) => {
                println!("rust: save file: {:?} {:?}", path, content);
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

                return vec![];
            }
            StringOrPackedByteArray::Binary(content) => {
                // create binary doc
                let binary_doc_handle = self.repo_handle.new_document();
                binary_doc_handle.with_doc_mut(|d| {
                    let mut tx = d.transaction();
                    let _ = tx.put(ROOT, "content", content.to_vec());
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

                    let file_entry = tx.put_object(files, path, ObjType::Map);
                    let _ = tx.put(
                        file_entry.unwrap(),
                        "url",
                        format!("automerge:{}", &binary_doc_handle.document_id()),
                    );
                    tx.commit();
                });

                return vec![binary_doc_handle];
            }
        }
    }
}

fn handle_changes(handle: DocHandle) -> impl futures::Stream<Item = Vec<automerge::Patch>> + Send {
    futures::stream::unfold(handle, |doc_handle| async {
        let heads_before = doc_handle.with_doc(|d| d.get_heads().to_vec());
        let _ = doc_handle.changed().await;
        let heads_after = doc_handle.with_doc(|d| d.get_heads().to_vec());
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

fn clone_doc(repo_handle: &RepoHandle, doc_handle: &DocHandle) -> DocHandle {
    let new_doc_handle = repo_handle.new_document();

    let _ =
        doc_handle.with_doc_mut(|mut main_d| new_doc_handle.with_doc_mut(|d| d.merge(&mut main_d)));

    return new_doc_handle;
}
