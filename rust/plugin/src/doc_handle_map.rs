use std::{collections::HashMap, sync::{Arc, Mutex}};

use automerge::{transaction::Transaction, AutoCommit, Automerge};
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
        let mut handles = self.handles.lock().unwrap();
    
        // ignore if doc is already in the map
        if handles.contains_key(&handle.document_id()) {
            return;
        }
        
        handles.insert(handle.document_id(), handle.clone());
        self.new_tx.unbounded_send(handle).unwrap();
    }

    pub(crate) fn current_handles(&self) -> Vec<DocHandle> {
        self.handles.lock().unwrap().values().cloned().collect()
    }

    pub(crate) fn get_handle(&self, doc_id: &DocumentId) -> Option<DocHandle> {
        self.handles.lock().unwrap().get(doc_id).cloned()
    }

    pub(crate) fn get_doc(&self, doc_id: &DocumentId) -> Option<Automerge> {
        if let Some(handle) = self.get_handle(doc_id) {
            return Some(handle.with_doc(|d| d.clone()))
        }
        return None
    }
}

