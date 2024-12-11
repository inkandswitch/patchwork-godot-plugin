use std::{
    borrow::Borrow,
    collections::HashMap,
    fs::File,
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
enum FileUpdate {
    Patch {
        patch: Patch,
        scene: PackedGodotScene,
    },
    Reload {
        content: String,
    },
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
    file_update_sender: Sender<FileUpdate>,
    file_update_receiver: Receiver<FileUpdate>,
    branches_metadata_sender: Sender<BranchesMetadataDoc>,
    branches_metadata_receiver: Receiver<BranchesMetadataDoc>,
}

//const SERVER_URL: &str = "localhost:8080";
const SERVER_URL: &str = "161.35.233.157:8080";

#[godot_api]
impl AutomergeFS {
    #[signal]
    fn patch_file(patch: Dictionary);

    #[signal]
    fn reload_file(path: String, content: String);

    #[signal]
    fn branch_list_changed(branches: Array<Variant>);

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

        let (file_update_sender, file_update_receiver) = channel::<FileUpdate>();
        let (branches_metadata_sender, branches_metadata_receiver) =
            channel::<BranchesMetadataDoc>();

        return Gd::from_init_fn(|base| Self {
            repo_handle,
            branches_metadata_doc_id,
            checked_out_doc_id_mutex: Arc::new(Mutex::new(None)),
            runtime,
            base,
            file_update_sender,
            file_update_receiver,
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
    fn checkout(&mut self, branch_doc_id: String) {
        let mut checked_out_doc_id = self.checked_out_doc_id_mutex.lock().unwrap();
        *checked_out_doc_id = Some(DocumentId::from_str(&branch_doc_id.to_string()).unwrap());
    }

    #[func]
    fn create_branch(&self, name: String) {
        let repo_handle = self.repo_handle.clone();
        let branches_metadata_doc_id = self.branches_metadata_doc_id.clone();

        self.runtime.spawn(async move {
            let branches_metadata_doc_handle = repo_handle
                .request_document(branches_metadata_doc_id)
                .await
                .unwrap();

            let branches_metadata_doc: BranchesMetadataDoc =
                branches_metadata_doc_handle.with_doc(|d| hydrate(d).unwrap());

            let main_doc_id =
                DocumentId::from_str(&branches_metadata_doc.main_doc_id.to_string()).unwrap();

            let new_doc_handle = repo_handle.new_document();

            // merge main into new doc
            let _ = repo_handle
                .request_document(main_doc_id)
                .await
                .unwrap()
                .with_doc_mut(|mut main_d| new_doc_handle.with_doc_mut(|d| d.merge(&mut main_d)));

            // add new doc to branches metadata doc
            branches_metadata_doc_handle.with_doc_mut(|d| {
                let mut branches_metadata_doc: BranchesMetadataDoc = hydrate(d).unwrap();

                branches_metadata_doc.branches.insert(
                    new_doc_handle.document_id().to_string(),
                    Branch {
                        name: name,
                        id: new_doc_handle.document_id().to_string(),
                    },
                );

                let mut tx = d.transaction();
                let _ = reconcile(&mut tx, branches_metadata_doc);
                tx.commit();
            })
        });
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
        while let Ok(update) = self.file_update_receiver.try_recv() {
            updates.push(update);
        }

        // Process all updates
        for file_update in updates {
            match file_update {
                FileUpdate::Patch { patch, scene } => {
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
                                                let attributes =
                                                    godot_scene::get_node_attributes(&node);
                                                if let Some(instance) = attributes.get("instance") {
                                                    let _ = dict
                                                        .insert("instance_path", instance.clone());
                                                } else if let Some(type_val) =
                                                    attributes.get("type")
                                                {
                                                    let _ = dict
                                                        .insert("instance_type", type_val.clone());
                                                }
                                            }

                                            self.base_mut()
                                                .emit_signal("patch_file", &[dict.to_variant()]);
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
                                            "patch_file",
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

                FileUpdate::Reload { content } => {
                    self.base_mut().emit_signal(
                        "reload_file",
                        &["res://main.tscn".to_variant(), content.to_variant()],
                    );
                }
            }
        }
    }

    #[func]
    fn start(&self) {
        let repo_handle_change_listener_checked_out_doc = self.repo_handle.clone();
        let file_update_sender = self.file_update_sender.clone();
        let checked_out_doc_id_mutex = self.checked_out_doc_id_mutex.clone();

        // listen for changes on checked out doc
        self.runtime.spawn(async move {
            let mut heads: Vec<ChangeHash> = vec![];

            let mut prev_doc_id: Option<DocumentId> = None;
            loop {
                let checked_out_doc_id = checked_out_doc_id_mutex.lock().unwrap().clone();
                let checked_out_doc_id_clone = checked_out_doc_id.clone();

                // Reset heads when switching to a different doc
                let checked_out_doc_has_changed = checked_out_doc_id != prev_doc_id;
                if checked_out_doc_has_changed {
                    prev_doc_id = checked_out_doc_id
                }

                if let Some(doc_id) = checked_out_doc_id_clone {
                    let doc_handle = repo_handle_change_listener_checked_out_doc
                        .request_document(doc_id)
                        .await
                        .unwrap();

                    // todo: this is realy bad but we can't wait on the doc handle
                    // because then it doesn't recheck the checked_out_doc_id unless the currently checked out doc changes
                    // doc_handle.changed().await.unwrap();
                    tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;

                    doc_handle.with_doc(|d| -> () {
                        let new_heads = d.get_heads();
                        let patches = d.diff(&heads, &new_heads, TextRepresentation::String);
                        heads = new_heads;

                        // Hydrate the current document state into a PackedGodotScene
                        let scene: PackedGodotScene = hydrate(d).unwrap();

                        if checked_out_doc_has_changed {
                            let _ = file_update_sender.send(FileUpdate::Reload {
                                content: serialize(scene),
                            });

                            print!("send reload")
                        } else {
                            for patch in patches {
                                let patch_with_scene = FileUpdate::Patch {
                                    patch,
                                    scene: scene.clone(),
                                };
                                let _ = file_update_sender.send(patch_with_scene);
                            }
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

        println!("save {:?}", checked_out_doc_id);

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
