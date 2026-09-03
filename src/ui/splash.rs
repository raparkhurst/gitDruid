//! The splash screen.
//!
//! Shown over the window rather than in a window of its own: a second window
//! costs a second entry in the dock and a flicker as it hands over, and buys
//! nothing here. It dismisses itself after a moment, or on the first click,
//! and can be turned off — a splash that cannot be got past is a splash that
//! gets resented.

use std::time::Duration;

use iced::widget::{column, container, image, mouse_area, opaque, rule, stack, text};
use iced::{Color, Element, Fill, Padding, Theme};

use crate::app::Message;
use crate::ui::style;

/// The artwork, carried in the binary: a packaged application has nowhere to
/// look for a file beside itself.
const IMAGE: &[u8] = include_bytes!("../../assets/splash.jpg");

/// How long it stays without being asked to go.
pub const DURATION: Duration = Duration::from_millis(2200);

/// The card's size. The artwork is 1.6:1 and is drawn to fill this exactly, so
/// the two have to agree or it will be letterboxed.
const WIDTH: f32 = 720.0;
const HEIGHT: f32 = 450.0;

/// The left third of the artwork is empty mist, which is where the text goes.
/// It is light there, so the text is dark — the usual pale-on-dark would
/// disappear into it.
const INK: Color = Color::from_rgb(0.09, 0.08, 0.07);
const INK_SOFT: Color = Color::from_rgba(0.09, 0.08, 0.07, 0.72);

pub fn view<'a>(screen: Element<'a, Message>) -> Element<'a, Message> {
    let art = image(image::Handle::from_bytes(IMAGE))
        .width(WIDTH)
        .height(HEIGHT)
        .content_fit(iced::ContentFit::Cover);

    let words = container(
        column![
            text("gitDruid").size(46).style(ink),
            container(rule::horizontal(1)).width(150).padding(
                Padding::default().top(6).bottom(10)
            ),
            text(concat!("version ", env!("CARGO_PKG_VERSION")))
                .size(13)
                .style(soft),
            text("Robert Parkhurst").size(13).style(soft),
        ]
        .spacing(2),
    )
    .padding(Padding::default().left(52))
    .width(Fill)
    .height(Fill)
    .align_y(iced::Center);

    let card = container(stack![art, words])
        .width(WIDTH)
        .height(HEIGHT)
        .clip(true)
        .style(style::dialog);

    // Anywhere on the splash dismisses it, including the artwork: someone who
    // wants to get on with it should not have to find a button.
    let splash = mouse_area(
        container(card)
            .center_x(Fill)
            .center_y(Fill)
            .width(Fill)
            .height(Fill)
            .style(style::scrim),
    )
    .on_press(Message::DismissSplash)
    .on_right_press(Message::DismissSplash);

    stack![screen, opaque(splash)].into()
}

fn ink(_theme: &Theme) -> text::Style {
    text::Style { color: Some(INK) }
}

fn soft(_theme: &Theme) -> text::Style {
    text::Style {
        color: Some(INK_SOFT),
    }
}
