//! Bridge between the GTK main loop and the async IPC client. The client lives on a
//! small tokio runtime; calls are awaited from GTK futures and events are forwarded
//! through a channel the main loop reads.

use std::future::Future;
use std::sync::Arc;

use linksy_ipc::linksy_core::Event;
use linksy_ipc::{Client, ClientError};

pub struct Bridge {
    rt: tokio::runtime::Runtime,
}

impl Bridge {
    pub fn new() -> Self {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name("linksy-ipc")
            .enable_all()
            .build()
            .expect("tokio runtime");
        Self { rt }
    }

    /// Connect to linksyd. Events arrive on the returned channel until the daemon goes
    /// away, at which point the channel closes.
    pub async fn connect(&self) -> Result<(Arc<Client>, async_channel::Receiver<Event>), String> {
        let handle = self.rt.spawn(async {
            let client = Arc::new(Client::connect().await.map_err(|e| e.to_string())?);
            let (tx, rx) = async_channel::unbounded();
            let mut events = client.events();
            tokio::spawn(async move {
                use tokio::sync::broadcast::error::RecvError;
                loop {
                    match events.recv().await {
                        Ok(e) => {
                            if tx.send(e).await.is_err() {
                                break;
                            }
                        }
                        Err(RecvError::Lagged(_)) => continue,
                        Err(RecvError::Closed) => break,
                    }
                }
            });
            Ok((client, rx))
        });
        handle.await.map_err(|e| e.to_string())?
    }

    /// Run an IPC call on the runtime and await its result from the GTK side.
    pub fn call<T, F, Fut>(&self, client: &Arc<Client>, f: F) -> impl Future<Output = Result<T, String>> + use<T, F, Fut>
    where
        T: Send + 'static,
        F: FnOnce(Arc<Client>) -> Fut + Send + 'static,
        Fut: Future<Output = Result<T, ClientError>> + Send + 'static,
    {
        let client = client.clone();
        let handle = self.rt.spawn(async move { f(client).await.map_err(|e| e.to_string()) });
        async move { handle.await.unwrap_or_else(|e| Err(e.to_string())) }
    }
}
