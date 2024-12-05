use std::{
    str::FromStr,
    sync::mpsc::{channel, Receiver, Sender},
};

use automerge::{ChangeHash, Patch, ScalarValue};
use autosurgeon::{hydrate, reconcile};
use godot::{obj::WithBaseField, prelude::*};

use automerge::patches::TextRepresentation;
use automerge_repo::{tokio::FsStorage, ConnDirection, DocumentId, Repo, RepoHandle};
use tokio::{net::TcpStream, runtime::Runtime};

use crate::godot_scene::{self, PackedGodotScene};

struct PatchWithScene {
    patch: Patch,
    scene: PackedGodotScene,
}

#[derive(GodotClass)]
#[class(no_init, base=Node)]
pub struct AutomergeFS {
    repo_handle: RepoHandle,
    runtime: Runtime,
    fs_doc_id: DocumentId,
    base: Base<Node>,
    sender: Sender<PatchWithScene>,
    receiver: Receiver<PatchWithScene>,
}

const SERVER_URL: &str = "localhost:8080"; //"godot-rust.onrender.com:80";

#[godot_api]
impl AutomergeFS {
    #[signal]
    fn file_changed(path: String, content: String);

    #[func]
    fn create(fs_doc_id: String) -> Gd<Self> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        let _guard = runtime.enter();

        let _ = tracing_subscriber::fmt::try_init();

        let storage = FsStorage::open("/tmp/automerge-godot-data").unwrap();
        let repo = Repo::new(None, Box::new(storage));
        let repo_handle = repo.run();
        let doc_id = DocumentId::from_str(&fs_doc_id).unwrap();

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

            repo_handle_clone
                .connect_tokio_io(SERVER_URL, stream, ConnDirection::Outgoing)
                .await
                .unwrap();
        });

        let (sender, receiver) = channel::<PatchWithScene>();

        return Gd::from_init_fn(|base| Self {
            repo_handle,
            fs_doc_id: doc_id,
            runtime,
            base,
            sender,
            receiver,
        });
    }

    #[func]
    fn stop(&self) {
        self.repo_handle.clone().stop().unwrap();

        // todo: shut down runtime
        //self.runtime.shutdown_background();
    }

    // needs to be called in godot on each frame
    #[func]
    fn refresh(&mut self) {
        // Collect all available updates
        let mut updates = Vec::new();
        while let Ok(update) = self.receiver.try_recv() {
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

                                    // Look up node in scene and get instance attribute if it exists
                                    if let Some(node) =
                                        godot_scene::get_node_by_path(&scene, node_path)
                                    {
                                        let attributes = godot_scene::get_node_attributes(&node);
                                        if let Some(instance) = attributes.get("instance") {
                                            let _ = dict.insert("instance", instance.clone());
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
        // listen for changes to fs doc
        let repo_handle_change_listener = self.repo_handle.clone();
        let fs_doc_id = self.fs_doc_id.clone();
        let sender = self.sender.clone();
        self.runtime.spawn(async move {
            let doc_handle = repo_handle_change_listener
                .request_document(fs_doc_id)
                .await
                .unwrap();

            let mut heads: Vec<ChangeHash> = vec![];

            loop {
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
                        let _ = sender.send(patch_with_scene);
                    }
                });
            }
        });
    }

    #[func]
    fn save(&self, path: String, content: String) {
        let repo_handle = self.repo_handle.clone();
        let fs_doc_id = self.fs_doc_id.clone();

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
