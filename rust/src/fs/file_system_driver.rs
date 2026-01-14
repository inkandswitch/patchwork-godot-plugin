use core::str;
use std::sync::atomic::Ordering;
use std::{path::PathBuf, sync::atomic::AtomicBool};
use std::sync::Arc;
use futures::Stream;
use futures::{
    channel::mpsc::{UnboundedReceiver, UnboundedSender},
    StreamExt,
};
#[cfg(not(target_os = "windows"))]
use rlimit::{setrlimit, getrlimit, Resource};
use tokio::{task::JoinHandle, time::{sleep, Duration}};
use notify::{RecursiveMode, Config};
use notify_debouncer_mini::{new_debouncer_opt, DebouncedEvent, Debouncer};
use notify::RecommendedWatcher as WatcherImpl;
use tracing::instrument;
use std::time::Duration as StdDuration;
use std::collections::{HashMap, HashSet};
use tokio::sync::Mutex;
use glob::Pattern;
use crate::helpers::utils::ToShortForm;

use super::file_utils::{calculate_file_hash, get_buffer_and_hash, FileContent};
use crate::project::branch_db::{SharedBranchDB, ThreadLocalBranchDB};

#[cfg(test)]
mod tests;

// static const var for debounce time
const DEBOUNCE_TIME: u64 = 100;

#[derive(Debug)]
pub enum FileSystemEvent {
    FileCreated(PathBuf, FileContent),
    FileModified(PathBuf, FileContent),
    FileDeleted(PathBuf),
}

impl ToShortForm for FileSystemEvent {
	fn to_short_form(&self) -> String {
		let content_type = match self {
			FileSystemEvent::FileCreated(_, content) => match content {
				FileContent::Scene(_) => "scene",
				FileContent::String(_) => "text",
				FileContent::Binary(_) => "binary",
				FileContent::Deleted => "deleted",
			},
			FileSystemEvent::FileModified(_, content) => match content {
				FileContent::Scene(_) => "scene",
				FileContent::String(_) => "text",
				FileContent::Binary(_) => "binary",
				FileContent::Deleted => "deleted",
			},
			_ => "deleted",
		};
		match self {
			FileSystemEvent::FileCreated(path, _) => format!("FileCreated({:?}, {})", path, content_type),
			FileSystemEvent::FileModified(path_buf, _) => format!("FileModified({:?}, {})", path_buf, content_type),
			FileSystemEvent::FileDeleted(path_buf) => format!("FileDeleted({:?}, {})", path_buf, content_type),
		}
	}
}

impl ToShortForm for Vec<FileSystemEvent> {
	fn to_short_form(&self) -> String {
		self.iter().map(|e| e.to_short_form()).collect::<Vec<String>>().join(", ")
	}
}


#[derive(Debug)]
pub enum FileSystemUpdateEvent {
    FileSaved(PathBuf, FileContent),
    FileDeleted(PathBuf),
	Pause,
	Resume
}

impl ToShortForm for FileSystemUpdateEvent {
	fn to_short_form(&self) -> String {
		let content_type = match self {
			FileSystemUpdateEvent::FileSaved(_, content) => match content {
				FileContent::Scene(_) => "scene",
				FileContent::String(_) => "text",
				FileContent::Binary(_) => "binary",
				FileContent::Deleted => "deleted",
			},
			FileSystemUpdateEvent::FileDeleted(_) => "deleted",
			FileSystemUpdateEvent::Pause => "<NONE>",
			FileSystemUpdateEvent::Resume => "<NONE>",
		};

		match self {
			FileSystemUpdateEvent::FileSaved(path, _) => format!("FileSaved({:?} {})", path, content_type),
			FileSystemUpdateEvent::FileDeleted(path) => format!("FileDeleted({:?} {})", path, content_type),
			FileSystemUpdateEvent::Pause => "Pause".to_string(),
			FileSystemUpdateEvent::Resume => "Resume".to_string(),
		}
	}
}

impl ToShortForm for Vec<FileSystemUpdateEvent> {
	fn to_short_form(&self) -> String {
		self.iter().map(|e| e.to_short_form()).collect::<Vec<String>>().join(", ")
	}
}

#[derive(Debug)]
pub struct FileSystemTask {
    watch_path: PathBuf,
    file_hashes: Arc<Mutex<HashMap<PathBuf, String>>>,
    ignore_globs: Vec<Pattern>,
	watcher: Arc<Mutex<Debouncer<WatcherImpl>>>,
	// atomic bool
	paused: Arc<AtomicBool>,
	found_ignored_paths: HashSet<PathBuf>,
	// thread-local copy for filesystem thread
	branch_db: ThreadLocalBranchDB,
}

#[derive(Debug)]
pub struct FileSystemDriver {
	task: FileSystemTask,
	output_rx: UnboundedReceiver<FileSystemEvent>,
	input_tx: UnboundedSender<FileSystemUpdateEvent>,
	handle: JoinHandle<()>,
	rt: Option<tokio::runtime::Runtime>,
}

impl FileSystemTask {
    // Check if a path should be ignored based on glob patterns
    fn should_ignore(&self, path: &PathBuf) -> bool {
		// TODO: We should check if it's a symlink or not, but right now it's sufficient to just check if it's outside of the watch path
		// check if it's outside of the watch path
		if path.is_symlink() {
			return true;
		}
		if !path.starts_with(&self.watch_path) {
			return true;
		}
        let path_str = path.to_string_lossy();
        self.ignore_globs.iter().any(|pattern| pattern.matches(&path_str))
    }


    fn _initialize_file_hashes(&self, watch_path: &PathBuf, file_hashes: &mut tokio::sync::MutexGuard<'_, HashMap<PathBuf, String>>, sym_links: &mut HashMap<PathBuf, PathBuf>, found_ignored_paths: &mut HashSet<PathBuf>) {
        if let Ok(entries) = std::fs::read_dir(watch_path) {
            for entry in entries.flatten() {
                let path = entry.path();

                // Skip if path matches any ignore pattern
                if self.should_ignore(&path) {
                    found_ignored_paths.insert(path.clone());
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
                    self._initialize_file_hashes(&path, file_hashes, sym_links, found_ignored_paths);
                }
            }
        }
    }

    // Initialize the hash map with existing files
    async fn initialize_file_hashes(&mut self, sym_links: &mut HashMap<PathBuf, PathBuf>) {
		let mut found_ignored_paths = HashSet::new();
        let mut file_hashes_guard: tokio::sync::MutexGuard<'_, HashMap<PathBuf, String>> = self.file_hashes.lock().await;
		file_hashes_guard.clear();
        self._initialize_file_hashes(&self.watch_path, &mut file_hashes_guard, sym_links, &mut found_ignored_paths);
		self.found_ignored_paths = found_ignored_paths;
    }

    // Handle file creation and modification events
    async fn handle_file_event(
        &self,
        path: PathBuf,
    ) -> Result<Option<FileSystemEvent>, notify::Error> {
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
				tracing::error!("failed to get file content {:?}", result);
				return Err(notify::Error::new(notify::ErrorKind::Generic("Failed to get file content".to_string())));
			}
			let (content, new_hash) = result.unwrap();
			let mut file_hashes: tokio::sync::MutexGuard<'_, HashMap<PathBuf, String>> = self.file_hashes.lock().await;
			if file_hashes.contains_key(&path) {
				let old_hash = file_hashes.get(&path).unwrap();
				if old_hash != &new_hash {
					tracing::trace!("file {:?} changed, hash {} -> {}", path, old_hash, new_hash);
					file_hashes.insert(path.clone(), new_hash);
					return Ok(Some(FileSystemEvent::FileModified(path, FileContent::from_buf(content))));
				}
			} else {
				// If the file is newly created, we want to emit a created event
				tracing::trace!("file {:?} created, hash {}", path, new_hash);
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

			if path.is_file() {
				let res = self.handle_file_event(path).await;
				if let Ok(Some(ret)) = res{
					events.push(ret);
				}
			} else if path.is_dir() {
				// Use Box::pin for the recursive call to avoid infinitely sized future
				let sub_events = Box::pin(self._scan_for_additive_changes(&path)).await;
				events.extend(sub_events);
			}
		}
		events
	}

	async fn scan_for_changes(&self) -> Vec<FileSystemEvent> {
		let mut events = self._scan_for_additive_changes(&self.watch_path).await;
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

	async fn process_notify_events(&mut self, notify_event: Vec<DebouncedEvent>, output_tx: &UnboundedSender<FileSystemEvent>) {
		for event in notify_event {
			if self.found_ignored_paths.contains(&event.path) {
				continue;
			}
			if self.should_ignore(&event.path) {
				self.found_ignored_paths.insert(event.path);
				continue;
			}
			let result = self.handle_file_event(event.path.clone()).await;
			if let Ok(Some(ret)) = result {
				output_tx.unbounded_send(ret).ok();
			}
		}
	}


	async fn main_loop(&mut self, notify_rx: &mut UnboundedReceiver<Result<Vec<DebouncedEvent>, notify::Error>>, input_rx: &mut UnboundedReceiver<FileSystemUpdateEvent>, output_tx: &UnboundedSender<FileSystemEvent>) {
		let mut sym_links = HashMap::new();
		self.initialize_file_hashes(&mut sym_links).await;
		self.stop_watching_paths(&self.found_ignored_paths).await;
		// Process both file system events and update events
		loop {
			// TODO: actually sync the stuff from the branch db into the file system
			if self.branch_db.is_stale().await {
				let _pull_result = self.branch_db.pull().await;
			}
			
			tokio::select! {
				// Handle file system events
				Some(notify_result) = notify_rx.next() => {
					if let Ok(notify_event) = notify_result {
						self.process_notify_events(notify_event, output_tx).await;
					}
				},
				// Handle update events
				Some(event) = input_rx.next() => {
					match event {
						FileSystemUpdateEvent::FileSaved(path, content) => {
							let result = self.handle_file_update(path, content).await;
							if result.is_err() {
								tracing::error!("failed to handle file update {:?}", result);
							}
						}
						FileSystemUpdateEvent::FileDeleted(path) => {
							let result = self.handle_delete_update(path).await;
							if result.is_err() {
								tracing::error!("failed to handle file delete {:?}", result);
							}
						}
						FileSystemUpdateEvent::Pause => {
							self.stop_watching_path(&self.watch_path).await;
							self.pause();
						}
						FileSystemUpdateEvent::Resume => {
							self.start_watching_path(&self.watch_path).await;
							self.stop_watching_paths(&self.found_ignored_paths).await;
							self.resume();
							// let events = self.scan_for_changes(&mut sym_links).await;
							// for event in events {
							// 	output_tx.unbounded_send(event).ok();
							// }
						}
					}
				},
			}
		}
	}

	async fn stop_watching_paths(&self, paths: &HashSet<PathBuf>) {
		let mut watcher = self.watcher.lock().await;
		for path in paths.iter() {
			let _ret = watcher.watcher().unwatch(path);
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

const MAX_OPEN_FILES: u64 = 100000000;


impl FileSystemDriver {
	fn increase_ulimit() {
		#[cfg(not(target_os = "windows"))]
		{
			let mut new_soft_limit = MAX_OPEN_FILES;
			let mut new_hard_limit = MAX_OPEN_FILES;
			let previous_result = getrlimit(Resource::NOFILE);
			if let Err(e) = previous_result {
				tracing::error!("failed to get ulimit {:?}", e);
			} else if let Ok((soft_limit, hard_limit)) = previous_result {
				tracing::debug!("soft ulimit {:?}", soft_limit);
				tracing::debug!("hard ulimit {:?}", hard_limit);
				if hard_limit > MAX_OPEN_FILES {
					new_hard_limit = hard_limit;
				}
				if soft_limit > MAX_OPEN_FILES {
					new_soft_limit = soft_limit;
				}
			}

			if let Err(e) = setrlimit(Resource::NOFILE, new_soft_limit, new_hard_limit) {
				tracing::error!("failed to set ulimit {:?}", e);
			}
			let result = getrlimit(Resource::NOFILE);
			if let Err(e) = result {
				tracing::error!("failed to set ulimit {:?}", e);
			} else if let Ok((soft_limit, hard_limit)) = result {
				if soft_limit < MAX_OPEN_FILES || hard_limit < MAX_OPEN_FILES {
					tracing::error!("failed to set ulimit; soft ulimit {:?}, hard ulimit {:?}", soft_limit, hard_limit);
				}
			}
		}
	}

	fn spawn_with_runtime(watch_path: PathBuf, ignore_globs: Vec<String>, rt: Option<tokio::runtime::Runtime>, shared_branch_db: SharedBranchDB) -> Self {
		// if macos, increase ulimit to 100000000
		Self::increase_ulimit();
		let (output_tx, output_rx) = futures::channel::mpsc::unbounded();
		let (input_tx, mut input_rx) = futures::channel::mpsc::unbounded();
		// Spawn the file system watcher in a separate task
		let notify_config = Config::default().with_follow_symlinks(false).with_ignore_globs(ignore_globs.clone());
		let ignore_globs: Vec<Pattern> = ignore_globs
			.into_iter()
			.filter_map(|glob_str| Pattern::new(&glob_str).ok())
			.collect();
		let (notify_tx, mut notify_rx) = futures::channel::mpsc::unbounded();

		let debouncer_config = notify_debouncer_mini::Config::default().with_timeout(Duration::from_millis(DEBOUNCE_TIME)).with_batch_mode(true).with_notify_config(notify_config);

		let mut debouncer = new_debouncer_opt::<_, WatcherImpl>(debouncer_config, move |event: Result<Vec<notify_debouncer_mini::DebouncedEvent>, notify::Error>| {
			notify_tx.unbounded_send(event).unwrap();
		}).unwrap();
		debouncer.watcher().watch(&watch_path, RecursiveMode::Recursive).unwrap();

		let rt_handle = if rt.is_some() {
			rt.as_ref().unwrap().handle().clone()
		} else {
			tokio::runtime::Handle::current()
		};
		
		let shared_branch_db_clone = shared_branch_db.clone();
		let watch_path_clone = watch_path.clone();
		let ignore_globs_clone = ignore_globs.clone();
		
		let handle = rt_handle.spawn(async move {
			let mut task = FileSystemTask {
				watch_path: watch_path_clone,
				file_hashes: Arc::new(Mutex::new(HashMap::new())),
				ignore_globs: ignore_globs_clone,
				watcher: Arc::new(Mutex::new(debouncer)),
				paused: Arc::new(AtomicBool::new(false)),
				found_ignored_paths: HashSet::new(),
				branch_db: ThreadLocalBranchDB::new(shared_branch_db_clone).await,
			};
			
			task.main_loop(&mut notify_rx, &mut input_rx, &output_tx).await;
		});
		
		// placeholder task for the driver
		// only used for blocking operations that don't need the actual task state
		let task = FileSystemTask {
			watch_path: watch_path.clone(),
			file_hashes: Arc::new(Mutex::new(HashMap::new())),
			ignore_globs: ignore_globs,
			watcher: Arc::new(Mutex::new(new_debouncer_opt::<_, WatcherImpl>(
				notify_debouncer_mini::Config::default(),
				|_| {}
			).unwrap())),
			paused: Arc::new(AtomicBool::new(false)),
			found_ignored_paths: HashSet::new(),
			branch_db: futures::executor::block_on(ThreadLocalBranchDB::new(shared_branch_db)),
		};
		Self {
			task,
			output_rx,
			input_tx,
			handle,
			rt: rt
		}
	}

	pub fn spawn(watch_path: PathBuf, ignore_globs: Vec<String>, shared_branch_db: SharedBranchDB) -> Self {
		let rt = tokio::runtime::Builder::new_multi_thread()
			.worker_threads(1)
			.thread_name("FileSystemDriver: watcher thread")
			.enable_all()
			.build()
			.unwrap();
		Self::spawn_with_runtime(watch_path, ignore_globs, Some(rt), shared_branch_db)
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
			std::thread::sleep(Duration::from_millis(100));
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
			std::thread::sleep(Duration::from_millis(100));
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
							tracing::error!("failed to write file {:?}", path);
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

	#[instrument(skip_all, level = tracing::Level::INFO)]
	pub fn batch_update_blocking(&self, updates: Vec<FileSystemUpdateEvent>) -> Vec<FileSystemEvent> {
		tracing::debug!("# of updates: {:?}", updates.len());
		tracing::trace!("updates: [{}]", updates.to_short_form());
		self.pause_task_blocking();
		tracing::trace!("batch_update_blocking after pause");
		let mut events: Vec<FileSystemEvent> = Vec::new();
		{
			let mut file_hashes = self.task.file_hashes.blocking_lock();
			for update in updates {
				match update {
					FileSystemUpdateEvent::FileSaved(path, content) => {
						let new_hash_str = content.to_hash();
						let mut modified = false;
						let mut created = false;
						if let Some(old_hash) = file_hashes.get(&path) {
							if old_hash != &new_hash_str {
								modified = true;
							}
						} else {
							created = true;
						}
						if modified || created {
							if let Ok(hash_str) = FileContent::write_file_content(&path, &content) {
								if new_hash_str != hash_str {
									tracing::error!("THIS SHOULD NOT HAPPEN: file {:?} previous calced hash {:?} != written hash {:?}", path, new_hash_str, hash_str);
								}
								if modified {
									tracing::trace!("file {:?} changed, hash {} -> {}", path, file_hashes.get(&path).unwrap(), new_hash_str);
									events.push(FileSystemEvent::FileModified(path.clone(), content));
								} else {
									tracing::trace!("file {:?} created, hash {}", path, new_hash_str);
									events.push(FileSystemEvent::FileCreated(path.clone(), content));
								}
								file_hashes.insert(path, hash_str);
							} else {
								tracing::error!("failed to write file {:?}", path);
							}
						} else {
							tracing::debug!("file {:?} already exists with same hash {:?}", path, new_hash_str);
						}
					}
					FileSystemUpdateEvent::FileDeleted(path) => {
						let _ = std::fs::remove_file(&path);
						if file_hashes.remove(&path).is_some() {
							events.push(FileSystemEvent::FileDeleted(path));
						}
					}
					_ => {
						continue;
					}
				}
			}
		}
		tracing::trace!("batch_update_blocking done, before resume");
		self.resume_task_blocking();
		tracing::debug!("batch_update_blocking done, updated files: {:?}", events.len());
		tracing::trace!("events: [{}]", events.to_short_form());
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

	pub fn has_events_pending(&self) -> bool {
		self.output_rx.size_hint().0 > 0
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

	pub fn stop(&self) {
		self.handle.abort();
	}

	pub fn get_all_files_blocking(&self) -> Vec<(PathBuf, FileContent)> {
		let file_hashes = self.task.file_hashes.blocking_lock();
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
