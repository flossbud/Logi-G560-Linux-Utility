//! System tray switch for the G560 Linux Utility lighting service.
//!
//! Implements the freedesktop StatusNotifierItem protocol over D-Bus (via
//! `ksni`), so it works on Wayland compositors with a tray host such as KDE
//! Plasma without any X11 or toolkit dependency. It talks to the running
//! service over its Unix socket using the same newline-delimited JSON
//! protocol as the GUI: left-click toggles screen matching, the menu exposes
//! the remaining switches.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::Command;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use ksni::blocking::{Handle, TrayMethods};
use ksni::menu::{CheckmarkItem, MenuItem, StandardItem, SubMenu};
use ksni::{Icon, ToolTip};
use logig560_api::protocol::{ClientCommand, ClientRequest, RequestResult, ServerMessage};
use logig560_api::{LightingMode, ManualZoneUpdate, RgbColor, ServiceSnapshot, ZoneId};

const SERVICE_UNIT: &str = "logig560-desktop.service";
const RECONNECT_DELAY: Duration = Duration::from_secs(2);

/// Preset static colours offered in the menu, applied to all four zones.
const PRESETS: &[(&str, RgbColor)] = &[
    (
        "Warm orange",
        RgbColor {
            red: 255,
            green: 64,
            blue: 0,
        },
    ),
    (
        "Warm white",
        RgbColor {
            red: 255,
            green: 180,
            blue: 110,
        },
    ),
    (
        "Cyan",
        RgbColor {
            red: 20,
            green: 200,
            blue: 244,
        },
    ),
    (
        "Purple",
        RgbColor {
            red: 140,
            green: 40,
            blue: 255,
        },
    ),
];

/// What the tray knows about the service, refreshed from every snapshot.
#[derive(Clone, Debug, Default)]
struct ServiceState {
    connected: bool,
    lights_enabled: bool,
    screen_matching: bool,
    pending: bool,
    capture_fps: f64,
}

impl ServiceState {
    fn from_snapshot(snapshot: &ServiceSnapshot) -> Self {
        Self {
            connected: true,
            lights_enabled: snapshot.lights_enabled,
            screen_matching: snapshot.mode == LightingMode::ContentAware,
            pending: snapshot.pending,
            capture_fps: snapshot.diagnostics.capture_rate_millihertz as f64 / 1000.0,
        }
    }
}

/// Requests the menu sends to the socket thread.
#[derive(Debug)]
enum Action {
    Send(ClientCommand),
    StartService,
}

struct G560Tray {
    state: ServiceState,
    actions: Sender<Action>,
}

impl G560Tray {
    fn send(&self, command: ClientCommand) {
        let _ = self.actions.send(Action::Send(command));
    }

    fn icon_colour(&self) -> Option<[u8; 3]> {
        if !self.state.connected {
            None
        } else if !self.state.lights_enabled {
            Some([0x55, 0x55, 0x55])
        } else if self.state.screen_matching {
            Some([0xf5, 0x7a, 0x1e])
        } else {
            Some([0x6c, 0x8e, 0xad])
        }
    }

    fn describe(&self) -> String {
        if !self.state.connected {
            return "Service not running".to_owned();
        }
        if !self.state.lights_enabled {
            return "Lights off".to_owned();
        }
        if self.state.screen_matching {
            if self.state.pending {
                "Screen matching: waiting for the monitor picker".to_owned()
            } else {
                format!("Screen matching on, {:.1} fps", self.state.capture_fps)
            }
        } else {
            "Static colour".to_owned()
        }
    }
}

/// Draw a filled disc (or a ring when `fill` is `None`) as an ARGB32 pixmap.
fn disc_icon(size: i32, fill: Option<[u8; 3]>) -> Icon {
    let centre = (size as f64 - 1.0) / 2.0;
    let radius = size as f64 / 2.0 - 1.0;
    let mut data = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let distance = ((x as f64 - centre).powi(2) + (y as f64 - centre).powi(2)).sqrt();
            let coverage = (radius - distance + 0.5).clamp(0.0, 1.0);
            let (alpha, rgb) = match fill {
                Some(rgb) => (coverage, rgb),
                None => {
                    let ring = (coverage - (radius - 2.5 - distance + 0.5).clamp(0.0, 1.0))
                        .clamp(0.0, 1.0);
                    (ring, [0x9a, 0x9a, 0x9a])
                }
            };
            data.extend_from_slice(&[(alpha * 255.0).round() as u8, rgb[0], rgb[1], rgb[2]]);
        }
    }
    Icon {
        width: size,
        height: size,
        data,
    }
}

impl ksni::Tray for G560Tray {
    fn id(&self) -> String {
        env!("CARGO_PKG_NAME").into()
    }

    fn title(&self) -> String {
        "G560 lighting".into()
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        let fill = self.icon_colour();
        vec![
            disc_icon(22, fill),
            disc_icon(32, fill),
            disc_icon(48, fill),
        ]
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            title: "G560 lighting".into(),
            description: self.describe(),
            ..Default::default()
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        if self.state.connected {
            let mode = if self.state.screen_matching {
                LightingMode::Manual
            } else {
                LightingMode::ContentAware
            };
            self.send(ClientCommand::SetMode { mode });
        } else {
            let _ = self.actions.send(Action::StartService);
        }
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let connected = self.state.connected;
        let presets = PRESETS
            .iter()
            .map(|(label, colour)| {
                let colour = *colour;
                StandardItem {
                    label: (*label).into(),
                    enabled: connected,
                    activate: Box::new(move |tray: &mut Self| {
                        tray.send(ClientCommand::SetMode {
                            mode: LightingMode::Manual,
                        });
                        tray.send(ClientCommand::SetManualZones {
                            updates: [
                                ZoneId::LeftRear,
                                ZoneId::LeftFront,
                                ZoneId::RightFront,
                                ZoneId::RightRear,
                            ]
                            .into_iter()
                            .map(|zone| ManualZoneUpdate {
                                zone,
                                color: colour,
                                brightness: 100,
                            })
                            .collect(),
                        });
                    }),
                    ..Default::default()
                }
                .into()
            })
            .collect();
        vec![
            CheckmarkItem {
                label: "Screen matching".into(),
                enabled: connected,
                checked: self.state.screen_matching,
                activate: Box::new(|tray: &mut Self| tray.activate(0, 0)),
                ..Default::default()
            }
            .into(),
            CheckmarkItem {
                label: "Lights on".into(),
                enabled: connected,
                checked: self.state.lights_enabled,
                activate: Box::new(|tray: &mut Self| {
                    tray.send(ClientCommand::SetLightsEnabled {
                        enabled: !tray.state.lights_enabled,
                    });
                }),
                ..Default::default()
            }
            .into(),
            SubMenu {
                label: "Static colour".into(),
                enabled: connected,
                submenu: presets,
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Choose monitor…".into(),
                enabled: connected,
                activate: Box::new(|tray: &mut Self| {
                    tray.send(ClientCommand::ChooseDesktopDisplay);
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Restart capture".into(),
                enabled: connected,
                activate: Box::new(|tray: &mut Self| tray.send(ClientCommand::RestartCapture)),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Start service".into(),
                enabled: !connected,
                activate: Box::new(|tray: &mut Self| {
                    let _ = tray.actions.send(Action::StartService);
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Quit tray".into(),
                icon_name: "application-exit".into(),
                activate: Box::new(|_| std::process::exit(0)),
                ..Default::default()
            }
            .into(),
        ]
    }
}

fn socket_path() -> Result<PathBuf> {
    let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR").context("XDG_RUNTIME_DIR is not set")?;
    Ok(PathBuf::from(runtime_dir).join("logig560.sock"))
}

fn start_service() {
    match Command::new("systemctl")
        .args(["--user", "start", SERVICE_UNIT])
        .status()
    {
        Ok(status) if status.success() => {}
        Ok(status) => eprintln!("systemctl --user start {SERVICE_UNIT} exited with {status}"),
        Err(error) => eprintln!("failed to run systemctl: {error}"),
    }
}

/// Forwards menu actions to the current socket connection (if any).
fn action_loop(actions: Receiver<Action>, writer: Arc<Mutex<Option<UnixStream>>>) {
    let mut next_id = 1u64;
    for action in actions {
        match action {
            Action::StartService => start_service(),
            Action::Send(command) => {
                let request = ClientRequest {
                    id: next_id,
                    command,
                };
                next_id += 1;
                let Ok(mut line) = serde_json::to_vec(&request) else {
                    continue;
                };
                line.push(b'\n');
                let mut guard = writer
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                if let Some(stream) = guard.as_mut()
                    && let Err(error) = stream.write_all(&line)
                {
                    eprintln!("socket write failed: {error}");
                    *guard = None;
                }
            }
        }
    }
}

/// Keeps one connection to the service open, feeding snapshots into the tray.
fn client_loop(path: PathBuf, handle: Handle<G560Tray>, writer: Arc<Mutex<Option<UnixStream>>>) {
    loop {
        if let Ok(stream) = UnixStream::connect(&path) {
            if let Ok(clone) = stream.try_clone() {
                *writer
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(clone);
            }
            for line in BufReader::new(stream).lines() {
                let Ok(line) = line else { break };
                let snapshot = match serde_json::from_str::<ServerMessage>(&line) {
                    Ok(ServerMessage::Ready { snapshot, .. })
                    | Ok(ServerMessage::Event { snapshot }) => Some(snapshot),
                    Ok(ServerMessage::Response {
                        result: RequestResult::Ok { snapshot },
                        ..
                    }) => Some(snapshot),
                    Ok(ServerMessage::Response {
                        result: RequestResult::Err { error },
                        ..
                    }) => {
                        eprintln!("service rejected a request: {error:?}");
                        None
                    }
                    Err(error) => {
                        eprintln!("unparsable service message: {error}");
                        None
                    }
                };
                if let Some(snapshot) = snapshot {
                    let state = ServiceState::from_snapshot(&snapshot);
                    handle.update(move |tray| tray.state = state);
                }
            }
        }
        *writer
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
        handle.update(|tray| tray.state = ServiceState::default());
        thread::sleep(RECONNECT_DELAY);
    }
}

fn main() -> Result<()> {
    let path = socket_path()?;
    let (actions, action_rx) = mpsc::channel();
    let tray = G560Tray {
        state: ServiceState::default(),
        actions,
    };
    let handle = tray
        .spawn()
        .context("failed to register the tray icon on D-Bus")?;
    let writer: Arc<Mutex<Option<UnixStream>>> = Arc::new(Mutex::new(None));
    thread::spawn({
        let writer = writer.clone();
        move || action_loop(action_rx, writer)
    });
    client_loop(path, handle, writer);
    Ok(())
}
