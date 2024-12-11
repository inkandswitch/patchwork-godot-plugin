use std::{
    borrow::Borrow,
    collections::HashMap,
    hash::Hash,
    str::FromStr,
    sync::{
        mpsc::{channel, Receiver, Sender},
        Arc, Mutex,
    },
};

use automerge::{ChangeHash, Patch, ScalarValue};
use autosurgeon::{hydrate, reconcile, Hydrate, Reconcile};
use godot::{global::print, obj::WithBaseField, prelude::*};

use automerge::patches::TextRepresentation;
use automerge_repo::{tokio::FsStorage, ConnDirection, DocumentId, Repo, RepoHandle};
use tokio::{net::TcpStream, runtime::Runtime};

use crate::godot_scene::{self, get_node_by_path, serialize, PackedGodotScene};

struct PatchWithScene {
    patch: Patch,
    scene: PackedGodotScene,
}

#[derive(Debug, Clone, Reconcile, Hydrate, PartialEq)]
struct BranchesMetadataDoc {
    main_doc_id: String,
    branches: HashMap<String, Branch>,
}

#[derive(Debug, Clone, Reconcile, Hydrate, PartialEq)]
struct Branch {
    name: String,
    id: String,
}

#[derive(GodotClass)]
#[class(no_init, base=Node)]
pub struct AutomergeFS {
    repo_handle: RepoHandle,
    runtime: Runtime,
    branches_metadata_doc_id: DocumentId,
    checked_out_doc_id_mutex: Arc<Mutex<Option<DocumentId>>>,
    base: Base<Node>,
    patch_sender: Sender<PatchWithScene>,
    patch_receiver: Receiver<PatchWithScene>,
    branches_metadata_sender: Sender<BranchesMetadataDoc>,
    branches_metadata_receiver: Receiver<BranchesMetadataDoc>,
}

//const SERVER_URL: &str = "localhost:8080";
const SERVER_URL: &str = "161.35.233.157:8080";

#[godot_api]
impl AutomergeFS {
    #[signal]
    fn file_changed(path: String, content: String);

    #[signal]
    fn branch_list_changed(branches: Dictionary);

    #[func]
    fn get_branches_metadata_doc_id(&self) -> String {
        return self.branches_metadata_doc_id.to_string();
    }

    #[func]
    // hack: pass in empty string to create a new doc
    // godot rust doens't seem to support Option args
    fn create(maybe_branches_metadata_doc_id: String) -> Gd<Self> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        let _guard = runtime.enter();

        let _ = tracing_subscriber::fmt::try_init();

        let storage = FsStorage::open("/tmp/automerge-godot-data").unwrap();
        let repo = Repo::new(None, Box::new(storage));
        let repo_handle = repo.run();
        let branches_metadata_doc_id = if maybe_branches_metadata_doc_id.is_empty() {
            let branches_doc_handle = repo_handle.new_document();
            let main_doc_handle = repo_handle.new_document();

            branches_doc_handle.with_doc_mut(|d| {
                let mut tx = d.transaction();
                let _ = reconcile(
                    &mut tx,
                    BranchesMetadataDoc {
                        main_doc_id: main_doc_handle.document_id().to_string(),
                        branches: HashMap::new(),
                    },
                );
                tx.commit();
            });

            branches_doc_handle.document_id()
        } else {
            DocumentId::from_str(&maybe_branches_metadata_doc_id).unwrap()
        };

        // connect repo
        let repo_handle_clone = repo_handle.clone();
        runtime.spawn(async move {
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

        let (patch_sender, patch_receiver) = channel::<PatchWithScene>();
        let (branches_metadata_sender, branches_metadata_receiver) =
            channel::<BranchesMetadataDoc>();

        return Gd::from_init_fn(|base| Self {
            repo_handle,
            branches_metadata_doc_id,
            checked_out_doc_id_mutex: Arc::new(Mutex::new(None)),
            runtime,
            base,
            patch_sender,
            patch_receiver,
            branches_metadata_sender,
            branches_metadata_receiver,
        });
    }

    #[func]
    fn stop(&self) {
        self.repo_handle.clone().stop().unwrap();

        // todo: shut down runtime
        //self.runtime.shutdown_background();
    }

    #[func]
    fn create_branch(&self, name: String, source: String) {
        let repo_handle = self.repo_handle.clone();

        self.runtime.spawn(async move {
            let doc_handle = repo_handle.new_document();
        });
    }

    #[func]
    fn checkout(&mut self, branch_doc_id: String) {
        let mut checked_out_doc_id = self.checked_out_doc_id_mutex.lock().unwrap();
        *checked_out_doc_id = Some(DocumentId::from_str(&branch_doc_id.to_string()).unwrap());
    }

    // needs to be called in godot on each frame
    #[func]
    fn refresh(&mut self) {
        // Get latest branches metadata update if any
        if let Ok(branches_metadata) = self.branches_metadata_receiver.try_recv() {
            let mut branches = Array::<Dictionary>::new();

            branches.push(
                &(dict! {
                    "name": "main".to_string(),
                    "id": branches_metadata.main_doc_id,
                }),
            );

            for (_, branch) in branches_metadata.branches {
                branches.push(
                    &(dict! {
                        "name": branch.name,
                        "id": branch.id,
                    }),
                );
            }
            self.base_mut()
                .emit_signal("branch_list_changed", &[branches.to_variant()]);
        }

        // Collect all available updates
        let mut updates = Vec::new();
        while let Ok(update) = self.patch_receiver.try_recv() {
            updates.push(update);
        }

        // Process all updates
        for patch_with_scene in updates {
            let PatchWithScene { patch, scene } = patch_with_scene;
            match patch.action {
                // handle update node
                automerge::PatchAction::PutMap {
                    key,
                    value,
                    conflict: _,
                } => match (patch.path.get(0), patch.path.get(1), patch.path.get(2)) {
                    (
                        Some((_, automerge::Prop::Map(maybe_nodes))),
                        Some((_, automerge::Prop::Map(node_path))),
                        Some((_, automerge::Prop::Map(prop_or_attr))),
                    ) => {
                        if maybe_nodes == "nodes" {
                            if let automerge::Value::Scalar(v) = value.0 {
                                if let ScalarValue::Str(smol_str) = v.as_ref() {
                                    let string_value = smol_str.to_string();

                                    let mut dict = dict! {
                                        "file_path": "res://main.tscn",
                                        "node_path": node_path.to_variant(),
                                        "type": if prop_or_attr == "properties" {
                                            "property_changed"
                                        } else {
                                            "attribute_changed"
                                        },
                                        "key": key,
                                        "value": string_value,
                                    };

                                    // Look up node in scene and get instance / type attribute if it exists
                                    if let Some(node) =
                                        godot_scene::get_node_by_path(&scene, node_path)
                                    {
                                        let attributes = godot_scene::get_node_attributes(&node);
                                        if let Some(instance) = attributes.get("instance") {
                                            let _ = dict.insert("instance_path", instance.clone());
                                        } else if let Some(type_val) = attributes.get("type") {
                                            let _ = dict.insert("instance_type", type_val.clone());
                                        }
                                    }

                                    self.base_mut()
                                        .emit_signal("file_changed", &[dict.to_variant()]);
                                }
                            }
                        }
                    }
                    _ => {}
                },

                // handle delete node
                automerge::PatchAction::DeleteMap { key: node_path } => {
                    if patch.path.len() != 1 {
                        continue;
                    }
                    match patch.path.get(0) {
                        Some((_, automerge::Prop::Map(key))) => {
                            if key == "nodes" {
                                self.base_mut().emit_signal(
                                    "file_changed",
                                    &[dict! {
                                      "file_path": "res://main.tscn",
                                      "node_path": node_path.to_variant(),
                                      "type": "node_deleted",
                                    }
                                    .to_variant()],
                                );
                            }
                        }
                        _ => {}
                    };
                }
                _ => {}
            }
        }
    }

    #[func]
    fn start(&self) {
        let repo_handle_change_listener_checked_out_doc = self.repo_handle.clone();
        let patch_sender = self.patch_sender.clone();
        let checked_out_doc_id_mutex = self.checked_out_doc_id_mutex.clone();

        // listen for changes on checked out doc
        self.runtime.spawn(async move {
            let mut heads: Vec<ChangeHash> = vec![];

            loop {
                let checked_out_doc_id = checked_out_doc_id_mutex.lock().unwrap().clone();

                if let Some(doc_id) = checked_out_doc_id {
                    let doc_handle = repo_handle_change_listener_checked_out_doc
                        .request_document(doc_id)
                        .await
                        .unwrap();

                    doc_handle.changed().await.unwrap();

                    doc_handle.with_doc(|d| -> () {
                        let new_heads = d.get_heads();
                        let patches = d.diff(&heads, &new_heads, TextRepresentation::String);
                        heads = new_heads;

                        // Hydrate the current document state into a PackedGodotScene
                        let scene: PackedGodotScene = hydrate(d).unwrap();

                        for patch in patches {
                            let patch_with_scene = PatchWithScene {
                                patch,
                                scene: scene.clone(),
                            };
                            let _ = patch_sender.send(patch_with_scene);
                        }
                    });
                }
            }
        });

        // listen for changes on branches metadata doc
        let metadata_sender = self.branches_metadata_sender.clone();
        let repo_handle_change_listener_branches_metadata_doc = self.repo_handle.clone();
        let metadata_doc_id = self.branches_metadata_doc_id.clone();

        self.runtime.spawn(async move {
            let doc_handle = repo_handle_change_listener_branches_metadata_doc
                .request_document(metadata_doc_id)
                .await
                .unwrap();

            loop {
                doc_handle.with_doc(|d| {
                    let branches_metadata_doc: BranchesMetadataDoc = hydrate(d).unwrap();
                    metadata_sender.send(branches_metadata_doc);
                });

                doc_handle.changed().await.unwrap();
            }
        });
    }

    #[func]
    fn save(&self, path: String, content: String) {
        let checked_out_doc_id = self.checked_out_doc_id_mutex.lock().unwrap().clone();

        if checked_out_doc_id.is_none() {
            println!("skip save");
            return;
        }

        println!("save");

        let repo_handle = self.repo_handle.clone();
        let fs_doc_id = checked_out_doc_id.as_ref().unwrap().clone();

        // todo: handle files that are not main.tscn
        if !path.ends_with("main.tscn") {
            return;
        }

        let scene = godot_scene::parse(&content).unwrap();

        self.runtime.spawn(async move {
            let doc_handle = repo_handle.request_document(fs_doc_id);
            let result = doc_handle.await.unwrap();

            result.with_doc_mut(|d| {
                let mut tx = d.transaction();
                let _ = reconcile(&mut tx, scene);
                tx.commit();
                return;
            });
        });
    }
}
