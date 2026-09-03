//! Shared styling.
//!
//! Everything is derived from the active palette, so the app reads correctly
//! whichever of the two is chosen. The vocabulary is deliberately narrow and
//! square-edged: a console draws with fills and hairlines, not with rounded
//! cards and shadows, and a git client is easier to scan when the only things
//! carrying colour are the ones that mean something.

use iced::widget::{button, container, text_editor, text_input};
use iced::{Background, Border, Color, Theme};

/// Corners. Not quite zero — a single pixel takes the hard edge off a fill
/// without making anything look like a card.
const RADIUS: f32 = 2.0;

const HAIRLINE: f32 = 1.0;

/// The same color at a different opacity, for tints that sit over a panel.
fn fade(color: Color, alpha: f32) -> Color {
    Color { a: alpha, ..color }
}

fn square(radius: f32) -> Border {
    Border {
        radius: radius.into(),
        ..Border::default()
    }
}

fn outlined(color: Color) -> Border {
    Border {
        color,
        width: HAIRLINE,
        radius: RADIUS.into(),
    }
}

/// The sidebar and other secondary surfaces.
pub fn panel(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();

    container::Style {
        background: Some(palette.background.weak.color.into()),
        ..container::Style::default()
    }
}

/// The surface the diff and the graph are drawn on.
pub fn canvas(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();

    container::Style {
        background: Some(palette.background.base.color.into()),
        ..container::Style::default()
    }
}

/// The `@@ ... @@` bar above each hunk.
pub fn hunk_header(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();

    container::Style {
        background: Some(fade(palette.primary.base.color, 0.16).into()),
        text_color: Some(palette.background.base.text),
        ..container::Style::default()
    }
}

pub fn addition(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();

    container::Style {
        background: Some(fade(palette.success.base.color, 0.18).into()),
        ..container::Style::default()
    }
}

pub fn deletion(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();

    container::Style {
        background: Some(fade(palette.danger.base.color, 0.18).into()),
        ..container::Style::default()
    }
}

pub fn context(_theme: &Theme) -> container::Style {
    container::Style::default()
}

/// A banner reporting the result of the last action. Outlined rather than
/// filled: it is a line of output, not a dialog.
pub fn notice(is_error: bool) -> impl Fn(&Theme) -> container::Style {
    move |theme| {
        let palette = theme.extended_palette();

        let base = if is_error {
            palette.danger.base.color
        } else {
            palette.success.base.color
        };

        container::Style {
            background: Some(fade(base, 0.10).into()),
            text_color: Some(palette.background.base.text),
            border: outlined(fade(base, 0.55)),
            ..container::Style::default()
        }
    }
}

/// A row in a list — a file, a ref, a commit.
///
/// Selection is a flat block of the accent colour behind the whole row, the
/// way a terminal marks a line, rather than a highlight around the text.
pub fn row(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let palette = theme.extended_palette();

        let background = if selected {
            Some(fade(palette.primary.base.color, 0.24))
        } else if matches!(status, button::Status::Hovered) {
            Some(fade(palette.background.strong.color, 0.35))
        } else {
            None
        };

        button::Style {
            background: background.map(Background::from),
            text_color: palette.background.base.text,
            border: square(RADIUS),
            ..button::Style::default()
        }
    }
}

/// A row in the file list.
///
/// `marked` is "a bulk action would touch this"; `showing` is "this is the
/// diff on screen". A marked row is tinted, and the one being shown is tinted
/// harder — so a selection of eight files reads as one block with the current
/// file picked out of it.
pub fn file_row(
    marked: bool,
    showing: bool,
) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let palette = theme.extended_palette();

        let background = match (showing, marked) {
            (true, _) => Some(fade(palette.primary.base.color, 0.30)),
            (false, true) => Some(fade(palette.primary.base.color, 0.15)),
            (false, false) if matches!(status, button::Status::Hovered) => {
                Some(fade(palette.background.strong.color, 0.35))
            }
            _ => None,
        };

        button::Style {
            background: background.map(Background::from),
            text_color: palette.background.base.text,
            border: square(RADIUS),
            ..button::Style::default()
        }
    }
}

/// One of the two file-list tabs, and the repository tabs' inner buttons.
///
/// An inactive tab is a label; an active one is a block of accent with an
/// underline of it, which is how a terminal UI marks the pane you are in.
pub fn tab(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let palette = theme.extended_palette();

        let background = if active {
            Some(fade(palette.primary.base.color, 0.22))
        } else if matches!(status, button::Status::Hovered) {
            Some(fade(palette.background.strong.color, 0.35))
        } else {
            None
        };

        let text_color = if active {
            palette.primary.base.color
        } else {
            muted(theme)
        };

        button::Style {
            background: background.map(Background::from),
            text_color,
            border: square(RADIUS),
            ..button::Style::default()
        }
    }
}

/// A repository tab.
///
/// The whole tab is one painted surface. The name and the ✕ are separate
/// buttons on top of it, so neither has to guess which of them a click was
/// meant for, but they paint nothing themselves — two adjacent fills, however
/// carefully their corners are matched, leave a seam down the middle that
/// makes the ✕ look like it is sitting beside the tab rather than in it.
pub fn tab_chip(active: bool) -> impl Fn(&Theme) -> container::Style {
    move |theme| {
        let palette = theme.extended_palette();

        let background = if active {
            fade(palette.primary.base.color, 0.22)
        } else {
            fade(palette.background.strong.color, 0.28)
        };

        container::Style {
            background: Some(background.into()),
            border: square(RADIUS),
            ..container::Style::default()
        }
    }
}

/// The name inside a tab: a hit target, drawn only as text.
pub fn tab_label(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let palette = theme.extended_palette();

        let text_color = match (active, status) {
            (true, _) => palette.primary.base.color,
            (false, button::Status::Hovered | button::Status::Pressed) => {
                palette.background.base.text
            }
            _ => muted(theme),
        };

        transparent(text_color)
    }
}

/// The ✕ inside a tab. It reddens under the pointer, so it reads as closing
/// the tab rather than switching to it.
pub fn tab_close(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let palette = theme.extended_palette();

        let text_color = match status {
            button::Status::Hovered | button::Status::Pressed => palette.danger.base.color,
            _ if active => fade(palette.primary.base.color, 0.75),
            _ => fade(palette.background.base.text, 0.35),
        };

        transparent(text_color)
    }
}

/// A button that paints nothing, for use over a surface that already has.
fn transparent(text_color: Color) -> button::Style {
    button::Style {
        background: None,
        text_color,
        border: Border::default(),
        ..button::Style::default()
    }
}

/// One choice in a row of them, in a form.
///
/// Unlike a tab, an unpicked option keeps its outline: a row of bare labels
/// gives no sign that any of them can be clicked, and these rows are the only
/// way to answer some of the questions in the settings dialog.
pub fn option(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| match selected {
        true => tab(true)(theme, status),
        false => toggle(theme, status),
    }
}

/// A line in the right-click menu. Flat until the pointer is on it, which is
/// the only feedback a menu needs.
pub fn menu_item(theme: &Theme, status: button::Status) -> button::Style {
    let palette = theme.extended_palette();

    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);

    button::Style {
        background: hovered.then(|| fade(palette.primary.base.color, 0.22).into()),
        text_color: match status {
            button::Status::Disabled => fade(palette.background.base.text, 0.35),
            _ => palette.background.base.text,
        },
        border: square(RADIUS),
        ..button::Style::default()
    }
}

/// A menu line that cannot be undone.
pub fn menu_danger(theme: &Theme, status: button::Status) -> button::Style {
    let palette = theme.extended_palette();

    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);

    button::Style {
        background: hovered.then(|| fade(palette.danger.base.color, 0.22).into()),
        text_color: palette.danger.base.color,
        border: square(RADIUS),
        ..button::Style::default()
    }
}

/// A small secondary button: an outline and a label, filled only on hover.
pub fn toggle(theme: &Theme, status: button::Status) -> button::Style {
    let palette = theme.extended_palette();

    let (background, text_color, border) = match status {
        button::Status::Hovered | button::Status::Pressed => (
            Some(fade(palette.primary.base.color, 0.22)),
            palette.primary.base.color,
            fade(palette.primary.base.color, 0.75),
        ),
        button::Status::Disabled => (None, fade(palette.background.base.text, 0.3), fade(palette.background.strong.color, 0.5)),
        button::Status::Active => (
            None,
            palette.background.base.text,
            fade(palette.background.strong.color, 0.9),
        ),
    };

    button::Style {
        background: background.map(Background::from),
        text_color,
        border: outlined(border),
        ..button::Style::default()
    }
}

/// The one button on screen that is filled: the primary action.
pub fn primary(theme: &Theme, status: button::Status) -> button::Style {
    let palette = theme.extended_palette();

    let background = match status {
        button::Status::Hovered | button::Status::Pressed => palette.primary.strong.color,
        button::Status::Disabled => fade(palette.primary.base.color, 0.25),
        button::Status::Active => palette.primary.base.color,
    };

    let text_color = match status {
        button::Status::Disabled => fade(palette.background.base.text, 0.45),
        _ => palette.primary.base.text,
    };

    button::Style {
        background: Some(background.into()),
        text_color,
        border: square(RADIUS),
        ..button::Style::default()
    }
}

/// A destructive confirmation, so "Delete" never looks like "Cancel".
pub fn danger(theme: &Theme, status: button::Status) -> button::Style {
    let palette = theme.extended_palette();

    let (background, text_color) = match status {
        button::Status::Hovered | button::Status::Pressed => {
            (Some(palette.danger.base.color), palette.danger.base.text)
        }
        button::Status::Disabled => (None, fade(palette.danger.base.color, 0.4)),
        button::Status::Active => (
            Some(fade(palette.danger.base.color, 0.14)),
            palette.danger.base.color,
        ),
    };

    button::Style {
        background: background.map(Background::from),
        text_color,
        border: outlined(fade(palette.danger.base.color, 0.7)),
        ..button::Style::default()
    }
}

/// The bar that asks for a name or a confirmation.
pub fn prompt(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();

    container::Style {
        background: Some(fade(palette.primary.base.color, 0.09).into()),
        border: outlined(fade(palette.primary.base.color, 0.55)),
        ..container::Style::default()
    }
}

/// The name box in the prompt bar.
pub fn input(theme: &Theme, status: text_input::Status) -> text_input::Style {
    let mut style = text_input::default(theme, status);
    let palette = theme.extended_palette();

    style.background = palette.background.base.color.into();
    style.border = outlined(fade(palette.background.strong.color, 0.9));

    if matches!(status, text_input::Status::Focused { .. }) {
        style.border = outlined(palette.primary.base.color);
    }

    style
}

/// The summary box, which is a text input dressed as the editor beneath it.
pub fn editor_input(theme: &Theme, status: text_input::Status) -> text_input::Style {
    let mut style = input(theme, status);

    style.background = theme.extended_palette().background.base.color.into();

    style
}

/// The commit message box.
pub fn editor(theme: &Theme, status: text_editor::Status) -> text_editor::Style {
    let mut style = text_editor::default(theme, status);
    let palette = theme.extended_palette();

    style.background = palette.background.base.color.into();
    style.border = outlined(fade(palette.background.strong.color, 0.9));

    if matches!(status, text_editor::Status::Focused { .. }) {
        style.border = outlined(palette.primary.base.color);
    }

    style
}

/// The dimmed backdrop behind a dialog. Dark in both palettes: the point is
/// to push the window back, not to tint it.
pub fn scrim(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Color::from_rgba(0.0, 0.0, 0.0, 0.55).into()),
        ..container::Style::default()
    }
}

/// The dialog itself.
pub fn dialog(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();

    container::Style {
        background: Some(palette.background.base.color.into()),
        border: outlined(fade(palette.background.strong.color, 1.0)),
        ..container::Style::default()
    }
}

/// A muted foreground for line numbers, counts and hints.
pub fn muted(theme: &Theme) -> Color {
    fade(theme.extended_palette().background.base.text, 0.5)
}

/// A ref badge beside a commit.
pub fn badge(color: Color) -> impl Fn(&Theme) -> container::Style {
    move |_theme| container::Style {
        background: Some(fade(color, 0.14).into()),
        text_color: Some(color),
        border: outlined(fade(color, 0.5)),
        ..container::Style::default()
    }
}

/// The hues branch lines cycle through, spaced far enough apart that two lanes
/// side by side never read as the same color.
///
/// None of them is the accent hue. The accent means "you can act on this", and
/// a branch line is not something to click.
const LANE_HUES: [f32; 8] = [205.0, 150.0, 45.0, 280.0, 96.0, 340.0, 185.0, 62.0];

/// The color of one lane in the commit graph.
///
/// Lanes are colored by index rather than by branch: a lane is only reused once
/// it empties, so a line keeps its color for as long as it is drawn, and
/// neighbours always differ.
pub fn lane(theme: &Theme, index: usize) -> Color {
    let hue = LANE_HUES[index % LANE_HUES.len()];

    // Held back from full saturation: eight bright lines next to each other
    // read as noise, and the graph is meant to be followed, not looked at.
    let (saturation, lightness) = if theme.extended_palette().is_dark {
        (0.48, 0.64)
    } else {
        (0.55, 0.42)
    };

    hsl(hue, saturation, lightness)
}

/// The ring drawn around a commit node, so a node sitting on top of a line
/// still reads as a node.
pub fn graph_background(theme: &Theme) -> Color {
    theme.extended_palette().background.base.color
}

fn hsl(hue: f32, saturation: f32, lightness: f32) -> Color {
    let chroma = (1.0 - (2.0 * lightness - 1.0).abs()) * saturation;
    let sector = hue / 60.0;
    let second = chroma * (1.0 - (sector % 2.0 - 1.0).abs());

    let (red, green, blue) = match sector as u32 {
        0 => (chroma, second, 0.0),
        1 => (second, chroma, 0.0),
        2 => (0.0, chroma, second),
        3 => (0.0, second, chroma),
        4 => (second, 0.0, chroma),
        _ => (chroma, 0.0, second),
    };

    let base = lightness - chroma / 2.0;

    Color::from_rgb(red + base, green + base, blue + base)
}
