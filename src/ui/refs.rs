//! The left sidebar: what refs the repository has, and what can be done to
//! them.
//!
//! Selecting a ref and acting on it are two steps rather than one. A row of
//! four buttons on every branch does not fit a sidebar, and it puts "Delete"
//! under the pointer of anyone reaching for "Check out".

use iced::widget::{Column, button, column, container, mouse_area, row, rule, scrollable, text};
use iced::{Center, Element, Fill, Font, Padding, Theme};

use crate::app::{Message, PromptKind, RefTarget, Repo, Target};
use crate::git::{Branch, Tag};
use crate::ui::style;

pub fn view(repo: &Repo, busy: bool) -> Element<'_, Message> {
    let mut content = column![heading(repo, busy)].width(Fill).height(Fill);

    content = content.push(rule::horizontal(1)).push(action_bar(repo, busy));

    let mut lists = Column::new().spacing(2).padding([6, 6]);

    lists = lists.push(section_title("Branches"));

    if repo.refs.local.is_empty() {
        lists = lists.push(empty("No branches yet."));
    }

    for branch in &repo.refs.local {
        lists = lists.push(branch_row(repo, branch, RefTarget::Local(branch.name.clone())));
    }

    if !repo.refs.remote.is_empty() {
        lists = lists.push(section_title("Remotes"));

        for branch in &repo.refs.remote {
            lists = lists.push(branch_row(
                repo,
                branch,
                RefTarget::Remote(branch.name.clone()),
            ));
        }
    }

    lists = lists.push(section_title("Tags"));

    if repo.refs.tags.is_empty() {
        lists = lists.push(empty("No tags yet."));
    }

    for tag in &repo.refs.tags {
        lists = lists.push(tag_row(repo, tag));
    }

    if !repo.refs.stashes.is_empty() {
        lists = lists.push(section_title("Stashes"));

        for stash in &repo.refs.stashes {
            lists = lists.push(stash_row(repo, stash));
        }
    }

    content = content.push(scrollable(lists).width(Fill).height(Fill));

    container(content)
        .width(250)
        .height(Fill)
        .style(style::panel)
        .into()
}

fn heading(repo: &Repo, busy: bool) -> Element<'_, Message> {
    // Both create at HEAD, which is the only start point available without a
    // commit selected in the graph.
    let ready = !busy && !repo.history.commits.is_empty();

    let new_branch = button(text("+ Branch").size(11))
        .padding([3, 8])
        .style(style::toggle)
        .on_press_maybe(
            ready.then(|| Message::Ask(PromptKind::NewBranch, String::new(), None)),
        );

    let new_tag = button(text("+ Tag").size(11))
        .padding([3, 8])
        .style(style::toggle)
        .on_press_maybe(ready.then(|| Message::Ask(PromptKind::NewTag, String::new(), None)));

    container(
        row![text("REFS").size(11).style(muted).width(Fill), new_branch, new_tag]
            .spacing(6)
            .align_y(Center),
    )
    .padding([8, 10])
    .width(Fill)
    .into()
}

/// What can be done to the selected ref. Empty when nothing is selected, so
/// the buttons never act on something the user cannot see.
fn action_bar(repo: &Repo, busy: bool) -> Element<'_, Message> {
    let Some(target) = &repo.selected_ref else {
        return container(
            text("Select a branch or tag to act on it.")
                .size(11)
                .style(muted),
        )
        .padding([7, 10])
        .width(Fill)
        .into();
    };

    let enabled = !busy;

    let buttons: Vec<Element<'_, Message>> = match target {
        RefTarget::Local(name) => {
            let is_head = repo
                .refs
                .local
                .iter()
                .any(|branch| branch.name == *name && branch.is_head);

            let merged = repo
                .refs
                .local
                .iter()
                .find(|branch| branch.name == *name)
                .is_some_and(|branch| branch.merged);

            let flow = repo.settings.flow();

            // Offered only for a branch whose prefix says where it goes; for
            // anything else "finish" would have to guess.
            let finish = flow.kind_of(name).map(|kind| {
                (
                    format!("Finish → {}", flow.merges_into(kind)),
                    Message::Ask(PromptKind::Finish, name.clone(), None),
                )
            });

            let mut buttons = vec![
                small(
                    "Check out",
                    (enabled && !is_head).then(|| Message::Checkout(name.clone())),
                    false,
                ),
                small(
                    "Merge",
                    (enabled && !is_head)
                        .then(|| Message::Ask(PromptKind::Merge, name.clone(), None)),
                    false,
                ),
                small(
                    "Rename",
                    enabled.then(|| Message::Ask(PromptKind::RenameBranch, name.clone(), None)),
                    false,
                ),
                small(
                    "Delete",
                    (enabled && !is_head).then(|| {
                        Message::Ask(
                            PromptKind::DeleteBranch { force: !merged },
                            name.clone(),
                            None,
                        )
                    }),
                    true,
                ),
            ];

            if let Some((label, message)) = finish {
                buttons.push(owned(label, enabled.then_some(message), false));
            }

            buttons
        }
        RefTarget::Remote(name) => vec![small(
            "Branch from here",
            enabled.then(|| Message::Ask(PromptKind::NewBranch, String::new(), Some(name.clone()))),
            false,
        )],
        RefTarget::Tag(name) => vec![small(
            "Delete tag",
            enabled.then(|| Message::Ask(PromptKind::DeleteTag, name.clone(), None)),
            true,
        )],
        RefTarget::Stash(index) => {
            let index = *index;

            vec![
                // Pop first: putting the work back and also keeping a copy of
                // it is the rarer thing to want.
                small("Pop", enabled.then_some(Message::ApplyStash(index, true)), false),
                small(
                    "Apply",
                    enabled.then_some(Message::ApplyStash(index, false)),
                    false,
                ),
                small(
                    "Drop",
                    enabled
                        .then(|| Message::Ask(PromptKind::DropStash(index), String::new(), None)),
                    true,
                ),
            ]
        }
    };

    // Two to a line: four of these will not fit across a 250px sidebar.
    let mut lines = Column::new().spacing(4);
    let mut current = row![].spacing(4);
    let mut count = 0;

    for element in buttons {
        current = current.push(element);
        count += 1;

        if count % 2 == 0 {
            lines = lines.push(current);
            current = row![].spacing(4);
        }
    }

    if count % 2 != 0 {
        lines = lines.push(current);
    }

    container(lines).padding([6, 10]).width(Fill).into()
}

fn small(label: &str, message: Option<Message>, destructive: bool) -> Element<'_, Message> {
    owned(label.to_owned(), message, destructive)
}

fn owned(
    label: String,
    message: Option<Message>,
    destructive: bool,
) -> Element<'static, Message> {
    let content = button(text(label).size(11).width(Fill).align_x(Center))
        .padding([3, 6])
        .width(Fill)
        .on_press_maybe(message);

    if destructive {
        content.style(style::danger).into()
    } else {
        content.style(style::toggle).into()
    }
}

fn branch_row<'a>(repo: &'a Repo, branch: &'a Branch, target: RefTarget) -> Element<'a, Message> {
    let selected = repo.selected_ref.as_ref() == Some(&target);

    let mut line = row![].spacing(6).align_y(Center);

    if branch.is_head {
        line = line.push(
            text("●")
                .size(9)
                .style(|theme: &Theme| text::Style {
                    color: Some(theme.extended_palette().success.base.color),
                }),
        );
    }

    line = line.push(
        text(&branch.name)
            .size(12)
            .width(Fill)
            .wrapping(text::Wrapping::None),
    );

    if branch.ahead > 0 || branch.behind > 0 {
        line = line.push(
            text(format!("↑{} ↓{}", branch.ahead, branch.behind))
                .size(10)
                .font(Font::MONOSPACE)
                .style(muted),
        );
    }

    mouse_area(
        button(line)
            .padding([4, 6])
            .width(Fill)
            .style(style::row(selected))
            .on_press(Message::SelectRef(target.clone())),
    )
    .on_right_press(Message::OpenMenu(Target::Ref(target)))
    .into()
}

fn tag_row<'a>(repo: &'a Repo, tag: &'a Tag) -> Element<'a, Message> {
    let target = RefTarget::Tag(tag.name.clone());
    let selected = repo.selected_ref.as_ref() == Some(&target);

    mouse_area(
        button(
            row![
                text(&tag.name)
                    .size(12)
                    .width(Fill)
                    .wrapping(text::Wrapping::None),
                text(&tag.short_id)
                    .size(10)
                    .font(Font::MONOSPACE)
                    .style(muted),
            ]
            .spacing(6)
            .align_y(Center),
        )
        .padding([4, 6])
        .width(Fill)
        .style(style::row(selected))
        .on_press(Message::SelectRef(target.clone())),
    )
    .on_right_press(Message::OpenMenu(Target::Ref(target)))
    .into()
}

fn stash_row<'a>(repo: &'a Repo, stash: &'a crate::git::Stash) -> Element<'a, Message> {
    let target = RefTarget::Stash(stash.index);
    let selected = repo.selected_ref.as_ref() == Some(&target);

    // git writes "WIP on main: 1234567 subject", which is mostly boilerplate;
    // what the person typed, or what they were on, is the end of it.
    let label = stash
        .message
        .split_once(": ")
        .map(|(_, rest)| rest.to_owned())
        .unwrap_or_else(|| stash.message.clone());

    mouse_area(
        button(
            row![
                text(format!("{}", stash.index))
                    .size(10)
                    .font(Font::MONOSPACE)
                    .width(14)
                    .style(muted),
                text(label)
                    .size(12)
                    .width(Fill)
                    .wrapping(text::Wrapping::None),
            ]
            .spacing(6)
            .align_y(Center),
        )
        .padding([4, 6])
        .width(Fill)
        .style(style::row(selected))
        .on_press(Message::SelectRef(target.clone())),
    )
    .on_right_press(Message::OpenMenu(Target::Ref(target)))
    .into()
}

fn section_title(label: &str) -> Element<'_, Message> {
    container(text(label.to_uppercase()).size(10).style(muted))
        .padding(Padding::default().top(8).bottom(2).left(4).right(4))
        .into()
}

fn empty(label: &str) -> Element<'_, Message> {
    container(text(label.to_owned()).size(11).style(muted))
        .padding([2, 6])
        .into()
}

fn muted(theme: &Theme) -> text::Style {
    text::Style {
        color: Some(style::muted(theme)),
    }
}
