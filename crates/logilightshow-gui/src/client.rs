//! Thin async client that keeps a persistent connection to the
//! LogiLightShow lighting service Unix socket. Retries on connect
//! failure, dispatches requests to correlate responses by id, and
//! fans out `Ready`/`Event` snapshots via a broadcast channel.

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result, anyhow};
use logilightshow_api::{
    ServiceSnapshot,
    protocol::{ClientCommand, ClientRequest, RequestResult, ServerMessage},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::UnixStream,
    sync::{Mutex, broadcast, mpsc, oneshot},
    time::sleep,
};
use tracing::{debug, info, warn};

const RECONNECT_INITIAL: Duration = Duration::from_millis(250);
const RECONNECT_MAX: Duration = Duration::from_secs(5);

pub fn default_socket_path() -> Result<PathBuf> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .context("XDG_RUNTIME_DIR is not set; user session runtime directory required")?;
    Ok(runtime.join("logilightshow.sock"))
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ConnectionState {
    Connecting,
    Connected,
    Disconnected { reason: String },
}

#[derive(Clone)]
pub struct ServiceClient {
    inner: Arc<Inner>,
}

struct Inner {
    request_tx: mpsc::Sender<Outbound>,
    snapshot_tx: broadcast::Sender<ServiceSnapshot>,
    state_tx: broadcast::Sender<ConnectionState>,
    next_id: AtomicU64,
    latest_snapshot: Mutex<Option<ServiceSnapshot>>,
    latest_state: Mutex<ConnectionState>,
}

struct Outbound {
    request: ClientRequest,
    response: oneshot::Sender<Result<RequestResult>>,
}

impl ServiceClient {
    pub fn spawn(socket_path: PathBuf) -> Self {
        let (request_tx, request_rx) = mpsc::channel::<Outbound>(32);
        let (snapshot_tx, _) = broadcast::channel::<ServiceSnapshot>(16);
        let (state_tx, _) = broadcast::channel::<ConnectionState>(16);
        let inner = Arc::new(Inner {
            request_tx,
            snapshot_tx: snapshot_tx.clone(),
            state_tx: state_tx.clone(),
            next_id: AtomicU64::new(1),
            latest_snapshot: Mutex::new(None),
            latest_state: Mutex::new(ConnectionState::Connecting),
        });
        let client_inner = inner.clone();
        tokio::spawn(async move {
            supervisor_loop(socket_path, client_inner, request_rx).await;
        });
        Self { inner }
    }

    pub fn subscribe_snapshots(&self) -> broadcast::Receiver<ServiceSnapshot> {
        self.inner.snapshot_tx.subscribe()
    }

    pub fn subscribe_state(&self) -> broadcast::Receiver<ConnectionState> {
        self.inner.state_tx.subscribe()
    }

    pub async fn current_snapshot(&self) -> Option<ServiceSnapshot> {
        self.inner.latest_snapshot.lock().await.clone()
    }

    pub async fn current_state(&self) -> ConnectionState {
        self.inner.latest_state.lock().await.clone()
    }

    pub async fn send(&self, command: ClientCommand) -> Result<RequestResult> {
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        let (response, response_rx) = oneshot::channel();
        self.inner
            .request_tx
            .send(Outbound {
                request: ClientRequest { id, command },
                response,
            })
            .await
            .map_err(|_| anyhow!("service client is not accepting requests"))?;
        response_rx
            .await
            .map_err(|_| anyhow!("client dropped response oneshot"))?
    }
}

async fn supervisor_loop(
    socket_path: PathBuf,
    inner: Arc<Inner>,
    mut request_rx: mpsc::Receiver<Outbound>,
) {
    let mut backoff = RECONNECT_INITIAL;
    loop {
        set_state(&inner, ConnectionState::Connecting).await;
        match UnixStream::connect(&socket_path).await {
            Ok(stream) => {
                info!(socket = %socket_path.display(), "connected to lighting service");
                backoff = RECONNECT_INITIAL;
                set_state(&inner, ConnectionState::Connected).await;
                if let Err(err) = run_connection(stream, &inner, &mut request_rx).await {
                    warn!(?err, "lighting service connection ended");
                    set_state(
                        &inner,
                        ConnectionState::Disconnected {
                            reason: err.to_string(),
                        },
                    )
                    .await;
                }
            }
            Err(err) => {
                debug!(?err, "lighting service unavailable, retrying");
                set_state(
                    &inner,
                    ConnectionState::Disconnected {
                        reason: err.to_string(),
                    },
                )
                .await;
            }
        }
        sleep(backoff).await;
        backoff = (backoff * 2).min(RECONNECT_MAX);
    }
}

async fn set_state(inner: &Arc<Inner>, state: ConnectionState) {
    *inner.latest_state.lock().await = state.clone();
    let _ = inner.state_tx.send(state);
}

async fn run_connection(
    stream: UnixStream,
    inner: &Arc<Inner>,
    request_rx: &mut mpsc::Receiver<Outbound>,
) -> Result<()> {
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half).lines();
    let mut pending: HashMap<u64, oneshot::Sender<Result<RequestResult>>> = HashMap::new();
    loop {
        tokio::select! {
            biased;
            outbound = request_rx.recv() => {
                let Some(outbound) = outbound else {
                    return Err(anyhow!("client request channel closed"));
                };
                let id = outbound.request.id;
                let mut text = serde_json::to_vec(&outbound.request)
                    .context("serialize client request")?;
                text.push(b'\n');
                if let Err(err) = write_half.write_all(&text).await {
                    let _ = outbound
                        .response
                        .send(Err(anyhow!("socket write failed: {err}")));
                    return Err(err.into());
                }
                if let Err(err) = write_half.flush().await {
                    let _ = outbound
                        .response
                        .send(Err(anyhow!("socket flush failed: {err}")));
                    return Err(err.into());
                }
                pending.insert(id, outbound.response);
            }
            line = reader.next_line() => {
                let text = match line {
                    Ok(Some(line)) => line,
                    Ok(None) => return Err(anyhow!("service closed the connection")),
                    Err(err) => return Err(err.into()),
                };
                if text.trim().is_empty() {
                    continue;
                }
                let message: ServerMessage = serde_json::from_str(&text)
                    .with_context(|| format!("parse server message: {text}"))?;
                match message {
                    ServerMessage::Ready { snapshot, .. }
                    | ServerMessage::Event { snapshot } => {
                        *inner.latest_snapshot.lock().await = Some(snapshot.clone());
                        let _ = inner.snapshot_tx.send(snapshot);
                    }
                    ServerMessage::Response { id, result } => {
                        if let Some(response) = pending.remove(&id) {
                            let _ = response.send(Ok(result));
                        } else {
                            warn!(id, "received response for unknown request id");
                        }
                    }
                }
            }
        }
    }
}
