//! Integration tests for launch-context detection and launcher-script
//! generation. Uses tempdirs to avoid touching the developer's real
//! ~/.local/bin.

use std::path::PathBuf;

use logig560_gui::setup::launch::{LaunchContext, detect_launch_context_with};

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
