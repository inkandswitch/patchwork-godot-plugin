use std::{
    collections::HashMap,
    str::FromStr,
    sync::{
        mpsc::{Receiver, Sender},
        Arc, Mutex,
    },
};

use automerge::{transaction::Transactable, Automerge, ChangeHash, ObjType, ReadDoc, ROOT};
use automerge_repo::{tokio::FsStorage, ConnDirection, DocHandle, DocumentId, Repo, RepoHandle};
use autosurgeon::{bytes, hydrate, reconcile, Hydrate, Reconcile};
use futures::StreamExt;
use godot::{obj, prelude::*};
use tokio::{net::TcpStream, runtime::Runtime};
use crate::{doc_state_map::DocStateMap, doc_utils::SimpleDocReader, DocHandleMap};

#[derive(Debug, Clone, Reconcile, Hydrate, PartialEq)]
struct BinaryFile {
    content: Vec<u8>
}


#[derive(Debug, Clone, Reconcile, Hydrate, PartialEq)]
struct FileEntry {
    content: Option<String>,
    url: Option<String>
}

#[derive(Debug, Clone, Reconcile, Hydrate, PartialEq)]
struct GodotProjectDoc {
    files: HashMap<String, FileEntry>,
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

#[derive(Clone)]
enum SyncEvent {
    DocChanged { doc_id: DocumentId },
}

#[derive(Clone)]
enum StringOrPackedByteArray {
    String(String),
    PackedByteArray(PackedByteArray)
}

#[derive(GodotClass)]
#[class(no_init, base=Node)]
pub struct GodotProject {
    base: Base<Node>,
    runtime: Runtime,
    repo_handle: RepoHandle,
    branches_metadata_doc_id: DocumentId,
    checked_out_doc_id: Arc<Mutex<Option<DocumentId>>>,
    doc_state_map: DocStateMap,
    doc_handle_map: DocHandleMap,
    sync_event_receiver: Receiver<SyncEvent>,
}

const SERVER_URL: &str = "104.131.179.247:8080";

#[godot_api]
impl GodotProject {
    #[signal]
    fn checked_out_branch(branch_id: String);

    #[signal]
    fn files_changed();

    #[signal]
    fn branches_changed();

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

        // todo: store in project folder
        let storage = FsStorage::open("/tmp/automerge-godot-data").unwrap();
        let repo = Repo::new(None, Box::new(storage));
        let repo_handle = repo.run();

        let (new_doc_tx, new_doc_rx) = futures::channel::mpsc::unbounded();

        let doc_state_map = DocStateMap::new();
        let doc_handle_map = DocHandleMap::new(new_doc_tx.clone());

        let checked_out_doc_id = Arc::new(Mutex::new(None));

        let branches_metadata_doc_id = if maybe_branches_metadata_doc_id.is_empty() {
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
            let branches_metadata_doc_handle_clone = branches_metadata_doc_handle.clone();

            // Add both docs to the states
            {
                doc_state_map.add_doc(project_doc_id, project_doc_handle.with_doc(|d| d.clone()));
                doc_handle_map.add_handle(project_doc_handle.clone());

                // Add branches metadata doc
                doc_state_map.add_doc(
                    branches_metadata_doc_handle.document_id(),
                    branches_metadata_doc_handle.with_doc(|d| d.clone()),
                );
                doc_handle_map.add_handle(branches_metadata_doc_handle.clone());
            }

            {
                let mut checked_out = checked_out_doc_id.lock().unwrap();
                *checked_out = Some(project_doc_id_clone.clone());
            }

            branches_metadata_doc_handle_clone.document_id()
        } else {
            let branches_metadata_doc_id =
                DocumentId::from_str(&maybe_branches_metadata_doc_id).unwrap();
            let branches_metadata_doc_id_clone = branches_metadata_doc_id.clone();
            let branches_metadata_doc_id_clone_2 = branches_metadata_doc_id.clone();

            let repo_handle_clone = repo_handle.clone();
            let doc_state_map = doc_state_map.clone();
            let doc_handle_map = doc_handle_map.clone();
            let checked_out_doc_id = checked_out_doc_id.clone();

            runtime.spawn(async move {
                let repo_handle_result = repo_handle_clone
                    .request_document(branches_metadata_doc_id)
                    .await
                    .unwrap();

                let branches_metadata: BranchesMetadataDoc =
                    repo_handle_result.with_doc(|d| hydrate(d).unwrap());

                let main_doc_id = DocumentId::from_str(&branches_metadata.main_doc_id).unwrap();
                let main_doc_id_clone = main_doc_id.clone();

                // Request main branch doc
                let main_doc_handle_result = repo_handle_clone
                    .request_document(main_doc_id)
                    .await
                    .unwrap();

                // Request all other branch docs
                let mut other_branches = Vec::new();
                for (branch_id, _) in &branches_metadata.branches {
                    let branch_doc_id = DocumentId::from_str(branch_id).unwrap();
                    let branch_doc_handle = repo_handle_clone
                        .request_document(branch_doc_id.clone())
                        .await
                        .unwrap();
                    other_branches.push((branch_doc_id, branch_doc_handle));
                }

                // Add both docs to the states
                {
                    let main_doc = main_doc_handle_result.with_doc(|d| d.clone());

                    // Add main doc
                    doc_state_map.add_doc(main_doc_id_clone.clone(), main_doc);
                    doc_handle_map.add_handle(main_doc_handle_result.clone());

                    // Add other branches
                    for (branch_doc_id, branch_doc_handle) in other_branches {
                        doc_state_map.add_doc(
                            branch_doc_id.clone(),
                            branch_doc_handle.with_doc(|d| d.clone()),
                        );
                        doc_handle_map.add_handle(branch_doc_handle);
                    }

                    // Add branches metadata doc
                    doc_state_map.add_doc(
                        branches_metadata_doc_id_clone.clone(),
                        repo_handle_result.with_doc(|d| d.clone()),
                    );
                    doc_handle_map.add_handle(repo_handle_result);
                }

                let mut checked_out = checked_out_doc_id.lock().unwrap();
                *checked_out = Some(main_doc_id_clone);
            });

            branches_metadata_doc_id_clone_2
        };

        let (sync_event_sender, sync_event_receiver) = std::sync::mpsc::channel();

        // Spawn connection task
        Self::spawn_connection_task(&runtime, repo_handle.clone());

        // Spawn sync task for all doc handles
        Self::spawn_sync_task(
            &runtime,
            new_doc_rx,
            doc_handle_map.clone(),
            doc_state_map.clone(),
            sync_event_sender.clone(),
        );

        return Gd::from_init_fn(|base| Self {
            base,
            runtime,
            repo_handle,
            branches_metadata_doc_id,
            checked_out_doc_id,
            doc_handle_map,
            doc_state_map,
            sync_event_receiver,
        });
    }

    // PUBLIC API

    #[func]
    fn stop(&self) {
        // todo: is this right?
        unsafe {
            let runtime = std::ptr::read(&self.runtime);
            runtime.shutdown_background();
        }
    }

    #[func]
    fn get_doc_id(&self) -> String {
        return self.branches_metadata_doc_id.to_string();
    }

    #[func]
    fn get_heads(&self) -> Array<Variant> /* String[] */ {
        let checked_out_doc_id = self.get_checked_out_doc_id();

        let doc = self.doc_state_map.get_doc(&checked_out_doc_id).unwrap_or_else(|| {
            panic!(
                "Failed to get doc for checked out doc id: {}",
                &checked_out_doc_id
            )
        });

        let heads = doc.get_heads();

        return heads
            .to_vec()
            .iter()
            .map(|h| h.to_string().to_variant())
            .collect::<Array<Variant>>();
    }

    #[func]
    fn list_all_files(&self) -> Array<Variant> /* String[] */ {
        let doc = self
            .doc_state_map
            .get_doc(&self.get_checked_out_doc_id())
            .unwrap_or_else(|| panic!("Failed to get checked out doc"));

        let files = match doc.get_obj_id(ROOT, "files") {
            Some(files) => files,
            _ => {
                return array![];
            }
        };


        let keys = doc.keys(files).collect::<Vec<String>>();
        return keys.into_iter().map(|k| k.to_variant()).collect::<Array<Variant>>();            
    }

    #[func]
    fn get_file(&self, path: String) -> Variant /* String? */ {
        let doc = self
            .doc_state_map
            .get_doc(&self.get_checked_out_doc_id())
            .unwrap_or_else(|| panic!("Failed to get checked out doc"));

        let files = doc.get(ROOT, "files").unwrap().unwrap().1;
        
        // does the file exist?
        let file_entry = match doc.get(files, path) {
            Ok(Some((automerge::Value::Object(ObjType::Map), file_entry))) => file_entry,
            _ => return Variant::nil(),
        };


        // try to read file as text
        match doc.get(&file_entry, "content") {
            Ok(Some((automerge::Value::Object(ObjType::Text), content))) => {
                match doc.text(content) {
                    Ok(text) => return text.to_variant(),
                    Err(_) => {}
                }
            },
            _ => {}
        }

        // ... otherwise try to read as linked binary doc

    
        if let Some(url) = doc.get_string(&file_entry, "url") {                                                                            

            // parse doc url
            let doc_id = match parse_automerge_url(&url) {
                Some(url) => url,
                _ => return Variant::nil()                    
            };

            // read content doc
            let content_doc = match self.doc_state_map.get_doc(&doc_id) {
                Some(doc) => doc,
                _ => return Variant::nil()
            };

            // get content of binary file
            if let Some(bytes) = content_doc.get_bytes(ROOT, "content") {
                return bytes.to_variant()
            };
        };
       
        // finally give up
        return Variant::nil()

    }

    #[func]
    fn get_file_at(&self, path: String, heads: Array<Variant> /* String[] */) -> Variant /* String? */
    {
        let doc = self
            .doc_state_map
            .get_doc(&self.get_checked_out_doc_id())
            .unwrap_or_else(|| panic!("Failed to get checked out doc"));

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
        let doc = self
            .doc_state_map
            .get_doc(&self.get_checked_out_doc_id())
            .unwrap_or_else(|| panic!("Failed to get checked out doc"));

        doc.get_changes(&[])
            .to_vec()
            .iter()
            .map(|c| c.hash().to_string().to_variant())
            .collect::<Array<Variant>>()
    }

    #[func]
    fn save_file(&self, path: String, content: Variant) {
    
        let content: StringOrPackedByteArray = if content.try_to::<String>().is_ok() {
            StringOrPackedByteArray::String(content.to::<String>())
        } else if content.try_to::<PackedByteArray>().is_ok() {
            StringOrPackedByteArray::PackedByteArray(content.to::<PackedByteArray>())
        } else {
            println!("invalid {:?}", path);
            return
        };


        let checked_out_doc_id = self.get_checked_out_doc_id();
        let checked_out_doc_id_clone = checked_out_doc_id.clone();
        let checked_out_doc_handle = self.get_doc_handle(checked_out_doc_id.clone());

        if let Some(project_doc_handle) = checked_out_doc_handle {

            project_doc_handle.with_doc_mut(|d| {
                let mut tx = d.transaction();

                let files = match tx.get(ROOT, "files") {
                    Ok(Some((automerge::Value::Object(ObjType::Map), files))) => files,
                    _ => panic!("Invalid project doc, doesn't have files map"),
                };

                match content {
                    StringOrPackedByteArray::String(content) => {
                        println!("write string {:}", path);

                        // get existing file url or create new one                        
                        let file_entry = match tx.get(&files, &path) {
                            Ok(Some((automerge::Value::Object(ObjType::Map), file_entry))) => file_entry,
                            _ => tx.put_object(files, &path, ObjType::Map).unwrap()
                        };
                
                        // delete url in file entry if it previously had one
                        if let Ok(Some((_, _))) = tx.get(&file_entry, "url") {
                            let _ = tx.delete(&file_entry, "url");
                        }
                    
                        // either get existing text or create new text
                        let content_key = match tx.get(&file_entry, "content") {
                            Ok(Some((automerge::Value::Object(ObjType::Text), content))) => content,
                            _ => tx.put_object(&file_entry, "content", ObjType::Text).unwrap(),
                        };
                        let _ = tx.update_text(&content_key, &content);
                
                    },
                    StringOrPackedByteArray::PackedByteArray(content) => {
                        println!("write binary {:}", path);

                        // create content doc
                        let content_doc_id = self.create_doc(|d| {
                            let mut tx = d.transaction();
                            let _ = tx.put(ROOT, "content", content.to_vec());
                            tx.commit();
                        });

                        // write url to content doc into project doc
                        let file_entry = tx.put_object(files, path, ObjType::Map);
                        let _ = tx.put(file_entry.unwrap(), "url", format!("automerge:{}", content_doc_id));            
                    },
                }
    

                tx.commit();
            });

            let new_doc = project_doc_handle.with_doc(|d| d.clone());
            self.doc_state_map.add_doc(checked_out_doc_id_clone, new_doc);
        } else {
            println!("too early {:?}", path)
        }
    }

    #[func]
    fn create_branch(&self, name: String) -> String {
        let branches_metadata_doc = self
            .doc_state_map
            .get_doc(&self.branches_metadata_doc_id)
            .unwrap_or_else(|| panic!("Failed to load branches metadata doc"));

        let mut branches_metadata: BranchesMetadataDoc = hydrate(&branches_metadata_doc)
            .unwrap_or_else(|e| panic!("Failed to hydrate branches metadata doc: {}", e));

        let main_doc_id = DocumentId::from_str(&branches_metadata.main_doc_id).unwrap();
        let new_doc_id = self.clone_doc(main_doc_id);

        branches_metadata.branches.insert(
            new_doc_id.to_string(),
            Branch {
                name,
                id: new_doc_id.to_string(),
            },
        );

        self.update_doc(self.branches_metadata_doc_id.clone(), |d| {
            let mut tx = d.transaction();
            reconcile(&mut tx, branches_metadata).unwrap();
            tx.commit();
        });

        new_doc_id.to_string()
    }

    #[func]
    fn checkout_branch(&mut self, branch_id: String) {
        let doc_id = if branch_id == "main" {
            let branches_metadata_doc = self
                .doc_state_map
                .get_doc(&self.branches_metadata_doc_id)
                .unwrap_or_else(|| panic!("couldn't load branches metadata doc {}", branch_id));

            let branches_metadata: BranchesMetadataDoc =
                hydrate(&branches_metadata_doc).expect("failed to hydrate branches metadata doc");

            DocumentId::from_str(&branches_metadata.main_doc_id).unwrap()
        } else {
            DocumentId::from_str(&branch_id).unwrap()
        };

        {
            let mut checked_out = self.checked_out_doc_id.lock().unwrap();
            *checked_out = Some(doc_id);
        } // Release the lock before emitting signal

        self.base_mut()
            .emit_signal("checked_out_branch", &[branch_id.to_variant()]);
    }

    #[func]
    fn get_branches(&self) -> Array<Variant> /* { name: String, id: String }[] */ {
        let maybe_branches_metadata_doc = self.get_doc(self.branches_metadata_doc_id.clone());

        let branches_metadata_doc = match maybe_branches_metadata_doc {
            Some(doc) => doc,
            None => {
                panic!("couldn't load branches_metadata_doc");
            }
        };

        let branches_metadata: BranchesMetadataDoc = hydrate(&branches_metadata_doc).unwrap();

        let mut branches = array![];

        // Add main branch
        branches.push(
            &dict! {
                "name": "main",
                "id": "main"
            }
            .to_variant(),
        );

        // Add other branches
        for (id, branch) in branches_metadata.branches {
            branches.push(
                &dict! {
                    "name": branch.name,
                    "id": id
                }
                .to_variant(),
            );
        }

        branches
    }

    #[func]
    fn get_checked_out_branch_id(&self) -> String {
        return self
            .checked_out_doc_id
            .lock()
            .unwrap()
            .clone()
            .unwrap()
            .to_string();
    }

    // these functions below should be extracted into a separate SyncRepo class

    // SYNC

    // needs to be called every frame to process the internal events
    #[func]
    fn process(&mut self) {
        let checked_out_doc_id = self.checked_out_doc_id.lock().unwrap().clone().unwrap();

        // Process all pending sync events
        while let Ok(event) = self.sync_event_receiver.try_recv() {
            match event {
                SyncEvent::DocChanged { doc_id } => {
                    println!("doc changed event {:?} {:?}", doc_id, checked_out_doc_id);

                    // Check if branches metadata doc changed
                    if doc_id == self.branches_metadata_doc_id {

                        // load branches metadata doc
                        let doc = self
                            .doc_state_map
                            .get_doc(&self.branches_metadata_doc_id)
                            .unwrap_or_else(|| panic!("Failed to get branches metadata doc"));
                        
                        let branches_metadata: BranchesMetadataDoc = hydrate(&doc)
                            .unwrap_or_else(|e| panic!("Failed to hydrate branches metadata doc: {:?}", e));

                        // check if it has new branches so we can load them
                        for (branch_doc_id, _) in branches_metadata.branches {
                            let branch_doc_id = DocumentId::from_str(&branch_doc_id).unwrap();
                            if self.doc_handle_map.get_doc(&branch_doc_id).is_none() {
                                self.request_doc(branch_doc_id);                            
                            }
                        }

                        self.base_mut().emit_signal("branches_changed", &[]);
                    }
                    // Check if checked out doc changed
                    else if doc_id == checked_out_doc_id {
                        self.base_mut().emit_signal("files_changed", &[]);
                    }
                }
            }
        }
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
        mut new_docs: futures::channel::mpsc::UnboundedReceiver<DocHandle>,
        doc_handle_map: DocHandleMap,    
        doc_state_map: DocStateMap,
        sync_event_sender: Sender<SyncEvent>,
    ) {
        let initial_handles = doc_handle_map
            .current_handles();

        runtime.spawn(async move {
            // This is a stream of SyncEvent
            let mut all_doc_changes = futures::stream::SelectAll::new();

            // First add a stream for all the initial documents to the SelectAll
            for doc_handle in initial_handles {
                let doc_id = doc_handle.document_id();
                let doc_handle = doc_handle.clone();

                let change_stream = handle_changes(doc_handle.clone()).filter_map(move |diff| {
                    let doc_id = doc_id.clone();
                    async move {
                        if diff.is_empty() {
                            None
                        } else {
                            Some(SyncEvent::DocChanged { doc_id: doc_id.clone() } )
                        }
                    }
                });

                all_doc_changes.push(change_stream.boxed());
            }

            // Now, drive the SelectAll and also wait for any new documents to  arrive and add
            // them to the selectall
            loop {
                futures::select! {
                    sync_event = all_doc_changes.select_next_some() => {

                        // update stored state of doc
                        let SyncEvent::DocChanged { doc_id } = sync_event.clone();
                        let doc_handle = doc_handle_map.get_doc(&doc_id.clone()).unwrap();
                        let doc = doc_handle.with_doc(|d| d.clone());
                        doc_state_map.add_doc(doc_id.clone(), doc);

                        // emit change event
                        sync_event_sender.send(sync_event).unwrap();                    
                    }
                    new_doc = new_docs.select_next_some() => {
                        let doc_id = new_doc.document_id();
                        let doc_handle = new_doc.clone();

                        println!("new doc!!!");

                        let change_stream = handle_changes(doc_handle.clone()).filter_map(move |diff| {
                            let doc_id = doc_id.clone();
                            async move {
                                if diff.is_empty() {
                                    None
                                } else {
                                    Some(SyncEvent::DocChanged { doc_id: doc_id.clone() } )
                                }
                            }
                        });

                        all_doc_changes.push(change_stream.boxed());
                    }
                }
            }
        });
    }

    // DOC ACCESS + MANIPULATION

    fn update_doc<F>(&self, doc_id: DocumentId, f: F)
    where
        F: FnOnce(&mut Automerge),
    {
        if let Some(doc_handle) = self.doc_handle_map.get_doc(&doc_id) {
            doc_handle.with_doc_mut(f);
            let new_doc = doc_handle.with_doc(|d| d.clone());
            self.doc_state_map.add_doc(doc_id, new_doc);
        }
    }


    fn create_doc<F>(&self, f: F) -> DocumentId
    where
        F: FnOnce(&mut Automerge),
    {
        let doc_handle = self.repo_handle.new_document();
        let doc_id = doc_handle.document_id();

        self.doc_handle_map.add_handle(doc_handle.clone());

        self.update_doc(doc_id.clone(), f);

        doc_id
    }

    
    fn request_doc(&self, doc_id: DocumentId) {
        let repo_handle = self.repo_handle.clone();
        let doc_state_map = self.doc_state_map.clone();
        let doc_handle_map = self.doc_handle_map.clone();
        let doc_id_clone = doc_id.clone();

        self.runtime.spawn(async move {
            let repo_handle_result = repo_handle
                .request_document(doc_id);
            
            let doc_handle = repo_handle_result.await.unwrap();
            let doc = doc_handle.with_doc(|d| d.clone());
            
            doc_state_map.add_doc(doc_id_clone, doc);
            doc_handle_map.add_handle(doc_handle);
        });
    }

    fn clone_doc(&self, doc_id: DocumentId) -> DocumentId {
        let new_doc_handle = self.repo_handle.new_document();
        let new_doc_id = new_doc_handle.document_id();
        let doc_handle = self.get_doc_handle(doc_id.clone());

        let changes_count = doc_handle
            .clone()
            .unwrap()
            .with_doc(|d| d.get_changes(&[]).len());

        println!("changes {:?}", changes_count);

        let doc_handle = doc_handle.unwrap_or_else(|| panic!("Couldn't clone doc_id: {}", &doc_id));
        let _ = doc_handle
            .with_doc_mut(|mut main_d| new_doc_handle.with_doc_mut(|d| d.merge(&mut main_d)));

        self.doc_handle_map.add_handle(new_doc_handle.clone());
        self.doc_state_map.add_doc(new_doc_id.clone(), new_doc_handle.with_doc(|d| d.clone()));

        new_doc_id
    }

    fn get_doc(&self, id: DocumentId) -> Option<Automerge> {
        self.doc_state_map.get_doc(&id.into())
    }

    fn get_doc_handle(&self, id: DocumentId) -> Option<DocHandle> {
        self.doc_handle_map.get_doc(&id.into())
    }

    fn get_checked_out_doc_id(&self) -> DocumentId {
        return self.checked_out_doc_id.lock().unwrap().clone().unwrap();
    }
}

fn handle_changes(handle: DocHandle) -> impl futures::Stream<Item = Vec<automerge::Patch>> + Send {
    futures::stream::unfold(handle, |doc_handle| async {
        let heads_before = doc_handle.with_doc(|d| d.get_heads().to_vec());
        let _ = doc_handle.changed().await;
        let heads_after = doc_handle.with_doc(|d| d.get_heads().to_vec());
        let diff = doc_handle.with_doc(|d| {
            d.diff(
                &heads_before,
                &heads_after,
                automerge::patches::TextRepresentation::String,
            )
        });

        Some((diff, doc_handle))
    })
}




fn parse_automerge_url(url: &str) -> Option<DocumentId> {
    const PREFIX: &str = "automerge:";
    if !url.starts_with(PREFIX) {
        return None;
    }
    
    let hash = &url[PREFIX.len()..];
    DocumentId::from_str(hash).ok()
}



