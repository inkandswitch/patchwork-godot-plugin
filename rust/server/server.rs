use automerge_repo::tokio::FsStorage;
use automerge_repo::{ConnDirection, Repo};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::runtime::Handle;
use tracing_subscriber;

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    tracing_subscriber::fmt::init();

    let storage = FsStorage::open("/tmp/automerge-server-data").unwrap();
    let repo = Repo::new(None, Box::new(storage));
    let repo_handle = repo.run();

    let handle = Handle::current();

    let repo_clone = repo_handle.clone();
    handle.spawn(async move {
        let port = std::env::var("PORT").unwrap_or_else(|_| "8080".to_string());
        let addr = format!("0.0.0.0:{}", port);
        let listener = TcpListener::bind(&addr).await.unwrap();

        println!("started server on localhost:{}", port);

        let mut running_handles = tokio::task::JoinSet::new();

        loop {
            tokio::select! {
              finished_task = running_handles.join_next() => {
                let Some(finished_task) = finished_task else {
                  continue;
                };

                match finished_task {
                  Ok(e) => {
                    println!("finished task: {:?}", e);
                  },
                  Err(e) => {
                    if e.is_panic() {
                      std::panic::resume_unwind(e.into_panic());
                    } else {
                      println!("error in task: {:?}", e);
                    }
                  }
                }
              },
            
              new_accept = listener.accept() => {
                match new_accept {
                    Ok((mut socket, addr)) => {
                      println!("client connected");

                      // Read first few bytes to check if it's HTTP
                      let mut buf = [0; 4];
                      match socket.peek(&mut buf).await {
                          Ok(_) => {
                              if buf.starts_with(b"GET ") || buf.starts_with(b"POST") {
                                  // It's an HTTP request, send 200 OK
                                  let response = "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 2\r\n\r\nok";
                                  let _ = socket.write_all(response.as_bytes()).await;
                                  continue;
                              }
                          }
                          Err(e) => {
                              println!("Error peeking socket: {:?}", e);
                              continue;
                          }
                      }

                      // Not HTTP, handle as automerge connection
                      running_handles.spawn({
                          let repo_clone = repo_clone.clone();
                          async move {
                              match repo_clone
                                  .connect_tokio_io(addr, socket, ConnDirection::Incoming)
                                  .await
                              {
                                  Ok(_) => println!("Client connection completed successfully"),
                                  Err(e) => println!("Client connection error: {:?}", e),
                              }
                          }
                      });
                    }
                  Err(e) => println!("couldn't get client: {:?}", e),
                }
              }
            }
        }
    });

    tokio::signal::ctrl_c().await.unwrap();

    repo_handle.stop().unwrap();
}
