//! The two palettes gitDruid ships with.
//!
//! Both are warm rather than neutral, and dim rather than saturated: the app
//! reads as a terminal, where colour marks something out — a branch line, an
//! added line, a warning — rather than decorating the surface it sits on.

use std::sync::LazyLock;

use iced::theme::Palette;
use iced::{Color, Theme};

/// Builds a colour from its hex literal, so the palettes below read as the
/// values a designer would write down.
const fn hex(value: u32) -> Color {
    Color::from_rgb8(
        ((value >> 16) & 0xFF) as u8,
        ((value >> 8) & 0xFF) as u8,
        (value & 0xFF) as u8,
    )
}

/// Dark on warm near-black. The default: this is a tool for a terminal window.
pub static DARK: LazyLock<Theme> = LazyLock::new(|| {
    Theme::custom(
        "Console".to_owned(),
        Palette {
            background: hex(0x1B1A18),
            text: hex(0xE6E1D7),
            primary: hex(0xD97757),
            success: hex(0x86B87A),
            warning: hex(0xD9A441),
            danger: hex(0xE0705C),
        },
    )
});

/// Neutral dark. No warmth and no character — the one to pick when the palette
/// should get out of the way.
pub static DARK_NEUTRAL: LazyLock<Theme> = LazyLock::new(|| {
    Theme::custom(
        "Dark".to_owned(),
        Palette {
            background: hex(0x131315),
            text: hex(0xE3E3E5),
            primary: hex(0x7FA7D9),
            success: hex(0x76BE8C),
            warning: hex(0xD2B06A),
            danger: hex(0xDE6E6E),
        },
    )
});

/// The Dracula palette, which many editors already use.
pub static DRACULA: LazyLock<Theme> = LazyLock::new(|| {
    Theme::custom(
        "Dracula".to_owned(),
        Palette {
            background: hex(0x282A36),
            text: hex(0xF8F8F2),
            primary: hex(0xBD93F9),
            success: hex(0x50FA7B),
            warning: hex(0xF1FA8C),
            danger: hex(0xFF5555),
        },
    )
});

/// Phosphor green on black. Every role is a shade of one hue, so the only
/// thing distinguishing a warning from a success is how bright it is.
pub static MATRIX: LazyLock<Theme> = LazyLock::new(|| {
    Theme::custom(
        "Matrix".to_owned(),
        Palette {
            background: hex(0x040A05),
            text: hex(0x33FF77),
            primary: hex(0x00E63C),
            success: hex(0x5CFF8F),
            warning: hex(0xB8FF4D),
            danger: hex(0xFF4D4D),
        },
    )
});

/// The same palette on paper, for anyone working in daylight.
pub static LIGHT: LazyLock<Theme> = LazyLock::new(|| {
    Theme::custom(
        "Parchment".to_owned(),
        Palette {
            background: hex(0xF3F1EA),
            text: hex(0x2A2825),
            primary: hex(0xB85C3C),
            success: hex(0x4C7A46),
            warning: hex(0x9A6B12),
            danger: hex(0xB03A28),
        },
    )
});

/// What the theme picker offers. A handful, not thirty: the point of a palette
/// is that everything in the window is drawn from it.
pub fn all() -> Vec<Theme> {
    vec![
        DARK.clone(),
        DARK_NEUTRAL.clone(),
        DRACULA.clone(),
        MATRIX.clone(),
        LIGHT.clone(),
    ]
}

pub fn default() -> Theme {
    DARK.clone()
}

/// The palette called `name`, or `None` for one gitDruid no longer ships.
///
/// Matched on the name the picker shows, which is what gets written to the
/// settings file: a number would be shorter and would stop meaning anything
/// the moment the list is reordered.
pub fn by_name(name: &str) -> Option<Theme> {
    all().into_iter().find(|theme| theme.to_string() == name)
}
