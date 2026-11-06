//! Bridge between the GTK main loop and the async IPC client. The client lives on a
//! small tokio runtime; calls are awaited from GTK futures and events are forwarded
//! through a channel the main loop reads.

use std::future::Future;
use std::sync::Arc;

use linksy_ipc::linksy_core::Event;
use linksy_ipc::{Client, ClientError, Conversation, FsProgress, HistoryEntry};

/// What the service pushes: live events, history changes, conversations and copies to and
/// from phones.
pub enum Update {
    Event(Box<Event>),
    History(HistoryEntry),
    Inbox(Conversation),
    Files(FsProgress),
    /// The user chose Quit in the tray.
    Quitting,
}

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
    pub async fn connect(&self) -> Result<(Arc<Client>, async_channel::Receiver<Update>), String> {
        let handle = self.rt.spawn(async {
            let client = Arc::new(Client::connect().await.map_err(|e| e.to_string())?);
            let (tx, rx) = async_channel::unbounded();
            let (mut events, mut history, mut inbox, mut files, mut quitting) = (client.events(), client.history_updates(), client.inbox_updates(), client.fs_progress(), client.quitting());
            tokio::spawn(async move {
                use tokio::sync::broadcast::error::RecvError;
                loop {
                    let update = tokio::select! {
                        e = events.recv() => match e {
                            Ok(e) => Update::Event(Box::new(e)),
                            Err(RecvError::Lagged(_)) => continue,
                            Err(RecvError::Closed) => break,
                        },
                        h = history.recv() => match h {
                            Ok(h) => Update::History(h),
                            Err(RecvError::Lagged(_)) => continue,
                            Err(RecvError::Closed) => break,
                        },
                        c = inbox.recv() => match c {
                            Ok(c) => Update::Inbox(c),
                            Err(RecvError::Lagged(_)) => continue,
                            Err(RecvError::Closed) => break,
                        },
                        p = files.recv() => match p {
                            Ok(p) => Update::Files(p),
                            Err(RecvError::Lagged(_)) => continue,
                            Err(RecvError::Closed) => break,
                        },
                        q = quitting.recv() => match q {
                            Ok(()) => Update::Quitting,
                            Err(RecvError::Lagged(_)) => continue,
                            Err(RecvError::Closed) => break,
                        },
                    };
                    if tx.send(update).await.is_err() {
                        break;
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
