//! Settling a conflicted file.
//!
//! Not a diff. A diff has one before and one after; a conflict has two afters
//! and no agreement about which is which, so it is drawn as what both sides
//! said, side by side down the file, with the choice attached to each place
//! they disagree.

use iced::widget::{Column, button, column, container, row, rule, scrollable, text};
use iced::{Center, Element, Fill, Font, Theme};

use crate::app::{Message, Repo};
use crate::git::{Conflict, ConflictSide, Region};
use crate::ui::style;

const CODE: f32 = 13.0;

pub fn view<'a>(repo: &'a Repo, conflict: &'a Conflict) -> Element<'a, Message> {
    column![header(conflict), rule::horizontal(1), body(repo, conflict)]
        .width(Fill)
        .height(Fill)
        .into()
}

pub fn loading(path: &std::path::Path) -> Element<'static, Message> {
    container(
        text(format!("Reading {}…", path.display()))
            .size(13)
            .style(muted),
    )
    .center_x(Fill)
    .center_y(Fill)
    .style(style::canvas)
    .into()
}

fn header<'a>(conflict: &'a Conflict) -> Element<'a, Message> {
    let path = conflict.path.clone();
    let settled = conflict.is_settled();

    let state = match (conflict.binary, conflict.unresolved()) {
        (true, _) => "binary — take one side or the other".to_owned(),
        (false, 0) => "settled — mark it resolved to stage it".to_owned(),
        (false, 1) => "1 place still in dispute".to_owned(),
        (false, count) => format!("{count} places still in dispute"),
    };

    let small = |label: &'static str, message: Message| {
        button(text(label).size(11))
            .padding([3, 8])
            .style(style::toggle)
            .on_press(message)
    };

    container(
        row![
            column![
                text(conflict.path.display().to_string()).size(14),
                text(state).size(11).style(muted),
            ]
            .spacing(2)
            .width(Fill),
            small(
                "Take all ours",
                Message::TakeSide(path.clone(), ConflictSide::Ours),
            ),
            small(
                "Take all theirs",
                Message::TakeSide(path.clone(), ConflictSide::Theirs),
            ),
            button(text("Mark resolved").size(11))
                .padding([3, 8])
                .style(match settled {
                    true => style::primary,
                    false => style::toggle,
                })
                .on_press(Message::MarkResolved(path)),
        ]
        .spacing(8)
        .align_y(Center),
    )
    .padding([10, 14])
    .width(Fill)
    .style(style::panel)
    .into()
}

fn body<'a>(repo: &'a Repo, conflict: &'a Conflict) -> Element<'a, Message> {
    if conflict.binary {
        return hint(
            "This file is not text, so there is nothing to read side by side. Take one whole \
             side or the other.",
        );
    }

    if conflict.regions.is_empty() {
        return hint("The file is empty.");
    }

    let mut content = Column::new().width(Fill);
    let mut disputed = 0;

    for region in &conflict.regions {
        match region {
            Region::Common(lines) => {
                for line in lines {
                    content = content.push(code(line, style::context));
                }
            }
            Region::Split {
                ours,
                theirs,
                base,
                ours_label,
                theirs_label,
            } => {
                content = content.push(choice(disputed, repo.busy));

                content = content.push(side(
                    &format!("ours — {}", label(ours_label, "HEAD")),
                    ours,
                    style::addition,
                ));

                if !base.is_empty() {
                    content = content.push(side("was", base, style::context));
                }

                content = content.push(side(
                    &format!("theirs — {}", label(theirs_label, "incoming")),
                    theirs,
                    style::deletion,
                ));

                disputed += 1;
            }
        }
    }

    container(scrollable(content).width(Fill).height(Fill))
        .width(Fill)
        .height(Fill)
        .style(style::canvas)
        .into()
}

/// The bar above each disputed place, carrying the three answers.
fn choice(index: usize, busy: bool) -> Element<'static, Message> {
    let pick = move |label: &'static str, side: ConflictSide| {
        button(text(label).size(11))
            .padding([3, 8])
            .style(style::toggle)
            .on_press_maybe((!busy).then_some(Message::Resolve(index, side)))
    };

    container(
        row![
            text(format!("conflict {}", index + 1))
                .size(11)
                .font(Font::MONOSPACE)
                .width(Fill),
            pick("Use ours", ConflictSide::Ours),
            pick("Use theirs", ConflictSide::Theirs),
            pick("Use both", ConflictSide::Both),
        ]
        .spacing(8)
        .align_y(Center),
    )
    .padding([5, 12])
    .width(Fill)
    .style(style::hunk_header)
    .into()
}

fn side<'a>(
    caption: &str,
    lines: &'a [String],
    tint: fn(&Theme) -> container::Style,
) -> Element<'a, Message> {
    let mut block = Column::new().width(Fill).push(
        container(text(caption.to_owned()).size(10).style(muted))
            .padding([2, 12])
            .width(Fill),
    );

    if lines.is_empty() {
        block = block.push(
            container(text("(nothing)").size(11).style(muted))
                .padding([1, 12])
                .width(Fill),
        );
    }

    for line in lines {
        block = block.push(code(line, tint));
    }

    block.into()
}

fn code<'a>(line: &'a str, tint: fn(&Theme) -> container::Style) -> Element<'a, Message> {
    container(
        text(line)
            .size(CODE)
            .font(Font::MONOSPACE)
            .wrapping(text::Wrapping::None),
    )
    .padding([0, 12])
    .width(Fill)
    .style(tint)
    .into()
}

/// A marker with nothing after it still says which side it was.
fn label(found: &str, fallback: &'static str) -> String {
    match found.is_empty() {
        true => fallback.to_owned(),
        false => found.to_owned(),
    }
}

fn hint(message: &str) -> Element<'_, Message> {
    container(text(message.to_owned()).size(13).style(muted))
        .center_x(Fill)
        .center_y(Fill)
        .padding([0, 40])
        .style(style::canvas)
        .into()
}

fn muted(theme: &Theme) -> text::Style {
    text::Style {
        color: Some(style::muted(theme)),
    }
}
