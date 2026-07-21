use serde::{Deserialize, Serialize};

use crate::{API_VERSION, ApiError, LightingMode, ManualZoneUpdate, ServiceSnapshot};

pub type RequestId = u64;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientRequest {
    pub id: RequestId,
    #[serde(flatten)]
    pub command: ClientCommand,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientCommand {
    GetSnapshot,
    SetLightsEnabled { enabled: bool },
    SetMode { mode: LightingMode },
    SetManualZones { updates: Vec<ManualZoneUpdate> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    Ready {
        api_version: u32,
        snapshot: ServiceSnapshot,
    },
    Response {
        id: RequestId,
        result: RequestResult,
    },
    Event {
        snapshot: ServiceSnapshot,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum RequestResult {
    Ok { snapshot: ServiceSnapshot },
    Err { error: ApiError },
}

impl ServerMessage {
    pub fn ready(snapshot: ServiceSnapshot) -> Self {
        Self::Ready {
            api_version: API_VERSION,
            snapshot,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AppConfig, ControllerModel, ManualZoneUpdate, RgbColor, ZoneId};

    #[test]
    fn client_request_round_trips_through_json() {
        let request = ClientRequest {
            id: 7,
            command: ClientCommand::SetManualZones {
                updates: vec![ManualZoneUpdate {
                    zone: ZoneId::LeftFront,
                    color: RgbColor {
                        red: 255,
                        green: 0,
                        blue: 32,
                    },
                    brightness: 80,
                }],
            },
        };
        let text = serde_json::to_string(&request).unwrap();
        let decoded: ClientRequest = serde_json::from_str(&text).unwrap();
        assert_eq!(decoded.id, 7);
        assert!(matches!(
            decoded.command,
            ClientCommand::SetManualZones { .. }
        ));
    }

    #[test]
    fn server_message_ready_round_trips_through_json() {
        let model = ControllerModel::new(AppConfig::default()).unwrap();
        let msg = ServerMessage::ready(model.snapshot());
        let text = serde_json::to_string(&msg).unwrap();
        let decoded: ServerMessage = serde_json::from_str(&text).unwrap();
        assert!(matches!(decoded, ServerMessage::Ready { .. }));
    }

    #[test]
    fn request_result_round_trips_both_variants() {
        let model = ControllerModel::new(AppConfig::default()).unwrap();
        let ok = RequestResult::Ok {
            snapshot: model.snapshot(),
        };
        let err = RequestResult::Err {
            error: ApiError::ModeConflict,
        };
        for result in [ok, err] {
            let text = serde_json::to_string(&result).unwrap();
            let decoded: RequestResult = serde_json::from_str(&text).unwrap();
            match (&result, &decoded) {
                (RequestResult::Ok { .. }, RequestResult::Ok { .. }) => {}
                (RequestResult::Err { .. }, RequestResult::Err { .. }) => {}
                _ => panic!("variant mismatch after round-trip"),
            }
        }
    }
}
