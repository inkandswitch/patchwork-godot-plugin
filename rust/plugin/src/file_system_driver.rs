use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::sync::mpsc::Sender;
use tokio::time::{sleep, Duration};
use notify::{Watcher, RecursiveMode, RecommendedWatcher as WatcherImpl, Config};
use std::sync::mpsc::channel;
use std::time::Duration as StdDuration;
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use ya_md5::{Md5Hasher, Hash, Md5Error};
use tokio::sync::Mutex;
use glob::Pattern;

#[derive(Debug)]
pub enum FileSystemEvent {
    FileCreated(PathBuf),
    FileModified(PathBuf),
    FileDeleted(PathBuf),
}

pub struct FileSystemDriver {
    tx: Sender<FileSystemEvent>,
    watch_path: PathBuf,
    file_hashes: Arc<Mutex<HashMap<PathBuf, String>>>,
    ignore_globs: Vec<Pattern>,
}

impl FileSystemDriver {
    pub fn new(watch_path: PathBuf, ignore_globs: Vec<String>) -> (Self, mpsc::Receiver<FileSystemEvent>) {
        let (tx, rx) = mpsc::channel(100);

        // Convert string globs to Pattern objects
        let ignore_patterns: Vec<Pattern> = ignore_globs
            .into_iter()
            .filter_map(|glob_str| Pattern::new(&glob_str).ok())
            .collect();

        (Self {
            tx,
            watch_path,
            file_hashes: Arc::new(Mutex::new(HashMap::new())),
            ignore_globs: ignore_patterns,
        }, rx)
    }

    // Check if a path should be ignored based on glob patterns
    fn should_ignore(path: &PathBuf, ignore_globs: &[Pattern]) -> bool {
        let path_str = path.to_string_lossy();
        ignore_globs.iter().any(|pattern| pattern.matches(&path_str))
    }

    // Calculate MD5 hash of a file
    fn calculate_file_hash(path: &PathBuf) -> Option<String> {
        if !path.is_file() {
            return None;
        }

        let mut file = match File::open(path) {
            Ok(file) => file,
            Err(_) => return None,
        };

        match Md5Hasher::hash(&mut file) {
            Ok(hash) => Some(format!("{}", hash)),
            Err(_) => None,
        }
    }

    fn _initialize_file_hashes(watch_path: &PathBuf, file_hashes: &mut tokio::sync::MutexGuard<'_, HashMap<PathBuf, String>>, ignore_globs: &[Pattern]) {
        if let Ok(entries) = std::fs::read_dir(watch_path) {
            for entry in entries.flatten() {
                let path = entry.path();

                // Skip if path matches any ignore pattern
                let path_str = path.to_string_lossy();
                if ignore_globs.iter().any(|pattern| pattern.matches(&path_str)) {
                    continue;
                }

                if path.is_file() {
                    if let Some(hash) = Self::calculate_file_hash(&path) {
                        file_hashes.insert(path, hash);
                    }
                } else if path.is_dir() {
                    Self::_initialize_file_hashes(&path, file_hashes, ignore_globs);
                }
            }
        }
    }

    // Initialize the hash map with existing files
    async fn initialize_file_hashes(watch_path: &PathBuf, file_hashes: &Arc<Mutex<HashMap<PathBuf, String>>>, ignore_globs: &[Pattern]) {
        let mut file_hashes: tokio::sync::MutexGuard<'_, HashMap<PathBuf, String>> = file_hashes.lock().await;
        Self::_initialize_file_hashes(watch_path, &mut file_hashes, ignore_globs);
    }

    pub async fn start(&self) {
        let tx = self.tx.clone();
        let watch_path = self.watch_path.clone();
        let file_hashes = self.file_hashes.clone();
        let ignore_globs = self.ignore_globs.clone();

        // Spawn the file system watcher in a separate task
        tokio::spawn(async move {
            Self::initialize_file_hashes(&watch_path, &file_hashes, &ignore_globs).await;

            let (notify_tx, mut notify_rx) = channel();
            let mut watcher = WatcherImpl::new(notify_tx, Config::default()).unwrap();

            // Start watching the directory
            watcher.watch(&watch_path, RecursiveMode::Recursive).unwrap();

            // Process file system events
            while let Ok(res) = notify_rx.recv() {
                match res {
                    Ok(event) => {
                        match event {
                            notify::Event {
                                kind: notify::EventKind::Create(_),
                                paths,
                                ..
                            } => {
                                for path in paths {
                                    // Skip if path matches any ignore pattern
                                    if Self::should_ignore(&path, &ignore_globs) {
                                        continue;
                                    }

                                    if path.is_file() {
                                        if let Some(hash) = Self::calculate_file_hash(&path) {
                                            {
                                                let mut file_hashes = file_hashes.lock().await;
                                                file_hashes.insert(path.clone(), hash);
                                            }
                                            tx.send(FileSystemEvent::FileCreated(path)).await.ok();
                                        }
                                    }
                                }
                            }
                            notify::Event {
                                kind: notify::EventKind::Modify(_),
                                paths,
                                ..
                            } => {
                                for path in paths {
                                    // Skip if path matches any ignore pattern
                                    if Self::should_ignore(&path, &ignore_globs) {
                                        continue;
                                    }

                                    if path.is_file() {
                                        if let Some(new_hash) = Self::calculate_file_hash(&path) {
                                            let mut file_hashes = file_hashes.lock().await;
                                            let hash_changed = file_hashes
                                                .get(&path)
                                                .map_or(true, |old_hash| old_hash != &new_hash);

                                            if hash_changed {
                                                file_hashes.insert(path.clone(), new_hash);
                                                tx.send(FileSystemEvent::FileModified(path)).await.ok();
                                            }
                                        }
                                    }
                                }
                            }
                            notify::Event {
                                kind: notify::EventKind::Remove(_),
                                paths,
                                ..
                            } => {
                                for path in paths {
                                    // Skip if path matches any ignore pattern
                                    if Self::should_ignore(&path, &ignore_globs) {
                                        continue;
                                    }

                                    let mut file_hashes = file_hashes.lock().await;
                                    if file_hashes.remove(&path).is_some() {
                                        tx.send(FileSystemEvent::FileDeleted(path)).await.ok();
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    Err(e) => {
                        eprintln!("Watch error: {:?}", e);
                    }
                }
            }
        });
    }

    pub async fn stop(&self) {
        // The watcher will be dropped when the task is dropped
        // No explicit cleanup needed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_file_system_watcher() {
        // Create a temporary directory for testing
        let dir = tempdir().unwrap();
        let dir_path = dir.path().to_path_buf();

        // Create the file system driver
        let (driver, mut rx) = FileSystemDriver::new(dir_path.clone(), vec![]);
        driver.start().await;

        // Give the watcher time to initialize
        sleep(Duration::from_millis(100)).await;

        // Create a test file
        let test_file = dir_path.join("test.txt");
        let mut file = File::create(&test_file).unwrap();
        file.write_all(b"test content").unwrap();

        // Wait for the create event
        if let Some(event) = rx.recv().await {
            match event {
                FileSystemEvent::FileCreated(path) => {
                    assert_eq!(path, test_file);
                }
                _ => panic!("Unexpected event"),
            }
        }

        // Modify the file with different content
        let mut file = File::options().write(true).open(&test_file).unwrap();
        file.write_all(b"modified content").unwrap();

        // Wait for the modify event
        if let Some(event) = rx.recv().await {
            match event {
                FileSystemEvent::FileModified(path) => {
                    assert_eq!(path, test_file);
                }
                _ => panic!("Unexpected event"),
            }
        }

        // Modify the file with the same content (should not trigger an event)
        let mut file = File::options().write(true).open(&test_file).unwrap();
        file.write_all(b"modified content").unwrap();

        // Delete the file
        std::fs::remove_file(&test_file).unwrap();

        // Wait for the delete event
        if let Some(event) = rx.recv().await {
            match event {
                FileSystemEvent::FileDeleted(path) => {
                    assert_eq!(path, test_file);
                }
                _ => panic!("Unexpected event"),
            }
        }
    }

    #[tokio::test]
    async fn test_ignore_globs() {
        // Create a temporary directory for testing
        let dir = tempdir().unwrap();
        let dir_path = dir.path().to_path_buf();

        // Create the file system driver with ignore globs
        let (driver, mut rx) = FileSystemDriver::new(dir_path.clone(), vec!["*.tmp".to_string()]);
        driver.start().await;

        // Give the watcher time to initialize
        sleep(Duration::from_millis(100)).await;

        // Create a test file that should be ignored
        let ignored_file = dir_path.join("test.tmp");
        let mut file = File::create(&ignored_file).unwrap();
        file.write_all(b"test content").unwrap();

        // Create a test file that should not be ignored
        let test_file = dir_path.join("test.txt");
        let mut file = File::create(&test_file).unwrap();
        file.write_all(b"test content").unwrap();

        // Wait for the create event (should only be for the non-ignored file)
        if let Some(event) = rx.recv().await {
            match event {
                FileSystemEvent::FileCreated(path) => {
                    assert_eq!(path, test_file);
                }
                _ => panic!("Unexpected event"),
            }
        }

        // Modify the ignored file (should not trigger an event)
        let mut file = File::options().write(true).open(&ignored_file).unwrap();
        file.write_all(b"modified content").unwrap();

        // Modify the non-ignored file (should trigger an event)
        let mut file = File::options().write(true).open(&test_file).unwrap();
        file.write_all(b"modified content").unwrap();

        // Wait for the modify event (should only be for the non-ignored file)
        if let Some(event) = rx.recv().await {
            match event {
                FileSystemEvent::FileModified(path) => {
                    assert_eq!(path, test_file);
                }
                _ => panic!("Unexpected event"),
            }
        }

        // Delete both files
        std::fs::remove_file(&ignored_file).unwrap();
        std::fs::remove_file(&test_file).unwrap();

        // Wait for the delete event (should only be for the non-ignored file)
        if let Some(event) = rx.recv().await {
            match event {
                FileSystemEvent::FileDeleted(path) => {
                    assert_eq!(path, test_file);
                }
                _ => panic!("Unexpected event"),
            }
        }
    }
}
