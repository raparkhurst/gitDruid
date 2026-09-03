//! gitDruid — a cross-platform git GUI.

// Keep a console window from appearing behind the app on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use iced::Font;

use git_druid::{app, ui};

fn main() -> iced::Result {
    iced::application(app::boot, app::update, ui::view)
        .title(app::title)
        .theme(app::theme)
        .subscription(app::subscription)
        // One typeface for the whole window. A git client is mostly paths,
        // hashes and diffs — things that line up — and a console reads as a
        // console because everything in it shares a grid.
        .default_font(Font::MONOSPACE)
        .window_size((1400.0, 840.0))
        .centered()
        .run()
}
