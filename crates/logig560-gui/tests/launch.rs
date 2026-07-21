//! Integration tests for launch-context detection and launcher-script
//! generation. Uses tempdirs to avoid touching the developer's real
//! ~/.local/bin.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use logig560_gui::setup::launch::{
    LaunchContext, detect_launch_context_with, ensure_launcher, render_desktop_unit,
    render_gaming_unit,
};

#[test]
fn detects_appimage_context_when_env_var_set() {
    let ctx = detect_launch_context_with(
        |name| match name {
            "APPIMAGE" => Some("/home/user/Downloads/G560.AppImage".to_string()),
            _ => None,
        },
        || Ok(PathBuf::from("/tmp/.mount_XXXX/usr/bin/logig560-gui")),
    );
    match ctx {
        LaunchContext::AppImage { appimage_path } => {
            assert_eq!(appimage_path, PathBuf::from("/home/user/Downloads/G560.AppImage"));
        }
        LaunchContext::DevBuild { .. } => panic!("expected AppImage context"),
    }
}

#[test]
fn detects_dev_build_when_no_appimage_env() {
    let ctx = detect_launch_context_with(
        |_| None,
        || Ok(PathBuf::from("/home/user/proj/target/release/logig560-gui")),
    );
    match ctx {
        LaunchContext::DevBuild { cli_binary } => {
            assert_eq!(
                cli_binary,
                PathBuf::from("/home/user/proj/target/release/logig560")
            );
        }
        LaunchContext::AppImage { .. } => panic!("expected DevBuild context"),
    }
}

#[test]
fn writes_launcher_script_with_exec_bit() {
    let dir = tempfile::tempdir().unwrap();
    let bin_dir = dir.path().join("bin");
    let appimage = PathBuf::from("/tmp/fake/G560.AppImage");

    let launcher = ensure_launcher(&bin_dir, &appimage).expect("ensure_launcher succeeded");

    assert_eq!(launcher, bin_dir.join("logig560"));
    let contents = fs::read_to_string(&launcher).unwrap();
    assert!(contents.starts_with("#!/bin/sh"), "shebang missing: {contents}");
    assert!(contents.contains("--cli"), "launcher must add --cli prefix: {contents}");
    assert!(
        contents.contains("/tmp/fake/G560.AppImage"),
        "launcher must embed AppImage path: {contents}",
    );
    let mode = fs::metadata(&launcher).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o755, "launcher must be user-executable");
}

#[test]
fn ensure_launcher_creates_missing_parent_dir() {
    let dir = tempfile::tempdir().unwrap();
    let bin_dir = dir.path().join("nested/local/bin");
    let appimage = PathBuf::from("/tmp/fake/G560.AppImage");

    let launcher = ensure_launcher(&bin_dir, &appimage).expect("ensure_launcher succeeded");
    assert!(launcher.is_file());
}

#[test]
fn ensure_launcher_shell_escapes_appimage_path() {
    let dir = tempfile::tempdir().unwrap();
    let bin_dir = dir.path().join("bin");
    // A path with a single quote would break naive quoting.
    let appimage = PathBuf::from("/tmp/weird 'name'/G560.AppImage");

    let launcher = ensure_launcher(&bin_dir, &appimage).expect("ensure_launcher succeeded");
    let contents = fs::read_to_string(&launcher).unwrap();
    // The script must be shell-parseable. Run `sh -n` (parse-only) to verify.
    let status = std::process::Command::new("sh")
        .arg("-n")
        .arg(&launcher)
        .status()
        .expect("sh available");
    assert!(status.success(), "generated launcher is not shell-parseable:\n{contents}");
}

#[test]
fn desktop_unit_appimage_uses_launcher_path() {
    let ctx = LaunchContext::AppImage {
        appimage_path: PathBuf::from("/home/user/G560.AppImage"),
    };
    let launcher = PathBuf::from("/home/user/.local/bin/logig560");
    let rendered = render_desktop_unit(&ctx, &launcher);
    assert!(
        rendered.contains("ExecStart=/home/user/.local/bin/logig560 run"),
        "rendered unit missing launcher ExecStart:\n{rendered}",
    );
    assert!(!rendered.contains("@LAUNCHER@"), "placeholder not substituted");
}

#[test]
fn desktop_unit_dev_build_uses_binary_path() {
    let cli = PathBuf::from("/home/user/proj/target/release/logig560");
    let ctx = LaunchContext::DevBuild { cli_binary: cli.clone() };
    let rendered = render_desktop_unit(&ctx, &cli);
    assert!(
        rendered.contains("ExecStart=/home/user/proj/target/release/logig560 run"),
        "rendered unit missing dev ExecStart:\n{rendered}",
    );
    assert!(!rendered.contains("@LAUNCHER@"), "placeholder not substituted");
}

#[test]
fn gaming_unit_appimage_uses_launcher_run_gaming() {
    let ctx = LaunchContext::AppImage {
        appimage_path: PathBuf::from("/home/user/G560.AppImage"),
    };
    let launcher = PathBuf::from("/home/user/.local/bin/logig560");
    let rendered = render_gaming_unit(&ctx, &launcher);
    assert!(
        rendered.contains("ExecStart=/home/user/.local/bin/logig560 run-gaming"),
        "rendered gaming unit missing launcher ExecStart:\n{rendered}",
    );
    assert!(rendered.contains("PartOf=gamescope-session-plus@steam.service"));
}
