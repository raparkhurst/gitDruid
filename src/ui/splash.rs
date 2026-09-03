//! The splash screen.
//!
//! Not a window of its own: a second window costs a second entry in the dock
//! and a flicker as it hands over, and buys nothing here. It runs in two
//! phases instead — on its own with nothing drawn behind it, then over the
//! application once that is worth showing — and dismisses itself, or goes on
//! the first click. Settings turns it off; a splash that cannot be got past is
//! a splash that gets resented.

use std::time::Duration;

use iced::widget::{container, image, mouse_area};
use iced::{Element, Fill};

use crate::app::Message;
use crate::ui::style;

/// The artwork, carried in the binary: a packaged application has nowhere to
/// look for a file beside itself.
const IMAGE: &[u8] = include_bytes!("../../assets/splash.jpg");

/// How long the splash is the only thing on screen.
///
/// Counted from the window opening rather than from startup: creating the
/// window and starting the graphics backend takes a second or more on a cold
/// run, and a clock that began before any of that had happened was mostly
/// spent by the time anything appeared.
pub const ALONE: Duration = Duration::from_secs(10);

/// How long it stays once the application is drawn behind it.
pub const OVER: Duration = Duration::from_secs(3);

/// The card's size. The artwork is 1.6:1 and is drawn to fill this exactly, so
/// the two have to agree or it will be letterboxed.
const WIDTH: f32 = 720.0;
const HEIGHT: f32 = 450.0;

/// The splash, on its own or over the window.
///
/// `alone` decides what surrounds the card: an opaque ground, so there is
/// nothing behind it at all, or a scrim, so the application shows through
/// dimmed.
pub fn view(alone: bool) -> Element<'static, Message> {
    let art = image(image::Handle::from_bytes(IMAGE))
        .width(WIDTH)
        .height(HEIGHT)
        .content_fit(iced::ContentFit::Cover);

    let card = container(art)
        .width(WIDTH)
        .height(HEIGHT)
        .style(style::dialog);

    // Anywhere on it dismisses it: someone who wants to get on with it should
    // not have to find a button.
    mouse_area(
        container(card)
            .center_x(Fill)
            .center_y(Fill)
            .width(Fill)
            .height(Fill)
            .style(match alone {
                true => style::canvas,
                false => style::scrim,
            }),
    )
    .on_press(Message::DismissSplash)
    .on_right_press(Message::DismissSplash)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_artwork_can_actually_be_decoded() {
        // The renderer draws nothing and says nothing when a decode fails, so
        // a missing codec looks exactly like a bug in the layout. This is the
        // check that tells the two apart.
        let decoded = ::image::load_from_memory(IMAGE).expect("the splash should decode");

        assert_eq!(
            (decoded.width(), decoded.height()),
            (1440, 900),
            "the card's proportions are built around this"
        );
    }

    #[test]
    fn the_card_matches_the_artwork_proportions() {
        let art = 1440.0 / 900.0;
        let card = WIDTH / HEIGHT;

        assert!(
            (art - card).abs() < 0.01,
            "the artwork would be cropped or letterboxed: {art} against {card}"
        );
    }
}
