//! Non-TUI domain logic: paths/locations, filesystem & archive (ZIP, tar.gz) panel backend, copy state, settings persistence,
//! find (glob/traversal), text/size formatting, disk summary.
//! Ratatui/crossterm stay in `ui`, dialog modules, `events`, `main` loop wiring, etc.

pub mod copy_ops;
pub mod copy_state;
pub mod disk_space;
pub mod file_ops;
pub mod find;
#[cfg(target_os = "linux")]
pub mod linux_home_trash;
pub mod location;
pub mod panel_backend;
pub mod settings;
pub mod text_format;
pub mod trash_delete;
