// BlastWA core library.
// every module is plain rust + tokio: testable headless, gui bolted on top.
pub mod config;
pub mod message;
pub mod campaign;
pub mod account;
pub mod autoreply;
pub mod api;
pub mod updater;

/// serializable IPC error contract shared by the tauri command layer
pub mod error;

pub mod browser;
