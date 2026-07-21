//! Setup & Service page helpers. These commands touch the local system
//! (systemd user session, udev rules) but never talk to the lighting
//! service socket. They are safe to invoke while the service is running.

pub mod launch;

use std::{
    env, fs,
    io::Write,
    os::unix::fs::OpenOptionsExt,
    path::PathBuf,
    process::{Command, Stdio},
};

use anyhow::{Context, Result, anyhow};
use serde::Serialize;

const UDEV_RULE_NAME: &str = "70-g560.rules";
const UDEV_RULE_CONTENT: &str = include_str!("../../../contrib/70-g560.rules");
const UDEV_RULE_INSTALL_PATH: &str = "/etc/udev/rules.d/70-g560.rules";
const SERVICE_UNIT_NAME: &str = "logig560-desktop.service";

#[derive(Debug, Serialize)]
pub struct UdevStatus {
    pub rule_present: bool,
    pub rule_path: String,
    pub device_present: bool,
    pub device_writable: bool,
}

#[derive(Debug, Serialize)]
pub struct ServiceStatus {
    pub unit_installed: bool,
    pub unit_path: String,
    pub enabled: bool,
    pub active: bool,
    pub failed: bool,
    pub last_status: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ActionOutcome {
    Ok { message: String },
    Err { error: String },
}

impl ActionOutcome {
    fn ok(message: impl Into<String>) -> Self {
        Self::Ok {
            message: message.into(),
        }
    }
    fn err(error: impl std::fmt::Display) -> Self {
        Self::Err {
            error: error.to_string(),
        }
    }
}

pub fn check_udev_status() -> UdevStatus {
    let rule_present = std::path::Path::new(UDEV_RULE_INSTALL_PATH).is_file();
    let (device_present, device_writable) = find_g560_device_status();
    UdevStatus {
        rule_present,
        rule_path: UDEV_RULE_INSTALL_PATH.to_string(),
        device_present,
        device_writable,
    }
}

fn find_g560_device_status() -> (bool, bool) {
    let Ok(entries) = fs::read_dir("/dev/bus/usb") else {
        return (false, false);
    };
    for bus in entries.flatten() {
        let Ok(children) = fs::read_dir(bus.path()) else {
            continue;
        };
        for device in children.flatten() {
            let path = device.path();
            if let Ok(mut file) = fs::OpenOptions::new().read(true).open(&path) {
                let mut buf = [0u8; 18];
                use std::io::Read;
                if file.read(&mut buf).is_ok() {
                    // USB device descriptor bytes 8..10 are idVendor (little-endian).
                    if buf.len() >= 12
                        && buf[8] == 0x6d
                        && buf[9] == 0x04
                        && buf[10] == 0x78
                        && buf[11] == 0x0a
                    {
                        let writable = fs::OpenOptions::new().write(true).open(&path).is_ok();
                        return (true, writable);
                    }
                }
            }
        }
    }
    (false, false)
}

pub fn install_udev_rule() -> ActionOutcome {
    let temp_dir = env::temp_dir().join("logig560-udev");
    if let Err(err) = fs::create_dir_all(&temp_dir) {
        return ActionOutcome::err(anyhow!("create temp dir: {err}"));
    }
    let temp_rule = temp_dir.join(UDEV_RULE_NAME);
    if let Err(err) = write_private(&temp_rule, UDEV_RULE_CONTENT.as_bytes()) {
        return ActionOutcome::err(err);
    }
    let script = format!(
        "install -o root -g root -m 0644 {src} {dst} && \
         udevadm control --reload-rules && \
         udevadm trigger --action=add --subsystem-match=usb \
             --attr-match=idVendor=046d --attr-match=idProduct=0a78",
        src = shell_quote(temp_rule.to_string_lossy().as_ref()),
        dst = shell_quote(UDEV_RULE_INSTALL_PATH),
    );
    match Command::new("pkexec")
        .arg("sh")
        .arg("-c")
        .arg(&script)
        .stdin(Stdio::null())
        .status()
    {
        Ok(status) if status.success() => {
            ActionOutcome::ok("udev rule installed; reconnect your G560 if it was plugged in")
        }
        Ok(status) => ActionOutcome::err(anyhow!("pkexec exited with {status}")),
        Err(err) => ActionOutcome::err(anyhow!("failed to launch pkexec: {err}")),
    }
}

pub fn service_unit_path() -> Result<PathBuf> {
    let base = directories::BaseDirs::new().context("no user directories available")?;
    Ok(base
        .config_dir()
        .join("systemd/user")
        .join(SERVICE_UNIT_NAME))
}

pub fn check_service_status() -> ServiceStatus {
    let unit_path = service_unit_path()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    let unit_installed = service_unit_path().map(|p| p.is_file()).unwrap_or(false);
    let enabled = systemctl_check(&["is-enabled", SERVICE_UNIT_NAME]);
    let active = systemctl_check(&["is-active", SERVICE_UNIT_NAME]);
    let failed = systemctl_check(&["is-failed", SERVICE_UNIT_NAME]);
    let last_status = systemctl_status_line();
    ServiceStatus {
        unit_installed,
        unit_path,
        enabled,
        active,
        failed,
        last_status,
    }
}

fn systemctl_check(args: &[&str]) -> bool {
    Command::new("systemctl")
        .arg("--user")
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn systemctl_status_line() -> Option<String> {
    let output = Command::new("systemctl")
        .arg("--user")
        .args([
            "show",
            "--property=ActiveState,SubState,Result",
            SERVICE_UNIT_NAME,
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub fn install_service_unit() -> ActionOutcome {
    use crate::setup::launch::{
        LaunchContext, detect_launch_context, ensure_launcher, home_bin_dir, render_desktop_unit,
    };

    let ctx = detect_launch_context();
    let exec_target = match &ctx {
        LaunchContext::AppImage { appimage_path } => {
            let bin_dir = match home_bin_dir() {
                Ok(p) => p,
                Err(err) => return ActionOutcome::err(err),
            };
            match ensure_launcher(&bin_dir, appimage_path) {
                Ok(path) => path,
                Err(err) => return ActionOutcome::err(err),
            }
        }
        LaunchContext::DevBuild { cli_binary } => {
            if !cli_binary.is_file() {
                return ActionOutcome::err(anyhow!(
                    "service binary not found at {}; build it with `cargo build --release`",
                    cli_binary.display()
                ));
            }
            cli_binary.clone()
        }
    };

    let unit_path = match service_unit_path() {
        Ok(path) => path,
        Err(err) => return ActionOutcome::err(err),
    };
    if let Some(parent) = unit_path.parent()
        && let Err(err) = fs::create_dir_all(parent)
    {
        return ActionOutcome::err(anyhow!("create {}: {err}", parent.display()));
    }
    let contents = render_desktop_unit(&ctx, &exec_target);
    if let Err(err) = write_private(&unit_path, contents.as_bytes()) {
        return ActionOutcome::err(err);
    }
    if !run_systemctl(&["daemon-reload"]) {
        return ActionOutcome::err(anyhow!("systemctl --user daemon-reload failed"));
    }
    if !run_systemctl(&["enable", "--now", SERVICE_UNIT_NAME]) {
        return ActionOutcome::err(anyhow!(
            "systemctl --user enable --now {SERVICE_UNIT_NAME} failed"
        ));
    }
    ActionOutcome::ok(format!("service unit installed at {}", unit_path.display()))
}

pub fn service_action(action: &str) -> ActionOutcome {
    match action {
        "start" | "stop" | "restart" | "reload" => {}
        _ => return ActionOutcome::err(anyhow!("unsupported systemctl action: {action}")),
    }
    if run_systemctl(&[action, SERVICE_UNIT_NAME]) {
        ActionOutcome::ok(format!("systemctl --user {action} succeeded"))
    } else {
        ActionOutcome::err(anyhow!("systemctl --user {action} failed"))
    }
}

fn run_systemctl(args: &[&str]) -> bool {
    Command::new("systemctl")
        .arg("--user")
        .args(args)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn write_private(path: &std::path::Path, content: &[u8]) -> Result<()> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .with_context(|| format!("open {}", path.display()))?;
    file.write_all(content)
        .with_context(|| format!("write {}", path.display()))?;
    file.sync_all().ok();
    Ok(())
}

fn shell_quote(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('\'');
    for ch in value.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

#[derive(Debug, Serialize)]
pub struct VersionInfo {
    pub gui_version: String,
    pub api_version: u32,
    pub bus_name: String,
    pub interface_name: String,
    pub build_target: String,
    pub distribution: Option<String>,
    pub session_type: Option<String>,
    pub desktop: Option<String>,
    pub kernel: Option<String>,
}

pub fn version_info() -> VersionInfo {
    VersionInfo {
        gui_version: env!("CARGO_PKG_VERSION").to_string(),
        api_version: logig560_api::API_VERSION,
        bus_name: logig560_api::BUS_NAME.to_string(),
        interface_name: logig560_api::INTERFACE_NAME.to_string(),
        build_target: format!("{}-{}", env::consts::ARCH, env::consts::OS),
        distribution: read_os_release_field("PRETTY_NAME"),
        session_type: env::var("XDG_SESSION_TYPE").ok(),
        desktop: env::var("XDG_CURRENT_DESKTOP").ok(),
        kernel: uname_release(),
    }
}

fn read_os_release_field(field: &str) -> Option<String> {
    let contents = fs::read_to_string("/etc/os-release").ok()?;
    for line in contents.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key == field {
            return Some(value.trim_matches('"').to_string());
        }
    }
    None
}

fn uname_release() -> Option<String> {
    let output = Command::new("uname").arg("-r").output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}
