//! The left sidebar: the file list, its two tabs, and the commit box.

use iced::widget::{
    Column, button, column, container, mouse_area, row, rule, scrollable, text, text_editor,
};
use iced::{Center, Element, Fill, FillPortion, Font, Padding, Theme};

use crate::app::{Message, Repo, Target};
use crate::git::{Change, FileEntry, Side};
use crate::ui::style;

pub fn view(repo: &Repo, busy: bool) -> Element<'_, Message> {
    let content = column![
        tab_bar(repo),
        rule::horizontal(1),
        file_list(repo),
        bulk_bar(repo, busy),
        rule::horizontal(1),
        commit_box(repo, busy),
    ]
    .width(Fill)
    .height(Fill);

    container(content)
        .width(320)
        .height(Fill)
        .style(style::panel)
        .into()
}

/// The two lists share the sidebar's whole height by taking turns, so a
/// repository with a hundred changed files still scrolls in one list rather
/// than two half-height ones.
fn tab_bar(repo: &Repo) -> Element<'_, Message> {
    container(
        row![tab(repo, Side::Worktree), tab(repo, Side::Index)]
            .spacing(6)
            .align_y(Center),
    )
    .padding([8, 10])
    .width(Fill)
    .into()
}

/// The bulk action, at the foot of the list it acts on.
///
/// It says what it will do rather than what it could do: with files marked it
/// names how many, and without any it says "all". Either way the count is on
/// the button, so nothing has to be counted by eye first.
fn bulk_bar(repo: &Repo, busy: bool) -> Element<'_, Message> {
    let side = repo.tab;
    let total = repo.snapshot.entries(side).len();
    let marked = repo.marked.len();

    let verb = match side {
        Side::Worktree => "Stage",
        Side::Index => "Unstage",
    };

    let label = match marked {
        0 => format!("{verb} all ({total})"),
        count => format!("{verb} {count} selected"),
    };

    let ready = !busy && total > 0;

    let mut line = row![
        button(text(label).size(11).width(Fill).align_x(Center))
            .padding([4, 8])
            .width(Fill)
            .style(style::toggle)
            .on_press_maybe(ready.then_some(Message::ToggleMany(side))),
    ]
    .spacing(4)
    .align_y(Center);

    if marked > 0 {
        line = line.push(
            button(text("Clear").size(11))
                .padding([4, 8])
                .style(style::toggle)
                .on_press(Message::ClearMarks),
        );
    }

    container(line)
        .padding(Padding::default().left(10).right(10).top(6).bottom(6))
        .width(Fill)
        .into()
}

fn tab(repo: &Repo, side: Side) -> Element<'_, Message> {
    let count = repo.snapshot.entries(side).len();

    button(
        text(format!("{} ({count})", side.title()))
            .size(12)
            .width(Fill)
            .align_x(Center),
    )
    .padding([4, 6])
    .width(FillPortion(1))
    .style(style::tab(repo.tab == side))
    .on_press(Message::SelectTab(side))
    .into()
}

fn file_list(repo: &Repo) -> Element<'_, Message> {
    let entries = repo.snapshot.entries(repo.tab);

    let list: Element<'_, Message> = if entries.is_empty() {
        container(
            text(empty_hint(repo.tab))
                .size(12)
                .style(|theme: &Theme| text::Style {
                    color: Some(style::muted(theme)),
                }),
        )
        .padding([8, 10])
        .into()
    } else {
        let mut rows = Column::new().spacing(2).padding([6, 6]);

        for entry in entries {
            rows = rows.push(file_row(repo, entry));
        }

        scrollable(rows).width(Fill).height(Fill).into()
    };

    container(list).width(Fill).height(Fill).into()
}

fn empty_hint(side: Side) -> &'static str {
    match side {
        Side::Worktree => "The working tree is clean.",
        Side::Index => "Nothing staged yet.",
    }
}

fn file_row<'a>(repo: &'a Repo, entry: &'a FileEntry) -> Element<'a, Message> {
    // Two different things: what a bulk action would touch, and whose diff is
    // on screen. They usually coincide, and when they do not the difference
    // matters, so they are drawn differently.
    let marked = repo.marked.contains(&entry.path);

    let showing = repo
        .selection
        .as_ref()
        .is_some_and(|selection| selection.side == entry.side && selection.path == entry.path);

    let toggle = button(
        text(toggle_glyph(entry.side))
            .size(12)
            .font(Font::MONOSPACE),
    )
    .padding([2, 7])
    .style(style::toggle)
    .on_press(Message::ToggleFile(entry.side, entry.path.clone()));

    let label = button(
        row![
            text(entry.change.badge())
                .size(12)
                .font(Font::MONOSPACE)
                .width(14)
                .style(move |theme: &Theme| text::Style {
                    color: Some(badge_color(theme, entry.change))
                }),
            text(entry.display()).size(13).width(Fill),
        ]
        .spacing(6)
        .align_y(Center),
    )
    .padding([4, 6])
    .width(Fill)
    .style(style::file_row(marked, showing))
    .on_press(Message::Select(entry.side, entry.path.clone()));

    // The whole row is the right-click target, including the stage button, so
    // there is no sliver of it that does nothing.
    mouse_area(row![toggle, label].spacing(4).align_y(Center))
        .on_right_press(Message::OpenMenu(Target::File(
            entry.side,
            entry.path.clone(),
        )))
        .into()
}

fn toggle_glyph(side: Side) -> &'static str {
    match side {
        Side::Worktree => "+",
        Side::Index => "−",
    }
}

fn badge_color(theme: &Theme, change: Change) -> iced::Color {
    let palette = theme.extended_palette();

    match change {
        Change::Added | Change::Untracked => palette.success.base.color,
        Change::Deleted | Change::Conflicted => palette.danger.base.color,
        Change::Modified | Change::TypeChange => palette.warning.base.color,
        Change::Renamed => palette.primary.base.color,
    }
}

fn commit_box(repo: &Repo, busy: bool) -> Element<'_, Message> {
    let staged = repo.snapshot.staged.len();
    let has_message = !repo.message.text().trim().is_empty();

    let ready = !busy && staged > 0 && has_message && repo.snapshot.pending_operation.is_none();

    let editor = text_editor(&repo.message)
        .placeholder("Commit message")
        .height(96)
        .padding(8)
        .size(13)
        .style(style::editor)
        .on_action(Message::EditMessage);

    let summary = match staged {
        0 => "Nothing staged".to_owned(),
        1 => "1 file staged".to_owned(),
        count => format!("{count} files staged"),
    };

    let action = button(text("Commit").size(12))
        .padding([6, 16])
        .style(style::primary)
        .on_press_maybe(ready.then_some(Message::Commit));

    column![
        container(editor).padding([8, 10]),
        container(
            row![
                text(summary)
                    .size(11)
                    .width(Fill)
                    .style(|theme: &Theme| text::Style {
                        color: Some(style::muted(theme))
                    }),
                action,
            ]
            .spacing(8)
            .align_y(Center)
        )
        .padding([0, 10]),
    ]
    .spacing(8)
    .padding(Padding::default().bottom(10))
    .width(Fill)
    .into()
}
