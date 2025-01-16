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

use automerge::{
    transaction::Transactable, ChangeHash, ObjType, Patch, ReadDoc, ScalarValue, ROOT,
};
use autosurgeon::{hydrate, reconcile, Hydrate, Reconcile};
use godot::{global::print, obj::WithBaseField, prelude::*};

use automerge::patches::TextRepresentation;
use automerge_repo::{tokio::FsStorage, ConnDirection, DocumentId, Repo, RepoHandle};
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

        return Gd::from_init_fn(|base| Self {
            repo_handle,
            project_doc_id,
            runtime,
            base,
        });
    }

    #[func]
    fn doc_id(&self) -> String {
        return self.project_doc_id.to_string();
    }

    #[func]
    fn get_file(path: String) {}

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
