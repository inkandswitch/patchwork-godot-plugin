use core::str;
use std::path::PathBuf;
use std::sync::Arc;
use futures::{
    channel::mpsc::{UnboundedReceiver, UnboundedSender},
    StreamExt,
};
use rlimit::{setrlimit, Resource};
use tokio::{task::JoinHandle, time::{sleep, Duration}};
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
use std::io;

use crate::{godot_parser::{parse_scene, recognize_scene, GodotScene}, godot_project::FileContent};

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
    FileCreated(PathBuf, FileContent),
    FileModified(PathBuf, FileContent),
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
	output_rx: UnboundedReceiver<FileSystemEvent>,
	input_tx: UnboundedSender<FileSystemUpdateEvent>,
	handle: JoinHandle<()>,
}

impl FileSystemDriver {
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

	fn is_file_binary(path: &PathBuf) -> bool {
		if !path.is_file() {
			return false;
		}

		let mut file = match File::open(path) {
			Ok(file) => file,
			Err(_) => return false,
		};

		// check the first 8000 bytes for a null byte
		let mut buffer = [0; 8000];
        if file.read(&mut buffer).is_err() {
            return false;
        }
		return Self::is_buf_binary(&buffer);
	}

	fn is_buf_binary(buf: &[u8]) -> bool {
		buf.iter().take(8000).filter(|&b| *b == 0).count() > 0
	}

	fn buf_to_file_content(buf: Vec<u8>) -> FileContent {
		// check the first 8000 bytes (or the entire file if it's less than 8000 bytes) for a null byte
		if Self::is_buf_binary(&buf) {
			return FileContent::Binary(buf);
		}
		let str = str::from_utf8(&buf);
		if str.is_err() {
			return FileContent::Binary(buf);
		}
        let string = str.unwrap().to_string();
        // check if the file is a scene or a tres
        if recognize_scene(&string) {
            let scene = parse_scene(&string);
            if scene.is_ok() {
                return FileContent::Scene(scene.unwrap());
            }
        }
        FileContent::String(string)
	}

	// get the buffer and hash of a file
	fn get_buffer_and_hash(path: &PathBuf) -> Result<(Vec<u8>, String), io::Error> {
		if !path.is_file() {
			return Err(io::Error::new(io::ErrorKind::Other, "Not a file"));
		}
		let buf = std::fs::read(path);
		if buf.is_err() {
			return Err(io::Error::new(io::ErrorKind::Other, "Failed to read file"));
		}
		let buf = buf.unwrap();
		let hash = Md5Hasher::hash_slice(&buf);
		let hash_str = format!("{}", hash);
		Ok((buf, hash_str))
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
    ) -> Result<(), notify::Error> {
        // Process symlinks and get the actual path
        let path = if let Ok(metadata) = std::fs::metadata(&path) {
            if metadata.is_symlink() {
                let target = std::fs::read_link(&path).unwrap();
                sym_links.insert(path.clone(), target);
                path.clone()
            } else {
                Self::desymlinkify(&path, sym_links)
            }
        } else {
            Self::desymlinkify(&path, sym_links)
        };

        // Skip if path matches any ignore pattern
        if Self::should_ignore(&path, ignore_globs) {
            return Ok(());
        }

        if path.is_file() {
			let result = Self::get_buffer_and_hash(&path);
			if result.is_err() {
				println!("rust: failed to get file content for {:?}", path);
				return Err(notify::Error::new(notify::ErrorKind::Generic("Failed to get file content".to_string())));
			}
			let (content, new_hash) = result.unwrap();
			let mut file_hashes: tokio::sync::MutexGuard<'_, HashMap<PathBuf, String>> = file_hashes.lock().await;
			if file_hashes.contains_key(&path) {
				let old_hash = file_hashes.get(&path).unwrap();
				if old_hash != &new_hash {
					output_tx.unbounded_send(FileSystemEvent::FileModified(path, Self::buf_to_file_content(content))).ok();
				}
			} else {
				// If the file is newly created, we want to emit a created event
				file_hashes.insert(path.clone(), new_hash);
				output_tx.unbounded_send(FileSystemEvent::FileCreated(path, Self::buf_to_file_content(content))).ok();
			}
        }
		Ok(())
    }

    pub async fn spawn(watch_path: PathBuf, ignore_globs: Vec<String>) -> Self {
        // if macos, increase ulimit to 100000000
        if cfg!(target_os = "macos") {
            setrlimit(Resource::NOFILE, 100000000, 100000000).unwrap();
        }

        let ignore_globs: Vec<Pattern> = ignore_globs
            .into_iter()
            .filter_map(|glob_str| Pattern::new(&glob_str).ok())
            .collect();
		let ig = ignore_globs.clone();
		let fh: Arc<Mutex<HashMap<PathBuf, String>>> = Arc::new(Mutex::new(HashMap::new()));
        let mut syml = HashMap::new();
		let wp: PathBuf = watch_path.clone();
		let (output_tx, output_rx) = futures::channel::mpsc::unbounded();
		let (input_tx, mut input_rx) = futures::channel::mpsc::unbounded();
		let file_hashes = fh.clone();
		let mut sym_links = syml.clone();
        // Spawn the file system watcher in a separate task
        let handle = tokio::spawn(async move {
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
                                        let result = Self::handle_file_event(
                                            path.clone(),
                                            &file_hashes,
                                            &output_tx,
                                            &ignore_globs,
                                            &mut sym_links,
                                        ).await;
                                        if result.is_err() {
                                            println!("rust: failed to handle file event {:?}", result);
                                        }
                                    }
                                }
                                notify::Event {
                                    kind: notify::EventKind::Modify(_),
                                    paths,
                                    ..
                                } => {
                                    for path in paths {
                                        let result = Self::handle_file_event(
                                            path.clone(),
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
		Self {
			watch_path: wp,
			file_hashes: fh,
			ignore_globs: ig,
			output_rx,
			input_tx,
			handle,
		}
    }

	pub async fn new_file(&self, path: PathBuf, content: FileContent) {
		self.input_tx.unbounded_send(FileSystemUpdateEvent::FileCreated(path, content)).ok();
	}

	pub async fn modify_file(&self, path: PathBuf, content: FileContent) {
		self.input_tx.unbounded_send(FileSystemUpdateEvent::FileModified(path, content)).ok();
	}

	pub async fn delete_file(&self, path: PathBuf) {
		self.input_tx.unbounded_send(FileSystemUpdateEvent::FileDeleted(path)).ok();
	}

	pub fn try_next(&mut self) -> Option<FileSystemEvent> {
		let res: Result<Option<FileSystemEvent>, futures::channel::mpsc::TryRecvError> = self.output_rx.try_next();
		if res.is_err() {
			return None;
		}
		res.unwrap()
	}

	pub async fn next(&mut self) -> Option<FileSystemEvent> {
		self.output_rx.next().await
	}

	pub async fn next_timeout(&mut self, timeout: Duration) -> Option<FileSystemEvent> {
		let res = tokio::time::timeout(timeout, self.output_rx.next()).await;
		if res.is_err() {
			return None;
		}
		res.unwrap()
	}

	pub async fn stop(&self) {
		self.handle.abort();
	}
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use std::{future, path};
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
        let mut driver = FileSystemDriver::spawn(dir_path.clone(), vec![]).await;


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
        if let Some(event) = driver.next_timeout(Duration::from_millis(100)).await {
            match event {
                FileSystemEvent::FileCreated(path, content) => {
                    assert_eq!(path, test_file);
                    assert_eq!(content, FileContent::String("test content".to_string()));
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

        if let Some(event) = driver.next_timeout(Duration::from_millis(100)).await {
            match event {
                FileSystemEvent::FileCreated(path, content) => {
                    assert_eq!(path, test_file2);
                    assert_eq!(content, FileContent::String("test content".to_string()));
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
        if let Some(event) = driver.next_timeout(Duration::from_millis(100)).await {
            match event {
                FileSystemEvent::FileModified(path, content) => {
                    assert_eq!(normalize_path(&dir_path, &path), normalize_path(&dir_path, &test_file));
                    assert_eq!(content, FileContent::String("modified content".to_string()));
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
        if let Some(event) = driver.next_timeout(Duration::from_millis(100)).await {
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
        let mut driver = FileSystemDriver::spawn(dir_path.clone(), vec!["*.tmp".to_string()]).await;

        // Give the watcher time to initialize
        sleep(Duration::from_millis(100)).await;


        // Create a test file that should not be ignored
        let test_file = dir_path.join("test.txt");
        let mut file = File::create(&test_file).unwrap();
        file.write_all(b"test content").unwrap();

        sleep(Duration::from_millis(100)).await;


        // Wait for the create event (should only be for the non-ignored file)
        if let Some(event) = driver.try_next() {
            match event {
                FileSystemEvent::FileCreated(path, content) => {
                    assert_eq!(normalize_path(&dir_path, &path), normalize_path(&dir_path, &test_file));
                    assert_eq!(content, FileContent::String("test content".to_string()));
                }
                _ => panic!("Unexpected event"),
            }
        }
        // Create a test file that should be ignored
        let ignored_file = dir_path.join("test.tmp");
        let mut file = File::create(&ignored_file).unwrap();
        file.write_all(b"test content").unwrap();

        sleep(Duration::from_millis(100)).await;

        if let Some(event) = driver.next_timeout(Duration::from_millis(100)).await {
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
        if let Some(event) = driver.next_timeout(Duration::from_millis(100)).await {
            match event {
                FileSystemEvent::FileModified(path, content) => {
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
        if let Some(event) = driver.next_timeout(Duration::from_millis(100)).await {
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
        assert!(driver.try_next().is_none());

    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn test_file_system_update_events() {
        // Create a temporary directory for testing
        let dir = tempdir().unwrap();
        // if macos, add /private/ to the start of the path
        let dir_path = dir.path().to_path_buf();
        let actual_path = dir_path.canonicalize().unwrap();

        // Create the file system driver
        let mut driver = FileSystemDriver::spawn(dir_path.clone(), vec![]).await;

        // Give the watcher time to initialize
        sleep(Duration::from_millis(100)).await;
        let test_content = "test content";
        let modified_content = "modified content";
        // Create a file via update event
        let test_file = dir_path.join("test.txt");
        driver.new_file(test_file.clone(), FileContent::String(test_content.to_string())).await;
        // sleep
        sleep(Duration::from_millis(100)).await;
        // check that there's no event for the file created
        assert!(driver.try_next().is_none());

        // check that the file exists and contains the test_content
        assert!(test_file.exists());
        assert_eq!(std::fs::read_to_string(&test_file).unwrap(), test_content);
        // modify the file
        driver.modify_file(test_file.clone(), FileContent::String(modified_content.to_string())).await;
        sleep(Duration::from_millis(100)).await;

        // check that there's no event for the file modified
        assert!(driver.try_next().is_none());
        // check that the file exists and contains the modified_content
        assert!(test_file.exists());
        assert_eq!(std::fs::read_to_string(&test_file).unwrap(), modified_content);
        // delete the file
        driver.delete_file(test_file.clone()).await;
        sleep(Duration::from_millis(100)).await;
        // check that there's no event for the file modified
        assert!(driver.try_next().is_none());

        // check that the file does not exist
        assert!(!test_file.exists());
    }


    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn test_file_system_large_number_of_files() {
        // we want to have at least 1000 files in the directory to test that we actually do raise the ulimit
        let dir = tempdir().unwrap();
        let dir_path = dir.path().to_path_buf();
        let mut driver = FileSystemDriver::spawn(dir_path.clone(), vec![]).await;
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
        while let Some(event) = driver.next_timeout(Duration::from_millis(1000)).await {
            let event_path = if let FileSystemEvent::FileCreated(path, _) = event {
                path
            } else {
                panic!("Unexpected event type {:?}", event);
            };
            found_paths.insert(event_path);
            count += 1;
        }
        assert_eq!(count, 1000);
        for path in test_paths {
            assert!(found_paths.contains(&path));
        }
    }

    // #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    // // run the test_file_system_large_number_of_files test 100 times
    // async fn test_file_system_large_number_of_files_100() {
    //     for _ in 0..100 {
    //         test_file_system_large_number_of_files().await;
    //     }
    // }
}
