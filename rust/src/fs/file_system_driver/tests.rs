// use std::collections::HashSet;
// use std::fs::File;
// use std::io::Write;
// use std::path::{Path, PathBuf};
// use std::time::Duration;
// use crate::fs::file_system_driver::{DEBOUNCE_TIME, FileSystemDriver, FileSystemEvent, FileSystemUpdateEvent};
// use crate::fs::file_utils::FileContent;
// use tempfile::tempdir;
// use tokio::{time::{sleep}};
// use crate::helpers::utils::ToShortForm;

// const WAIT_TIME: u64 = DEBOUNCE_TIME * 2;

// fn replace_res_prefix(watch_path: &PathBuf, path: &Path) -> PathBuf {
// 	if path.to_string_lossy().starts_with("res://") {
// 		return watch_path.join(path.to_string_lossy().replace("res://", ""));
// 	}
// 	path.to_path_buf()
// }
// // Helper function to normalize paths for comparison
// fn normalize_path(watch_path: &PathBuf, path: &Path) -> PathBuf {
// 	// On macOS, /var is a symlink to /private/var, so we need to resolve it
// 	// if it begins with res://, replace it with the watch_path
// 	let path = replace_res_prefix(watch_path, path);
// 	if cfg!(target_os = "macos") {
// 		// Try to canonicalize the path, which resolves symlinks
// 		if let Ok(canonical) = path.canonicalize() {
// 			return canonical;
// 		}
// 	}
// 	// If canonicalization fails or we're not on macOS, just return the path as is
// 	path
// }

// #[tokio::test]
// async fn test_file_system_watcher() {
// 	// Create a temporary directory for testing
// 	let dir = tempdir().unwrap();
// 	// if macos, add /private/ to the start of the path
// 	let dir_path = dir.path().to_path_buf();

// 	// Create the file system driver
// 	let mut driver = FileSystemDriver::spawn_with_runtime(dir_path.clone(), vec!["*.tmp".to_string()], None);


// 	// Give the watcher time to initialize
// 	sleep(Duration::from_millis(2000)).await;

// 	// Create a test file
// 	let test_file = dir_path.join("test.txt");
// 	{
// 		let mut file = File::create(&test_file).unwrap();
// 		file.write_all(b"test content").unwrap();
// 		file.sync_all().unwrap();
// 	}

// 	// Wait for the create event
// 	if let Some(event) = driver.next_timeout(Duration::from_millis(WAIT_TIME)).await {
// 		match event {
// 			FileSystemEvent::FileCreated(path, content) => {
// 				assert_eq!(path, test_file);
// 				assert_eq!(content, FileContent::String("test content".to_string()));
// 			}
// 			_ => panic!("Unexpected event"),
// 		}
// 	}

// 	let test_file2 = dir_path.join("test2.txt");
// 	{
// 		let mut file = File::create(&test_file2).unwrap();
// 		file.write_all(b"test content").unwrap();
// 		file.sync_all().unwrap();
// 	}

// 	if let Some(event) = driver.next_timeout(Duration::from_millis(WAIT_TIME)).await {
// 		match event {
// 			FileSystemEvent::FileCreated(path, content) => {
// 				assert_eq!(path, test_file2);
// 				assert_eq!(content, FileContent::String("test content".to_string()));
// 			}
// 			_ => panic!("Unexpected event"),
// 		}
// 	}

// 	// Modify the file with different content
// 	{
// 		let mut file = File::options().write(true).open(&test_file).unwrap();
// 		file.write_all(b"modified content").unwrap();
// 		file.sync_all().unwrap();
// 	}

// 	// Wait for the modify event
// 	if let Some(event) = driver.next_timeout(Duration::from_millis(WAIT_TIME)).await {
// 		match event {
// 			FileSystemEvent::FileModified(path, content) => {
// 				assert_eq!(normalize_path(&dir_path, &path), normalize_path(&dir_path, &test_file));
// 				assert_eq!(content, FileContent::String("modified content".to_string()));
// 			}
// 			_ => panic!("Unexpected event"),
// 		}
// 	}

// 	// Modify the file with the same content (should not trigger an event)
// 	let mut file = File::options().write(true).open(&test_file).unwrap();
// 	file.write_all(b"modified content").unwrap();

// 	// Delete the file
// 	std::fs::remove_file(&test_file).unwrap();

// 	// Wait for the delete event
// 	// have it timeout after 100ms
// 	if let Some(event) = driver.next_timeout(Duration::from_millis(WAIT_TIME)).await {
// 		match event {
// 			FileSystemEvent::FileDeleted(path) => {
// 				assert_eq!(normalize_path(&dir_path, &path), normalize_path(&dir_path, &test_file));
// 			}
// 			_ => panic!("Unexpected event"),
// 		}
// 	} else {
// 		panic!("No event received");
// 	}
// }

// #[tokio::test]
// async fn test_ignore_globs() {
// 	// Create a temporary directory for testing
// 	let dir = tempdir().unwrap();
// 	// if macos, add /private/ to the start of the path
// 	let dir_path = if cfg!(target_os = "macos") {
// 		let mut path = dir.path().to_path_buf();
// 		let private = PathBuf::from("/private");
// 		path = private.join(path);
// 		path
// 	} else {
// 		dir.path().to_path_buf()
// 	};

// 	// Create the file system driver with ignore globs
// 	let mut driver = FileSystemDriver::spawn_with_runtime(dir_path.clone(), vec!["*.tmp".to_string()], None);

// 	// Give the watcher time to initialize
// 	sleep(Duration::from_millis(100)).await;


// 	// Create a test file that should not be ignored
// 	let test_file = dir_path.join("test.txt");
// 	let mut file = File::create(&test_file).unwrap();
// 	file.write_all(b"test content").unwrap();



// 	// Wait for the create event (should only be for the non-ignored file)
// 	if let Some(event) = driver.next_timeout(Duration::from_millis(WAIT_TIME)).await {
// 		match event {
// 			FileSystemEvent::FileCreated(path, content) => {
// 				assert_eq!(normalize_path(&dir_path, &path), normalize_path(&dir_path, &test_file));
// 				assert_eq!(content, FileContent::String("test content".to_string()));
// 			}
// 			_ => panic!("Unexpected event"),
// 		}
// 	} else {
// 		panic!("No event received");
// 	}
// 	// Create a test file that should be ignored
// 	let ignored_file = dir_path.join("test.tmp");
// 	let mut file = File::create(&ignored_file).unwrap();
// 	file.write_all(b"test content").unwrap();


// 	if let Some(event) = driver.next_timeout(Duration::from_millis(WAIT_TIME)).await {
// 		panic!("Unexpected event {:?}", event.to_short_form());
// 	}
// 	{
// 		// Modify the ignored file (should not trigger an event)
// 		let mut file = File::options().write(true).open(&ignored_file).unwrap();
// 		file.write_all(b"modified content").unwrap();
// 	}
// 	{
// 		// Modify the non-ignored file (should trigger an event)
// 		let mut file = File::options().write(true).open(&test_file).unwrap();
// 		file.write_all(b"modified content").unwrap();
// 		// close it
// 		file.sync_all().unwrap();
// 	}
// 	// Wait for the modify event (should only be for the non-ignored file)
// 	if let Some(event) = driver.next_timeout(Duration::from_millis(WAIT_TIME)).await {
// 		match event {
// 			FileSystemEvent::FileModified(path, _) => {
// 				assert_eq!(normalize_path(&dir_path, &path), normalize_path(&dir_path, &test_file));
// 			}
// 			_ => panic!("Unexpected event"),
// 		}
// 	} else {
// 		panic!("No event received");
// 	}

// 	// Delete both files
// 	std::fs::remove_file(&test_file).unwrap();

// 	// Wait for the delete event (should only be for the non-ignored file)
// 	if let Some(event) = driver.next_timeout(Duration::from_millis(WAIT_TIME)).await {
// 		match event {
// 			FileSystemEvent::FileDeleted(path) => {
// 				assert_eq!(normalize_path(&dir_path, &path), normalize_path(&dir_path, &test_file));
// 			}
// 			_ => panic!("Unexpected event"),
// 		}
// 	} else {
// 		panic!("No event received");
// 	}
// 	std::fs::remove_file(&ignored_file).unwrap();

// 	// try_next should return None
// 	assert!(driver.next_timeout(Duration::from_millis(WAIT_TIME)).await.is_none());

// }

// #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
// async fn test_file_system_update_events() {
// 	// Create a temporary directory for testing
// 	let dir = tempdir().unwrap();
// 	// if macos, add /private/ to the start of the path
// 	let dir_path = dir.path().to_path_buf();

// 	// Create the file system driver
// 	let mut driver = FileSystemDriver::spawn_with_runtime(dir_path.clone(), vec![], None);

// 	// Give the watcher time to initialize
// 	sleep(Duration::from_millis(100)).await;
// 	let test_content = "test content";
// 	let modified_content = "modified content";
// 	// Create a file via update event
// 	let test_file = dir_path.join("test.txt");
// 	let result = driver.save_file(test_file.clone(), FileContent::String(test_content.to_string())).await;
// 	assert!(result.is_ok());
// 	// sleep
// 	sleep(Duration::from_millis(WAIT_TIME)).await;
// 	// check that there's no event for the file created
// 	assert!(driver.try_next().is_none());

// 	// check that the file exists and contains the test_content
// 	assert!(test_file.exists());
// 	assert_eq!(std::fs::read_to_string(&test_file).unwrap(), test_content);
// 	// modify the file
// 	let result = driver.save_file(test_file.clone(), FileContent::String(modified_content.to_string())).await;
// 	assert!(result.is_ok());
// 	sleep(Duration::from_millis(WAIT_TIME)).await;

// 	// check that there's no event for the file modified
// 	assert!(driver.try_next().is_none());
// 	// check that the file exists and contains the modified_content
// 	assert!(test_file.exists());
// 	assert_eq!(std::fs::read_to_string(&test_file).unwrap(), modified_content);
// 	// delete the file
// 	let result = driver.delete_file(test_file.clone()).await;
// 	assert!(result.is_ok());
// 	sleep(Duration::from_millis(WAIT_TIME)).await;
// 	// check that there's no event for the file modified
// 	assert!(driver.try_next().is_none());

// 	// check that the file does not exist
// 	assert!(!test_file.exists());
// }


// #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
// async fn test_file_system_large_number_of_files() {
// 	// we want to have at least 1000 files in the directory to test that we actually do raise the ulimit
// 	let dir = tempdir().unwrap();
// 	let dir_path = dir.path().to_path_buf();
// 	let mut driver = FileSystemDriver::spawn_with_runtime(dir_path.clone(), vec!["*.tmp".to_string()], None);
// 	// give the watcher time to initialize
// 	sleep(Duration::from_millis(100)).await;
// 	// create 1000 files
// 	let mut test_paths = HashSet::new();
// 	for i in 0..1000 {
// 		let test_path = dir_path.join(format!("test_{}.txt", i));
// 		let mut file = File::create(&test_path).unwrap();
// 		file.write_all(b"test content").unwrap();
// 		file.sync_all().unwrap();
// 		test_paths.insert(test_path);
// 	}
// 	// wait for the watcher to process the events
// 	sleep(Duration::from_millis(100)).await;
// 	// check that the files exist
// 	// for i in 0..1000 {
// 	// 	let file = dir_path.join(format!("test_{}.txt", i));
// 	// 	assert!(file.exists());
// 	// }
// 	// now check to see if we have 1000 events
// 	let mut found_paths = HashSet::new();
// 	while let Some(event) = driver.next_timeout(Duration::from_millis(10000)).await {
// 		let event_path = if let FileSystemEvent::FileCreated(path, _) = event {
// 			path
// 		} else if let FileSystemEvent::FileModified(path, _) = event {
// 			path
// 		} else {
// 			panic!("Unexpected event type {:?}", event.to_short_form());
// 		};
// 		found_paths.insert(event_path);
// 		if found_paths.len() == test_paths.len() {
// 			break;
// 		}
// 	}
// 	// assert_eq!(count, 1000);
// 	for path in test_paths {
// 		assert!(found_paths.contains(&path));
// 	}
// }

// #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
// async fn test_file_system_batch_update() {
// 	let dir = tempdir().unwrap();
// 	let dir_path = dir.path().to_path_buf();
// 	let mut driver = FileSystemDriver::spawn_with_runtime(dir_path.clone(), vec![], None);
// 	// update 1000 files
// 	let mut updates = Vec::new();
// 	for i in 0..1000 {
// 		let test_path = dir_path.join(format!("test_{}.txt", i));
// 		updates.push(FileSystemUpdateEvent::FileSaved(test_path, FileContent::String("test content".to_string())));
// 	}
// 	driver.batch_update(updates).await;
// 	// wait for the watcher to process the events
// 	sleep(Duration::from_millis(100)).await;
// 	// check that the files exist
// 	{
// 		let hashes = driver.task.file_hashes.clone();
// 		let hash_table = hashes.lock().await;
// 		for i in 0..1000 {
// 			let file = dir_path.join(format!("test_{}.txt", i));
// 			assert!(file.exists());
// 			assert!(hash_table.contains_key(&file));
// 		}
// 	}
// 	// there should be no events emitted
// 	if let Some(event) = driver.next_timeout(Duration::from_millis(100)).await {
// 		assert!(false, "Unexpected event type {:?}", event.to_short_form());
// 	}
// 	sleep(Duration::from_millis(1000)).await;
// 	// write a single file
// 	let test_path = dir_path.join("test_woop.txt");
// 	let mut file = File::create(&test_path).unwrap();
// 	file.write_all(b"test content").unwrap();
// 	file.sync_all().unwrap();
// 	sleep(Duration::from_millis(100)).await;
// 	// check that the file exists
// 	assert!(test_path.exists());
// 	// check that we got an event for it
// 	if let Some(event) = driver.next().await {
// 		match event {
// 			FileSystemEvent::FileCreated(path, content) => {
// 				assert_eq!(path, test_path);
// 				assert_eq!(content, FileContent::String("test content".to_string()));
// 			}
// 			_ => panic!("Unexpected event"),
// 		}
// 	} else {
// 		panic!("No event received");
// 	}

// }
// // test blocking file system update
// #[test]
// fn test_file_system_blocking_update() {
// 	// set the default tokio runtime to multi_thread
// 	let dir = tempdir().unwrap();
// 	let dir_path = dir.path().to_path_buf();
// 	// spawn with the default runtime
// 	let mut driver = FileSystemDriver::spawn(dir_path.clone(), vec![]);
// 	// update 1000 files
// 	let mut updates = Vec::new();
// 	for i in 0..1000 {
// 		let test_path = dir_path.join(format!("test_{}.txt", i));
// 		updates.push(FileSystemUpdateEvent::FileSaved(test_path, FileContent::String("test content".to_string())));
// 	}
// 	driver.batch_update_blocking(updates);
// 	// wait for the watcher to process the events
// 	// check that the files exist
// 	{
// 		let hashes = driver.task.file_hashes.clone();
// 		let hash_table = hashes.blocking_lock();
// 		for i in 0..1000 {
// 			let file = dir_path.join(format!("test_{}.txt", i));
// 			assert!(file.exists());
// 			assert!(hash_table.contains_key(&file));
// 		}
// 	}
// 	// there should be no events emitted
// 	if let Some(event) = driver.try_next() {
// 		assert!(false, "Unexpected event type {:?}", event.to_short_form());
// 	}
// 	std::thread::sleep(Duration::from_millis(1000));
// 	// write a single file
// 	let test_path = dir_path.join("test_woop.txt");
// 	let mut file = File::create(&test_path).unwrap();
// 	file.write_all(b"test content").unwrap();
// 	file.sync_all().unwrap();
// 	std::thread::sleep(Duration::from_millis(1000));
// 	// check that the file exists
// 	assert!(test_path.exists());
// 	// check that we got an event for it
// 	if let Some(event) = driver.try_next() {
// 		match event {
// 			FileSystemEvent::FileCreated(path, content) => {
// 				assert_eq!(path, test_path);
// 				assert_eq!(content, FileContent::String("test content".to_string()));
// 			}
// 			_ => panic!("Unexpected event"),
// 		}
// 	} else {
// 		panic!("No event received");
// 	}
// }

// // #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
// // // run the test_file_system_large_number_of_files test 100 times
// // async fn test_file_system_large_number_of_files_100() {
// //     for _ in 0..100 {
// //         test_file_system_large_number_of_files().await;
// //     }
// // }
