use core::str;
use std::sync::atomic::Ordering;
use std::{path::PathBuf, sync::atomic::AtomicBool};
use std::sync::Arc;
use futures::{
    channel::mpsc::{UnboundedReceiver, UnboundedSender},
    StreamExt,
};
#[cfg(not(target_os = "windows"))]
use rlimit::{setrlimit, Resource};
use tokio::{task::JoinHandle, time::{sleep, Duration}};
use notify::{Watcher, RecursiveMode, Config, Event, EventHandler};
use notify_debouncer_mini::{new_debouncer_opt, DebouncedEvent, Debouncer};
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
use crate::file_utils::{calculate_file_hash, get_buffer_and_hash, FileContent};

use crate::{godot_parser::{parse_scene, recognize_scene, GodotScene}};

// static const var for debounce time
const DEBOUNCE_TIME: u64 = 100;


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
    FileSaved(PathBuf, FileContent),
    FileDeleted(PathBuf),
	Pause,
	Resume
}

#[derive(Debug, Clone)]
pub struct FileSystemTask {
    watch_path: PathBuf,
    file_hashes: Arc<Mutex<HashMap<PathBuf, String>>>,
    ignore_globs: Vec<Pattern>,
	watcher: Arc<Mutex<Debouncer<WatcherImpl>>>,
	// atomic bool
	paused: Arc<AtomicBool>,
}

#[derive(Debug)]
pub struct FileSystemDriver {
	task: FileSystemTask,
	output_rx: UnboundedReceiver<FileSystemEvent>,
	input_tx: UnboundedSender<FileSystemUpdateEvent>,
	handle: JoinHandle<()>,
}

impl FileSystemTask {
    // Check if a path should be ignored based on glob patterns
    fn should_ignore(&self, path: &PathBuf) -> bool {
        let path_str = path.to_string_lossy();
        self.ignore_globs.iter().any(|pattern| pattern.matches(&path_str))
    }


    fn _initialize_file_hashes(&self, watch_path: &PathBuf, file_hashes: &mut tokio::sync::MutexGuard<'_, HashMap<PathBuf, String>>, sym_links: &mut HashMap<PathBuf, PathBuf>) {
        if let Ok(entries) = std::fs::read_dir(watch_path) {
            for entry in entries.flatten() {
                let path = entry.path();

                // Skip if path matches any ignore pattern
                if self.should_ignore(&path) {
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
                    if let Some(hash) = calculate_file_hash(&path) {
                        file_hashes.insert(path, hash);
                    }
                } else if path.is_dir() {
                    self._initialize_file_hashes(&path, file_hashes, sym_links);
                }
            }
        }
    }

    // Initialize the hash map with existing files
    async fn initialize_file_hashes(&self, sym_links: &mut HashMap<PathBuf, PathBuf>) {
        let mut file_hashes_guard: tokio::sync::MutexGuard<'_, HashMap<PathBuf, String>> = self.file_hashes.lock().await;
		file_hashes_guard.clear();
        self._initialize_file_hashes(&self.watch_path, &mut file_hashes_guard, sym_links);
    }



    fn desymlinkify(path: &PathBuf, sym_links: &HashMap<PathBuf, PathBuf>) -> PathBuf {
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
        &self,
        path: PathBuf,
        sym_links: &mut HashMap<PathBuf, PathBuf>,
    ) -> Result<Option<FileSystemEvent>, notify::Error> {
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
        if self.should_ignore(&path) {
            return Ok(None);
        }
		if !path.exists() {
			// If the file doesn't exist, we want to emit a deleted event
			let mut file_hashes: tokio::sync::MutexGuard<'_, HashMap<PathBuf, String>> = self.file_hashes.lock().await;
			if file_hashes.contains_key(&path) {
				file_hashes.remove(&path);
				return Ok(Some(FileSystemEvent::FileDeleted(path)));
			}
			return Ok(None);
		}

        if path.is_file() {
			let mut result = get_buffer_and_hash(&path);
			if result.is_err() {
				sleep(StdDuration::from_millis(DEBOUNCE_TIME)).await;
				result = get_buffer_and_hash(&path);
			}
			if result.is_err() {
				println!("rust: failed to get file content {:?}", result);
				return Err(notify::Error::new(notify::ErrorKind::Generic("Failed to get file content".to_string())));
			}
			let (content, new_hash) = result.unwrap();
			let mut file_hashes: tokio::sync::MutexGuard<'_, HashMap<PathBuf, String>> = self.file_hashes.lock().await;
			if file_hashes.contains_key(&path) {
				let old_hash = file_hashes.get(&path).unwrap();
				if old_hash != &new_hash {
					return Ok(Some(FileSystemEvent::FileModified(path, FileContent::from_buf(content))));
				}
			} else {
				// If the file is newly created, we want to emit a created event
				file_hashes.insert(path.clone(), new_hash);
				return Ok(Some(FileSystemEvent::FileCreated(path, FileContent::from_buf(content))));
			}
        }
		Ok(None)
    }

	// handle syncs from patchwork
    pub async fn handle_file_update(
        &self,
        path: PathBuf,
		content: FileContent,
    ) -> Result<(), notify::Error> {
		// Skip if path matches any ignore pattern
		if self.should_ignore(&path) {
			return Ok(());
		}

		// Write the file content to disk
		if let Ok(hash_str) = FileContent::write_file_content(&path, &content) {
			let mut file_hashes = self.file_hashes.lock().await;
			file_hashes.insert(path.clone(), hash_str);
		} else {
			return Err(notify::Error::new(notify::ErrorKind::Generic("Failed to write file".to_string())));
		}
		Ok(())
    }

	pub fn handle_file_update_blocking(
        &self,
        path: PathBuf,
		content: FileContent,
    ) -> Result<(), notify::Error> {
		// Skip if path matches any ignore pattern
		if self.should_ignore(&path) {
			return Ok(());
		}

		// Write the file content to disk
		if let Ok(hash_str) = FileContent::write_file_content(&path, &content) {
			let mut file_hashes = self.file_hashes.blocking_lock();
			file_hashes.insert(path.clone(), hash_str);
		} else {
			return Err(notify::Error::new(notify::ErrorKind::Generic("Failed to write file".to_string())));
		}
		Ok(())
    }


    pub async fn handle_delete_update(
        &self,
        path: PathBuf,
    ) -> Result<(), notify::Error> {
        // Skip if path matches any ignore pattern
        if self.should_ignore(&path) {
            return Ok(());
        }

		// Delete the file from disk
		if std::fs::remove_file(&path.canonicalize().unwrap()).is_ok() {
			// Remove the hash from our tracking
			let mut file_hashes = self.file_hashes.lock().await;
			file_hashes.remove(&path);

		} else {
            return Err(notify::Error::new(notify::ErrorKind::Generic("Failed to delete file".to_string())));
        }
        Ok(())
    }

	pub fn handle_delete_update_blocking(
        &self,
        path: PathBuf,
    ) -> Result<(), notify::Error> {
        // Skip if path matches any ignore pattern
		if self.should_ignore(&path) {
			return Ok(());
		}

		// Delete the file from disk
		if std::fs::remove_file(&path.canonicalize().unwrap()).is_ok() {
			// Remove the hash from our tracking
			let mut file_hashes = self.file_hashes.blocking_lock();
			file_hashes.remove(&path);
		} else {
			return Err(notify::Error::new(notify::ErrorKind::Generic("Failed to delete file".to_string())));
		}
		Ok(())
	}

	// Scan for changes in the file system
	async fn _scan_for_additive_changes(
		&self,
		watch_path: &PathBuf,
		sym_links: &mut HashMap<PathBuf, PathBuf>,
	) -> Vec<FileSystemEvent>
	{
		let mut events = Vec::new();
		let entries = std::fs::read_dir(watch_path);
		if entries.is_err() {
			return events;
		}
		let entries = entries.unwrap();
		for entry in entries.flatten() {
			let path = entry.path();
			// Skip if path matches any ignore pattern
			if self.should_ignore(&path) {
				continue;
			}
			if let Ok(metadata) = path.metadata() {
				if metadata.is_symlink() {
					let target = std::fs::read_link(&path).unwrap();
					sym_links.insert(path.clone(), target);
				}
			}

			if path.is_file() {
				let res = self.handle_file_event(path, sym_links).await;
				if let Ok(Some(ret)) = res{
					events.push(ret);
				}
			} else if path.is_dir() {
				// Use Box::pin for the recursive call to avoid infinitely sized future
				let sub_events = Box::pin(self._scan_for_additive_changes(&path, sym_links)).await;
				events.extend(sub_events);
			}
		}
		events
	}

	async fn scan_for_changes(&self, sym_links: &mut HashMap<PathBuf, PathBuf>) -> Vec<FileSystemEvent> {
		let mut events = self._scan_for_additive_changes(&self.watch_path, sym_links).await;
		// check the file_hashes for removed files
		let mut to_remove = Vec::new();
		let mut file_hashes = self.file_hashes.lock().await;
		for (path, _) in file_hashes.iter() {
			if !path.exists() {
				to_remove.push(path.clone());
			}
		}
		for path in to_remove {
			file_hashes.remove(&path);
			events.push(FileSystemEvent::FileDeleted(path));
		}
		events
	}

	async fn main_loop(&self, notify_rx: &mut UnboundedReceiver<Result<Vec<DebouncedEvent>, notify::Error>>, input_rx: &mut UnboundedReceiver<FileSystemUpdateEvent>, output_tx: &UnboundedSender<FileSystemEvent>) {
		let mut sym_links = HashMap::new();
		self.initialize_file_hashes(&mut sym_links).await;
		// Process both file system events and update events
		loop {
			tokio::select! {
				// Handle file system events
				Some(notify_result) = notify_rx.next() => {
					if let Ok(notify_event) = notify_result {
						for event in notify_event {
							let result = self.handle_file_event(event.path, &mut sym_links).await;
							if let Ok(Some(ret)) = result {
								output_tx.unbounded_send(ret).ok();
							}
						}
					}
				},
				// Handle update events
				Some(event) = input_rx.next() => {
					match event {
						FileSystemUpdateEvent::FileSaved(path, content) => {
							let result = self.handle_file_update(path, content).await;
							if result.is_err() {
								println!("rust: failed to handle file update {:?}", result);
							}
						}
						FileSystemUpdateEvent::FileDeleted(path) => {
							let result = self.handle_delete_update(path).await;
							if result.is_err() {
								println!("rust: failed to handle file delete {:?}", result);
							}
						}
						FileSystemUpdateEvent::Pause => {
							self.pause();
							self.stop_watching_path(&self.watch_path).await;
						}
						FileSystemUpdateEvent::Resume => {
							self.start_watching_path(&self.watch_path).await;
							let events = self.scan_for_changes(&mut sym_links).await;
							for event in events {
								output_tx.unbounded_send(event).ok();
							}
							self.resume();
						}
					}
				},
			}
		}
	}

	async fn stop_watching_path(&self, path: &PathBuf) {
		let _ = self.watcher.lock().await.watcher().unwatch(path);
	}

	fn stop_watching_path_blocking(&self, path: &PathBuf) {
		let _ = self.watcher.blocking_lock().watcher().unwatch(path);
	}

	async fn start_watching_path(&self, path: &PathBuf) {
		let _ = self.watcher.lock().await.watcher().watch(path, RecursiveMode::Recursive);
	}

	fn start_watching_path_blocking(&self, path: &PathBuf) {
		let _ = self.watcher.blocking_lock().watcher().watch(path, RecursiveMode::Recursive);
	}

	async fn add_ignore_glob(&mut self, glob: &str) {
		self.ignore_globs.push(Pattern::new(glob).unwrap());
	}
	pub fn is_paused(&self) -> bool {
		self.paused.load(Ordering::Relaxed)
	}


	fn pause(&self) {
		self.paused.store(true, Ordering::Relaxed);
	}

	fn resume(&self) {
		self.paused.store(false, Ordering::Relaxed);
	}

}



impl FileSystemDriver {



    pub fn spawn(watch_path: PathBuf, ignore_globs: Vec<String>) -> Self {
        // if macos, increase ulimit to 100000000
		#[cfg(not(target_os = "windows"))]
		setrlimit(Resource::NOFILE, 100000000, 100000000).unwrap();

        let ignore_globs: Vec<Pattern> = ignore_globs
            .into_iter()
            .filter_map(|glob_str| Pattern::new(&glob_str).ok())
            .collect();
		let (output_tx, output_rx) = futures::channel::mpsc::unbounded();
		let (input_tx, mut input_rx) = futures::channel::mpsc::unbounded();
        // Spawn the file system watcher in a separate task
        // Create a futures channel for notify events

        // Create a custom event handler that uses the UnboundedSender
        // make an arc mutable
		let notify_config = Config::default().with_follow_symlinks(false);
        // let watcher_arc = Arc::new(Mutex::new(WatcherImpl::new(event_handler, notify_config).unwrap()));
        // {
		// 	// Start watching the directory
        //     let mut watcher = watcher_arc.lock().await;
        //     watcher.watch(&watch_path, RecursiveMode::Recursive).unwrap();
        // }
        let (notify_tx, mut notify_rx) = futures::channel::mpsc::unbounded();

		let debouncer_config= notify_debouncer_mini::Config::default().with_timeout(Duration::from_millis(DEBOUNCE_TIME)).with_batch_mode(true).with_notify_config(notify_config);

		let mut debouncer = new_debouncer_opt::<_,WatcherImpl>(debouncer_config, move |event: Result<Vec<notify_debouncer_mini::DebouncedEvent>, notify::Error>| {
			notify_tx.unbounded_send(event).unwrap();
		}).unwrap();
		debouncer.watcher().watch(&watch_path, RecursiveMode::Recursive).unwrap();

		let task: FileSystemTask = FileSystemTask {
			watch_path: watch_path.clone(),
			file_hashes: Arc::new(Mutex::new(HashMap::new())),
			ignore_globs: ignore_globs,
			watcher: Arc::new(Mutex::new(debouncer)),
			paused: Arc::new(AtomicBool::new(false))
		};

		let this_task = task.clone();

        let handle = tokio::spawn(async move {
            this_task.main_loop(&mut notify_rx, &mut input_rx, &output_tx).await;
        });
		Self {
			task,
			output_rx,
			input_tx,
			handle,
		}
    }



	pub fn save_file_async(&self, path: PathBuf, content: FileContent) {
		self.input_tx.unbounded_send(FileSystemUpdateEvent::FileSaved(path, content)).ok();
	}

	pub fn delete_file_async(&self, path: PathBuf) {
		self.input_tx.unbounded_send(FileSystemUpdateEvent::FileDeleted(path)).ok();
	}

	fn get_existing_parent_path(path: &PathBuf) -> Option<PathBuf> {
		let mut parent = Some(path.as_path());
		while parent.is_some() {
			if parent.as_ref().unwrap().exists() {
				break;
			}
			parent = parent.unwrap().parent();
		}
		if parent.is_some() {
			return Some(parent.unwrap().to_path_buf());
		}
		None
	}

	pub async fn save_file(&self, path: PathBuf, content: FileContent) -> Result<(), notify::Error> {
		let parent = Self::get_existing_parent_path(&path);
		if parent.is_some() {
			let parent = parent.unwrap();
			self.task.stop_watching_path(&parent).await;
			let result = self.task.handle_file_update(path.clone(), content).await;
			self.task.start_watching_path(&parent).await;
			return result;
		} else {
			let result = self.task.handle_file_update(path.clone(), content).await;
			return result;
		}
	}

	pub fn save_file_blocking(&self, path: PathBuf, content: FileContent) -> Result<(), notify::Error> {
		let parent = Self::get_existing_parent_path(&path);
		if parent.is_some() {
			let parent = parent.unwrap();
			self.task.stop_watching_path_blocking(&parent);
			let result = self.task.handle_file_update_blocking(path.clone(), content);
			self.task.start_watching_path_blocking(&parent);
			return result;
		} else {
			let result = self.task.handle_file_update_blocking(path.clone(), content);
			return result;
		}
	}

	async fn pause_task(&self) {
		self.input_tx.unbounded_send(FileSystemUpdateEvent::Pause).ok();
		while !self.task.is_paused() {
			sleep(Duration::from_millis(100)).await;
		}
	}

	fn pause_task_blocking(&self) {
		self.input_tx.unbounded_send(FileSystemUpdateEvent::Pause).ok();
		while !self.task.is_paused() {
			sleep(Duration::from_millis(100));
		}
	}

	async fn resume_task(&self) {
		self.input_tx.unbounded_send(FileSystemUpdateEvent::Resume).ok();
		while self.task.is_paused() {
			sleep(Duration::from_millis(100)).await;
		}
	}

	fn resume_task_blocking(&self) {
		self.input_tx.unbounded_send(FileSystemUpdateEvent::Resume).ok();
		while self.task.is_paused() {
			sleep(Duration::from_millis(100));
		}
	}

	pub async fn batch_update(&self, updates: Vec<FileSystemUpdateEvent>) {
		self.pause_task().await;
		{
			let mut file_hashes = self.task.file_hashes.lock().await;
			for update in updates {
				match update {
					FileSystemUpdateEvent::FileSaved(path, content) => {
						if let Ok(hash_str) = FileContent::write_file_content(&path, &content) {
							file_hashes.insert(path.clone(), hash_str);
						} else {
							println!("rust: failed to write file {:?}", path);
						}
					}
					FileSystemUpdateEvent::FileDeleted(path) => {
						file_hashes.remove(&path);
					}
					FileSystemUpdateEvent::Pause => {
						continue;
					}
					FileSystemUpdateEvent::Resume => {
						continue;
					}
				}
			}
		}
		self.resume_task().await;
	}

	pub fn batch_update_blocking(&self, updates: Vec<FileSystemUpdateEvent>) -> Vec<FileSystemEvent> {
		self.pause_task_blocking();
		let mut file_hashes = self.task.file_hashes.blocking_lock();
		let mut events = Vec::new();
		for update in updates {
			match update {
				FileSystemUpdateEvent::FileSaved(path, content) => {
					if let Ok(hash_str) = FileContent::write_file_content(&path, &content) {
						if let Some(old_hash) = file_hashes.get_mut(&path) {
							if old_hash != &hash_str {
								events.push(FileSystemEvent::FileModified(path.clone(), content));
							}
						} else {
							events.push(FileSystemEvent::FileCreated(path.clone(), content));
						}
						file_hashes.insert(path, hash_str);
					} else {
						println!("rust: failed to write file {:?}", path);
					}
				}
				FileSystemUpdateEvent::FileDeleted(path) => {
					if file_hashes.remove(&path).is_some() {
						events.push(FileSystemEvent::FileDeleted(path));
					}
				}
				_ => {
					continue;
				}
			}
		}
		self.resume_task_blocking();
		events
	}

	pub async fn delete_file(&self, path: PathBuf) -> Result<(), notify::Error> {
		if !path.exists() {
			return Err(notify::Error::new(notify::ErrorKind::Generic("File does not exist".to_string())));
		}
		self.task.stop_watching_path(&path).await;
		let result = self.task.handle_delete_update(path.clone()).await;
		return result;
	}

	pub fn delete_file_blocking(&self, path: PathBuf) -> Result<(), notify::Error> {
		if !path.exists() {
			return Err(notify::Error::new(notify::ErrorKind::Generic("File does not exist".to_string())));
		}
		self.task.stop_watching_path_blocking(&path);
		let result = self.task.handle_delete_update_blocking(path.clone());
		return result;
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

	pub fn get_all_files_blocking(&self) -> Vec<(PathBuf, FileContent)> {
		let mut file_hashes = self.task.file_hashes.blocking_lock();
		file_hashes.iter().filter_map(|(path, _hash)| {
			if path.is_file() {
				let content = std::fs::read(path);
				if content.is_ok() {
					return Some((path.clone(), FileContent::from_buf(content.unwrap())));
				}
			}
			None
		}).collect()
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
	use godot::classes::physics_server_3d::G6dofJointAxisFlag;

	const WAIT_TIME: u64 = DEBOUNCE_TIME * 2;

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
        let mut driver = FileSystemDriver::spawn(dir_path.clone(), vec![]);


        // Give the watcher time to initialize
        sleep(Duration::from_millis(2000)).await;

        // Create a test file
        let test_file = dir_path.join("test.txt");
        {
            let mut file = File::create(&test_file).unwrap();
            file.write_all(b"test content").unwrap();
            file.sync_all().unwrap();
        }

        // Wait for the create event
        if let Some(event) = driver.next_timeout(Duration::from_millis(WAIT_TIME)).await {
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

        if let Some(event) = driver.next_timeout(Duration::from_millis(WAIT_TIME)).await {
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
        if let Some(event) = driver.next_timeout(Duration::from_millis(WAIT_TIME)).await {
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
        if let Some(event) = driver.next_timeout(Duration::from_millis(WAIT_TIME)).await {
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
        let mut driver = FileSystemDriver::spawn(dir_path.clone(), vec!["*.tmp".to_string()]);

        // Give the watcher time to initialize
        sleep(Duration::from_millis(100)).await;


        // Create a test file that should not be ignored
        let test_file = dir_path.join("test.txt");
        let mut file = File::create(&test_file).unwrap();
        file.write_all(b"test content").unwrap();



        // Wait for the create event (should only be for the non-ignored file)
        if let Some(event) = driver.next_timeout(Duration::from_millis(WAIT_TIME)).await {
            match event {
                FileSystemEvent::FileCreated(path, content) => {
                    assert_eq!(normalize_path(&dir_path, &path), normalize_path(&dir_path, &test_file));
                    assert_eq!(content, FileContent::String("test content".to_string()));
                }
                _ => panic!("Unexpected event"),
            }
        } else {
			panic!("No event received");
		}
        // Create a test file that should be ignored
        let ignored_file = dir_path.join("test.tmp");
        let mut file = File::create(&ignored_file).unwrap();
        file.write_all(b"test content").unwrap();


        if let Some(event) = driver.next_timeout(Duration::from_millis(WAIT_TIME)).await {
            panic!("Unexpected event {:?}", event);
        }
		{
			// Modify the ignored file (should not trigger an event)
			let mut file = File::options().write(true).open(&ignored_file).unwrap();
			file.write_all(b"modified content").unwrap();
		}
		{
			// Modify the non-ignored file (should trigger an event)
			let mut file = File::options().write(true).open(&test_file).unwrap();
			file.write_all(b"modified content").unwrap();
			// close it
			file.sync_all().unwrap();
		}
		// Wait for the modify event (should only be for the non-ignored file)
        if let Some(event) = driver.next_timeout(Duration::from_millis(WAIT_TIME)).await {
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

        // Wait for the delete event (should only be for the non-ignored file)
        if let Some(event) = driver.next_timeout(Duration::from_millis(WAIT_TIME)).await {
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
        assert!(driver.next_timeout(Duration::from_millis(WAIT_TIME)).await.is_none());

    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn test_file_system_update_events() {
        // Create a temporary directory for testing
        let dir = tempdir().unwrap();
        // if macos, add /private/ to the start of the path
        let dir_path = dir.path().to_path_buf();
        let actual_path = dir_path.canonicalize().unwrap();

        // Create the file system driver
        let mut driver = FileSystemDriver::spawn(dir_path.clone(), vec![]);

        // Give the watcher time to initialize
        sleep(Duration::from_millis(100)).await;
        let test_content = "test content";
        let modified_content = "modified content";
        // Create a file via update event
        let test_file = dir_path.join("test.txt");
        let result = driver.save_file(test_file.clone(), FileContent::String(test_content.to_string())).await;
		assert!(result.is_ok());
        // sleep
        sleep(Duration::from_millis(WAIT_TIME)).await;
        // check that there's no event for the file created
        assert!(driver.try_next().is_none());

        // check that the file exists and contains the test_content
        assert!(test_file.exists());
        assert_eq!(std::fs::read_to_string(&test_file).unwrap(), test_content);
        // modify the file
        let result = driver.save_file(test_file.clone(), FileContent::String(modified_content.to_string())).await;
		assert!(result.is_ok());
        sleep(Duration::from_millis(WAIT_TIME)).await;

        // check that there's no event for the file modified
        assert!(driver.try_next().is_none());
        // check that the file exists and contains the modified_content
        assert!(test_file.exists());
        assert_eq!(std::fs::read_to_string(&test_file).unwrap(), modified_content);
        // delete the file
        let result = driver.delete_file(test_file.clone()).await;
		assert!(result.is_ok());
        sleep(Duration::from_millis(WAIT_TIME)).await;
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
        let mut driver = FileSystemDriver::spawn(dir_path.clone(), vec![]);
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
        while let Some(event) = driver.next_timeout(Duration::from_millis(10000)).await {
            let event_path = if let FileSystemEvent::FileCreated(path, _) = event {
                path
            } else if let FileSystemEvent::FileModified(path, _) = event {
				path
			} else {
				panic!("Unexpected event type {:?}", event);
            };
            found_paths.insert(event_path);
			if found_paths.len() == test_paths.len() {
				break;
			}
        }
        // assert_eq!(count, 1000);
        for path in test_paths {
            assert!(found_paths.contains(&path));
        }
    }

	#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
	async fn test_file_system_batch_update() {
		let dir = tempdir().unwrap();
		let dir_path = dir.path().to_path_buf();
		let mut driver = FileSystemDriver::spawn(dir_path.clone(), vec![]);
		// update 1000 files
		let mut updates = Vec::new();
		for i in 0..1000 {
			let test_path = dir_path.join(format!("test_{}.txt", i));
			updates.push(FileSystemUpdateEvent::FileSaved(test_path, FileContent::String("test content".to_string())));
		}
		driver.batch_update(updates).await;
		// wait for the watcher to process the events
		sleep(Duration::from_millis(100)).await;
		// check that the files exist
		{
			let hashes = driver.task.file_hashes.clone();
			let hash_table = hashes.lock().await;
			for i in 0..1000 {
				let file = dir_path.join(format!("test_{}.txt", i));
				assert!(file.exists());
				assert!(hash_table.contains_key(&file));
			}
		}
		// there should be no events emitted
		if let Some(event) = driver.next_timeout(Duration::from_millis(100)).await {
			assert!(false, "Unexpected event type {:?}", event);
		}
		sleep(Duration::from_millis(1000)).await;
		// write a single file
		let test_path = dir_path.join("test_woop.txt");
		let mut file = File::create(&test_path).unwrap();
		file.write_all(b"test content").unwrap();
		file.sync_all().unwrap();
		sleep(Duration::from_millis(100)).await;
		// check that the file exists
		assert!(test_path.exists());
		// check that we got an event for it
		if let Some(event) = driver.next().await {
			match event {
				FileSystemEvent::FileCreated(path, content) => {
					assert_eq!(path, test_path);
					assert_eq!(content, FileContent::String("test content".to_string()));
				}
				_ => panic!("Unexpected event"),
			}
		} else {
			panic!("No event received");
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
