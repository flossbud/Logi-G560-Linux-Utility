use std::{
    fs, io,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{UnixListener, UnixStream, unix::OwnedWriteHalf},
    sync::{mpsc, oneshot, watch},
};
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use crate::ServiceSnapshot;

use super::protocol::{ClientRequest, RequestResult, ServerMessage};

pub struct IpcRequest {
    pub request: ClientRequest,
    pub response: oneshot::Sender<RequestResult>,
}

pub fn default_socket_path() -> Result<PathBuf> {
    let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .context("XDG_RUNTIME_DIR is not set; user session runtime directory required")?;
    Ok(runtime_dir.join("logig560.sock"))
}

pub fn bind_listener(path: &Path) -> Result<UnixListener> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create socket directory {}", parent.display()))?;
    }
    match fs::remove_file(path) {
        Ok(()) => debug!(path = %path.display(), "removed stale service socket"),
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => {
            return Err(err).with_context(|| format!("remove stale socket {}", path.display()));
        }
    }
    let listener = UnixListener::bind(path)
        .with_context(|| format!("bind service socket {}", path.display()))?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .with_context(|| format!("restrict socket permissions on {}", path.display()))?;
    Ok(listener)
}

pub async fn run_accept_loop(
    listener: UnixListener,
    request_tx: mpsc::Sender<IpcRequest>,
    snapshot_rx: watch::Receiver<ServiceSnapshot>,
    shutdown: CancellationToken,
) {
    loop {
        tokio::select! {
            biased;
            _ = shutdown.cancelled() => break,
            accept = listener.accept() => match accept {
                Ok((stream, _addr)) => {
                    let request_tx = request_tx.clone();
                    let snapshot_rx = snapshot_rx.clone();
                    let shutdown = shutdown.clone();
                    tokio::spawn(handle_client(stream, request_tx, snapshot_rx, shutdown));
                }
                Err(err) => {
                    warn!(?err, "service socket accept failed");
                    if shutdown.is_cancelled() {
                        break;
                    }
                }
            }
        }
    }
    debug!("service accept loop exited");
}

async fn handle_client(
    stream: UnixStream,
    request_tx: mpsc::Sender<IpcRequest>,
    mut snapshot_rx: watch::Receiver<ServiceSnapshot>,
    shutdown: CancellationToken,
) {
    let (read_half, write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half).lines();
    let mut writer = write_half;
    let initial = snapshot_rx.borrow_and_update().clone();
    if let Err(err) = send_message(&mut writer, &ServerMessage::ready(initial)).await {
        debug!(?err, "failed to send ready to new client");
        return;
    }
    loop {
        tokio::select! {
            biased;
            _ = shutdown.cancelled() => break,
            changed = snapshot_rx.changed() => {
                if changed.is_err() {
                    break;
                }
                let snapshot = snapshot_rx.borrow_and_update().clone();
                if let Err(err) = send_message(&mut writer, &ServerMessage::Event { snapshot }).await {
                    debug!(?err, "client dropped during snapshot broadcast");
                    break;
                }
            }
            line = reader.next_line() => match line {
                Ok(Some(text)) if text.trim().is_empty() => continue,
                Ok(Some(text)) => {
                    let request: ClientRequest = match serde_json::from_str(&text) {
                        Ok(request) => request,
                        Err(err) => {
                            warn!(?err, text = %text, "malformed client request");
                            continue;
                        }
                    };
                    let id = request.id;
                    let (response, response_rx) = oneshot::channel();
                    if request_tx
                        .send(IpcRequest { request, response })
                        .await
                        .is_err()
                    {
                        debug!("service main dropped request channel");
                        break;
                    }
                    let result = match response_rx.await {
                        Ok(result) => result,
                        Err(_) => {
                            debug!("service main dropped response oneshot");
                            break;
                        }
                    };
                    if let Err(err) = send_message(&mut writer, &ServerMessage::Response { id, result }).await {
                        debug!(?err, "client dropped during response");
                        break;
                    }
                }
                Ok(None) => break,
                Err(err) => {
                    debug!(?err, "client read failed");
                    break;
                }
            }
        }
    }
    debug!("client task exited");
    let _ = writer.shutdown().await;
}

async fn send_message(writer: &mut OwnedWriteHalf, message: &ServerMessage) -> io::Result<()> {
    let mut text = serde_json::to_vec(message)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    text.push(b'\n');
    writer.write_all(&text).await?;
    writer.flush().await
}
