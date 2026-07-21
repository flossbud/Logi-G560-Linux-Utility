//! Tauri command handlers. Each is a thin wrapper that translates the
//! JSON payload from the frontend into a typed `ClientCommand` and awaits
//! the service response.

use logig560_api::{
    LightingMode, ManualZoneUpdate, ServiceSnapshot,
    protocol::{ClientCommand, RequestResult},
};

use crate::client::{ConnectionState, ServiceClient};

#[derive(serde::Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum CommandOutcome {
    Ok { snapshot: ServiceSnapshot },
    Err { error: String },
}

impl From<RequestResult> for CommandOutcome {
    fn from(result: RequestResult) -> Self {
        match result {
            RequestResult::Ok { snapshot } => CommandOutcome::Ok { snapshot },
            RequestResult::Err { error } => CommandOutcome::Err {
                error: error.to_string(),
            },
        }
    }
}

fn to_outcome(result: anyhow::Result<RequestResult>) -> CommandOutcome {
    match result {
        Ok(result) => result.into(),
        Err(err) => CommandOutcome::Err {
            error: err.to_string(),
        },
    }
}

#[tauri::command]
pub async fn get_snapshot(state: tauri::State<'_, ServiceClient>) -> Result<CommandOutcome, ()> {
    Ok(to_outcome(state.send(ClientCommand::GetSnapshot).await))
}

/// Log a message from the frontend into the Rust tracing subscriber.
/// Handy for user bug reports and diagnosing UI-side issues without
/// needing the WebView devtools open.
#[tauri::command]
pub async fn frontend_log(message: String) -> Result<(), ()> {
    tracing::info!(target: "frontend", "{message}");
    Ok(())
}

#[tauri::command]
pub async fn set_lights_enabled(
    enabled: bool,
    state: tauri::State<'_, ServiceClient>,
) -> Result<CommandOutcome, ()> {
    Ok(to_outcome(
        state
            .send(ClientCommand::SetLightsEnabled { enabled })
            .await,
    ))
}

#[tauri::command]
pub async fn set_mode(
    mode: LightingMode,
    state: tauri::State<'_, ServiceClient>,
) -> Result<CommandOutcome, ()> {
    Ok(to_outcome(
        state.send(ClientCommand::SetMode { mode }).await,
    ))
}

#[tauri::command]
pub async fn set_manual_zones(
    updates: Vec<ManualZoneUpdate>,
    state: tauri::State<'_, ServiceClient>,
) -> Result<CommandOutcome, ()> {
    Ok(to_outcome(
        state.send(ClientCommand::SetManualZones { updates }).await,
    ))
}

#[tauri::command]
pub async fn choose_desktop_display(
    state: tauri::State<'_, ServiceClient>,
) -> Result<CommandOutcome, ()> {
    Ok(to_outcome(
        state.send(ClientCommand::ChooseDesktopDisplay).await,
    ))
}

#[tauri::command]
pub async fn restart_capture(state: tauri::State<'_, ServiceClient>) -> Result<CommandOutcome, ()> {
    Ok(to_outcome(state.send(ClientCommand::RestartCapture).await))
}

#[tauri::command]
pub async fn mark_setup_complete(
    state: tauri::State<'_, ServiceClient>,
) -> Result<CommandOutcome, ()> {
    Ok(to_outcome(
        state.send(ClientCommand::MarkSetupComplete).await,
    ))
}

#[tauri::command]
pub async fn get_connection_state(
    state: tauri::State<'_, ServiceClient>,
) -> Result<ConnectionState, ()> {
    Ok(state.current_state().await)
}

#[tauri::command]
pub async fn get_cached_snapshot(
    state: tauri::State<'_, ServiceClient>,
) -> Result<Option<ServiceSnapshot>, ()> {
    Ok(state.current_snapshot().await)
}

#[tauri::command]
pub async fn check_udev_status() -> Result<crate::setup::UdevStatus, ()> {
    Ok(tokio::task::spawn_blocking(crate::setup::check_udev_status)
        .await
        .unwrap_or_else(|_| crate::setup::UdevStatus {
            rule_present: false,
            rule_path: String::new(),
            device_present: false,
            device_writable: false,
        }))
}

#[tauri::command]
pub async fn install_udev_rule() -> Result<crate::setup::ActionOutcome, ()> {
    Ok(tokio::task::spawn_blocking(crate::setup::install_udev_rule)
        .await
        .unwrap_or_else(|err| crate::setup::ActionOutcome::Err {
            error: format!("blocking task failed: {err}"),
        }))
}

#[tauri::command]
pub async fn check_service_status() -> Result<crate::setup::ServiceStatus, ()> {
    Ok(
        tokio::task::spawn_blocking(crate::setup::check_service_status)
            .await
            .unwrap_or_else(|_| crate::setup::ServiceStatus {
                unit_installed: false,
                unit_path: String::new(),
                enabled: false,
                active: false,
                failed: false,
                last_status: None,
            }),
    )
}

#[tauri::command]
pub async fn install_service_unit() -> Result<crate::setup::ActionOutcome, ()> {
    Ok(
        tokio::task::spawn_blocking(crate::setup::install_service_unit)
            .await
            .unwrap_or_else(|err| crate::setup::ActionOutcome::Err {
                error: format!("blocking task failed: {err}"),
            }),
    )
}

#[tauri::command]
pub async fn service_action(action: String) -> Result<crate::setup::ActionOutcome, ()> {
    Ok(
        tokio::task::spawn_blocking(move || crate::setup::service_action(&action))
            .await
            .unwrap_or_else(|err| crate::setup::ActionOutcome::Err {
                error: format!("blocking task failed: {err}"),
            }),
    )
}

#[tauri::command]
pub async fn version_info() -> Result<crate::setup::VersionInfo, ()> {
    Ok(tokio::task::spawn_blocking(crate::setup::version_info)
        .await
        .unwrap_or_else(|_| crate::setup::version_info()))
}

#[tauri::command]
pub async fn check_gaming_service_status() -> Result<crate::setup::ServiceStatus, ()> {
    Ok(
        tokio::task::spawn_blocking(crate::setup::check_gaming_service_status)
            .await
            .unwrap_or_else(|_| crate::setup::ServiceStatus {
                unit_installed: false,
                unit_path: String::new(),
                enabled: false,
                active: false,
                failed: false,
                last_status: None,
            }),
    )
}

#[tauri::command]
pub async fn install_gaming_service_unit() -> Result<crate::setup::ActionOutcome, ()> {
    Ok(
        tokio::task::spawn_blocking(crate::setup::install_gaming_service_unit)
            .await
            .unwrap_or_else(|err| crate::setup::ActionOutcome::Err {
                error: format!("blocking task failed: {err}"),
            }),
    )
}

#[tauri::command]
pub async fn gaming_service_action(action: String) -> Result<crate::setup::ActionOutcome, ()> {
    Ok(
        tokio::task::spawn_blocking(move || crate::setup::gaming_service_action(&action))
            .await
            .unwrap_or_else(|err| crate::setup::ActionOutcome::Err {
                error: format!("blocking task failed: {err}"),
            }),
    )
}

#[tauri::command]
pub async fn uninstall_service_unit() -> Result<crate::setup::ActionOutcome, ()> {
    Ok(
        tokio::task::spawn_blocking(crate::setup::uninstall_service_unit)
            .await
            .unwrap_or_else(|err| crate::setup::ActionOutcome::Err {
                error: format!("blocking task failed: {err}"),
            }),
    )
}

#[tauri::command]
pub async fn uninstall_gaming_service_unit() -> Result<crate::setup::ActionOutcome, ()> {
    Ok(
        tokio::task::spawn_blocking(crate::setup::uninstall_gaming_service_unit)
            .await
            .unwrap_or_else(|err| crate::setup::ActionOutcome::Err {
                error: format!("blocking task failed: {err}"),
            }),
    )
}

#[tauri::command]
pub async fn relink_launcher() -> Result<crate::setup::ActionOutcome, ()> {
    Ok(tokio::task::spawn_blocking(crate::setup::relink_launcher)
        .await
        .unwrap_or_else(|err| crate::setup::ActionOutcome::Err {
            error: format!("blocking task failed: {err}"),
        }))
}

#[derive(serde::Serialize)]
pub struct LauncherReport {
    pub context: &'static str,
    pub appimage_path: Option<String>,
    pub state: crate::setup::launch::LauncherState,
}

#[tauri::command]
pub async fn verify_launcher_path() -> Result<LauncherReport, ()> {
    Ok(tokio::task::spawn_blocking(|| {
        use crate::setup::launch::{LaunchContext, detect_launch_context, verify_launcher};
        match detect_launch_context() {
            LaunchContext::AppImage { appimage_path } => {
                let state = verify_launcher(&appimage_path)
                    .unwrap_or(crate::setup::launch::LauncherState::Unknown);
                LauncherReport {
                    context: "appimage",
                    appimage_path: Some(appimage_path.to_string_lossy().to_string()),
                    state,
                }
            }
            LaunchContext::DevBuild { .. } => LauncherReport {
                context: "dev",
                appimage_path: None,
                state: crate::setup::launch::LauncherState::Unknown,
            },
        }
    })
    .await
    .unwrap_or(LauncherReport {
        context: "unknown",
        appimage_path: None,
        state: crate::setup::launch::LauncherState::Unknown,
    }))
}
