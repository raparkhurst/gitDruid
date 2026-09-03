//! The diff pane: a file's hunks, each stageable on its own.

use iced::widget::{Column, button, column, container, row, scrollable, text};
use iced::{Element, Fill, Font, Theme, Top};

use crate::app::Message;
use crate::git::{Content, FileDiff, Hunk, Line, Origin, Side, Source};
use crate::ui::style;

/// Rendering a very large diff costs more than it is worth, so past this many
/// lines the remaining hunks are summarised instead of drawn.
const LINE_BUDGET: usize = 3_000;

const CODE_SIZE: f32 = 13.0;
const GUTTER_WIDTH: f32 = 92.0;

pub fn view(diff: &FileDiff) -> Element<'_, Message> {
    column![header(diff), body(diff)]
        .width(Fill)
        .height(Fill)
        .into()
}

/// Shown in place of the pane when nothing is selected.
pub fn placeholder(hint: &str) -> Element<'static, Message> {
    container(
        text(hint.to_owned())
            .size(14)
            .style(|theme: &Theme| text::Style {
                color: Some(style::muted(theme)),
            }),
    )
    .center_x(Fill)
    .center_y(Fill)
    .style(style::canvas)
    .into()
}

fn header(diff: &FileDiff) -> Element<'_, Message> {
    let (added, removed) = diff.counts();

    let counts = row![
        text(format!("+{added}"))
            .size(12)
            .font(Font::MONOSPACE)
            .style(|theme: &Theme| {
                text::Style {
                    color: Some(theme.extended_palette().success.base.color),
                }
            }),
        text(format!("−{removed}"))
            .size(12)
            .font(Font::MONOSPACE)
            .style(|theme: &Theme| {
                text::Style {
                    color: Some(theme.extended_palette().danger.base.color),
                }
            }),
    ]
    .spacing(10);

    // The centre column swaps between the graph and whatever was picked out of
    // it, so every replacement of the graph carries the way back — to the
    // commit when the diff came from one, since that is where it was picked.
    let (label, target) = match &diff.source {
        Source::Working(_) => ("← History", Message::ShowHistory),
        Source::Commit(_) => ("← Commit", Message::ShowCommit),
    };

    let back = button(text(label).size(11))
        .padding([3, 8])
        .style(style::toggle)
        .on_press(target);

    let mut line = row![
        back,
        column![
            text(title(diff)).size(14),
            text(subtitle(diff))
                .size(11)
                .style(|theme: &Theme| text::Style {
                    color: Some(style::muted(theme))
                }),
        ]
        .spacing(2)
        .width(Fill),
        counts,
    ]
    .spacing(14)
    .align_y(iced::Center);

    // Nothing can be staged out of a commit: the change already happened.
    if let Some(side) = diff.source.side() {
        line = line.push(
            button(text(file_action_label(side)).size(12))
                .padding([4, 10])
                .style(style::toggle)
                .on_press(Message::ToggleFile(side, diff.path.clone())),
        );
    }

    container(line)
        .padding([10, 14])
        .width(Fill)
        .style(style::panel)
        .into()
}

fn subtitle(diff: &FileDiff) -> String {
    match &diff.source {
        Source::Working(side) => {
            format!("{} · {}", side.title().to_lowercase(), diff.change.label())
        }
        Source::Commit(id) => format!("in {:.7} · {}", id, diff.change.label()),
    }
}

fn title(diff: &FileDiff) -> String {
    match &diff.old_path {
        Some(old) if old != &diff.path => {
            format!("{} → {}", old.display(), diff.path.display())
        }
        _ => diff.path.display().to_string(),
    }
}

fn file_action_label(side: Side) -> &'static str {
    match side {
        Side::Worktree => "Stage file",
        Side::Index => "Unstage file",
    }
}

fn hunk_action_label(side: Side) -> &'static str {
    match side {
        Side::Worktree => "Stage hunk",
        Side::Index => "Unstage hunk",
    }
}

fn body(diff: &FileDiff) -> Element<'_, Message> {
    let content: Element<'_, Message> = match &diff.content {
        Content::Binary => {
            note("Binary file — gitDruid can stage it whole, but has no lines to show.")
        }
        Content::Empty => note("No line changes. Only the file's mode differs."),
        Content::Text(hunks) => hunk_list(diff, hunks),
    };

    container(scrollable(content).width(Fill).height(Fill))
        .style(style::canvas)
        .height(Fill)
        .into()
}

fn hunk_list<'a>(diff: &'a FileDiff, hunks: &'a [Hunk]) -> Element<'a, Message> {
    let mut list = Column::new().width(Fill).spacing(14).padding([8, 0]);

    let mut budget = LINE_BUDGET;
    let mut drawn = 0;

    for (index, hunk) in hunks.iter().enumerate() {
        // Always draw the first hunk, however large, so a single huge hunk is
        // never replaced entirely by a summary.
        if budget == 0 && index > 0 {
            break;
        }

        list = list.push(hunk_view(diff, hunk, index));

        budget = budget.saturating_sub(hunk.lines.len());
        drawn += 1;
    }

    if drawn < hunks.len() {
        list = list.push(note(format!(
            "{} more hunk(s) hidden to keep scrolling responsive — stage the whole file, or commit in \
             smaller pieces.",
            hunks.len() - drawn
        )));
    }

    list.into()
}

fn hunk_view<'a>(diff: &'a FileDiff, hunk: &'a Hunk, index: usize) -> Element<'a, Message> {
    let mut header = row![
        text(hunk.header.clone())
            .size(12)
            .font(Font::MONOSPACE)
            .width(Fill),
    ]
    .spacing(12)
    .align_y(iced::Center);

    if let Some(side) = diff.source.side() {
        header = header.push(
            button(text(hunk_action_label(side)).size(11))
                .padding([3, 8])
                .style(style::toggle)
                .on_press(Message::ToggleHunk(index)),
        );
    }

    let bar = container(header)
    .padding([5, 12])
    .width(Fill)
    .style(style::hunk_header);

    let mut lines = Column::new().width(Fill);

    for line in &hunk.lines {
        lines = lines.push(line_view(line));
    }

    column![bar, lines].width(Fill).into()
}

fn line_view(line: &Line) -> Element<'_, Message> {
    let background = match line.origin {
        Origin::Addition => style::addition,
        Origin::Deletion => style::deletion,
        _ => style::context,
    };

    let body = row![
        text(gutter(line))
            .size(CODE_SIZE)
            .font(Font::MONOSPACE)
            .width(GUTTER_WIDTH)
            .style(|theme: &Theme| text::Style {
                color: Some(style::muted(theme))
            }),
        text(format!("{}{}", line.origin.sign(), line.text()))
            .size(CODE_SIZE)
            .font(Font::MONOSPACE)
            .width(Fill),
    ]
    .spacing(8)
    .align_y(Top);

    container(body)
        .width(Fill)
        .padding([1, 10])
        .style(background)
        .into()
}

/// The old and new line numbers, right-aligned into two fixed columns.
fn gutter(line: &Line) -> String {
    if line.origin == Origin::NoNewline {
        return String::new();
    }

    let old = line.old_lineno.map(|n| n.to_string()).unwrap_or_default();
    let new = line.new_lineno.map(|n| n.to_string()).unwrap_or_default();

    format!("{old:>5} {new:>5}")
}

fn note(message: impl Into<String>) -> Element<'static, Message> {
    container(
        text(message.into())
            .size(12)
            .style(|theme: &Theme| text::Style {
                color: Some(style::muted(theme)),
            }),
    )
    .padding(16)
    .width(Fill)
    .into()
}
