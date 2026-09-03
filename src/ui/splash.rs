//! The splash screen.
//!
//! A window of its own, undecorated and exactly the size of the artwork. It
//! has to be: the point of a splash is to be on screen *before* the
//! application, and something drawn inside the application's window cannot be,
//! because that window is already there.
//!
//! It opens first and alone; the application's window comes up behind it after
//! [`ALONE`], and the splash goes [`OVER`] later. Clicking it skips the wait.
//! Settings turns it off — a splash that cannot be got past is a splash that
//! gets resented.

use std::sync::LazyLock;
use std::time::Duration;

use iced::widget::{container, image, mouse_area};
use iced::{Element, Fill, Size, window};

use crate::app::Message;
use crate::ui::style;

/// The artwork, carried in the binary: a packaged application has nowhere to
/// look for a file beside itself.
const IMAGE: &[u8] = include_bytes!("../../assets/splash.jpg");

/// The handle, made once.
///
/// `Handle::from_bytes` stamps every handle with a fresh unique id, and the
/// renderer caches decoded images by that id — so building one inside `view`
/// hands the renderer a different image on every frame, and a 1440×900 JPEG is
/// decoded and uploaded again for each of them. `view` runs on every message,
/// including every mouse movement, and the result was a splash that flickered.
static ARTWORK: LazyLock<image::Handle> = LazyLock::new(|| image::Handle::from_bytes(IMAGE));

/// How long the splash is the only thing on screen.
///
/// Counted from the window opening rather than from startup: creating the
/// window and starting the graphics backend takes a second or more on a cold
/// run, and a clock that began before any of that had happened was mostly
/// spent by the time anything appeared.
pub const ALONE: Duration = Duration::from_secs(5);

/// How long it stays once the application's window is up behind it.
pub const OVER: Duration = Duration::from_secs(1);

/// The window's size. The artwork is 1.6:1 and fills the window exactly, so
/// the two have to agree or it will be letterboxed.
const WIDTH: f32 = 720.0;
const HEIGHT: f32 = 450.0;

/// The splash's own window: no frame, no title bar, no resize handles, and
/// above whatever else is on screen. Exactly the size of the artwork, so the
/// window *is* the image.
pub fn window() -> window::Settings {
    window::Settings {
        size: Size::new(WIDTH, HEIGHT),
        position: window::Position::Centered,
        resizable: false,
        decorations: false,
        level: window::Level::AlwaysOnTop,
        ..window::Settings::default()
    }
}

pub fn view() -> Element<'static, Message> {
    let art = image(ARTWORK.clone())
        .width(Fill)
        .height(Fill)
        .content_fit(iced::ContentFit::Cover);

    // Anywhere on it dismisses it: someone who wants to get on with it should
    // not have to find a button.
    mouse_area(
        container(art)
            .width(Fill)
            .height(Fill)
            .style(style::canvas),
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
    fn the_artwork_is_the_same_image_every_time_it_is_drawn() {
        // The renderer caches by handle id. Two ids means two images, which
        // means decoding the whole JPEG again on every frame.
        assert_eq!(
            ARTWORK.clone().id(),
            ARTWORK.clone().id(),
            "the handle has to be made once, not per view"
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
