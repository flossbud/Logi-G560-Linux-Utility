//! Tauri command handlers. Each is a thin wrapper that translates the
//! JSON payload from the frontend into a typed `ClientCommand` and awaits
//! the service response.

use logilightshow_api::{
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
