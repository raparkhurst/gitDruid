//! The view layer. `view` is the only entry point the app calls.

pub mod commit;
pub mod diff;
pub mod files;
pub mod graph;
pub mod menu;
pub mod refs;
pub mod settings;
pub mod style;
pub mod theme;

use std::path::Path;

use iced::widget::{
    button, column, container, opaque, pick_list, row, rule, scrollable, stack, text, text_input,
};
use iced::{Center, Element, Fill, Padding, Theme};

use crate::app::{Focus, GitDruid, Message, Prompt, PromptKind, Repo};

/// Longest repository name a tab shows before it is cut.
const TAB_NAME: usize = 20;

pub fn view(state: &GitDruid) -> Element<'_, Message> {
    let body: Element<'_, Message> = match state.active() {
        Some(repo) => workspace(repo),
        None if !state.opening.is_empty() => opening(state),
        None => welcome(),
    };

    let mut screen = column![toolbar(state), rule::horizontal(1)];

    if let Some(notice) = &state.notice {
        screen = screen.push(notice_bar(notice));
    }

    // A question that is blocking an action belongs across the top of the
    // window, next to whichever button raised it, not tucked into a column.
    if let Some(repo) = state.active()
        && let Some(prompt) = &repo.prompt
    {
        screen = screen.push(prompt_bar(&repo.settings, prompt));
    }

    let screen = screen.push(body).width(Fill).height(Fill);

    // `opaque` stops clicks reaching the window behind, which is what makes
    // this a dialog rather than a panel drawn on top of a live app.
    if state.settings_open {
        return stack![screen, opaque(settings::view(state))].into();
    }

    match &state.menu {
        Some(open) => menu::overlay(state, open, screen.into()),
        None => screen.into(),
    }
}

fn toolbar(state: &GitDruid) -> Element<'_, Message> {
    let open = button(text("Open…").size(12))
        .padding([5, 12])
        .style(style::toggle)
        .on_press(Message::PickRepo);

    let busy = state.active().is_some_and(|repo| repo.busy);

    let refresh = button(text(if busy { "Working…" } else { "Refresh" }).size(12))
        .padding([5, 12])
        .style(style::toggle)
        .on_press_maybe((!busy && state.active().is_some()).then_some(Message::Refresh));

    let configure = button(text("Settings").size(12))
        .padding([5, 12])
        .style(style::toggle)
        .on_press(Message::OpenSettings);

    let themes = pick_list(theme::all(), Some(state.theme.clone()), Message::ThemeChanged)
        .text_size(11)
        .padding([4, 8]);

    let top = row![open, tab_bar(state), themes, configure, refresh]
        .spacing(8)
        .align_y(Center);

    let mut bar = column![top].spacing(6);

    // The active repository's branch and location, under its tab rather than
    // beside it: with several tabs open there is no room for both on one line,
    // and this line is about whichever one is in front.
    if let Some(repo) = state.active() {
        let mut details = row![branch_label(repo)].spacing(10).align_y(Center);

        if let Some(operation) = &repo.snapshot.pending_operation {
            details = details.push(text(format!("{operation} in progress")).size(12).style(
                |theme: &Theme| text::Style {
                    color: Some(theme.extended_palette().danger.base.color),
                },
            ));
        }

        details = details.push(remote_actions(repo));
        details = details.push(path_label(repo));

        bar = bar.push(details);
    }

    container(bar)
        .padding([8, 12])
        .width(Fill)
        .style(style::panel)
        .into()
}

/// One tab per open repository, plus one for each still being read.
///
/// The tabs scroll rather than shrink: a name squeezed to nothing is no use
/// for telling two checkouts apart, which is the only reason the tab is there.
fn tab_bar(state: &GitDruid) -> Element<'_, Message> {
    let mut tabs = row![].spacing(4).align_y(Center);

    for (index, repo) in state.repos.iter().enumerate() {
        tabs = tabs.push(repo_tab(repo, index == state.active));
    }

    for path in &state.opening {
        tabs = tabs.push(opening_tab(path));
    }

    scrollable(tabs)
        .direction(scrollable::Direction::Horizontal(
            scrollable::Scrollbar::new().width(4).scroller_width(4),
        ))
        .width(Fill)
        .into()
}

fn repo_tab(repo: &Repo, active: bool) -> Element<'_, Message> {
    let path = repo.path().to_path_buf();

    let name = button(
        text(clip(&repo.name(), TAB_NAME))
            .size(12)
            .wrapping(text::Wrapping::None),
    )
    .padding(Padding::default().top(4).bottom(4).left(9).right(3))
    .style(style::tab_label(active))
    .on_press(Message::SelectRepo(path.clone()));

    let close = button(text("✕").size(9))
        .padding(Padding::default().top(4).bottom(4).left(3).right(8))
        .style(style::tab_close(active))
        .on_press(Message::CloseRepo(path));

    chip(row![name, close], active)
}

/// A tab for a repository that is still being read. There is nothing to switch
/// to yet, so the name is inert and only the ✕ does anything.
fn opening_tab(path: &Path) -> Element<'_, Message> {
    let label = container(
        text(format!("{}…", clip(&directory(path), TAB_NAME)))
            .size(12)
            .wrapping(text::Wrapping::None)
            .style(muted),
    )
    .padding(Padding::default().top(4).bottom(4).left(9).right(3));

    let close = button(text("✕").size(9))
        .padding(Padding::default().top(4).bottom(4).left(3).right(8))
        .style(style::tab_close(false))
        .on_press(Message::CloseRepo(path.to_path_buf()));

    chip(row![label, close], false)
}

/// Paints the tab, once, behind whatever sits in it.
fn chip<'a>(
    content: iced::widget::Row<'a, Message>,
    active: bool,
) -> Element<'a, Message> {
    container(content.spacing(0).align_y(Center))
        .style(style::tab_chip(active))
        .into()
}

fn directory(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn clip(name: &str, limit: usize) -> String {
    if name.chars().count() <= limit {
        return name.to_owned();
    }

    let kept: String = name.chars().take(limit.saturating_sub(1)).collect();

    format!("{kept}…")
}

fn muted(theme: &Theme) -> text::Style {
    text::Style {
        color: Some(style::muted(theme)),
    }
}

/// Fetch, pull and push, with what each would move.
///
/// The counts are on the buttons rather than beside them: "Push ↑2" says both
/// what the button does and why it is worth pressing, in the width of a label.
fn remote_actions(repo: &Repo) -> Element<'_, Message> {
    let Some(tracking) = &repo.refs.tracking else {
        return text("no remote").size(11).style(muted).into();
    };

    let ready = !repo.busy;

    let pull = match tracking.behind {
        0 => "Pull".to_owned(),
        behind => format!("Pull ↓{behind}"),
    };

    let push = match tracking.ahead {
        0 => "Push".to_owned(),
        ahead => format!("Push ↑{ahead}"),
    };

    row![
        small("Fetch", ready.then_some(Message::Fetch)),
        small(
            pull,
            ready.then(|| Message::Ask(PromptKind::Pull, tracking.remote.clone(), None)),
        ),
        small(
            push,
            ready.then(|| Message::Ask(PromptKind::Push, tracking.remote.clone(), None)),
        ),
    ]
    .spacing(4)
    .align_y(Center)
    .into()
}

fn small(label: impl text::IntoFragment<'static>, message: Option<Message>) -> Element<'static, Message> {
    button(text(label).size(11))
        .padding([3, 8])
        .style(style::toggle)
        .on_press_maybe(message)
        .into()
}

/// The bar that asks for a name, or for a yes.
fn prompt_bar<'a>(
    config: &'a crate::settings::Settings,
    prompt: &'a Prompt,
) -> Element<'a, Message> {
    // The question sits at the left; the spacer pushes the answer to the right
    // edge, where the eye is already going for the buttons.
    let mut line = row![text(question(prompt)).size(12), text("").width(Fill)]
        .spacing(10)
        .align_y(Center);

    if prompt.kind.needs_name() {
        line = line.push(
            text_input(placeholder(prompt.kind), &prompt.value)
                .size(12)
                .padding([4, 6])
                .width(260)
                .style(style::input)
                .on_input(Message::PromptChanged)
                .on_submit(Message::PromptSubmit),
        );
    }

    let destructive = matches!(
        prompt.kind,
        PromptKind::DeleteBranch { .. } | PromptKind::DeleteTag | PromptKind::Discard { .. }
    );

    let confirm = button(text(verb(prompt.kind)).size(11))
        .padding([4, 12])
        .on_press(Message::PromptSubmit);

    let confirm: Element<'_, Message> = if destructive {
        confirm.style(style::danger).into()
    } else {
        confirm.style(style::primary).into()
    };

    let line = line.push(confirm).push(
        button(text("Cancel").size(11))
            .padding([4, 12])
            .style(style::toggle)
            .on_press(Message::PromptCancel),
    );

    // Which sort of branch this is changes both its name and where it starts,
    // so the choice belongs with the box the name is typed into.
    let body: Element<'_, Message> = match prompt.kind {
        PromptKind::NewBranch => match settings::flow_picker(config, prompt.flow, &prompt.value) {
            Some(picker) => column![line, picker].spacing(6).into(),
            None => line.into(),
        },
        _ => line.into(),
    };

    container(
        container(body)
            .padding([7, 10])
            .width(Fill)
            .style(style::prompt),
    )
    .padding([6, 10])
    .width(Fill)
    .into()
}

fn question(prompt: &Prompt) -> String {
    match prompt.kind {
        PromptKind::NewBranch => match &prompt.at {
            Some(at) => format!("New branch at {at}"),
            None => "New branch at HEAD".to_owned(),
        },
        PromptKind::RenameBranch => format!("Rename {}", prompt.subject),
        PromptKind::NewTag => match &prompt.at {
            Some(at) => format!("New tag at {at}"),
            None => "New tag at HEAD".to_owned(),
        },
        PromptKind::DeleteBranch { force: true } => format!(
            "{} has commits HEAD cannot reach. Deleting it loses them.",
            prompt.subject
        ),
        PromptKind::DeleteBranch { force: false } => {
            format!("Delete {}? It is already merged.", prompt.subject)
        }
        PromptKind::DeleteTag => format!("Delete tag {}?", prompt.subject),
        PromptKind::Merge => format!("Merge {} into the current branch?", prompt.subject),
        PromptKind::Discard { .. } => match prompt.paths.len() {
            0 | 1 => format!(
                "Discard the unstaged changes to {}? This cannot be undone.",
                prompt.subject
            ),
            count => format!(
                "Discard the unstaged changes to {count} files? This cannot be undone."
            ),
        },
        PromptKind::Finish => format!("Finish {}?", prompt.subject),
        PromptKind::Pull => format!("Fetch {} and merge it into this branch?", prompt.subject),
        PromptKind::Push => format!("Push this branch to {}?", prompt.subject),
    }
}

fn placeholder(kind: PromptKind) -> &'static str {
    match kind {
        PromptKind::NewTag => "Tag name",
        _ => "Branch name",
    }
}

fn verb(kind: PromptKind) -> &'static str {
    match kind {
        PromptKind::NewBranch => "Create",
        PromptKind::RenameBranch => "Rename",
        PromptKind::NewTag => "Tag",
        PromptKind::DeleteBranch { force: true } => "Delete anyway",
        PromptKind::DeleteBranch { force: false } => "Delete",
        PromptKind::DeleteTag => "Delete",
        PromptKind::Merge => "Merge",
        PromptKind::Discard { .. } => "Discard",
        PromptKind::Finish => "Finish",
        PromptKind::Pull => "Pull",
        PromptKind::Push => "Push",
    }
}

fn branch_label(repo: &Repo) -> Element<'_, Message> {
    let head = &repo.snapshot.head;

    let mut label = if head.detached {
        format!("detached at {}", head.label)
    } else {
        head.label.clone()
    };

    if head.unborn {
        label.push_str(" (no commits yet)");
    }

    if let Some(upstream) = &head.upstream
        && (upstream.ahead > 0 || upstream.behind > 0)
    {
        label.push_str(&format!(" ↑{} ↓{}", upstream.ahead, upstream.behind));
    }

    text(label)
        .size(12)
        .style(|theme: &Theme| text::Style {
            color: Some(style::muted(theme)),
        })
        .into()
}

/// Where the open repository actually lives. Two clones share a directory name,
/// so the name alone does not say which checkout is on screen.
fn path_label(repo: &Repo) -> Element<'_, Message> {
    text(abbreviate(&repo.snapshot.path))
        .size(11)
        .wrapping(text::Wrapping::None)
        .style(|theme: &Theme| text::Style {
            color: Some(style::muted(theme)),
        })
        .into()
}

/// Writes a path under the home directory as `~/…`, and drops the trailing
/// separator libgit2 puts on a working directory.
fn abbreviate(path: &Path) -> String {
    let shown = match std::env::var_os("HOME") {
        Some(home) => match path.strip_prefix(home) {
            Ok(rest) => format!("~/{}", rest.display()),
            Err(_) => path.display().to_string(),
        },
        None => path.display().to_string(),
    };

    shown.trim_end_matches('/').to_owned()
}

fn notice_bar(notice: &crate::app::Notice) -> Element<'_, Message> {
    let dismiss = button(text("✕").size(12))
        .padding([2, 8])
        .style(style::toggle)
        .on_press(Message::DismissNotice);

    container(
        container(
            row![text(notice.text.clone()).size(12).width(Fill), dismiss]
                .spacing(10)
                .align_y(Center),
        )
        .padding([6, 10])
        .width(Fill)
        .style(style::notice(notice.is_error)),
    )
    .padding([6, 10])
    .width(Fill)
    .into()
}

fn workspace(repo: &Repo) -> Element<'_, Message> {
    row![
        refs::view(repo, repo.busy),
        rule::vertical(1),
        centre(repo),
        rule::vertical(1),
        files::view(repo, repo.busy),
    ]
    .width(Fill)
    .height(Fill)
    .into()
}

/// The middle column shows one thing at a time. The graph is the resting
/// state; picking a file or a commit replaces it, and there is a way back.
fn centre(repo: &Repo) -> Element<'_, Message> {
    match repo.focus {
        Focus::History => graph::view(repo),
        Focus::Commit => match &repo.detail {
            Some(detail) => commit::view(repo, detail),
            None => commit::loading(),
        },
        Focus::CommitFile => match (&repo.detail_diff, &repo.detail_file) {
            (Some(file_diff), _) => diff::view(file_diff),
            (None, Some(file)) => commit::loading_file(file),
            (None, None) => commit::loading(),
        },
        Focus::File => match &repo.diff {
            Some(file_diff) => diff::view(file_diff),
            None if repo.selection.is_some() => diff::placeholder("Loading diff…"),
            None if repo.busy => diff::placeholder("Loading…"),
            None => diff::placeholder("Select a file to see what changed."),
        },
    }
}

/// Shown while the first repository is still being read, so the window is
/// never blank with a tab already on it.
fn opening(state: &GitDruid) -> Element<'_, Message> {
    let names: Vec<String> = state.opening.iter().map(|path| directory(path)).collect();

    container(
        text(format!("Opening {}…", names.join(", ")))
            .size(14)
            .style(muted),
    )
    .center_x(Fill)
    .center_y(Fill)
    .style(style::canvas)
    .into()
}

fn welcome() -> Element<'static, Message> {
    container(
        column![
            text("gitDruid").size(28),
            text("Open a git repository to see what's changed and build a commit.")
                .size(13)
                .style(muted),
            button(text("Open a repository").size(13))
                .padding([8, 18])
                .style(style::primary)
                .on_press(Message::PickRepo),
            text("…or drop a folder onto this window")
                .size(12)
                .style(|theme: &Theme| text::Style {
                    color: Some(style::muted(theme))
                }),
        ]
        .spacing(14)
        .align_x(Center),
    )
    .center_x(Fill)
    .center_y(Fill)
    .style(style::canvas)
    .into()
}
