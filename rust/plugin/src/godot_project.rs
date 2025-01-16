use std::{
    collections::HashMap,
    str::FromStr,
    sync::{Arc, RwLock},
};

use automerge::{transaction::Transactable, Automerge, ObjType, ReadDoc, ROOT};
use automerge_repo::{tokio::FsStorage, ConnDirection, DocumentId, Repo, RepoHandle};
use autosurgeon::{reconcile, Hydrate, Reconcile};
use godot::prelude::*;
use tokio::{net::TcpStream, runtime::Runtime};

#[derive(Debug, Clone, Reconcile, Hydrate, PartialEq)]
struct GodotProjectDoc {
    files: HashMap<String, String>,
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
pub struct GodotProject {
    repo_handle: RepoHandle,
    runtime: Runtime,
    project_doc_id: DocumentId,
    base: Base<Node>,
    project_doc_state: Arc<RwLock<Automerge>>,
}

//const SERVER_URL: &str = "localhost:8080";
const SERVER_URL: &str = "161.35.233.157:8080";

#[godot_api]
impl GodotProject {
    #[func]
    // hack: pass in empty string to create a new doc
    // godot rust doens't seem to support Option args
    fn create(maybe_project_doc_id: String) -> Gd<Self> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        let _guard = runtime.enter();

        let _ = tracing_subscriber::fmt::try_init();

        let storage = FsStorage::open("/tmp/automerge-godot-data").unwrap();
        let repo = Repo::new(None, Box::new(storage));
        let repo_handle = repo.run();
        let project_doc_id = if maybe_project_doc_id.is_empty() {
            let project_doc_handle = repo_handle.new_document();

            project_doc_handle.with_doc_mut(|d| {
                let mut tx = d.transaction();
                let _ = reconcile(
                    &mut tx,
                    GodotProjectDoc {
                        files: HashMap::new(),
                    },
                );
                tx.commit();
            });

            project_doc_handle.document_id()
        } else {
            DocumentId::from_str(&maybe_project_doc_id).unwrap()
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

        let repo_handle_clone_2 = repo_handle.clone();
        let project_doc_state: Arc<RwLock<Automerge>> = Arc::new(RwLock::new(Automerge::new()));
        let project_doc_state_clone = project_doc_state.clone();
        let project_doc_id_clone = project_doc_id.clone();

        // sync project doc heads
        runtime.spawn(async move {
            let doc_handle = repo_handle_clone_2
                .request_document(project_doc_id_clone)
                .await
                .unwrap();

            let doc_handle_clone = doc_handle.clone();

            loop {
                let doc = doc_handle_clone.with_doc(|d| d.clone());

                {
                    let mut write_state = project_doc_state_clone.write().unwrap();
                    *write_state = doc
                }

                doc_handle.changed().await.unwrap();
            }
        });

        return Gd::from_init_fn(|base| Self {
            repo_handle,
            project_doc_id,
            runtime,
            base,
            project_doc_state,
        });
    }

    #[func]
    fn get_doc_id(&self) -> String {
        return self.project_doc_id.to_string();
    }

    fn get_doc(&self) -> Automerge {
        return self.project_doc_state.read().unwrap().clone();
    }

    #[func]
    fn get_heads(&self) -> Array<Variant> {
        let heads = self.get_doc().get_heads();

        return heads
            .to_vec()
            .iter()
            .map(|h| h.to_string().to_variant())
            .collect::<Array<Variant>>();
    }

    #[func]
    fn get_file(&self, path: String) -> Variant /* string or null */ {
        let doc = self.get_doc();

        let files = doc.get(ROOT, "files").unwrap().unwrap().1;

        return match doc.get(files, path) {
            Ok(Some((value, _))) => value.into_string().unwrap_or_default().to_variant(),
            _ => Variant::nil(),
        };
    }

    #[func]
    fn save_file(&self, path: String, content: String) {
        let project_doc_id = self.project_doc_id.clone();
        let repo_handle = self.repo_handle.clone();
        let path_clone = path.clone();

        self.runtime.spawn(async move {
            let doc_handle = repo_handle.request_document(project_doc_id);
            let result = doc_handle.await.unwrap();

            result.with_doc_mut(|d| {
                let mut tx = d.transaction();

                let files = match tx.get(ROOT, "files") {
                    Ok(Some((automerge::Value::Object(ObjType::Map), files))) => files,
                    _ => panic!("Invalid project doc, doesn't have files map"),
                };

                if let Err(e) = tx.put(files, path, content) {
                    panic!("Failed to save file: {:?}", e);
                }

                tx.commit();
                return;
            });

            println!("saved {:?}", path_clone);
        });
    }
}
