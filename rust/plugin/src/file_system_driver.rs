use std::path::PathBuf;
use std::sync::Arc;
use futures::{
    channel::mpsc::{UnboundedReceiver, UnboundedSender},
    StreamExt,
};
use rlimit::{setrlimit, Resource};
use tokio::time::{sleep, Duration};
use notify::{Watcher, RecursiveMode, Config, Event, EventHandler};
// if on macos, use kqueue, otherwise use recommended
#[cfg(target_os = "macos")]
use notify::KqueueWatcher as WatcherImpl;
#[cfg(not(target_os = "macos"))]
use notify::RecommendedWatcher as WatcherImpl;
use std::sync::mpsc::channel;
use std::time::Duration as StdDuration;
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Write};
use ya_md5::{Md5Hasher, Hash, Md5Error};
use tokio::sync::Mutex;
use glob::Pattern;

use crate::godot_project::FileContent;

// Custom event handler that bridges between std::sync::mpsc and futures::channel::mpsc
struct UnboundedEventHandler {
    sender: UnboundedSender<Result<Event, notify::Error>>,
}

impl EventHandler for UnboundedEventHandler {
    fn handle_event(&mut self, event: Result<Event, notify::Error>) {
        // Forward the event to the UnboundedSender
        let _ = self.sender.unbounded_send(event);
    }
}

#[derive(Debug)]
pub enum FileSystemEvent {
    FileCreated(PathBuf),
    FileModified(PathBuf),
    FileDeleted(PathBuf),
}

#[derive(Debug)]
pub enum FileSystemUpdateEvent {
    FileCreated(PathBuf, FileContent),
    FileModified(PathBuf, FileContent),
    FileDeleted(PathBuf),
}

pub struct FileSystemDriver {
    watch_path: PathBuf,
    file_hashes: Arc<Mutex<HashMap<PathBuf, String>>>,
    ignore_globs: Vec<Pattern>,
}

impl FileSystemDriver {
    pub fn new(watch_path: PathBuf, ignore_globs: Vec<String>) -> Self {

        // Convert string globs to Pattern objects
        let ignore_patterns: Vec<Pattern> = ignore_globs
            .into_iter()
            .filter_map(|glob_str| Pattern::new(&glob_str).ok())
            .collect();

        Self {
            watch_path,
            file_hashes: Arc::new(Mutex::new(HashMap::new())),
            ignore_globs: ignore_patterns,
        }
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

    fn _initialize_file_hashes(watch_path: &PathBuf, file_hashes: &mut tokio::sync::MutexGuard<'_, HashMap<PathBuf, String>>, sym_links: &mut HashMap<PathBuf, PathBuf>, ignore_globs: &[Pattern]) {
        if let Ok(entries) = std::fs::read_dir(watch_path) {
            for entry in entries.flatten() {
                let path = entry.path();

                // Skip if path matches any ignore pattern
                let path_str = path.to_string_lossy();
                if ignore_globs.iter().any(|pattern| pattern.matches(&path_str)) {
                    continue;
                }
                if let Ok(metadata) = path.metadata() {
                    if metadata.is_symlink() {
                        let target = std::fs::read_link(&path).unwrap();
                        sym_links.insert(path.clone(), target);
                    }
                }

                if path.is_file() {
                    // check if the path is a symlink
                    if let Some(hash) = Self::calculate_file_hash(&path) {
                        file_hashes.insert(path, hash);
                    }
                } else if path.is_dir() {
                    Self::_initialize_file_hashes(&path, file_hashes, sym_links, ignore_globs);
                }
            }
        }
    }

    // Initialize the hash map with existing files
    async fn initialize_file_hashes(watch_path: &PathBuf, file_hashes: &Arc<Mutex<HashMap<PathBuf, String>>>, sym_links: &mut HashMap<PathBuf, PathBuf>, ignore_globs: &[Pattern]) {
        let mut file_hashes: tokio::sync::MutexGuard<'_, HashMap<PathBuf, String>> = file_hashes.lock().await;
        Self::_initialize_file_hashes(watch_path, &mut file_hashes, sym_links, ignore_globs);
    }

    // Write file content to disk
    fn write_file_content(path: &PathBuf, content: &FileContent) -> std::io::Result<String> {
        // Check if the file exists
        let file_exists = path.exists();

        // Open the file with the appropriate mode
        let mut file = if file_exists {
            // If file exists, open it for writing (truncate)
            File::options().write(true).truncate(true).open(path)?
        } else {
            // If file doesn't exist, create it
            File::create(path)?
        };

        // Write the content based on its type
        match content {
            FileContent::String(text) => {
                let hash = Md5Hasher::hash_slice(text.as_bytes());
                let hash_str = format!("{}", hash);
                if let Ok(_) = file.write_all(text.as_bytes()) {
                    return Ok(hash_str);
                }
            }
            FileContent::Binary(data) => {
                let hash = Md5Hasher::hash_slice(data);
                let hash_str = format!("{}", hash);
                if let Ok(_) = file.write_all(data) {
                    return Ok(hash_str);
                }
            }
            FileContent::Scene(scene) => {
                let text = scene.serialize();
                let hash = Md5Hasher::hash_slice(text.as_bytes());
                let hash_str = format!("{}", hash);
                if let Ok(_) = file.write_all(text.as_bytes()) {
                    return Ok(hash_str);
                }
            }
        }
        Err(std::io::Error::new(std::io::ErrorKind::Other, "Failed to write file"))

    }


    pub fn desymlinkify(path: &PathBuf, sym_links: &HashMap<PathBuf, PathBuf>) -> PathBuf {
        let mut new_path = path.clone();
        // let mut target_max_len = 0;
        // for (src, target) in sym_links.iter() {
        // 	if path.starts_with(target) && target.to_str().unwrap().len() > target_max_len {
        // 		target_max_len = target.to_str().unwrap().len();
        // 		new_path = src.join(path.strip_prefix(target).unwrap());
        // 	}
        // }
        new_path
    }

    // Handle file creation and modification events
    async fn handle_file_event(
        path: PathBuf,
        file_hashes: &Arc<Mutex<HashMap<PathBuf, String>>>,
        output_tx: &UnboundedSender<FileSystemEvent>,
        ignore_globs: &[Pattern],
        sym_links: &mut HashMap<PathBuf, PathBuf>,
    ) {
        // Process symlinks and get the actual path
        let path = if let Ok(metadata) = std::fs::metadata(&path) {
            if metadata.is_symlink() {
                let target = std::fs::read_link(&path).unwrap();
                sym_links.insert(path.clone(), target);
                path
            } else {
                Self::desymlinkify(&path, sym_links)
            }
        } else {
            Self::desymlinkify(&path, sym_links)
        };

        // Skip if path matches any ignore pattern
        if Self::should_ignore(&path, ignore_globs) {
            return;
        }

        if path.is_file() {
            let new_hash = Self::calculate_file_hash(&path);

            if let Some(new_hash) = new_hash {
                let mut file_hashes = file_hashes.lock().await;
                if file_hashes.contains_key(&path) {
                    let old_hash = file_hashes.get(&path).unwrap();
                    if old_hash != &new_hash {
                        file_hashes.insert(path.clone(), new_hash);
                        output_tx.unbounded_send(FileSystemEvent::FileModified(path)).ok();
                    }
                } else {
                    // If the file is newly created, we want to emit a created event
                    file_hashes.insert(path.clone(), new_hash);
                    output_tx.unbounded_send(FileSystemEvent::FileCreated(path)).ok();
                }
            }
        }
    }



    pub async fn start(&self,
        output_tx: UnboundedSender<FileSystemEvent>,
        mut input_rx: UnboundedReceiver<FileSystemUpdateEvent>) {

        // if macos, increase ulimit to 100000000
        if cfg!(target_os = "macos") {
            setrlimit(Resource::NOFILE, 100000000, 100000000).unwrap();
        }

        let watch_path = self.watch_path.clone();
        let file_hashes = self.file_hashes.clone();
        let ignore_globs = self.ignore_globs.clone();
        let mut sym_links = HashMap::new();

        // Spawn the file system watcher in a separate task
        tokio::spawn(async move {
            Self::initialize_file_hashes(&watch_path, &file_hashes, &mut sym_links, &ignore_globs).await;

            // Create a futures channel for notify events
            let (notify_tx, mut notify_rx) = futures::channel::mpsc::unbounded();

            // Create a custom event handler that uses the UnboundedSender
            let event_handler = UnboundedEventHandler {
                sender: notify_tx,
            };

            // Create the watcher with our custom event handler
            // don't canonicalize the path

            let mut watcher = WatcherImpl::new(event_handler, Config::default().with_follow_symlinks(false)).unwrap();

            // Start watching the directory
            watcher.watch(&watch_path, RecursiveMode::Recursive).unwrap();

            // Process both file system events and update events
            loop {
                tokio::select! {
                    // Handle file system events
                    Some(notify_result) = notify_rx.next() => {
                        if let Ok(notify_event) = notify_result {
                            match notify_event {
                                notify::Event {
                                    kind: notify::EventKind::Create(_),
                                    paths,
                                    ..
                                } => {
                                    for path in paths {
                                        Self::handle_file_event(
                                            path,
                                            &file_hashes,
                                            &output_tx,
                                            &ignore_globs,
                                            &mut sym_links,
                                        ).await;
                                    }
                                }
                                notify::Event {
                                    kind: notify::EventKind::Modify(_),
                                    paths,
                                    ..
                                } => {
                                    for path in paths {
                                        Self::handle_file_event(
                                            path,
                                            &file_hashes,
                                            &output_tx,
                                            &ignore_globs,
                                            &mut sym_links,
                                        ).await;
                                    }
                                }
                                notify::Event {
                                    kind: notify::EventKind::Remove(_),
                                    paths,
                                    ..
                                } => {
                                    for path in paths {
                                        let path = if (sym_links.contains_key(&path)) {
                                            // rmeove it
                                            sym_links.remove(&path);
                                            path
                                        } else {
                                            Self::desymlinkify(&path, &sym_links)
                                        };
                                        // Skip if path matches any ignore pattern
                                        if Self::should_ignore(&path, &ignore_globs) {
                                            continue;
                                        }

                                        let mut file_hashes = file_hashes.lock().await;
                                        if file_hashes.remove(&path).is_some() {
                                            output_tx.unbounded_send(FileSystemEvent::FileDeleted(path)).ok();
                                        }
                                    }
                                }
                                _ => {
                                    println!("rust: unexpected event {:?}", notify_event);
                                }
                            }
                        }
                    },
                    // Handle update events
                    Some(event) = input_rx.next() => {
                        match event {
                            FileSystemUpdateEvent::FileCreated(path, content) => {
                                // Skip if path matches any ignore pattern
                                if Self::should_ignore(&path, &ignore_globs) {
                                    continue;
                                }

                                // Write the file content to disk
                                if let Ok(hash_str) = Self::write_file_content(&path, &content) {
                                    let mut file_hashes = file_hashes.lock().await;
                                    file_hashes.insert(path.clone(), hash_str);
                                } else {
                                    println!("rust: failed to write file {:?}", path);
                                }
                            }
                            FileSystemUpdateEvent::FileModified(path, content) => {
                                // Skip if path matches any ignore pattern
                                if Self::should_ignore(&path, &ignore_globs) {
                                    continue;
                                }

                                // Write the file content to disk
                                if let Ok(hash_str) = Self::write_file_content(&path, &content) {
                                    let mut file_hashes = file_hashes.lock().await;
                                    file_hashes.insert(path.clone(), hash_str);
                                } else {
                                    println!("rust: failed to write file {:?}", path);
                                }
                            }
                            FileSystemUpdateEvent::FileDeleted(path) => {
                                // Skip if path matches any ignore pattern
                                if Self::should_ignore(&path, &ignore_globs) {
                                    continue;
                                }

                                // Delete the file from disk
                                if std::fs::remove_file(&path.canonicalize().unwrap()).is_ok() {
                                    // Remove the hash from our tracking
                                    let mut file_hashes = file_hashes.lock().await;
                                    file_hashes.remove(&path);

                                } else {
                                    println!("rust: failed to delete file {:?}", path);
                                }
                            }
                        }
                    },
                    else => break
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
    use std::collections::HashSet;
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use std::path;
    use tempfile::tempdir;
    use tokio::sync::mpsc;
    use std::path::Path;
    use autosurgeon::Doc;

    fn replace_res_prefix(watch_path: &PathBuf, path: &Path) -> PathBuf {
        if path.to_string_lossy().starts_with("res://") {
            return watch_path.join(path.to_string_lossy().replace("res://", ""));
        }
        path.to_path_buf()
    }
    // Helper function to normalize paths for comparison
    fn normalize_path(watch_path: &PathBuf, path: &Path) -> PathBuf {
        // On macOS, /var is a symlink to /private/var, so we need to resolve it
        // if it begins with res://, replace it with the watch_path
        let mut path = replace_res_prefix(watch_path, path);
        if cfg!(target_os = "macos") {
            // Try to canonicalize the path, which resolves symlinks
            if let Ok(canonical) = path.canonicalize() {
                return canonical;
            }
        }
        // If canonicalization fails or we're not on macOS, just return the path as is
        path
    }

    #[tokio::test]
    async fn test_file_system_watcher() {
        // Create a temporary directory for testing
        let dir = tempdir().unwrap();
        // if macos, add /private/ to the start of the path
        let dir_path = dir.path().to_path_buf();

        // Create the file system driver
        let driver = FileSystemDriver::new(dir_path.clone(), vec![]);

        // Create channels for input and output events
        let (output_tx, mut output_rx) = futures::channel::mpsc::unbounded();
        let (input_tx, input_rx) = futures::channel::mpsc::unbounded();

        driver.start(output_tx, input_rx).await;

        // Give the watcher time to initialize
        sleep(Duration::from_millis(2000)).await;

        // Create a test file
        let test_file = dir_path.join("test.txt");
        {
            let mut file = File::create(&test_file).unwrap();
            file.write_all(b"test content").unwrap();
            file.sync_all().unwrap();
        }
        sleep(Duration::from_millis(100)).await;

        // Wait for the create event
        if let Some(event) = output_rx.next().await {
            match event {
                FileSystemEvent::FileCreated(path) => {
                    assert_eq!(path, test_file);
                }
                _ => panic!("Unexpected event"),
            }
        }

        let test_file2 = dir_path.join("test2.txt");
        {
            let mut file = File::create(&test_file2).unwrap();
            file.write_all(b"test content").unwrap();
            file.sync_all().unwrap();
        }
        sleep(Duration::from_millis(100)).await;

        if let Some(event) = output_rx.next().await {
            match event {
                FileSystemEvent::FileCreated(path) => {
                    assert_eq!(path, test_file2);
                }
                _ => panic!("Unexpected event"),
            }
        }

        // Modify the file with different content
        {
            let mut file = File::options().write(true).open(&test_file).unwrap();
            file.write_all(b"modified content").unwrap();
            file.sync_all().unwrap();
        }

        // Wait for the modify event
        if let Some(event) = output_rx.next().await {
            match event {
                FileSystemEvent::FileModified(path) => {
                    assert_eq!(normalize_path(&dir_path, &path), normalize_path(&dir_path, &test_file));
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
        // have it timeout after 100ms
        let timeout = tokio::time::timeout(Duration::from_millis(100), output_rx.next());
        if let Ok(Some(event)) = timeout.await {
            match event {
                FileSystemEvent::FileDeleted(path) => {
                    assert_eq!(normalize_path(&dir_path, &path), normalize_path(&dir_path, &test_file));
                }
                _ => panic!("Unexpected event"),
            }
        } else {
            panic!("No event received");
        }
    }

    #[tokio::test]
    async fn test_ignore_globs() {
        // Create a temporary directory for testing
        let dir = tempdir().unwrap();
        // if macos, add /private/ to the start of the path
        let dir_path = if cfg!(target_os = "macos") {
            let mut path = dir.path().to_path_buf();
            let private = PathBuf::from("/private");
            path = private.join(path);
            path
        } else {
            dir.path().to_path_buf()
        };

        // Create the file system driver with ignore globs
        let driver = FileSystemDriver::new(dir_path.clone(), vec!["*.tmp".to_string()]);

        // Create channels for input and output events
        let (output_tx, mut output_rx) = futures::channel::mpsc::unbounded();
        let (input_tx, input_rx) = futures::channel::mpsc::unbounded();

        driver.start(output_tx, input_rx).await;

        // Give the watcher time to initialize
        sleep(Duration::from_millis(100)).await;


        // Create a test file that should not be ignored
        let test_file = dir_path.join("test.txt");
        let mut file = File::create(&test_file).unwrap();
        file.write_all(b"test content").unwrap();

        sleep(Duration::from_millis(100)).await;


        // Wait for the create event (should only be for the non-ignored file)
        if let Ok(Some(event)) = output_rx.try_next() {
            match event {
                FileSystemEvent::FileCreated(path) => {
                    assert_eq!(normalize_path(&dir_path, &path), normalize_path(&dir_path, &test_file));
                }
                _ => panic!("Unexpected event"),
            }
        }
        // Create a test file that should be ignored
        let ignored_file = dir_path.join("test.tmp");
        let mut file = File::create(&ignored_file).unwrap();
        file.write_all(b"test content").unwrap();

        sleep(Duration::from_millis(100)).await;

        if let Ok(Some(event)) = output_rx.try_next() {
            panic!("Unexpected event {:?}", event);
        }

        // Modify the ignored file (should not trigger an event)
        let mut file = File::options().write(true).open(&ignored_file).unwrap();
        file.write_all(b"modified content").unwrap();

        // Modify the non-ignored file (should trigger an event)
        let mut file = File::options().write(true).open(&test_file).unwrap();
        file.write_all(b"modified content").unwrap();
        // close it
        file.sync_all().unwrap();
        sleep(Duration::from_millis(100)).await;

        // Wait for the modify event (should only be for the non-ignored file)
        if let Ok(Some(event)) = output_rx.try_next() {
            match event {
                FileSystemEvent::FileModified(path) => {
                    assert_eq!(normalize_path(&dir_path, &path), normalize_path(&dir_path, &test_file));
                }
                _ => panic!("Unexpected event"),
            }
        } else {
            panic!("No event received");
        }

        // Delete both files
        std::fs::remove_file(&test_file).unwrap();
        sleep(Duration::from_millis(100)).await;

        // Wait for the delete event (should only be for the non-ignored file)
        if let Ok(Some(event)) = output_rx.try_next() {
            match event {
                FileSystemEvent::FileDeleted(path) => {
                    assert_eq!(normalize_path(&dir_path, &path), normalize_path(&dir_path, &test_file));
                }
                _ => panic!("Unexpected event"),
            }
        } else {
            panic!("No event received");
        }
        std::fs::remove_file(&ignored_file).unwrap();

        // try_next should return None
        assert!(output_rx.try_next().is_err());

    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn test_file_system_update_events() {
        // Create a temporary directory for testing
        let dir = tempdir().unwrap();
        // if macos, add /private/ to the start of the path
        let dir_path = dir.path().to_path_buf();
        let actual_path = dir_path.canonicalize().unwrap();

        // Create the file system driver
        let driver = FileSystemDriver::new(dir_path.clone(), vec![]);

        // Create channels for input and output events
        let (output_tx, mut output_rx) = futures::channel::mpsc::unbounded();
        let (input_tx, input_rx) = futures::channel::mpsc::unbounded();

        driver.start(output_tx, input_rx).await;

        // Give the watcher time to initialize
        sleep(Duration::from_millis(100)).await;
        let test_content = "test content";
        let modified_content = "modified content";
        // Create a file via update event
        let test_file = dir_path.join("test.txt");
        input_tx.unbounded_send(FileSystemUpdateEvent::FileCreated(
            test_file.clone(),
            FileContent::String(test_content.to_string()),
        )).unwrap();
        // sleep
        sleep(Duration::from_millis(100)).await;
        // check that there's no event for the file created
        assert!(output_rx.try_next().is_err());

        // check that the file exists and contains the test_content
        assert!(test_file.exists());
        assert_eq!(std::fs::read_to_string(&test_file).unwrap(), test_content);
        // modify the file
        input_tx.unbounded_send(FileSystemUpdateEvent::FileModified(
            test_file.clone(),
            FileContent::String(modified_content.to_string()),
        )).unwrap();
        sleep(Duration::from_millis(100)).await;

        // check that there's no event for the file modified
        assert!(output_rx.try_next().is_err());
        // check that the file exists and contains the modified_content
        assert!(test_file.exists());
        assert_eq!(std::fs::read_to_string(&test_file).unwrap(), modified_content);
        // delete the file
        input_tx.unbounded_send(FileSystemUpdateEvent::FileDeleted(
            test_file.clone(),
        )).unwrap();
        sleep(Duration::from_millis(100)).await;
        // check that there's no event for the file modified
        assert!(output_rx.try_next().is_err());

        // check that the file does not exist
        assert!(!test_file.exists());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn test_file_system_large_number_of_files() {
        // we want to have at least 1000 files in the directory to test that we actually do raise the ulimit
        let dir = tempdir().unwrap();
        let dir_path = dir.path().to_path_buf();
        let driver = FileSystemDriver::new(dir_path.clone(), vec![]);
        let (output_tx, mut output_rx) = futures::channel::mpsc::unbounded();
        let (input_tx, input_rx) = futures::channel::mpsc::unbounded();
        driver.start(output_tx, input_rx).await;
        // give the watcher time to initialize
        sleep(Duration::from_millis(100)).await;
        // create 1000 files
        let mut test_paths = HashSet::new();
        for i in 0..1000 {
            let test_path = dir_path.join(format!("test_{}.txt", i));
            let mut file = File::create(&test_path).unwrap();
            file.write_all(b"test content").unwrap();
            file.sync_all().unwrap();
            test_paths.insert(test_path);
        }
        // wait for the watcher to process the events
        sleep(Duration::from_millis(100)).await;
        // check that the files exist
        // for i in 0..1000 {
        // 	let file = dir_path.join(format!("test_{}.txt", i));
        // 	assert!(file.exists());
        // }
        // now check to see if we have 1000 events
        let mut found_paths = HashSet::new();
        let mut count = 0;
        let mut timeout = tokio::time::timeout(Duration::from_millis(1000), output_rx.next());
        while let Ok(event) = timeout.await {
            assert!(event.is_some());
            let event = event.unwrap();
            let event_path = if let FileSystemEvent::FileCreated(path) = event {
                path
            } else {
                if let FileSystemEvent::FileModified(path) = event{
                    found_paths.insert(path);
                    timeout = tokio::time::timeout(Duration::from_millis(1000), output_rx.next());
                    continue;
                }
                panic!("Unexpected event type {:?}", event);
            };
            found_paths.insert(event_path);
            timeout = tokio::time::timeout(Duration::from_millis(1000), output_rx.next());
        }
        // assert that the number of found paths is equal to the number of test paths
        assert_eq!(found_paths.len(), test_paths.len());
    }
}
