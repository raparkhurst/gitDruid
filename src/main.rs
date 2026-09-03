//! gitDruid — a cross-platform git GUI.

// Keep a console window from appearing behind the app on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use iced::Font;

use git_druid::{app, ui};

fn main() -> iced::Result {
    // A daemon rather than an application: the splash is a window in its own
    // right, and only a daemon can have more than one.
    iced::daemon(app::boot, app::update, ui::view)
        .title(app::title)
        .theme(app::theme)
        .subscription(app::subscription)
        // One typeface for the whole window. A git client is mostly paths,
        // hashes and diffs — things that line up — and a console reads as a
        // console because everything in it shares a grid.
        .default_font(Font::MONOSPACE)
        .run()
}
