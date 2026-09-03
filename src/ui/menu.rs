//! The right-click menu.
//!
//! iced has no context menu, so this is one built out of a `stack`: a
//! transparent sheet over the whole window that swallows the next click, and
//! the menu itself above it, offset by padding to sit under the pointer. The
//! sheet is what makes clicking anywhere else dismiss it.

use std::collections::BTreeSet;
use std::path::PathBuf;

use iced::widget::{Column, button, container, mouse_area, opaque, row, rule, stack, text};
use iced::{Element, Fill, Padding, Point, Size, Theme};

use crate::app::{GitDruid, Menu, Message, PromptKind, RefTarget, Repo, Target};
use crate::git::{self, Change, Ignore, Side};
use crate::settings::Flow;
use crate::ui::style;

const WIDTH: f32 = 236.0;
const ITEM_HEIGHT: f32 = 25.0;
const SEPARATOR_HEIGHT: f32 = 7.0;
const PADDING: f32 = 8.0;

/// Puts `screen` under a menu.
pub fn overlay<'a>(
    state: &'a GitDruid,
    menu: &'a Menu,
    screen: Element<'a, Message>,
) -> Element<'a, Message> {
    let Some(repo) = state.active() else {
        return screen;
    };

    let flow = repo.settings.flow();
    let items = items(&Context::of(repo, &flow), &menu.target);

    if items.is_empty() {
        return screen;
    }

    let at = place(menu.at, &items, state.window);

    let sheet = opaque(
        mouse_area(container(text("")).width(Fill).height(Fill))
            .on_press(Message::CloseMenu)
            .on_right_press(Message::CloseMenu),
    );

    let panel = container(container(draw(items)).width(WIDTH).style(style::dialog))
        .padding(Padding::default().left(at.x).top(at.y))
        .width(Fill)
        .height(Fill);

    stack![screen, sheet, panel].into()
}

/// Keeps the menu inside the window, so one opened near an edge still opens
/// somewhere it can be read.
fn place(at: Point, items: &[Item], window: Size) -> Point {
    let height: f32 = items
        .iter()
        .map(|item| match item {
            Item::Separator => SEPARATOR_HEIGHT,
            _ => ITEM_HEIGHT,
        })
        .sum::<f32>()
        + PADDING;

    Point::new(
        at.x.min(window.width - WIDTH - 8.0).max(0.0),
        at.y.min(window.height - height - 8.0).max(0.0),
    )
}

fn draw(items: Vec<Item>) -> Element<'static, Message> {
    let mut column = Column::new().padding([4, 4]).width(Fill);

    for item in items {
        column = column.push(match item {
            Item::Separator => container(rule::horizontal(1))
                .padding([3, 4])
                .width(Fill)
                .into(),
            Item::Heading(label) => container(
                text(label)
                    .size(10)
                    .style(|theme: &Theme| text::Style {
                        color: Some(style::muted(theme)),
                    }),
            )
            .padding([4, 7])
            .width(Fill)
            .into(),
            Item::Action {
                label,
                message,
                destructive,
            } => entry(label, message, destructive),
        });
    }

    column.into()
}

fn entry(label: String, message: Message, destructive: bool) -> Element<'static, Message> {
    button(
        row![text(label).size(11).width(Fill)]
            .align_y(iced::Center)
            .height(ITEM_HEIGHT - 2.0),
    )
    .padding([0, 7])
    .width(Fill)
    .style(match destructive {
        true => style::menu_danger,
        false => style::menu_item,
    })
    .on_press(message)
    .into()
}

/// Everything the menu needs to know about the repository it is over.
///
/// Named separately from `Repo` so that what goes in a menu can be decided —
/// and checked — without building an application around it.
pub struct Context<'a> {
    pub snapshot: &'a git::Snapshot,
    pub refs: &'a git::Refs,
    pub history: &'a git::History,
    pub flow: &'a Flow,
    /// The multi-selection the menu was opened over. A menu that offers to act
    /// on one file while several are highlighted is offering the wrong thing.
    pub marked: &'a BTreeSet<PathBuf>,
}

impl<'a> Context<'a> {
    fn of(repo: &'a Repo, flow: &'a Flow) -> Self {
        Self {
            snapshot: &repo.snapshot,
            refs: &repo.refs,
            history: &repo.history,
            flow,
            marked: &repo.marked,
        }
    }
}

pub enum Item {
    Heading(String),
    Separator,
    Action {
        label: String,
        message: Message,
        destructive: bool,
    },
}

impl Item {
    /// The text on the line, for a caller that wants to know what a menu
    /// offers without drawing it.
    pub fn label(&self) -> &str {
        match self {
            Item::Heading(label) => label,
            Item::Separator => "",
            Item::Action { label, .. } => label,
        }
    }

    pub fn is_destructive(&self) -> bool {
        matches!(self, Item::Action { destructive: true, .. })
    }
}

fn action(label: impl Into<String>, message: Message) -> Item {
    Item::Action {
        label: label.into(),
        message,
        destructive: false,
    }
}

fn dangerous(label: impl Into<String>, message: Message) -> Item {
    Item::Action {
        label: label.into(),
        message,
        destructive: true,
    }
}

/// What a right-click on `target` offers.
pub fn items(context: &Context<'_>, target: &Target) -> Vec<Item> {
    match target {
        Target::File(side, path) => file_items(context, *side, path),
        Target::Commit(id) => commit_items(context, id),
        Target::Ref(target) => ref_items(context, target),
    }
}

fn file_items(context: &Context<'_>, side: Side, path: &std::path::Path) -> Vec<Item> {
    let Some(entry) = context.snapshot.find(side, path) else {
        return Vec::new();
    };

    // The menu acts on the selection when the clicked row is part of one.
    // Opening it over a row outside the selection has already narrowed the
    // selection to that row, so this is only ever true for a real one.
    let selected = context
        .marked
        .iter()
        .filter(|marked| context.snapshot.find(side, marked).is_some())
        .count();

    let many = selected > 1 && context.marked.contains(path);

    let name = match many {
        true => format!("{selected} files selected"),
        false => path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string()),
    };

    let verb = match side {
        Side::Worktree => "Stage",
        Side::Index => "Unstage",
    };

    let mut items = vec![
        Item::Heading(name),
        match many {
            true => action(
                format!("{verb} these {selected} files"),
                Message::ToggleMany(side),
            ),
            false => action(
                format!("{verb} this file"),
                Message::ToggleFile(side, path.to_path_buf()),
            ),
        },
        action("Show the diff", Message::Select(side, path.to_path_buf())),
    ];

    // Ignoring is per-pattern rather than per-selection: three files rarely
    // want the same rule, and a wrong one is quiet until it bites.
    //
    // Ignoring something already tracked does nothing until it is removed from
    // the index, so the offer is only made for files git is not watching yet.
    if !many && entry.change == Change::Untracked {
        items.push(Item::Separator);

        for (label, scope) in [
            ("Ignore this file", Ignore::File),
            ("Ignore this extension", Ignore::Extension),
            ("Ignore this folder", Ignore::Folder),
        ] {
            if let Some(pattern) = crate::git::pattern(path, scope) {
                items.push(action(
                    format!("{label}  ({pattern})"),
                    Message::Ignore(pattern),
                ));
            }
        }
    }

    if side == Side::Worktree {
        items.push(Item::Separator);

        let label = match (many, entry.change) {
            (true, _) => format!("Discard changes to these {selected} files"),
            (false, Change::Untracked) => "Delete this file".to_owned(),
            (false, _) => "Discard these changes".to_owned(),
        };

        items.push(dangerous(
            label,
            Message::Ask(
                PromptKind::Discard { side },
                path.display().to_string(),
                None,
            ),
        ));
    }

    items
}

fn commit_items(context: &Context<'_>, id: &str) -> Vec<Item> {
    let short = format!("{id:.7}");

    let summary = context
        .history
        .find(id)
        .map(|commit| commit.summary.clone())
        .unwrap_or_else(|| short.clone());

    vec![
        Item::Heading(summary),
        action("Show this commit", Message::SelectCommit(id.to_owned())),
        Item::Separator,
        action("Cherry-pick onto this branch", Message::CherryPick(id.to_owned())),
        action("Revert this commit", Message::Revert(id.to_owned())),
        Item::Separator,
        action(
            "Branch here",
            Message::Ask(PromptKind::NewBranch, String::new(), Some(id.to_owned())),
        ),
        action(
            "Tag here",
            Message::Ask(PromptKind::NewTag, String::new(), Some(id.to_owned())),
        ),
        Item::Separator,
        // The three differ only in what they take with them, so the labels
        // say that rather than naming the modes.
        action(
            "Reset here, keeping the changes staged",
            Message::Reset(id.to_owned(), git::Reset::Soft),
        ),
        action(
            "Reset here, keeping the changes",
            Message::Reset(id.to_owned(), git::Reset::Mixed),
        ),
        dangerous(
            "Reset here, discarding the changes",
            Message::Ask(PromptKind::ResetHard, id.to_owned(), None),
        ),
        Item::Separator,
        action(format!("Copy id  ({short})"), Message::CopyText(id.to_owned())),
    ]
}

fn ref_items(context: &Context<'_>, target: &RefTarget) -> Vec<Item> {
    let name = target.name().to_owned();

    match target {
        RefTarget::Local(_) => {
            let is_head = context
                .refs
                .local
                .iter()
                .any(|branch| branch.name == name && branch.is_head);

            let merged = context
                .refs
                .local
                .iter()
                .find(|branch| branch.name == name)
                .is_some_and(|branch| branch.merged);

            let flow = context.flow;

            let mut items = vec![Item::Heading(name.clone())];

            if !is_head {
                items.push(action("Check out", Message::Checkout(name.clone())));
                items.push(action(
                    "Merge into this branch",
                    Message::Ask(PromptKind::Merge, name.clone(), None),
                ));
            }

            if let Some(kind) = flow.kind_of(&name) {
                items.push(action(
                    format!("Finish into {}", flow.merges_into(kind)),
                    Message::Ask(PromptKind::Finish, name.clone(), None),
                ));
            }

            items.push(Item::Separator);
            items.push(action(
                "Rename…",
                Message::Ask(PromptKind::RenameBranch, name.clone(), None),
            ));

            if !is_head {
                items.push(dangerous(
                    "Delete",
                    Message::Ask(
                        PromptKind::DeleteBranch { force: !merged },
                        name.clone(),
                        None,
                    ),
                ));
            }

            items
        }
        RefTarget::Remote(_) => vec![
            Item::Heading(name.clone()),
            action(
                "Branch from here",
                Message::Ask(PromptKind::NewBranch, String::new(), Some(name)),
            ),
        ],
        RefTarget::Tag(_) => vec![
            Item::Heading(name.clone()),
            dangerous(
                "Delete tag",
                Message::Ask(PromptKind::DeleteTag, name, None),
            ),
        ],
        RefTarget::Stash(index) => {
            let index = *index;

            vec![
                Item::Heading(format!("stash {index}")),
                action("Pop — put it back and remove it", Message::ApplyStash(index, true)),
                action("Apply — put it back and keep it", Message::ApplyStash(index, false)),
                Item::Separator,
                dangerous(
                    "Drop",
                    Message::Ask(PromptKind::DropStash(index), String::new(), None),
                ),
            ]
        }
    }
}
