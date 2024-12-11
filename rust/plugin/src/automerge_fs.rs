use std::{
    collections::HashMap,
    hash::Hash,
    str::FromStr,
    sync::mpsc::{channel, Receiver, Sender},
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

struct BranchUpdate {
    branches: HashMap<String, String>,
}

#[derive(GodotClass)]
#[class(no_init, base=Node)]
pub struct AutomergeFS {
    repo_handle: RepoHandle,
    runtime: Runtime,
    branches_doc_id: DocumentId,
    fs_doc_id: Option<DocumentId>,
    base: Base<Node>,
    patch_sender: Sender<PatchWithScene>,
    patch_receiver: Receiver<PatchWithScene>,
    branch_sender: Sender<BranchUpdate>,
    branch_receiver: Receiver<BranchUpdate>,
}

//const SERVER_URL: &str = "localhost:8080";
const SERVER_URL: &str = "161.35.233.157:8080";

#[godot_api]
impl AutomergeFS {
    #[signal]
    fn file_changed(path: String, content: String);

    #[func]
    fn get_branches_doc_id(&self) -> String {
        match &self.fs_doc_id {
            Some(doc_id) => doc_id.to_string(),
            None => String::new(),
        }
    }

    #[func]
    // hack: pass in empty string to create a new doc
    // godot rust doens't seem to support Option args
    fn create(maybe_branches_doc_id: String) -> Gd<Self> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        let _guard = runtime.enter();

        let _ = tracing_subscriber::fmt::try_init();

        let storage = FsStorage::open("/tmp/automerge-godot-data").unwrap();
        let repo = Repo::new(None, Box::new(storage));
        let repo_handle = repo.run();
        let branches_doc_id = if maybe_branches_doc_id.is_empty() {
            let handle = repo_handle.new_document();
            handle.document_id()
        } else {
            DocumentId::from_str(&maybe_branches_doc_id).unwrap()
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
        let (branch_sender, branch_receiver) = channel::<BranchUpdate>();

        return Gd::from_init_fn(|base| Self {
            repo_handle,
            branches_doc_id,
            fs_doc_id: None,
            runtime,
            base,
            patch_sender,
            patch_receiver,
            branch_sender,
            branch_receiver,
        });
    }

    #[func]
    fn stop(&self) {
        self.repo_handle.clone().stop().unwrap();

        // todo: shut down runtime
        //self.runtime.shutdown_background();
    }

    #[func]
    fn checkout(&mut self, fs_doc_id: String) {
        self.fs_doc_id = Some(DocumentId::from_str(&fs_doc_id).unwrap());
    }

    // needs to be called in godot on each frame
    #[func]
    fn refresh(&mut self) {
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
        // listen for changes to fs doc
        let repo_handle_change_listener = self.repo_handle.clone();
        let fs_doc_id = self.fs_doc_id.clone();
        let sender = self.patch_sender.clone();
        self.runtime.spawn(async move {
            let mut heads: Vec<ChangeHash> = vec![];

            loop {
                if let Some(doc_id) = fs_doc_id.clone() {
                    let doc_handle = repo_handle_change_listener
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
                            let _ = sender.send(patch_with_scene);
                        }
                    });
                }
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
        if let Some(fs_doc_id) = fs_doc_id {
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
}
