use automerge::Automerge;
use ::safer_ffi::prelude::*;
use samod::{ConnDirection, ConnFinishedReason, ConnectionInfo, DocHandle, DocumentId, Repo, Stopped};
use futures::stream::FuturesUnordered;
use futures::{FutureExt, Stream};
use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use std::{collections::HashMap, str::FromStr};
use tokio::task::JoinHandle;

use crate::helpers::branch::{BinaryDocState, BranchState, BranchStateForkInfo, BranchStateMergeInfo, BranchStateRevertInfo};
use crate::fs::file_utils::FileContent;
use crate::parser::godot_parser::GodotScene;
use crate::helpers::doc_utils::SimpleDocReader;
use crate::helpers::utils::ToShortForm;
use crate::helpers::utils::{
    ChangeType, ChangedFile, CommitMetadata, MergeMetadata, commit_with_attribution_and_timestamp, get_default_patch_log, heads_to_vec_string, print_branch_state
};
use crate::{
    helpers::branch::{BranchesMetadataDoc, GodotProjectDoc, ForkInfo, MergeInfo, Branch},
    helpers::utils::get_linked_docs_of_branch,
};
use automerge::{
    transaction::Transactable, ChangeHash, ObjType, ReadDoc,
    ROOT,
};
use autosurgeon::{hydrate, reconcile };
use futures::{
    channel::mpsc::{UnboundedReceiver, UnboundedSender},
    StreamExt,
};

use tokio::{net::TcpStream, runtime::Runtime};

const SERVER_REPO_ID: &str = "sync-server";

#[derive(Clone)]
pub enum InputEvent {
    CreateBranch {
        name: String,
        source_branch_doc_id: DocumentId,
    },

    CreateMergePreviewBranch {
        source_branch_doc_id: DocumentId,
        target_branch_doc_id: DocumentId,
    },

	CreateRevertPreviewBranch {
		branch_doc_id: DocumentId,
		files: Vec<(String, FileContent)>,
		revert_to: Vec<ChangeHash>,
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

	RevertTo {
		branch_doc_handle: DocHandle,
		heads: Option<Vec<ChangeHash>>,
        files: Vec<(String, FileContent)>,
		revert_to: Vec<ChangeHash>,
	},

    SetUserName {
        name: String,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum DocHandleType {
    Binary,
    Unknown,
}

#[derive(Clone)]
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

    PeerConnectionInfoChanged {
        peer_connection_info: Option<ConnectionInfo>,
    },
}

enum SubscriptionMessage {
    Changed {
        doc_handle: DocHandle,
    },
    Added {
        doc_handle: DocHandle,
    },
}


impl BranchState {
    pub fn is_synced(&self) -> bool {
        self.synced_heads == self.doc_handle.with_document(|d| d.get_heads())
    }
}

struct DriverState {
    tx: UnboundedSender<OutputEvent>,
    repo_handle: Repo,

    user_name: Option<String>,

    main_branch_doc_handle: DocHandle,
    branches_metadata_doc_handle: DocHandle,

    binary_doc_states: HashMap<DocumentId, BinaryDocState>,
    branch_states: HashMap<DocumentId, BranchState>,

    pending_branch_doc_ids: HashSet<DocumentId>,
    pending_binary_doc_ids: HashSet<DocumentId>,

	// TODO (Samod): No request_document!
    requesting_binary_docs: FuturesUnordered<
        Pin<Box<dyn Future<Output = (String, Result<Option<DocHandle>, Stopped>)> + Send>>,
    >,
    requesting_branch_docs: FuturesUnordered<
        Pin<Box<dyn Future<Output = (String, Result<Option<DocHandle>, Stopped>)> + Send>>,
    >,

    subscribed_doc_ids: HashSet<DocumentId>,
    all_doc_changes: futures::stream::SelectAll<
        std::pin::Pin<Box<dyn Stream<Item = SubscriptionMessage> + Send>>,
    >,

    // heads that the frontend has for each branch doc
    heads_in_frontend: HashMap<DocumentId, Vec<ChangeHash>>,

	// TODO (Samod): Remove this hack
	peer: Option<ConnectionInfo>
}

pub enum ConnectionThreadError {
	ConnectionThreadDied(String),
	ConnectionThreadError(String),
}

pub struct ProjectDriver {
    runtime: Runtime,
    repo_handle: Repo,
	server_url: String,
	connection_thread_output_rx: Option<UnboundedReceiver<String>>,
	retries: u32,
    connection_thread: Option<JoinHandle<()>>,
    spawned_thread: Option<JoinHandle<()>>,
}

impl ProjectDriver {
    pub async fn create(storage_folder_path: String, server_url: String) -> Self {
        let runtime: Runtime = tokio::runtime::Builder::new_multi_thread()
			.worker_threads(1)
            .enable_all()
			.thread_name("GodotProjectDriver: worker thread")
            .build()
            .unwrap();

        let _guard = runtime.enter();

        // let storage = FsStorage::open(storage_folder_path).unwrap();

        // let repo = Repo::new(None, Box::new(storage));
        // let repo_handle = repo.run();

		let storage = samod::storage::TokioFilesystemStorage::new(storage_folder_path);

		let repo_handle = Repo::build_tokio()
			.with_storage(storage)
			.load()
			.await;

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

				let connection = repo_handle_clone
                    .connect_tokio_io(stream, ConnDirection::Outgoing).unwrap();
                let completed = connection.finished().await;
				tracing::error!("connection terminated because of: {:?}", completed);
            	connection_thread_tx.unbounded_send(format!("{:?}", completed)).unwrap();
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

            tx.unbounded_send(OutputEvent::Initialized { project_doc_id: branches_metadata_doc_handle.document_id().clone() }).unwrap();

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
				peer: None
            };

            state.update_branch_doc_state(state.main_branch_doc_handle.clone());
            state.subscribe_to_doc_handle(state.branches_metadata_doc_handle.clone());
            state.subscribe_to_doc_handle(state.main_branch_doc_handle.clone());

			// TODO (Samod): We need to find an alternative to this line:
            //     let mut sync_server_conn_info_changes = repo_handle.peer_conn_info_changes(RepoId::from(SERVER_REPO_ID)).fuse();
			// AFAIK there's no equivalent in samod. We need to track repo_handle.connected_peers()
			// and dispatch an event when there's a difference.
            loop {
				// TODO (Samod): Remove this hack once Alex makes his PR in samod
				let peers = repo_handle.connected_peers().await;
				let first = peers.first();
				if first != state.peer.as_ref() {
					tx.unbounded_send(OutputEvent::PeerConnectionInfoChanged { peer_connection_info: first.cloned() }).unwrap();
					state.peer = first.cloned();
				}

                futures::select! {
                    next = state.requesting_binary_docs.next() => {
                        if let Some((path, result)) = next {
                            match result {
                                Ok(Some(doc_handle)) => {
                                    state.add_binary_doc_handle(&path, &doc_handle);
                                },
								Ok(None) => tracing::error!("binary doc not found"),
                                Err(_) => {
                                    tracing::error!("error requesting binary doc: repo stopped");
                                }
                            }
                        }
                    },

                    next = state.requesting_branch_docs.next() => {
                        if let Some((branch_name, result)) = next {
                            match result {
                                Ok(Some(doc_handle)) => {
                                    state.pending_branch_doc_ids.remove(&doc_handle.document_id());
                                    state.update_branch_doc_state(doc_handle.clone());
                                    state.subscribe_to_doc_handle(doc_handle.clone());
                                    tracing::debug!("added branch doc: {:?}", branch_name);

                                }
								Ok(None) => tracing::error!("branch doc not found"),
                                Err(_) => {
                                    tracing::error!("error requesting branch doc: repo stopped");
                                }
                            }
                        }
                    },

                    message = state.all_doc_changes.select_next_some() => {
                       let doc_handle = match message {
                            SubscriptionMessage::Changed { doc_handle } => {
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

                                    state.requesting_branch_docs.push(repo_handle.find(branch_id.clone()).map(|doc_handle| {
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
                                state.create_branch(name.clone(), source_branch_doc_id.clone()).await;
                            },

							InputEvent::CreateRevertPreviewBranch { branch_doc_id, files, revert_to } => {
								state.create_revert_preview_branch(branch_doc_id, files, revert_to).await;
							},

                            InputEvent::CreateMergePreviewBranch { source_branch_doc_id, target_branch_doc_id } => {
                                state.create_merge_preview_branch(source_branch_doc_id, target_branch_doc_id).await;
                            },

                            InputEvent::DeleteBranch { branch_doc_id } => {
                                state.delete_branch(branch_doc_id);
                            },

                            InputEvent::MergeBranch { source_branch_doc_id, target_branch_doc_id } => {
                                state.merge_branch(source_branch_doc_id, target_branch_doc_id);
                            },

                            InputEvent::SaveFiles { branch_doc_handle, files, heads } => {
                                state.save_files(branch_doc_handle, files, heads, false, None).await;
                            },

							InputEvent::InitialCheckin { branch_doc_handle, files, heads } => {
                                state.save_files(branch_doc_handle, files, heads, true, None).await;
                            },

							InputEvent::RevertTo { branch_doc_handle, files, heads, revert_to } => {
                                state.save_files(branch_doc_handle, files, heads, false, Some(revert_to)).await;
                            },

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
    repo_handle: &Repo,
    branches_metadata_doc_id: &Option<DocumentId>,
    user_name: &Option<String>,
) -> ProjectDocHandles {
    match branches_metadata_doc_id {
        // load existing project
        Some(doc_id) => {
            tracing::debug!("loading existing project: {:?}", doc_id);

            let branches_metadata_doc_handle = repo_handle
				// TODO (Samod): No request_document!
                .find(doc_id.clone())
                .await
                .unwrap_or_else(|e| {
                    panic!("failed init, can't load branches metadata doc: {:?}", e);
                }).unwrap_or_else(|| {
					// TODO (Samod): Can we panic here or do we have to fail gracefully?
                    panic!("failed init, no branches metadata doc");
                });

            let branches_metadata: BranchesMetadataDoc =
                branches_metadata_doc_handle.with_document(|d| {
                    hydrate(d).unwrap_or_else(|_| {
                        panic!("failed init, can't hydrate metadata doc");
                    })
                });

            let main_branch_doc_handle = repo_handle
                .find(DocumentId::from_str(&branches_metadata.main_doc_id).unwrap())
                .await
                .unwrap_or_else(|_| {
                    panic!("failed init, can't load main branchs doc");
                }).unwrap_or_else(|| {
					// TODO (Samod): Can we panic here or do we have to fail gracefully?
                    panic!("failed init, no branches metadata doc");
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
            let main_branch_doc_handle = repo_handle.create(Automerge::new()).await.unwrap();
            main_branch_doc_handle.with_document(|d| {
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
                        branch_id: Some(main_branch_doc_handle.document_id().clone()),
                        merge_metadata: None,
						reverted_to: None,
                        changed_files: None,
						is_setup: Some(true)
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
					merged_into: None,
					reverted_to: None,
                },
            )]);
            let branches_clone = branches.clone();

            // create new branches metadata doc
            let branches_metadata_doc_handle = repo_handle.create(Automerge::new()).await.unwrap();
            branches_metadata_doc_handle.with_document(|d| {
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
						reverted_to: None,
                        changed_files: None,
						is_setup: Some(true)
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
    async fn create_branch(&mut self, name: String, source_branch_doc_id: DocumentId) {
        let source_branch_doc_handle = self
            .branch_states
            .get(&source_branch_doc_id)
            .unwrap()
            .doc_handle
            .clone();

        let new_branch_handle = clone_doc(&self.repo_handle, &source_branch_doc_handle).await;

        let branch = Branch {
            name: name.clone(),
            id: new_branch_handle.document_id().to_string(),
            fork_info: Some(ForkInfo {
                forked_from: source_branch_doc_id.to_string(),
                forked_at: source_branch_doc_handle
                    .with_document(|d| d.get_heads())
                    .iter()
                    .map(|h| h.to_string())
                    .collect(),
            }),
            merge_info: None,
			created_by: self.user_name.clone(),
			merged_into: None,
			reverted_to: None,
        };

        self.tx
            .unbounded_send(OutputEvent::CompletedCreateBranch {
                branch_doc_id: new_branch_handle.document_id().clone(),
            })
            .unwrap();

        self.branches_metadata_doc_handle.with_document(|d| {
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
					reverted_to: None,
                    changed_files: None,
					is_setup: Some(true)
                },
            );
        });
		tracing::debug!("driver: created new branch: {:?}", new_branch_handle.document_id());
    }

	async fn create_revert_preview_branch(
		&mut self,
		branch_doc_id: DocumentId,
		files: Vec<(String, FileContent)>,
		revert_to: Vec<ChangeHash>,
	) {
		tracing::debug!("driver: create revert preview branch");
		let branch_state = self.branch_states.get(&branch_doc_id).unwrap();
		let current_doc_id = branch_state.doc_handle.document_id();
		let current_heads = branch_state.doc_handle.with_document(|d| d.get_heads());
		// create a new branch doc, merge the original branch doc into it, and then commit the changes
        let revert_preview_branch_doc_handle = clone_doc(&self.repo_handle, &branch_state.doc_handle).await;
		let revert_preview_doc_id = revert_preview_branch_doc_handle.document_id();

        let branch = Branch {
            name: format!(
                "{} <- {}",
                revert_to.to_short_form(), current_heads.to_short_form()
            ),
            id: revert_preview_doc_id.to_string(),
            fork_info: Some(ForkInfo {
                forked_from: current_doc_id.to_string(),
                forked_at: heads_to_vec_string(current_heads.clone()),
            }),
            merge_info: None,
			created_by: self.user_name.clone(),
			merged_into: None,
			reverted_to: Some(revert_to.iter().map(|h| h.to_string()).collect()),
        };
        self.branches_metadata_doc_handle.with_document(|d| {
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
					reverted_to: None,
                    changed_files: None,
					is_setup: Some(true)
                },
            );
        });

		self.save_files(revert_preview_branch_doc_handle.clone(), files, Some(current_heads), false, Some(revert_to)).await;

		self.tx
			.unbounded_send(OutputEvent::CompletedCreateBranch {
				branch_doc_id: revert_preview_doc_id.clone(),
			})
			.unwrap();

	}

    async fn create_merge_preview_branch(
        &mut self,
        source_branch_doc_id: DocumentId,
        target_branch_doc_id: DocumentId,
    ) {
        tracing::debug!("driver: create merge preview branch");

        let source_branch_state = self.branch_states.get(&source_branch_doc_id).unwrap();
        let target_branch_state = self.branch_states.get(&target_branch_doc_id).unwrap();

        let merge_preview_branch_doc_handle = self.repo_handle.create(Automerge::new()).await.unwrap();

        source_branch_state
            .doc_handle
            .with_document(|source_branch_doc| {
                merge_preview_branch_doc_handle.with_document(|merge_preview_branch_doc| {
                    let _ = merge_preview_branch_doc.merge(source_branch_doc);
                });
            });

        target_branch_state
            .doc_handle
            .with_document(|target_branch_doc| {
                merge_preview_branch_doc_handle.with_document(|merge_preview_branch_doc| {
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
			merged_into: None,
			reverted_to: None,
        };

        self.branches_metadata_doc_handle.with_document(|d| {
            let mut branches_metadata: BranchesMetadataDoc = hydrate(d).unwrap();
            let mut tx = d.transaction();
            branches_metadata.branches.insert(branch.id.clone(), branch);
            let _ = reconcile(&mut tx, branches_metadata);
            commit_with_attribution_and_timestamp(
                tx,
                &CommitMetadata {
                    username: self.user_name.clone(),
                    branch_id: Some(source_branch_doc_id),
                    merge_metadata: None,
					reverted_to: None,
                    changed_files: None,
					is_setup: Some(true)
                },
            );
        });

        self.tx
            .unbounded_send(OutputEvent::CompletedCreateBranch {
                branch_doc_id: merge_preview_branch_doc_handle.document_id().clone(),
            })
            .unwrap();
    }

    // delete branch isn't fully implemented right now deletes are not propagated to the frontend
    // right now this is just useful to clean up merge preview branches
    fn delete_branch(&mut self, branch_doc_id: DocumentId) {
        tracing::debug!("driver: delete branch {:?}", branch_doc_id);

        self.branches_metadata_doc_handle.with_document(|d| {
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
					reverted_to: None,
                    changed_files: None,
					is_setup: Some(true)
                },
            );
        });
    }

    async fn save_files(
        &mut self,
        branch_doc_handle: DocHandle,
        file_entries: Vec<(String, FileContent)>,
        heads: Option<Vec<ChangeHash>>,
		new_project: bool,
		revert: Option<Vec<ChangeHash>>
    ) {
        let mut binary_entries: Vec<(String, DocHandle)> = Vec::new();
        let mut text_entries: Vec<(String, &String)> = Vec::new();
        let mut scene_entries: Vec<(String, &GodotScene)> = Vec::new();
		let mut deleted_entries: Vec<String> = Vec::new();
		let is_revert = revert.is_some();

        for (path, content) in file_entries.iter() {
            match content {
                FileContent::Binary(content) => {
                    let binary_doc_handle = self.repo_handle.create(Automerge::new()).await.unwrap();
                    binary_doc_handle.with_document(|d| {
                        let mut tx = d.transaction();
                        let _ = tx.put(ROOT, "content", content.clone());
                        commit_with_attribution_and_timestamp(
                            tx,
                            &CommitMetadata {
                                username: self.user_name.clone(),
                                branch_id: None,
                                merge_metadata: None,
								reverted_to: None,
                                changed_files: None,
								is_setup: Some(new_project)
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
        branch_doc_handle.with_document(|d| {
            let mut tx = match heads {
                Some(heads) => d.transaction_at(
                    get_default_patch_log(),
                    &heads,
                ),
                None => d.transaction(),
            };

            let mut changes : Vec<ChangedFile> = Vec::new();
            let files = tx.get_obj_id(ROOT, "files").unwrap();

            // write text entries to doc
            for (path, content) in text_entries {
                // get existing file url or create new one
                let (file_entry, change_type) = match tx.get(&files, &path) {
                    Ok(Some((automerge::Value::Object(ObjType::Map), file_entry))) => (file_entry, ChangeType::Modified),
                    _ => (tx.put_object(&files, &path, ObjType::Map).unwrap(), ChangeType::Added)
                };

                changes.push(ChangedFile {
                    path,
                    change_type
                });

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
                // get the change flag
                let change_type = match tx.get(&files, &path) {
                    Ok(Some(_)) => ChangeType::Modified,
                    _ => ChangeType::Added
                };
                // godot_scene.reconcile(&mut tx, path.clone());
				let scene_file = tx.get_obj_id(&files, &path).unwrap_or_else(|| tx.put_object(&files, &path, ObjType::Map).unwrap());
				autosurgeon::reconcile_prop(&mut tx, &scene_file, "structured_content", godot_scene).unwrap_or_else(|e| {
					tracing::error!("error reconciling scene: {}", e);
					panic!("error reconciling scene: {}", e);
				});
                changes.push(ChangedFile {
                    path,
                    change_type
                });
            }

            // write binary entries to doc
            for (path, binary_doc_handle) in binary_entries {
                // get the change flag
                let change_type = match tx.get(&files, &path) {
                    Ok(Some(_)) => ChangeType::Modified,
                    _ => ChangeType::Added
                };

                let file_entry = tx.put_object(&files, &path, ObjType::Map);
                let _ = tx.put(
                    file_entry.unwrap(),
                    "url",
                    format!("automerge:{}", &binary_doc_handle.document_id()),
                );

                changes.push(ChangedFile {
                    path,
                    change_type
                });
            }

			for path in deleted_entries {
				let _ = tx.delete(&files, &path);
                changes.push(ChangedFile {
                    path,
                    change_type: ChangeType::Removed
                });
			}

            commit_with_attribution_and_timestamp(
                tx,
                &CommitMetadata {
                    username: self.user_name.clone(),
                    branch_id: Some(branch_doc_handle.document_id().clone()),
                    merge_metadata: None,
					reverted_to: match revert {
						Some(revert) => Some(heads_to_vec_string(revert)),
						None => None,
					},
                    changed_files: Some(changes),
					is_setup: Some(new_project)
                },
            );
        });

        // update heads in frontend
		if !new_project && !is_revert {
			self.heads_in_frontend.insert(
				branch_doc_handle.document_id().clone(),
				branch_doc_handle.with_document(|d| d.get_heads()),
			);
		}

        tracing::debug!("save on branch {:?} {:?}", branch_doc_handle.document_id(), self.heads_in_frontend);
    }

    fn merge_branch(&mut self, source_branch_doc_id: DocumentId, target_branch_doc_id: DocumentId) {
        let source_branch_state = self.branch_states.get(&source_branch_doc_id).unwrap();
        let target_branch_state = self.branch_states.get(&target_branch_doc_id).unwrap();

        source_branch_state
            .doc_handle
            .with_document(|source_branch_doc| {
                target_branch_state
                    .doc_handle
                    .with_document(|target_branch_doc| {
                        let _ = target_branch_doc.merge(source_branch_doc);
                    });
            });

		let mut original_branch_id = None;
        // if the branch has some merge_info we know that it's a merge preview branch
        let merge_metadata = if source_branch_state.merge_info.is_some() {
            let original_branch_state = self
                .branch_states
                .get(&source_branch_state.fork_info.as_ref().unwrap().forked_from)
                .unwrap();

			original_branch_id = Some(original_branch_state.doc_handle.document_id().to_string());

            Some(MergeMetadata {
                merged_branch_id: original_branch_state.doc_handle.document_id().clone(),
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
            target_branch_state.doc_handle.with_document(|d| {
                let mut tx = d.transaction();

                // do a dummy change that we can attach some metadata to
                let changed = tx.get_int(&ROOT, "_changed").unwrap_or(0);
                let _ = tx.put(ROOT, "_changed", changed + 1);

                commit_with_attribution_and_timestamp(
                    tx,
                    &CommitMetadata {
                        username: self.user_name.clone(),
                        branch_id: Some(target_branch_doc_id),
                        merge_metadata: Some(merge_metadata),
						reverted_to: None,
                        changed_files: None,
						is_setup: Some(false)
                    },
                );
            });
			let mut branch = self.get_branches_metadata()
			.branches
			.get(&source_branch_state.doc_handle.document_id().to_string()).unwrap().clone();
			branch.merged_into = original_branch_id;
			self.branches_metadata_doc_handle.with_document(|d| {
				let mut branches_metadata: BranchesMetadataDoc = hydrate(d).unwrap();
				let mut tx = d.transaction();
				branches_metadata.branches.insert(branch.id.clone(), branch);
				let _ = reconcile(&mut tx, branches_metadata);
				commit_with_attribution_and_timestamp(
					tx,
					&CommitMetadata {
						username: self.user_name.clone(),
						branch_id: Some(source_branch_doc_id),
						merge_metadata: None,
						reverted_to: None,
                        changed_files: None,
						is_setup: Some(false)
					},
				);
			});
			// self.branch_states.get_mut(&source_branch_doc_id).unwrap().merged_into = Some(target_branch_doc_id);
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
						merged_into: match branch.merged_into {
							Some(merged_into) => match DocumentId::from_str(&merged_into) {
								Ok(merged_into) => Some(merged_into),
								Err(_) => None,
							},
							None => None,
						},
						revert_info: match branch.reverted_to {
							Some(reverted_to) => Some(BranchStateRevertInfo {
								reverted_to: reverted_to.iter().map(|h| ChangeHash::from_str(h).unwrap()).collect(),
							}),
							None => None,
						},
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
                    .find(doc_id.clone())
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
            branch_state.synced_heads = branch_doc_handle.with_document(|d| d.get_heads());

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

    fn add_binary_doc_handle(&mut self, _path: &String, binary_doc_handle: &DocHandle) {
        self.binary_doc_states.insert(
            binary_doc_handle.document_id().clone(),
            BinaryDocState {
                doc_handle: Some(binary_doc_handle.clone()),
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
                    branch_state.synced_heads = branch_state.doc_handle.with_document(|d| d.get_heads());
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

        self.subscribed_doc_ids.insert(doc_handle.document_id().clone());
        self.all_doc_changes
            .push(handle_changes(doc_handle.clone()).boxed());
        self.all_doc_changes.push(
            futures::stream::once(async move { SubscriptionMessage::Added { doc_handle } }).boxed(),
        );
    }

    fn get_branches_metadata(&self) -> BranchesMetadataDoc {
        let branches_metadata: BranchesMetadataDoc = self
            .branches_metadata_doc_handle
            .with_document(|d| hydrate(d).unwrap());

        return branches_metadata;
    }
}

async fn clone_doc(repo_handle: &Repo, doc_handle: &DocHandle) -> DocHandle {
    let new_doc_handle = repo_handle.create(Automerge::new()).await.unwrap();

    let _ =
        doc_handle.with_document(|mut main_d| new_doc_handle.with_document(|d| d.merge(&mut main_d)));

    return new_doc_handle;
}

fn handle_changes(handle: DocHandle) -> impl futures::Stream<Item = SubscriptionMessage> + Send {
    futures::stream::unfold(handle, |doc_handle| async {
		// There's currently a bug where removing this line causes changed() to not resolve the future (despite this line not actually doing anything).
		// So, it'll spam with Changed events.
        let _ = doc_handle.with_document(|d| d.get_heads());
		// TODO: this will probably break on upgrading automerge_repo because changed() is currently greedy, but will eventually check
		// to see if there's an actual change before resolving the future. We rely on the greedy behavior here.
        // let _ = doc_handle.changed().await;

		// TODO (Samod): Does this even work???
		doc_handle.changes().next().await;

        Some((
            SubscriptionMessage::Changed {
                doc_handle: doc_handle.clone(),
                // diff,
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
                        .map_or(true, |handle| handle.with_document(|d| d.get_heads().is_empty()))
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
