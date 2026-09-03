//! One commit out of the graph, read-only.
//!
//! This is deliberately not the staging diff: nothing here can be staged, so
//! nothing here offers to. It answers "what did this commit do", and gives the
//! two things worth doing from a point in history — branching and tagging.

use iced::widget::{Column, button, column, container, row, rule, scrollable, text};
use iced::{Center, Element, Fill, Font, Theme};

use crate::app::{Message, PromptKind, Repo};
use crate::git::{Change, ChangedFile, CommitDetail};
use crate::ui::style;

pub fn view<'a>(repo: &'a Repo, detail: &'a CommitDetail) -> Element<'a, Message> {
    column![header(detail), rule::horizontal(1), body(repo, detail)]
        .width(Fill)
        .height(Fill)
        .into()
}

pub fn loading() -> Element<'static, Message> {
    container(text("Reading commit…").size(13).style(muted))
        .center_x(Fill)
        .center_y(Fill)
        .style(style::canvas)
        .into()
}

fn header(detail: &CommitDetail) -> Element<'_, Message> {
    let back = button(text("← History").size(11))
        .padding([3, 8])
        .style(style::toggle)
        .on_press(Message::ShowHistory);

    let branch_here = button(text("Branch here").size(11))
        .padding([3, 8])
        .style(style::toggle)
        .on_press(Message::Ask(
            PromptKind::NewBranch,
            String::new(),
            Some(detail.id.clone()),
        ));

    let tag_here = button(text("Tag here").size(11))
        .padding([3, 8])
        .style(style::toggle)
        .on_press(Message::Ask(
            PromptKind::NewTag,
            String::new(),
            Some(detail.id.clone()),
        ));

    container(
        row![
            back,
            text(&detail.short_id)
                .size(13)
                .font(Font::MONOSPACE)
                .width(Fill),
            branch_here,
            tag_here,
        ]
        .spacing(8)
        .align_y(Center),
    )
    .padding([8, 10])
    .width(Fill)
    .style(style::panel)
    .into()
}

fn body<'a>(repo: &'a Repo, detail: &'a CommitDetail) -> Element<'a, Message> {
    let mut content = Column::new().spacing(10).padding([10, 12]);

    content = content.push(text(&detail.message).size(13));

    let mut facts = format!("{} <{}>  ·  {}", detail.author, detail.email, detail.when);

    if detail.parents.len() > 1 {
        // A merge is worth calling out: its diff is against the first parent
        // only, so the file list below is not the whole story.
        facts.push_str(&format!(
            "  ·  merge of {}",
            detail.parents.join(" and ")
        ));
    }

    content = content.push(text(facts).size(11).style(muted));
    content = content.push(rule::horizontal(1));

    let summary = match detail.files.len() {
        0 => "No files changed".to_owned(),
        1 => "1 file changed".to_owned(),
        count => format!("{count} files changed"),
    };

    content = content.push(text(summary).size(11).style(muted));

    for file in &detail.files {
        content = content.push(file_row(repo, file));
    }

    if detail.truncated {
        content = content.push(
            text("More files were changed than are listed here.")
                .size(11)
                .style(muted),
        );
    }

    container(scrollable(content).width(Fill).height(Fill))
        .width(Fill)
        .height(Fill)
        .style(style::canvas)
        .into()
}

fn file_row<'a>(repo: &'a Repo, file: &'a ChangedFile) -> Element<'a, Message> {
    let change = file.change;

    let selected = repo
        .detail_file
        .as_ref()
        .is_some_and(|open| open.path == file.path);

    let line = row![
        text(change.badge())
            .size(12)
            .font(Font::MONOSPACE)
            .width(14)
            .style(move |theme: &Theme| text::Style {
                color: Some(badge_color(theme, change)),
            }),
        text(file.display()).size(12).width(Fill),
        text(format!("+{}", file.added))
            .size(11)
            .font(Font::MONOSPACE)
            .style(|theme: &Theme| text::Style {
                color: Some(theme.extended_palette().success.base.color),
            }),
        text(format!("−{}", file.removed))
            .size(11)
            .font(Font::MONOSPACE)
            .style(|theme: &Theme| text::Style {
                color: Some(theme.extended_palette().danger.base.color),
            }),
    ]
    .spacing(8)
    .align_y(Center);

    // Every file here has a diff behind it, so every row opens one.
    button(line)
        .padding([3, 6])
        .width(Fill)
        .style(style::row(selected))
        .on_press(Message::SelectCommitFile(file.clone()))
        .into()
}

pub fn loading_file(file: &ChangedFile) -> Element<'_, Message> {
    container(
        column![
            text(format!("Reading {}…", file.display())).size(13),
            button(text("← Commit").size(11))
                .padding([3, 8])
                .style(style::toggle)
                .on_press(Message::ShowCommit),
        ]
        .spacing(12)
        .align_x(Center),
    )
    .center_x(Fill)
    .center_y(Fill)
    .style(style::canvas)
    .into()
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

fn muted(theme: &Theme) -> text::Style {
    text::Style {
        color: Some(style::muted(theme)),
    }
}
