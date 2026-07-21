use tauri::{Emitter, Manager};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

mod client;
mod commands;

use client::{ServiceClient, default_socket_path};

fn main() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("logilightshow_gui=info,warn"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();

    let socket_path = match default_socket_path() {
        Ok(path) => path,
        Err(err) => {
            eprintln!("cannot determine service socket path: {err}");
            std::process::exit(1);
        }
    };
    info!(socket = %socket_path.display(), "starting LogiLightShow GUI");

    tauri::Builder::default()
        .setup(move |app| {
            let client =
                tauri::async_runtime::block_on(async { ServiceClient::spawn(socket_path.clone()) });
            app.manage(client.clone());

            let handle = app.handle().clone();
            let mut snapshots = client.subscribe_snapshots();
            tauri::async_runtime::spawn(async move {
                loop {
                    match snapshots.recv().await {
                        Ok(snapshot) => {
                            if let Err(err) = handle.emit("snapshot-changed", &snapshot) {
                                warn!(?err, "failed to emit snapshot-changed");
                            }
                        }
                        Err(err) => {
                            warn!(?err, "snapshot subscription ended");
                            break;
                        }
                    }
                }
            });

            let mut states = client.subscribe_state();
            let state_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    match states.recv().await {
                        Ok(state) => {
                            if let Err(err) = state_handle.emit("connection-state", &state) {
                                warn!(?err, "failed to emit connection-state");
                            }
                        }
                        Err(err) => {
                            warn!(?err, "connection-state subscription ended");
                            break;
                        }
                    }
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_snapshot,
            commands::set_lights_enabled,
            commands::set_mode,
            commands::set_manual_zones,
            commands::get_connection_state,
            commands::get_cached_snapshot,
        ])
        .run(tauri::generate_context!())
        .expect("failed to launch LogiLightShow GUI");
}
