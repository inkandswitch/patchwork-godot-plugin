use std::{
    collections::HashMap,
    str::FromStr,
    sync::{Arc, Mutex},
};

use automerge::{transaction::Transactable, Automerge, ChangeHash, ObjType, ReadDoc, ROOT};
use automerge_repo::{tokio::FsStorage, ConnDirection, DocHandle, DocumentId, Repo, RepoHandle};
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
    base: Base<Node>,
    runtime: Runtime,
    repo_handle: RepoHandle,
    branches_metadata_doc_id: DocumentId,
    checked_out_doc_id: Arc<Mutex<Option<DocumentId>>>,
    docs_state: Arc<Mutex<HashMap<DocumentId, Automerge>>>,
    doc_handles_state: Arc<Mutex<HashMap<DocumentId, DocHandle>>>,
}

//const SERVER_URL: &str = "localhost:8080";
const SERVER_URL: &str = "161.35.233.157:8080";

#[godot_api]
impl GodotProject {
    #[func]
    // hack: pass in empty string to create a new doc
    // godot rust doens't seem to support Option args
    fn create(maybe_branches_metadata_doc_id: String) -> Gd<Self> {
        let runtime: Runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        let _guard = runtime.enter();

        let _ = tracing_subscriber::fmt::try_init();

        let storage = FsStorage::open("/tmp/automerge-godot-data").unwrap();
        let repo = Repo::new(None, Box::new(storage));
        let repo_handle = repo.run();

        let docs_state: Arc<Mutex<HashMap<DocumentId, Automerge>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let doc_handles_state: Arc<Mutex<HashMap<DocumentId, DocHandle>>> =
            Arc::new(Mutex::new(HashMap::new()));

        let (branches_metadata_doc_id, checked_out_doc_id) =
            if maybe_branches_metadata_doc_id.is_empty() {
                // Create new project doc
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
                let project_doc_id = project_doc_handle.document_id();

                // Create new branches metadata doc
                let branches_metadata_doc_handle = repo_handle.new_document();
                branches_metadata_doc_handle.with_doc_mut(|d| {
                    let mut tx = d.transaction();
                    let _ = reconcile(
                        &mut tx,
                        BranchesMetadataDoc {
                            main_doc_id: project_doc_id.to_string(),
                            branches: HashMap::new(),
                        },
                    );
                    tx.commit();
                });

                let project_doc_id_clone = project_doc_id.clone();
                let project_doc_id_clone_2 = project_doc_id.clone();
                let branches_metadata_doc_handle_clone = branches_metadata_doc_handle.clone();

                // Add both docs to the states
                {
                    let mut docs = docs_state.lock().unwrap();
                    let mut doc_handles = doc_handles_state.lock().unwrap();

                    // Add project doc
                    docs.insert(project_doc_id, project_doc_handle.with_doc(|d| d.clone()));
                    doc_handles.insert(project_doc_id_clone, project_doc_handle);

                    // Add branches metadata doc
                    docs.insert(
                        branches_metadata_doc_handle.document_id(),
                        branches_metadata_doc_handle.with_doc(|d| d.clone()),
                    );
                    doc_handles.insert(
                        branches_metadata_doc_handle.document_id(),
                        branches_metadata_doc_handle,
                    );
                }

                (
                    branches_metadata_doc_handle_clone.document_id(),
                    Some(project_doc_id_clone_2.clone()),
                )
            } else {
                let branches_metadata_doc_id =
                    DocumentId::from_str(&maybe_branches_metadata_doc_id).unwrap();
                (branches_metadata_doc_id, None) // Will be populated when doc syncs
            };

        // Spawn connection task
        Self::spawn_connection_task(&runtime, repo_handle.clone());

        // todo: handle sync for multiple docs
        // Spawn sync task for branches metadata doc
        /*Self::spawn_sync_task(
            &runtime,
            repo_handle.clone(),
            branches_metadata_doc_id.clone(),
            docs_state.clone(),
            doc_handles_state.clone(),
        );*/

        println!("checked out? {:?}", checked_out_doc_id);

        return Gd::from_init_fn(|base| Self {
            base,
            runtime,
            repo_handle,
            branches_metadata_doc_id,
            checked_out_doc_id: Arc::new(Mutex::new(checked_out_doc_id)),
            docs_state,
            doc_handles_state,
        });
    }

    fn spawn_connection_task(runtime: &Runtime, repo_handle: RepoHandle) {
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
    }

    fn spawn_sync_task(
        runtime: &Runtime,
        repo_handle: RepoHandle,
        project_doc_id: DocumentId,
        docs_state: Arc<Mutex<HashMap<DocumentId, Automerge>>>,
        doc_handles_state: Arc<Mutex<HashMap<DocumentId, DocHandle>>>,
    ) {
        let repo_handle_clone = repo_handle.clone();
        let docs_state_clone = docs_state.clone();
        let project_doc_id_clone = project_doc_id.clone();
        let doc_handles_state_clone = doc_handles_state.clone();

        runtime.spawn(async move {
            let doc_handle = repo_handle_clone
                .request_document(project_doc_id_clone)
                .await
                .unwrap();

            {
                let mut write_handle = doc_handles_state_clone.lock().unwrap();
                write_handle.insert(project_doc_id.clone(), doc_handle.clone());
            }

            loop {
                let doc = doc_handle.with_doc(|d| d.clone());

                {
                    let mut write_state: std::sync::MutexGuard<'_, HashMap<DocumentId, Automerge>> =
                        docs_state_clone.lock().unwrap();
                    write_state.insert(project_doc_id.clone(), doc);
                }

                doc_handle.changed().await.unwrap();
            }
        });
    }

    #[func]
    fn get_doc_id(&self) -> Variant /* String? */ {
        match &*self.checked_out_doc_id.lock().unwrap() {
            Some(doc_id) => doc_id.to_string().to_variant(),
            None => Variant::nil(),
        }
    }

    fn get_doc(&self) -> Automerge {
        return self
            .docs_state
            .lock()
            .unwrap()
            .get(self.checked_out_doc_id.lock().unwrap().as_ref().unwrap())
            .unwrap()
            .clone();
    }

    #[func]
    fn get_heads(&self) -> Array<Variant> /* String[] */ {
        let heads = self.get_doc().get_heads();

        return heads
            .to_vec()
            .iter()
            .map(|h| h.to_string().to_variant())
            .collect::<Array<Variant>>();
    }

    #[func]
    fn get_file(&self, path: String) -> Variant /* String? */ {
        let doc = self.get_doc();

        let files = doc.get(ROOT, "files").unwrap().unwrap().1;

        return match doc.get(files, path) {
            Ok(Some((value, _))) => value.into_string().unwrap_or_default().to_variant(),
            _ => Variant::nil(),
        };
    }

    #[func]
    fn get_file_at(&self, path: String, heads: Array<Variant> /* String[] */) -> Variant /* String? */
    {
        let doc = self.get_doc();
        let heads: Vec<ChangeHash> = heads
            .iter_shared()
            .map(|h| ChangeHash::from_str(h.to_string().as_str()).unwrap())
            .collect();

        let files = doc.get(ROOT, "files").unwrap().unwrap().1;

        return match doc.get_at(files, path, &heads) {
            Ok(Some((value, _))) => value.into_string().unwrap_or_default().to_variant(),
            _ => Variant::nil(),
        };
    }

    #[func]
    fn get_changes(&self) -> Array<Variant> /* String[]  */ {
        self.get_doc()
            .get_changes(&[])
            .to_vec()
            .iter()
            .map(|c| c.hash().to_string().to_variant())
            .collect::<Array<Variant>>()
    }

    #[func]
    fn save_file(&self, path: String, content: String) {
        let path_clone = path.clone();
        let project_doc_id = self.checked_out_doc_id.lock().unwrap().clone();
        let project_doc_id_clone = project_doc_id.clone();

        if let Some(project_doc_handle) = self
            .doc_handles_state
            .lock()
            .unwrap()
            .get(&project_doc_id.unwrap())
        {
            project_doc_handle.with_doc_mut(|d| {
                let mut tx = d.transaction();

                let files = match tx.get(ROOT, "files") {
                    Ok(Some((automerge::Value::Object(ObjType::Map), files))) => files,
                    _ => panic!("Invalid project doc, doesn't have files map"),
                };

                if let Err(e) = tx.put(files, path, content) {
                    panic!("Failed to save file: {:?}", e);
                }

                println!("save {:?}", path_clone);

                tx.commit();
            });

            let new_doc = project_doc_handle.with_doc(|d| d.clone());

            let mut write_state = self.docs_state.lock().unwrap();
            write_state.insert(project_doc_id_clone.unwrap(), new_doc);
        } else {
            println!("too early {:?}", path)
        }
    }
}
