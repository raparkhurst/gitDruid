//! gitDruid — a cross-platform git GUI desktop client.
//!
//! The crate is split in three:
//!
//! - [`git`] wraps libgit2 in plain, owned data. It knows nothing about the UI.
//! - [`app`] holds the state and the update loop, and drives every git call on
//!   a background task.
//! - [`ui`] renders that state and produces messages.
//! - [`settings`] reads and writes the two configuration files.
//! - [`desktop`] adds the app to a Linux applications menu, when asked.

pub mod app;
pub mod desktop;
pub mod git;
pub mod settings;
pub mod ui;
