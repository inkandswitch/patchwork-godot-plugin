use std::{
    fs::File,
    str::FromStr,
    sync::mpsc::{channel, Receiver, Sender},
};

use automerge::{ChangeHash, Patch, ScalarValue};
use autosurgeon::{hydrate, reconcile};
use godot::{classes::node, obj::WithBaseField, prelude::*};

use automerge::patches::TextRepresentation;
use automerge_repo::{tokio::FsStorage, ConnDirection, DocumentId, Repo, RepoHandle};
use tokio::{net::TcpStream, runtime::Runtime};

use crate::godot_scene::{self, PackedGodotScene};

#[derive(GodotClass)]
#[class(no_init, base=Node)]
pub struct AutomergeFS {
    repo_handle: RepoHandle,
    runtime: Runtime,
    fs_doc_id: DocumentId,
    base: Base<Node>,
    sender: Sender<FileChange>,
    receiver: Receiver<FileChange>,
}

struct FileChange {
    file_path: String,
    patch: SceneChangePatch,
}

pub enum SceneChangePatch {
    Change {
        node_path: String,
        properties: Dictionary,
        attributes: Dictionary,
    },
    Delete {
        node_path: String,
    },
}

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
                let res = TcpStream::connect("127.0.0.1:8080").await;
                if let Err(e) = res {
                    println!("error connecting: {:?}", e);
                    continue;
                }
                break res.unwrap();
            };

            println!("connect repo");

            repo_handle_clone
                .connect_tokio_io("127.0.0.1:8080", stream, ConnDirection::Outgoing)
                .await
                .unwrap();
        });

        let (sender, receiver) = channel::<FileChange>();

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
        let update = self.receiver.try_recv();

        match update {
            Ok(file_change) => {
                let patch_dict: Dictionary = match file_change.patch {
                    SceneChangePatch::Change {
                        node_path,
                        properties,
                        attributes,
                    } => dict! {
                      "type": "update",
                      "node_path": node_path,
                      "properties": properties,
                      "attributes": attributes
                    },
                    SceneChangePatch::Delete { node_path } => dict! {
                      "type" : "delete",
                      "node_path": node_path
                    },
                };

                self.base_mut().emit_signal(
                    "file_changed",
                    &[file_change.file_path.to_variant(), patch_dict.to_variant()],
                );
            }
            Err(_) => (),
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

            // No need to clone sender since we already have a clone from outside the spawn

            loop {
                doc_handle.changed().await.unwrap();

                doc_handle.with_doc(|d| -> () {
                    let new_heads = d.get_heads();
                    let patches = d.diff(&heads, &new_heads, TextRepresentation::String);
                    heads = new_heads;

                    let scene: PackedGodotScene = hydrate(d).unwrap();

                    for patch in patches {
                        match patch.action {
                            // handle update node
                            automerge::PatchAction::PutMap {
                                key,
                                value,
                                conflict,
                            } => match (patch.path.get(0), patch.path.get(1)) {
                                (
                                    Some((_, automerge::Prop::Map(key))),
                                    Some((_, automerge::Prop::Map(node_path))),
                                ) => {
                                    if key == "nodes" {
                                        if let Some(node) =
                                            godot_scene::get_node_by_path(&scene, node_path)
                                        {
                                            /*sender
                                            .send(FileChange {
                                                file_path: String::from("res://main.tscn"), // todo: generalize
                                                patch: SceneChangePatch::Change {
                                                    node_path: node_path.to_string(),
                                                    properties: Dictionary::from_iter(
                                                        godot_scene::get_node_properties(&node)
                                                            .iter()
                                                            .map(|(k, v)| {
                                                                (k.clone(), v.clone())
                                                            }),
                                                    ),
                                                    attributes: Dictionary::from_iter(
                                                        godot_scene::get_node_attributes(&node)
                                                            .iter()
                                                            .map(|(k, v)| {
                                                                (k.clone(), v.clone())
                                                            }),
                                                    ),
                                                },
                                            })
                                            .unwrap();*/
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
                                            /*sender
                                            .send(FileChange {
                                                file_path: String::from("res://main.tscn"), // todo: generalize
                                                patch: SceneChangePatch::Delete {
                                                    node_path: node_path.to_string(),
                                                },
                                            })
                                            .unwrap();*/
                                        }
                                    }
                                    _ => {}
                                };
                            }
                            _ => {}
                        }
                    }
                });
            }
        });
    }

    #[func]
    fn save(&self, path: String, content: String) {
        let repo_handle = self.repo_handle.clone();
        let fs_doc_id = self.fs_doc_id.clone();

        println!("save {:?}", path);

        // todo: handle files that are not main.tscn
        if (!path.ends_with("main.tscn")) {
            return;
        }

        let scene = godot_scene::parse(&content).unwrap();

        //println!("Scene contents: {:#?}", scene);

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
