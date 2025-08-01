use ::safer_ffi::prelude::*;
use automerge_repo::{PeerConnectionInfo, Repo, RepoError, RepoId};
use futures::stream::FuturesUnordered;
use futures::{FutureExt, Stream};
use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use std::{collections::HashMap, str::FromStr};
use tokio::task::JoinHandle;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

use crate::file_utils::FileContent;
use crate::godot_parser::GodotScene;
use crate::godot_project::{ForkInfo, MergeInfo};
use crate::utils::{
    commit_with_attribution_and_timestamp, print_branch_state, CommitMetadata, MergeMetadata, ToShortForm,
};
use crate::{
    godot_parser,
    godot_project::{BranchesMetadataDoc, GodotProjectDoc},
    utils::get_linked_docs_of_branch,
};
use automerge::{
    patches::TextRepresentation, transaction::Transactable, ChangeHash, ObjType, PatchLog, ReadDoc,
    TextEncoding, ROOT,
};
use automerge_repo::{tokio::FsStorage, ConnDirection, DocHandle, DocumentId, RepoHandle};
use autosurgeon::{hydrate, reconcile};
use futures::{
    channel::mpsc::{UnboundedReceiver, UnboundedSender},
    StreamExt,
};

use tokio::{net::TcpStream, runtime::Runtime};

use crate::{doc_utils::SimpleDocReader, godot_project::Branch};


const SERVER_REPO_ID: &str = "sync-server";

#[derive(Debug, Clone)]
pub enum InputEvent {
    CreateBranch {
        name: String,
        source_branch_doc_id: DocumentId,
    },

    CreateMergePreviewBranch {
        source_branch_doc_id: DocumentId,
        target_branch_doc_id: DocumentId,
    },

    MergeBranch {
        source_branch_doc_id: DocumentId,
        target_branch_doc_id: DocumentId,
    },

    DeleteBranch {
        branch_doc_id: DocumentId,
    },

    SaveFiles {
        branch_doc_handle: DocHandle,
        heads: Option<Vec<ChangeHash>>,
        files: Vec<(String, FileContent)>,
    },

	InitialCheckin {
		branch_doc_handle: DocHandle,
		heads: Option<Vec<ChangeHash>>,
        files: Vec<(String, FileContent)>,
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
        trigger_reload: bool,
    },

    CompletedCreateBranch {
        branch_doc_id: DocumentId,
    },

    CompletedShutdown,

    PeerConnectionInfoChanged {
        peer_connection_info: PeerConnectionInfo,
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

#[derive(Debug, Clone)]
pub struct BinaryDocState {
    doc_handle: Option<DocHandle>, // is null if the binary doc is being requested but not loaded yet
    path: String,
}

#[derive(Debug, Clone)]
pub struct BranchStateForkInfo {
    pub forked_from: DocumentId,
    pub forked_at: Vec<ChangeHash>,
}

#[derive(Debug, Clone)]
pub struct BranchStateMergeInfo {
    pub merge_into: DocumentId,
    pub merge_at: Vec<ChangeHash>,
}

#[derive(Debug, Clone)]
pub struct BranchState {
    pub name: String,
    pub doc_handle: DocHandle,
    pub linked_doc_ids: HashSet<DocumentId>,
    pub synced_heads: Vec<ChangeHash>,
    pub fork_info: Option<BranchStateForkInfo>,
    pub merge_info: Option<BranchStateMergeInfo>,
    pub is_main: bool,
	pub created_by: Option<String>,
}

impl BranchState {
    pub fn is_synced(&self) -> bool {
        self.synced_heads == self.doc_handle.with_doc(|d| d.get_heads())
    }
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

    requesting_binary_docs: FuturesUnordered<
        Pin<Box<dyn Future<Output = (String, Result<DocHandle, RepoError>)> + Send>>,
    >,
    requesting_branch_docs: FuturesUnordered<
        Pin<Box<dyn Future<Output = (String, Result<DocHandle, RepoError>)> + Send>>,
    >,

    subscribed_doc_ids: HashSet<DocumentId>,
    all_doc_changes: futures::stream::SelectAll<
        std::pin::Pin<Box<dyn Stream<Item = SubscriptionMessage> + Send>>,
    >,

    // heads that the frontend has for each branch doc
    heads_in_frontend: HashMap<DocumentId, Vec<ChangeHash>>,
}

pub enum ConnectionThreadError {
	ConnectionThreadDied(String),
	ConnectionThreadError(String),
}

#[derive(Debug)]
pub struct GodotProjectDriver {
    runtime: Runtime,
    repo_handle: RepoHandle,
	server_url: String,
	connection_thread_output_rx: Option<UnboundedReceiver<String>>,
	retries: u32,
    connection_thread: Option<JoinHandle<()>>,
    spawned_thread: Option<JoinHandle<()>>,
}

impl GodotProjectDriver {
    pub fn create(storage_folder_path: String, server_url: String) -> Self {
        let runtime: Runtime = tokio::runtime::Builder::new_multi_thread()
			.worker_threads(4)
            .enable_all()
			.thread_name("GodotProjectDriver: worker thread")
            .build()
            .unwrap();

        let _guard = runtime.enter();

        let storage = FsStorage::open(storage_folder_path).unwrap();

        let repo = Repo::new(None, Box::new(storage));
        let repo_handle = repo.run();

        return Self {
            runtime,
            repo_handle,
			server_url,
			retries: 0,
			connection_thread_output_rx: None,
            connection_thread: None,
            spawned_thread: None,
        };
    }

    pub fn spawn(
        &mut self,
        rx: UnboundedReceiver<InputEvent>,
        tx: UnboundedSender<OutputEvent>,
        branches_metadata_doc_id: Option<DocumentId>,
        user_name: Option<String>,
    ) {
        if self.connection_thread.is_some() || self.spawned_thread.is_some() {
            tracing::warn!("driver already spawned");
            return;
        }

        self.respawn_connection_thread();

        // Spawn sync task for all doc handles
        self.spawned_thread =
            Some(self.spawn_driver_task(rx, tx, branches_metadata_doc_id, &user_name));
    }

    pub fn teardown(&mut self) {
        if let Some(connection_thread) = self.connection_thread.take() {
            connection_thread.abort();
        }
        if let Some(spawned_thread) = self.spawned_thread.take() {
            spawned_thread.abort();
        }
    }

	fn connection_thread_died(&self) -> bool {
		if let Some(connection_thread) = &self.connection_thread {
			return connection_thread.is_finished();
		}
		false
	}

	pub fn connection_thread_get_last_error(&mut self) -> Option<ConnectionThreadError> {
		if let Some(connection_thread_rx) = &mut self.connection_thread_output_rx {
			let mut error_str = String::new();
			while let Ok(Some(error)) = connection_thread_rx.try_next() {
				error_str.push_str("\n");
				error_str.push_str(&error);
			}
			if self.connection_thread_died() {
				self.retries += 1;
				return Some(ConnectionThreadError::ConnectionThreadDied(error_str));
			}
			if !error_str.is_empty() {
				return Some(ConnectionThreadError::ConnectionThreadError(error_str));
			}
		}
		None
	}

	pub fn respawn_connection_thread(&mut self) -> bool {
		if let Some(connection_thread) = self.connection_thread.take() {
			if !connection_thread.is_finished() {
				tracing::warn!("WARNING: connection thread is not finished, aborting");
				connection_thread.abort();
			}
		}
		if self.retries > 6 {
			tracing::error!("connection thread failed too many times, aborting");
			return false;
		}
		let (connection_thread_tx, connection_thread_rx) = futures::channel::mpsc::unbounded();
		self.connection_thread_output_rx = Some(connection_thread_rx);
		self.connection_thread = Some(self.spawn_connection_task(connection_thread_tx));
		return true;
	}

    fn spawn_connection_task(&self, connection_thread_tx: UnboundedSender<String>) -> JoinHandle<()> {
        let repo_handle_clone = self.repo_handle.clone();
		let retries = self.retries;
		let server_url = self.server_url.clone();
        return self.runtime.spawn(async move {
            tracing::info!("start a client");
			let backoff = 2_f64.powf(retries as f64) * 100.0;
			if retries > 0 {
				tracing::error!("connection thread failed, retrying in {}ms...", backoff);
				tokio::time::sleep(std::time::Duration::from_millis(backoff as u64)).await;
			}
            loop {
				tracing::info!("Attempting to connect to automerge repo...");
                // Start a client.
                let stream = loop {
                    // Try to connect to a peer
                    let res = TcpStream::connect(server_url.clone()).await;
                    if let Err(e) = res {
                        tracing::error!("error connecting: {:?}", e);
                        // sleep for 1 second
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                        continue;
                    }
                    break res.unwrap();
                };
                tracing::info!("Connected successfully!");

                match repo_handle_clone
                    .connect_tokio_io(server_url.clone(), stream, ConnDirection::Outgoing)
                    .await
                {
                    Ok(completed) => {
                        let error = completed.await;
                        tracing::error!("connection terminated because of: {:?}", error);
                        connection_thread_tx.unbounded_send(error.to_string()).unwrap();
                    }
                    Err(e) => {
                        tracing::error!("Failed to connect: {:?}", e);
                        connection_thread_tx.unbounded_send(e.to_string()).unwrap();

                        // sleep for 5 seconds
                        tokio::time::sleep(std::time::Duration::from_secs(5)).await;

                        continue;
                    }
                }

            }
        });
    }

    fn spawn_driver_task(
        &self,
        mut rx: UnboundedReceiver<InputEvent>,
        tx: UnboundedSender<OutputEvent>,
        branches_metadata_doc_id: Option<DocumentId>,
        user_name: &Option<String>,
    ) -> JoinHandle<()> {
        let repo_handle = self.repo_handle.clone();
        let user_name = user_name.clone();

        // let filter = EnvFilter::new("info").add_directive("automerge_repo=info".parse().unwrap());
        // if let Err(e) = tracing_subscriber::registry()
        //     .with(tracing_subscriber::fmt::layer().with_writer(std::io::stdout))
        //     .with(filter)
        //     .try_init()
        // {
        //     tracing::error!("Failed to initialize tracing subscriber: {:?}", e);
        // } else {
        //     tracing::info!("Tracing subscriber initialized");
        // }

        return self.runtime.spawn(async move {
            // destructure project doc handles
            let ProjectDocHandles { branches_metadata_doc_handle, main_branch_doc_handle } = init_project_doc_handles(&repo_handle, &branches_metadata_doc_id, &user_name).await;

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
                heads_in_frontend: HashMap::new(),
            };

            state.update_branch_doc_state(state.main_branch_doc_handle.clone());
            state.subscribe_to_doc_handle(state.branches_metadata_doc_handle.clone());
            state.subscribe_to_doc_handle(state.main_branch_doc_handle.clone());

            let mut sync_server_conn_info_changes = repo_handle.peer_conn_info_changes(RepoId::from(SERVER_REPO_ID)).fuse();

            loop {
                let repo_handle_clone = repo_handle.clone();

                futures::select! {

                    next = sync_server_conn_info_changes.next() => {
                        if let Some(info) = next {
                            // TODO: do we need to update the synced_heads here?
                            tx.unbounded_send(OutputEvent::PeerConnectionInfoChanged { peer_connection_info: info.clone() }).unwrap();
                        };
                    },

                    next = state.requesting_binary_docs.next() => {
                        if let Some((path, result)) = next {
                            match result {
                                Ok(doc_handle) => {
                                    state.add_binary_doc_handle(&path, &doc_handle);
                                },
                                Err(e) => {
                                    tracing::error!("error requesting binary doc: {:?}", e);
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
                                    tracing::debug!("added branch doc: {:?}", branch_name);

                                }
                                Err(e) => {
                                    tracing::error!("error requesting branch doc: {:?}", e);
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
                            InputEvent::CreateBranch { name, source_branch_doc_id } => {
                                state.create_branch(name.clone(), source_branch_doc_id.clone());
                            },

                            InputEvent::CreateMergePreviewBranch { source_branch_doc_id, target_branch_doc_id } => {
                                state.create_merge_preview_branch(source_branch_doc_id, target_branch_doc_id);
                            },

                            InputEvent::DeleteBranch { branch_doc_id } => {
                                state.delete_branch(branch_doc_id);
                            },

                            InputEvent::MergeBranch { source_branch_doc_id, target_branch_doc_id } => {
                                state.merge_branch(source_branch_doc_id, target_branch_doc_id);
                            },

                            InputEvent::SaveFiles { branch_doc_handle, files, heads } => {
                                state.save_files(branch_doc_handle, files, heads, false);
                            },

							InputEvent::InitialCheckin { branch_doc_handle, files, heads } => {
                                state.save_files(branch_doc_handle, files, heads, true);
                            },

                            InputEvent::StartShutdown => {
                                tracing::info!("shutting down");

                                let result = repo_handle_clone.stop();

                                tracing::info!("shutdown result: {:?}", result);

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

async fn init_project_doc_handles(
    repo_handle: &RepoHandle,
    branches_metadata_doc_id: &Option<DocumentId>,
    user_name: &Option<String>,
) -> ProjectDocHandles {
    match branches_metadata_doc_id {
        // load existing project
        Some(doc_id) => {
            tracing::debug!("loading existing project: {:?}", doc_id);

            let branches_metadata_doc_handle = repo_handle
                .request_document(doc_id.clone())
                .await
                .unwrap_or_else(|e| {
                    panic!("failed init, can't load branches metadata doc: {:?}", e);
                });

            let branches_metadata: BranchesMetadataDoc =
                branches_metadata_doc_handle.with_doc(|d| {
                    hydrate(d).unwrap_or_else(|_| {
                        panic!("failed init, can't hydrate metadata doc");
                    })
                });

            let main_branch_doc_handle: DocHandle = repo_handle
                .request_document(DocumentId::from_str(&branches_metadata.main_doc_id).unwrap())
                .await
                .unwrap_or_else(|_| {
                    panic!("failed init, can't load main branchs doc");
                });

            return ProjectDocHandles {
                branches_metadata_doc_handle: branches_metadata_doc_handle.clone(),
                main_branch_doc_handle: main_branch_doc_handle.clone(),
            };
        }

        // create new project
        None => {
            tracing::debug!("creating new project");

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
                commit_with_attribution_and_timestamp(
                    tx,
                    &CommitMetadata {
                        username: user_name.clone(),
                        branch_id: Some(main_branch_doc_handle.document_id().to_string()),
                        merge_metadata: None,
                    },
                );
            });

            let main_branch_doc_id = main_branch_doc_handle.document_id().to_string();
            let main_branch_doc_id_clone = main_branch_doc_id.clone();
            let branches = HashMap::from([(
                main_branch_doc_id,
                Branch {
                    name: String::from("main"),
                    id: main_branch_doc_handle.document_id().to_string(),
                    fork_info: None,
                    merge_info: None,
					created_by: user_name.clone(),
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
                commit_with_attribution_and_timestamp(
                    tx,
                    &CommitMetadata {
                        username: user_name.clone(),
                        branch_id: None,
                        merge_metadata: None,
                    },
                );
            });

            return ProjectDocHandles {
                branches_metadata_doc_handle: branches_metadata_doc_handle.clone(),
                main_branch_doc_handle: main_branch_doc_handle.clone(),
            };
        }
    }
}

impl DriverState {
    fn create_branch(&mut self, name: String, source_branch_doc_id: DocumentId) {
        let source_branch_doc_handle = self
            .branch_states
            .get(&source_branch_doc_id)
            .unwrap()
            .doc_handle
            .clone();

        let new_branch_handle = clone_doc(&self.repo_handle, &source_branch_doc_handle);

        let branch = Branch {
            name: name.clone(),
            id: new_branch_handle.document_id().to_string(),
            fork_info: Some(ForkInfo {
                forked_from: source_branch_doc_id.to_string(),
                forked_at: source_branch_doc_handle
                    .with_doc(|d| d.get_heads())
                    .iter()
                    .map(|h| h.to_string())
                    .collect(),
            }),
            merge_info: None,
			created_by: self.user_name.clone(),
        };

        self.tx
            .unbounded_send(OutputEvent::CompletedCreateBranch {
                branch_doc_id: new_branch_handle.document_id(),
            })
            .unwrap();

        self.branches_metadata_doc_handle.with_doc_mut(|d| {
            let mut branches_metadata: BranchesMetadataDoc = hydrate(d).unwrap();
            let mut tx = d.transaction();
            branches_metadata.branches.insert(branch.id.clone(), branch);
            let _ = reconcile(&mut tx, branches_metadata);
            commit_with_attribution_and_timestamp(
                tx,
                &CommitMetadata {
                    username: self.user_name.clone(),
                    branch_id: None,
                    merge_metadata: None,
                },
            );
        });
    }

    fn create_merge_preview_branch(
        &mut self,
        source_branch_doc_id: DocumentId,
        target_branch_doc_id: DocumentId,
    ) {
        tracing::debug!("driver: create merge preview branch");

        let source_branch_state = self.branch_states.get(&source_branch_doc_id).unwrap();
        let target_branch_state = self.branch_states.get(&target_branch_doc_id).unwrap();

        let merge_preview_branch_doc_handle = self.repo_handle.new_document();

        source_branch_state
            .doc_handle
            .with_doc_mut(|source_branch_doc| {
                merge_preview_branch_doc_handle.with_doc_mut(|merge_preview_branch_doc| {
                    let _ = merge_preview_branch_doc.merge(source_branch_doc);
                });
            });

        target_branch_state
            .doc_handle
            .with_doc_mut(|target_branch_doc| {
                merge_preview_branch_doc_handle.with_doc_mut(|merge_preview_branch_doc| {
                    let _ = merge_preview_branch_doc.merge(target_branch_doc);
                });
            });

        let branch = Branch {
            name: format!(
                "{} <- {}",
                target_branch_state.name, source_branch_state.name
            ),
            id: merge_preview_branch_doc_handle.document_id().to_string(),
            fork_info: Some(ForkInfo {
                forked_from: source_branch_doc_id.to_string(),
                forked_at: source_branch_state
                    .synced_heads
                    .iter()
                    .map(|h| h.to_string())
                    .collect(),
            }),
            merge_info: Some(MergeInfo {
                merge_into: target_branch_doc_id.to_string(),
                merge_at: target_branch_state
                    .synced_heads
                    .iter()
                    .map(|h| h.to_string())
                    .collect(),
            }),
			created_by: self.user_name.clone(),
        };

        self.branches_metadata_doc_handle.with_doc_mut(|d| {
            let mut branches_metadata: BranchesMetadataDoc = hydrate(d).unwrap();
            let mut tx = d.transaction();
            branches_metadata.branches.insert(branch.id.clone(), branch);
            let _ = reconcile(&mut tx, branches_metadata);
            commit_with_attribution_and_timestamp(
                tx,
                &CommitMetadata {
                    username: self.user_name.clone(),
                    branch_id: Some(source_branch_doc_id.to_string()),
                    merge_metadata: None,
                },
            );
        });

        self.tx
            .unbounded_send(OutputEvent::CompletedCreateBranch {
                branch_doc_id: merge_preview_branch_doc_handle.document_id(),
            })
            .unwrap();
    }

    // delete branch isn't fully implemented right now deletes are not propagated to the frontend
    // right now this is just useful to clean up merge preview branches
    fn delete_branch(&mut self, branch_doc_id: DocumentId) {
        tracing::debug!("driver: delete branch {:?}", branch_doc_id);

        self.branches_metadata_doc_handle.with_doc_mut(|d| {
            let mut tx = d.transaction();
            let mut branches_metadata: BranchesMetadataDoc = hydrate(&mut tx).unwrap();
            branches_metadata
                .branches
                .remove(&branch_doc_id.to_string());
            let _ = reconcile(&mut tx, branches_metadata);
            commit_with_attribution_and_timestamp(
                tx,
                &CommitMetadata {
                    username: self.user_name.clone(),
                    branch_id: None,
                    merge_metadata: None,
                },
            );
        });
    }

    fn save_files(
        &mut self,
        branch_doc_handle: DocHandle,
        file_entries: Vec<(String, FileContent)>,
        heads: Option<Vec<ChangeHash>>,
		new_project: bool
    ) {
        let branch_doc_state = self
            .branch_states
            .get(&branch_doc_handle.document_id())
            .unwrap()
            .clone();

        let mut binary_entries: Vec<(String, DocHandle)> = Vec::new();
        let mut text_entries: Vec<(String, &String)> = Vec::new();
        let mut scene_entries: Vec<(String, &GodotScene)> = Vec::new();
		let mut deleted_entries: Vec<String> = Vec::new();

        for (path, content) in file_entries.iter() {
            match content {
                FileContent::Binary(content) => {
                    let binary_doc_handle = self.repo_handle.new_document();
                    binary_doc_handle.with_doc_mut(|d| {
                        let mut tx = d.transaction();
                        let _ = tx.put(ROOT, "content", content.clone());
                        commit_with_attribution_and_timestamp(
                            tx,
                            &CommitMetadata {
                                username: self.user_name.clone(),
                                branch_id: None,
                                merge_metadata: None,
                            },
                        );
                    });

                    self.add_binary_doc_handle(path, &binary_doc_handle);
                    binary_entries.push((path.clone(), binary_doc_handle));
                }
                FileContent::String(content) => {
                    text_entries.push((path.clone(), content));
                }
                FileContent::Scene(godot_scene) => {
                    scene_entries.push((path.clone(), godot_scene));
                }
                FileContent::Deleted => {
					deleted_entries.push(path.clone());
                }
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

                // delete structured content in file entry if it previously had one
                if let Ok(Some((_, _))) = tx.get(&file_entry, "structured_content") {
                    let _ = tx.delete(&file_entry, "structured_content");
                }

                // either get existing text or create new text
                let content_key = match tx.get(&file_entry, "content") {
                    Ok(Some((automerge::Value::Object(ObjType::Text), content))) => content,
                    _ => tx
                        .put_object(&file_entry, "content", ObjType::Text)
                        .unwrap(),
                };
                let _ = tx.update_text(&content_key, &content);
            }

            // write scene entries to doc
            for (path, godot_scene) in scene_entries {
                godot_scene.reconcile(&mut tx, path);
            }

            // write binary entries to doc
            for (path, binary_doc_handle) in binary_entries {
                let file_entry = tx.put_object(&files, &path, ObjType::Map);
                let _ = tx.put(
                    file_entry.unwrap(),
                    "url",
                    format!("automerge:{}", &binary_doc_handle.document_id()),
                );
            }

			for path in deleted_entries {
				let _ = tx.delete(&files, &path);
			}

            commit_with_attribution_and_timestamp(
                tx,
                &CommitMetadata {
                    username: self.user_name.clone(),
                    branch_id: Some(branch_doc_state.doc_handle.document_id().to_string()),
                    merge_metadata: None,
                },
            );
        });

        // update heads in frontend
		if !new_project {
			self.heads_in_frontend.insert(
				branch_doc_handle.document_id(),
				branch_doc_handle.with_doc(|d| d.get_heads()),
			);
		}

        tracing::debug!("save on branch {:?} {:?}", branch_doc_state.name, self.heads_in_frontend);
    }

    fn merge_branch(&mut self, source_branch_doc_id: DocumentId, target_branch_doc_id: DocumentId) {
        let source_branch_state = self.branch_states.get(&source_branch_doc_id).unwrap();
        let target_branch_state = self.branch_states.get(&target_branch_doc_id).unwrap();

        source_branch_state
            .doc_handle
            .with_doc_mut(|source_branch_doc| {
                target_branch_state
                    .doc_handle
                    .with_doc_mut(|target_branch_doc| {
                        let _ = target_branch_doc.merge(source_branch_doc);
                    });
            });

        // if the branch has some merge_info we know that it's a merge preview branch
        let merge_metadata = if source_branch_state.merge_info.is_some() {
            let original_branch_state = self
                .branch_states
                .get(&source_branch_state.fork_info.as_ref().unwrap().forked_from)
                .unwrap();

            Some(MergeMetadata {
                merged_branch_id: original_branch_state.doc_handle.document_id().to_string(),
                merged_at_heads: original_branch_state.synced_heads.clone(),
                forked_at_heads: original_branch_state
                    .fork_info
                    .as_ref()
                    .unwrap()
                    .forked_at
                    .clone(),
            })
        } else {
            // todo: implement this case
            None
        };

        if let Some(merge_metadata) = merge_metadata {
            target_branch_state.doc_handle.with_doc_mut(|d| {
                let mut tx = d.transaction();

                // do a dummy change that we can attach some metadata to
                let changed = tx.get_int(&ROOT, "_changed").unwrap_or(0);
                let _ = tx.put(ROOT, "_changed", changed + 1);

                commit_with_attribution_and_timestamp(
                    tx,
                    &CommitMetadata {
                        username: self.user_name.clone(),
                        branch_id: Some(target_branch_doc_id.to_string()),
                        merge_metadata: Some(merge_metadata),
                    },
                );
            });
        }
    }

    fn update_branch_doc_state(&mut self, branch_doc_handle: DocHandle) {
        let branch_state = match self.branch_states.get_mut(&branch_doc_handle.document_id()) {
            Some(branch_state) => branch_state,
            None => {
                let branch = self
                    .get_branches_metadata()
                    .branches
                    .get(&branch_doc_handle.document_id().to_string())
                    .unwrap()
                    .clone();

                self.branch_states.insert(
                    branch_doc_handle.document_id().clone(),
                    BranchState {
                        name: branch.name.clone(),
                        doc_handle: branch_doc_handle.clone(),
                        linked_doc_ids: HashSet::new(),
                        synced_heads: Vec::new(),
                        fork_info: match branch.fork_info {
                            Some(fork_info) => Some(BranchStateForkInfo {
                                forked_from: DocumentId::from_str(&fork_info.forked_from).unwrap(),
                                forked_at: fork_info
                                    .forked_at
                                    .iter()
                                    .map(|h| ChangeHash::from_str(h).unwrap())
                                    .collect(),
                            }),
                            None => None,
                        },
                        merge_info: match branch.merge_info {
                            Some(merge_info) => Some(BranchStateMergeInfo {
                                merge_into: DocumentId::from_str(&merge_info.merge_into).unwrap(),
                                merge_at: merge_info
                                    .merge_at
                                    .iter()
                                    .map(|h| ChangeHash::from_str(h).unwrap())
                                    .collect(),
                            }),
                            None => None,
                        },
                        is_main: branch_doc_handle.document_id()
                            == self.main_branch_doc_handle.document_id(),
						created_by: branch.created_by.clone(),
                    },
                );
                self.branch_states
                    .get_mut(&branch_doc_handle.document_id())
                    .unwrap()
            }
        };

        let linked_docs = get_linked_docs_of_branch(&branch_doc_handle);

        // load binary docs if not already loaded
        for (path, doc_id) in linked_docs.iter() {
            if self.binary_doc_states.get(&doc_id).is_some()
                || self.pending_binary_doc_ids.contains(&doc_id)
            {
                continue;
            }

            self.pending_binary_doc_ids.insert(doc_id.clone());

            let path = path.clone();
            self.requesting_binary_docs.push(
                self.repo_handle
                    .request_document(doc_id.clone())
                    .map(|doc_handle| (path, doc_handle))
                    .boxed(),
            );
        }

        // update linked doc ids
        branch_state.linked_doc_ids = linked_docs.values().cloned().collect();

        let missing_binary_doc_ids =
            get_missing_binary_doc_ids(&branch_state, &self.binary_doc_states);

        // check if all linked docs have been loaded
        if missing_binary_doc_ids.is_empty() {
            branch_state.synced_heads = branch_doc_handle.with_doc(|d| d.get_heads());

            print_branch_state("branch doc state immediately loaded", &branch_state);

            self.tx
                .unbounded_send(OutputEvent::BranchStateChanged {
                    branch_state: branch_state.clone(),
                    trigger_reload: !does_frontend_have_branch_at_heads(
                        &self.heads_in_frontend,
                        &branch_state
                    ),
                })
                .unwrap();
        }
    }

    fn add_binary_doc_handle(&mut self, path: &String, binary_doc_handle: &DocHandle) {
        self.binary_doc_states.insert(
            binary_doc_handle.document_id().clone(),
            BinaryDocState {
                doc_handle: Some(binary_doc_handle.clone()),
                path: path.clone(),
            },
        );

        let _ = &self
            .tx
            .unbounded_send(OutputEvent::NewDocHandle {
                doc_handle: binary_doc_handle.clone(),
                doc_handle_type: DocHandleType::Binary,
            })
            .unwrap();

        // tracing::debug!("add_binary_doc_handle {:?} {:?}", path, binary_doc_handle.document_id());

        // check all branch states that link to this doc
        for branch_state in self.branch_states.values_mut() {
            if branch_state
                .linked_doc_ids
                .contains(&binary_doc_handle.document_id())
            {
                let missing_binary_doc_ids =
                    get_missing_binary_doc_ids(&branch_state, &self.binary_doc_states);

                // check if all linked docs have been loaded
                if missing_binary_doc_ids.is_empty() {
                    branch_state.synced_heads = branch_state.doc_handle.with_doc(|d| d.get_heads());
                    self.tx
                        .unbounded_send(OutputEvent::BranchStateChanged {
                            branch_state: branch_state.clone(),
                            trigger_reload: !does_frontend_have_branch_at_heads(
                                &self.heads_in_frontend,
                                &branch_state
                            ),
                        })
                        .unwrap();

                    tracing::debug!("branch {:?} (id: {:?}): state loaded with heads {}", branch_state.name, branch_state.doc_handle.document_id(), branch_state.synced_heads.to_short_form());
                } else {
                    tracing::debug!("branch {:?} (id: {:?}): state still missing {:?} binary docs", branch_state.name, branch_state.doc_handle.document_id(), missing_binary_doc_ids.len());
					tracing::trace!("missing binary doc ids: {:?}", missing_binary_doc_ids);
                }
            }
        }
    }

    pub fn subscribe_to_doc_handle(&mut self, doc_handle: DocHandle) {
        if self.subscribed_doc_ids.contains(&doc_handle.document_id()) {
            return;
        }

        self.subscribed_doc_ids.insert(doc_handle.document_id());
        self.all_doc_changes
            .push(handle_changes(doc_handle.clone()).boxed());
        self.all_doc_changes.push(
            futures::stream::once(async move { SubscriptionMessage::Added { doc_handle } }).boxed(),
        );
    }

    fn get_branches_metadata(&self) -> BranchesMetadataDoc {
        let branches_metadata: BranchesMetadataDoc = self
            .branches_metadata_doc_handle
            .with_doc(|d| hydrate(d).unwrap());

        return branches_metadata;
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
            SubscriptionMessage::Changed {
                doc_handle: doc_handle.clone(),
                diff,
            },
            doc_handle,
        ))
    })
}

fn get_missing_binary_doc_ids(
    branch_state: &BranchState,
    binary_doc_states: &HashMap<DocumentId, BinaryDocState>,
) -> Vec<DocumentId> {
    branch_state
        .linked_doc_ids
        .iter()
        .filter(|doc_id| {
            binary_doc_states
                .get(doc_id)
                .map_or(true, |binary_doc_state| {
                    binary_doc_state
                        .doc_handle
                        .as_ref()
                        .map_or(true, |handle| handle.with_doc(|d| d.get_heads().is_empty()))
                })
        })
        .cloned()
        .collect::<Vec<_>>()
}

fn does_frontend_have_branch_at_heads(
    heads_in_frontend: &HashMap<DocumentId, Vec<ChangeHash>>,
    branch_state: &BranchState,
) -> bool {
    tracing::trace!(
        "Checking if frontend has branch {:?} (id: {:?}) at heads {:?}",
        branch_state.name,
        branch_state.doc_handle.document_id(),
        heads_in_frontend
    );

    if let Some(synced_heads) = heads_in_frontend.get(&branch_state.doc_handle.document_id()) {
		let result = synced_heads == &branch_state.synced_heads;
		tracing::trace!("comparing {:?} == {:?}: {:?}", synced_heads, branch_state.synced_heads, result);
        tracing::info!("Frontend has branch {:?} (id: {:?}) at heads {:?}: {:?}", branch_state.name, branch_state.doc_handle.document_id(), synced_heads, result);
        result
    } else {
		tracing::info!("no synced heads found for branch {:?} (id: {:?})", branch_state.name, branch_state.doc_handle.document_id());
        false
    }
}
