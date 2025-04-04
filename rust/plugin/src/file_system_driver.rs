use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::sync::mpsc::Sender;
use tokio::time::{sleep, Duration};
use notify::{Watcher, RecursiveMode, RecommendedWatcher as WatcherImpl, Config};
use std::sync::mpsc::channel;
use std::time::Duration as StdDuration;

#[derive(Debug)]
pub enum FileSystemEvent {
    FileCreated(PathBuf),
    FileModified(PathBuf),
    FileDeleted(PathBuf),
}

pub struct FileSystemDriver {
    tx: Sender<FileSystemEvent>,
    watch_path: PathBuf,
}

impl FileSystemDriver {
    pub fn new(watch_path: PathBuf) -> (Self, mpsc::Receiver<FileSystemEvent>) {
        let (tx, rx) = mpsc::channel(100);
        (Self { tx, watch_path }, rx)
    }

    pub async fn start(&self) {
        let tx = self.tx.clone();
        let watch_path = self.watch_path.clone();

        // Spawn the file system watcher in a separate task
        tokio::spawn(async move {
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
                                    tx.send(FileSystemEvent::FileCreated(path)).await.ok();
                                }
                            }
                            notify::Event {
                                kind: notify::EventKind::Modify(_),
                                paths,
                                ..
                            } => {
                                for path in paths {
                                    tx.send(FileSystemEvent::FileModified(path)).await.ok();
                                }
                            }
                            notify::Event {
                                kind: notify::EventKind::Remove(_),
                                paths,
                                ..
                            } => {
                                for path in paths {
                                    tx.send(FileSystemEvent::FileDeleted(path)).await.ok();
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
        let (driver, mut rx) = FileSystemDriver::new(dir_path.clone());
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

        // Modify the file
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
}
