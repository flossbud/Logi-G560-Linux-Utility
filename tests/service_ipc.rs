//! End-to-end tests for the service Unix-socket IPC layer.
//!
//! The runtime is not exercised here because it requires real USB hardware.
//! Instead we run the accept loop against a fake request handler and prove that
//! the wire protocol round-trips correctly.

use std::time::Duration;

use logig560::{
    AppConfig, ControllerModel, LightingMode, ManualZoneUpdate, RgbColor, ZoneId,
    service::{
        ClientCommand, ClientRequest, RequestResult, ServerMessage,
        ipc::{IpcRequest, bind_listener, run_accept_loop},
    },
};
use tempfile::tempdir;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::UnixStream,
    sync::{mpsc, watch},
    time::timeout,
};
use tokio_util::sync::CancellationToken;

async fn read_message(reader: &mut BufReader<tokio::net::unix::OwnedReadHalf>) -> ServerMessage {
    let mut line = String::new();
    timeout(Duration::from_secs(2), reader.read_line(&mut line))
        .await
        .expect("timed out waiting for server message")
        .expect("read failed");
    assert!(!line.is_empty(), "connection closed unexpectedly");
    serde_json::from_str(line.trim_end())
        .unwrap_or_else(|err| panic!("failed to parse server message {line:?}: {err}"))
}

async fn send_request(writer: &mut tokio::net::unix::OwnedWriteHalf, request: &ClientRequest) {
    let mut text = serde_json::to_vec(request).unwrap();
    text.push(b'\n');
    writer.write_all(&text).await.unwrap();
    writer.flush().await.unwrap();
}

#[tokio::test]
async fn ready_message_lands_on_connect_and_manual_update_round_trips() {
    let dir = tempdir().unwrap();
    let socket = dir.path().join("logig560.sock");
    let listener = bind_listener(&socket).unwrap();
    let model = ControllerModel::new(AppConfig::default()).unwrap();
    let (snapshot_tx, snapshot_rx) = watch::channel(model.snapshot());
    let (request_tx, mut request_rx) = mpsc::channel::<IpcRequest>(16);
    let shutdown = CancellationToken::new();

    let accept_shutdown = shutdown.clone();
    let accept = tokio::spawn(run_accept_loop(
        listener,
        request_tx,
        snapshot_rx,
        accept_shutdown,
    ));

    // Fake service main: mutate the model based on incoming requests and echo the snapshot back.
    let mut model_handler = model.clone();
    let handler = tokio::spawn(async move {
        while let Some(request) = request_rx.recv().await {
            let result = match request.request.command {
                ClientCommand::GetSnapshot => RequestResult::Ok {
                    snapshot: model_handler.snapshot(),
                },
                ClientCommand::SetLightsEnabled { enabled } => {
                    model_handler.set_lights_enabled(enabled);
                    let _ = snapshot_tx.send(model_handler.snapshot());
                    RequestResult::Ok {
                        snapshot: model_handler.snapshot(),
                    }
                }
                ClientCommand::SetMode { mode } => {
                    model_handler.set_mode(mode);
                    let _ = snapshot_tx.send(model_handler.snapshot());
                    RequestResult::Ok {
                        snapshot: model_handler.snapshot(),
                    }
                }
                ClientCommand::SetManualZones { updates } => {
                    match model_handler.apply_manual_updates(&updates) {
                        Ok(_) => {
                            let _ = snapshot_tx.send(model_handler.snapshot());
                            RequestResult::Ok {
                                snapshot: model_handler.snapshot(),
                            }
                        }
                        Err(error) => RequestResult::Err { error },
                    }
                }
                ClientCommand::ChooseDesktopDisplay
                | ClientCommand::RestartCapture
                | ClientCommand::MarkSetupComplete => RequestResult::Ok {
                    snapshot: model_handler.snapshot(),
                },
            };
            let _ = request.response.send(result);
        }
    });

    let stream = UnixStream::connect(&socket).await.unwrap();
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);

    match read_message(&mut reader).await {
        ServerMessage::Ready { api_version, .. } => {
            assert_eq!(api_version, logig560::API_VERSION);
        }
        other => panic!("expected Ready as first message, got {other:?}"),
    }

    let request = ClientRequest {
        id: 42,
        command: ClientCommand::SetManualZones {
            updates: vec![ManualZoneUpdate {
                zone: ZoneId::LeftFront,
                color: RgbColor {
                    red: 255,
                    green: 0,
                    blue: 0,
                },
                brightness: 100,
            }],
        },
    };
    send_request(&mut write_half, &request).await;

    // The service pushes a snapshot event before the tagged response because it
    // broadcasts as soon as the model advances. Accept either ordering.
    let mut saw_response = false;
    let mut saw_event = false;
    for _ in 0..3 {
        if saw_response && saw_event {
            break;
        }
        match read_message(&mut reader).await {
            ServerMessage::Response { id, result } => {
                assert_eq!(id, 42);
                assert!(matches!(result, RequestResult::Ok { .. }));
                saw_response = true;
            }
            ServerMessage::Event { snapshot } => {
                assert!(
                    snapshot
                        .manual_zones
                        .iter()
                        .any(|z| z.zone == ZoneId::LeftFront && z.color.red == 255)
                );
                saw_event = true;
            }
            other => panic!("unexpected message: {other:?}"),
        }
    }
    assert!(saw_response, "no tagged response received");
    assert!(saw_event, "no broadcast snapshot event received");

    shutdown.cancel();
    let _ = accept.await;
    handler.abort();
}

#[tokio::test]
async fn invalid_manual_update_returns_api_error() {
    let dir = tempdir().unwrap();
    let socket = dir.path().join("logig560.sock");
    let listener = bind_listener(&socket).unwrap();
    let mut model = ControllerModel::new(AppConfig {
        mode: LightingMode::ContentAware,
        ..AppConfig::default()
    })
    .unwrap();
    let (_snapshot_tx, snapshot_rx) = watch::channel(model.snapshot());
    let (request_tx, mut request_rx) = mpsc::channel::<IpcRequest>(16);
    let shutdown = CancellationToken::new();

    let accept = tokio::spawn(run_accept_loop(
        listener,
        request_tx,
        snapshot_rx,
        shutdown.clone(),
    ));

    let handler = tokio::spawn(async move {
        while let Some(request) = request_rx.recv().await {
            let result = match request.request.command {
                ClientCommand::SetManualZones { updates } => {
                    match model.apply_manual_updates(&updates) {
                        Ok(_) => RequestResult::Ok {
                            snapshot: model.snapshot(),
                        },
                        Err(error) => RequestResult::Err { error },
                    }
                }
                ClientCommand::GetSnapshot
                | ClientCommand::SetLightsEnabled { .. }
                | ClientCommand::SetMode { .. }
                | ClientCommand::ChooseDesktopDisplay
                | ClientCommand::RestartCapture
                | ClientCommand::MarkSetupComplete => RequestResult::Ok {
                    snapshot: model.snapshot(),
                },
            };
            let _ = request.response.send(result);
        }
    });

    let stream = UnixStream::connect(&socket).await.unwrap();
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);
    // Drain Ready.
    let _ = read_message(&mut reader).await;

    let request = ClientRequest {
        id: 7,
        command: ClientCommand::SetManualZones {
            updates: vec![ManualZoneUpdate {
                zone: ZoneId::LeftFront,
                color: RgbColor {
                    red: 0,
                    green: 0,
                    blue: 0,
                },
                brightness: 50,
            }],
        },
    };
    send_request(&mut write_half, &request).await;
    match read_message(&mut reader).await {
        ServerMessage::Response {
            id,
            result: RequestResult::Err { error },
        } => {
            assert_eq!(id, 7);
            assert_eq!(error, logig560::ApiError::ModeConflict);
        }
        other => panic!("expected typed error response, got {other:?}"),
    }

    shutdown.cancel();
    let _ = accept.await;
    handler.abort();
}
