//! Launch-context detection. Distinguishes an AppImage-mounted GUI from
//! a dev build so downstream code (launcher, systemd unit generation)
//! can choose the right ExecStart target.

use std::io;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchContext {
    /// GUI is running from an AppImage. `appimage_path` is the absolute
    /// path the user launched (from the runtime-provided APPIMAGE env var),
    /// which is stable across process lifetimes as long as the file is
    /// not moved or renamed.
    AppImage { appimage_path: PathBuf },
    /// Dev build. `cli_binary` is the sibling `logig560` binary next to
    /// the GUI binary — the same convention as today's setup module.
    DevBuild { cli_binary: PathBuf },
}

pub fn detect_launch_context() -> LaunchContext {
    detect_launch_context_with(
        |name| std::env::var(name).ok(),
        std::env::current_exe,
    )
}

pub fn detect_launch_context_with<E, C>(env: E, current_exe: C) -> LaunchContext
where
    E: Fn(&str) -> Option<String>,
    C: Fn() -> io::Result<PathBuf>,
{
    if let Some(path) = env("APPIMAGE") {
        return LaunchContext::AppImage {
            appimage_path: PathBuf::from(path),
        };
    }
    // Dev-build fallback: derive the sibling CLI path from current exe.
    // If current_exe fails (rare), fall back to a relative "logig560".
    let cli_binary = current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|p| p.join("logig560")))
        .unwrap_or_else(|| PathBuf::from("logig560"));
    LaunchContext::DevBuild { cli_binary }
}
