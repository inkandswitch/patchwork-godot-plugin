use automerge_repo::{DocumentId, Storage, StorageError};
use futures::future::BoxFuture;
use futures::FutureExt;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;

pub struct SimpleStorage;

impl Storage for SimpleStorage {
    fn get(&self, _id: DocumentId) -> BoxFuture<'static, Result<Option<Vec<u8>>, StorageError>> {
        futures::future::ready(Ok(None)).boxed()
    }

    fn list_all(&self) -> BoxFuture<'static, Result<Vec<DocumentId>, StorageError>> {
        futures::future::ready(Ok(Vec::new())).boxed()
    }

    fn append(
        &self,
        _id: DocumentId,
        _chunk: Vec<u8>,
    ) -> BoxFuture<'static, Result<(), StorageError>> {
        futures::future::ready(Ok(())).boxed()
    }

    fn compact(
        &self,
        _id: DocumentId,
        _chunk: Vec<u8>,
    ) -> BoxFuture<'static, Result<(), StorageError>> {
        futures::future::ready(Ok(())).boxed()
    }
}

#[derive(Clone, Debug, Default)]
pub struct InMemoryStorage {
    documents: Arc<Mutex<HashMap<DocumentId, Vec<u8>>>>,
}

impl InMemoryStorage {
    pub fn add_document(&self, doc_id: DocumentId, mut doc: Vec<u8>) {
        let mut documents = self.documents.lock();
        let entry = documents.entry(doc_id).or_insert_with(Default::default);
        entry.append(&mut doc);
    }

    pub fn contains_document(&self, doc_id: DocumentId) -> bool {
        self.documents.lock().contains_key(&doc_id)
    }

    pub fn fork(&self) -> Self {
        Self {
            documents: Arc::new(Mutex::new(self.documents.lock().clone())),
        }
    }
}

impl Storage for InMemoryStorage {
    fn get(&self, id: DocumentId) -> BoxFuture<'static, Result<Option<Vec<u8>>, StorageError>> {
        futures::future::ready(Ok(self.documents.lock().get(&id).cloned())).boxed()
    }

    fn list_all(&self) -> BoxFuture<'static, Result<Vec<DocumentId>, StorageError>> {
        futures::future::ready(Ok(self.documents.lock().keys().cloned().collect())).boxed()
    }

    fn append(
        &self,
        id: DocumentId,
        mut changes: Vec<u8>,
    ) -> BoxFuture<'static, Result<(), StorageError>> {
        let mut documents = self.documents.lock();
        let entry = documents.entry(id).or_insert_with(Default::default);
        entry.append(&mut changes);
        futures::future::ready(Ok(())).boxed()
    }

    fn compact(
        &self,
        id: DocumentId,
        full_doc: Vec<u8>,
    ) -> BoxFuture<'static, Result<(), StorageError>> {
        let mut documents = self.documents.lock();
        documents.insert(id, full_doc);
        futures::future::ready(Ok(())).boxed()
    }
}
