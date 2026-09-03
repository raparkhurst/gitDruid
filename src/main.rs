//! gitDruid — a cross-platform git GUI.

// Keep a console window from appearing behind the app on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use iced::{Font, Size, window};

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
        .window(window())
        .run()
}

fn window() -> window::Settings {
    window::Settings {
        size: Size::new(1400.0, 840.0),
        position: window::Position::Centered,

        // Wayland and X11 match a window to its launcher by this id, and show
        // the launcher's icon in the dock when they match. It has to be the
        // basename of the installed .desktop file, which is `gitdruid`.
        #[cfg(target_os = "linux")]
        platform_specific: window::settings::PlatformSpecific {
            application_id: "gitdruid".to_owned(),
            ..window::settings::PlatformSpecific::default()
        },

        ..window::Settings::default()
    }
}
