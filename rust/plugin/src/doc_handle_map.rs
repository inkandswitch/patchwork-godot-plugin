use std::{collections::HashMap, sync::{Arc, Mutex}};

use automerge_repo::{DocHandle, DocumentId};


#[derive(Clone)]
pub(crate) struct DocHandleMap {
    handles: Arc<Mutex<HashMap<DocumentId, DocHandle>>>,
    new_tx: futures::channel::mpsc::UnboundedSender<DocHandle>,
}


impl DocHandleMap {
    pub(crate) fn new(new_tx: futures::channel::mpsc::UnboundedSender<DocHandle>) -> Self {
        Self {
            handles: Arc::new(Mutex::new(HashMap::new())),
            new_tx,
        }
    }

    pub(crate) fn add_handle(&self, handle: DocHandle) {
        self.handles.lock().unwrap().insert(handle.document_id(), handle.clone());
        self.new_tx.unbounded_send(handle).unwrap();
    }

    pub(crate) fn current_handles(&self) -> Vec<DocHandle> {
        self.handles.lock().unwrap().values().cloned().collect()
    }

    pub(crate) fn get_doc(&self, doc_id: &DocumentId) -> Option<DocHandle> {
        self.handles.lock().unwrap().get(doc_id).cloned()
    }
}

