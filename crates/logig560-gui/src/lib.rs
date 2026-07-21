//! Library surface for logig560-gui. Exposes modules that host
//! integration tests exercise; the Tauri binary in src/main.rs
//! consumes the same modules via `use logig560_gui::…`.

pub mod client;
pub mod commands;
pub mod setup;
