use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

use async_stream::stream;
use futures::Stream;
use md5::Digest;
use notify::{Config, RecommendedWatcher, RecursiveMode};
use notify_debouncer_mini::{DebouncedEvent, new_debouncer_opt};
use tokio::{
    sync::{
        Mutex,
        mpsc::{self},
    },
    task::JoinSet,
    time::sleep,
};
use tokio_stream::wrappers::UnboundedReceiverStream;

use crate::{
    fs::{
        file_utils::FileSystemEvent,
        file_utils::{FileContent, calculate_file_hash, get_buffer_and_hash},
    },
    project::branch_db::BranchDb,
};

// TODO (Lilith): This works, but I'm not sure this complicated
// of a class is necessary...

// Can we just do this naively, provide all FS events to the caller,
// then check the hashes against automerge to see if it's actually changed?

/// Watches a directory for filesystem changes, and emits them as a stream.
#[derive(Debug, Clone)]
pub struct FileSystemWatcher {
    watch_path: PathBuf,
    file_hashes: Arc<Mutex<HashMap<PathBuf, Digest>>>,
    branch_db: BranchDb,
    found_ignored_paths: Arc<Mutex<HashSet<PathBuf>>>,
}

impl FileSystemWatcher {
    // We need to manually desugar the async function here because bug
    // See: https://stackoverflow.com/questions/79851524
    fn initialize_file_hashes_recur(
        &self,
        watch_path: PathBuf,
    ) -> impl Future<Output = tokio::io::Result<()>> + Send {
        async move {
            let mut dir = tokio::fs::read_dir(watch_path).await?;
            let mut set = JoinSet::new();
            while let Some(entry) = dir.next_entry().await? {
                let path = entry.path();

                // Skip if path matches any ignore pattern
                if self.branch_db.should_ignore(&path) {
                    {
                        let mut found_ignored_paths = self.found_ignored_paths.lock().await;
                        found_ignored_paths.insert(path.clone());
                    }
                    continue;
                }

                if path.is_file() {
                    let file_hashes = self.file_hashes.clone();
                    set.spawn(async move {
                        if let Some(hash) = calculate_file_hash(&path).await {
                            let mut file_hashes = file_hashes.lock().await;
                            file_hashes.insert(path, hash);
                        }
                        Ok(())
                    });
                } else if path.is_dir() {
                    let path = path.clone();
                    let this = self.clone();
                    set.spawn(async move { this.initialize_file_hashes_recur(path).await });
                }
            }
            while let Some(_) = set.join_next().await {}
            Ok(())
        }
    }

    // Initialize the hash map with existing files
    async fn initialize_file_hashes(&self) {
        self.initialize_file_hashes_recur(self.watch_path.clone())
            .await
            .unwrap();
    }

    // Handle file creation and modification events
    async fn handle_file_event(
        &self,
        path: PathBuf,
    ) -> Result<Option<FileSystemEvent>, notify::Error> {
        // Skip if path matches any ignore pattern
        if self.branch_db.should_ignore(&path) {
            return Ok(None);
        }
        if !path.exists() {
            // If the file doesn't exist, we want to emit a deleted event
            let mut file_hashes = self.file_hashes.lock().await;
            if file_hashes.contains_key(&path) {
                file_hashes.remove(&path);
                return Ok(Some(FileSystemEvent::FileDeleted(path)));
            }
            return Ok(None);
        }

        if path.is_file() {
            let mut result = get_buffer_and_hash(&path).await;
            // TODO: is this still necessary?
            if result.is_err() {
                sleep(Duration::from_millis(100)).await;
                result = get_buffer_and_hash(&path).await;
            }
            if result.is_err() {
                tracing::error!("failed to get file content {:?}", result);
                return Err(notify::Error::new(notify::ErrorKind::Generic(
                    "Failed to get file content".to_string(),
                )));
            }
            let (content, new_hash) = result.unwrap();
            let mut file_hashes = self.file_hashes.lock().await;
            if file_hashes.contains_key(&path) {
                let old_hash = file_hashes.get(&path).unwrap();
                if old_hash != &new_hash {
                    tracing::trace!(
                        "file {:?} changed, hash {:?} -> {:?}",
                        path,
                        old_hash,
                        new_hash
                    );
                    file_hashes.insert(path.clone(), new_hash);
                    return Ok(Some(FileSystemEvent::FileModified(
                        path,
                        FileContent::from_buf(content),
                    )));
                }
            } else {
                // If the file is newly created, we want to emit a created event
                tracing::trace!("file {:?} created, hash {:?}", path, new_hash);
                file_hashes.insert(path.clone(), new_hash);
                return Ok(Some(FileSystemEvent::FileCreated(
                    path,
                    FileContent::from_buf(content),
                )));
            }
        }
        Ok(None)
    }

    async fn process_notify_event(&self, event: DebouncedEvent) -> Option<FileSystemEvent> {
        if self.branch_db.should_ignore(&event.path) {
            return None;
        }
        let result = self.handle_file_event(event.path.clone()).await;
        if let Ok(Some(ret)) = result {
            return Some(ret);
        }
        return None;
    }

    // Watch the filesystem for meaningful changes
    pub async fn start_watching(
        path: PathBuf,
        branch_db: BranchDb,
    ) -> impl Stream<Item = FileSystemEvent> {
        let (notify_tx, notify_rx) = mpsc::unbounded_channel();
        let notify_config = Config::default()
            .with_follow_symlinks(false);

        let debouncer_config = notify_debouncer_mini::Config::default()
            .with_timeout(Duration::from_millis(100))
            .with_batch_mode(true)
            .with_notify_config(notify_config);

        let notify_tx_clone = notify_tx.clone();
        let mut debouncer = new_debouncer_opt::<_, RecommendedWatcher>(
            debouncer_config,
            move |event: Result<Vec<notify_debouncer_mini::DebouncedEvent>, notify::Error>| {
                notify_tx_clone.send(event).unwrap();
            },
        )
        .unwrap();

        // Begin the watch
        // I'm assuming that notify uses good RAII and stops watching when we kill the handle.... hopefully.
        debouncer
            .watcher()
            .watch(&path, RecursiveMode::Recursive)
            .unwrap();

        let this = FileSystemWatcher {
            watch_path: path,
            file_hashes: Arc::new(Mutex::new(HashMap::new())),
            branch_db,
            found_ignored_paths: Arc::new(Mutex::new(HashSet::new())),
        };

        this.initialize_file_hashes().await;
        for path in this.found_ignored_paths.lock().await.iter() {
            let _ret = debouncer.watcher().unwatch(path);
        }
        let stream = UnboundedReceiverStream::new(notify_rx);
        // Process both file system events and update eventss
        stream! {
            // move the debouncer into the returned stream
            let _keep_alive = debouncer;
            // Handle file system events
            for await notify_events in stream {
                let Ok(notify_events) = notify_events else {
                    continue;
                };
                tracing::debug!("Heard filesystem event list of {:?} items", notify_events.len());
                for notify_event in notify_events {
                    tracing::trace!("Heard filesystem event {:?}", notify_event);
                    if let Some(evt) = this.process_notify_event(notify_event).await {
                        yield evt;
                    }
                }
            }
            tracing::debug!("fs_watcher shutting down!");
        }
    }
}
