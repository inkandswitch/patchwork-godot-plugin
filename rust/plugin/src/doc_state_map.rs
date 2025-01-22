use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use automerge::Automerge;
use automerge_repo::{DocHandle, DocumentId};

#[derive(Clone)]
pub(crate) struct DocStateMap {
    states: Arc<Mutex<HashMap<DocumentId, Automerge>>>,
}

impl DocStateMap {
    pub(crate) fn new() -> Self {
        Self {
            states: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub(crate) fn add_doc(&self, doc_id: DocumentId, doc: Automerge) {
        self.states.lock().unwrap().insert(doc_id, doc);
    }

    pub(crate) fn get_doc(&self, doc_id: &DocumentId) -> Option<Automerge> {
        self.states.lock().unwrap().get(doc_id).cloned()
    }

    pub(crate) fn current_docs(&self) -> Vec<(DocumentId, Automerge)> {
        self.states
            .lock()
            .unwrap()
            .iter()
            .map(|(id, doc)| (id.clone(), doc.clone()))
            .collect()
    }
}
