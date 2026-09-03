//! The settings dialog.
//!
//! Two files are edited here, and which one is being edited is the first thing
//! the dialog says. In the repository scope every box is allowed to be empty,
//! and an empty box shows the global value as its placeholder — so "what will
//! I get if I leave this alone" is answered without having to look anywhere
//! else.

use iced::widget::{Column, button, column, container, row, rule, scrollable, text, text_input};
use iced::{Center, Element, Fill, Theme};

use crate::app::{GitDruid, Message, Prompt, PromptKind};
use crate::settings::{Kind, Mode, Scope, Settings, keys};
use crate::ui::style;

/// Wide enough for a path to a key without wrapping.
const WIDTH: f32 = 620.0;

const LABEL: f32 = 150.0;

pub fn view(state: &GitDruid) -> Element<'_, Message> {
    let settings = state.settings();
    let scope = state.settings_scope;
    let has_repo = state.active().is_some();

    let body = Column::new()
        .spacing(14)
        .push(scope_row(scope, has_repo))
        .push(rule::horizontal(1))
        .push(branching(settings, scope))
        .push(rule::horizontal(1))
        .push(prefixes(settings, scope))
        .push(rule::horizontal(1))
        .push(authentication(settings, scope));

    let dialog = column![
        header(),
        rule::horizontal(1),
        container(scrollable(container(body).padding([14, 16])).height(Fill)).height(Fill),
        rule::horizontal(1),
        footer(state, scope),
    ]
    .width(WIDTH)
    .height(560);

    container(container(dialog).style(style::dialog))
        .center_x(Fill)
        .center_y(Fill)
        .style(style::scrim)
        .into()
}

fn header() -> Element<'static, Message> {
    container(
        row![
            text("SETTINGS").size(12).width(Fill),
            button(text("✕").size(11))
                .padding([2, 8])
                .style(style::toggle)
                .on_press(Message::CloseSettings),
        ]
        .align_y(Center),
    )
    .padding([9, 12])
    .width(Fill)
    .style(style::panel)
    .into()
}

fn footer(state: &GitDruid, scope: Scope) -> Element<'_, Message> {
    let where_to = match scope {
        Scope::Global => crate::settings::global_path()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "no HOME to write to".to_owned()),
        Scope::Repo => state
            .active()
            .map(|repo| crate::settings::repo_path(repo.path()).display().to_string())
            .unwrap_or_default(),
    };

    // A path has no spaces in it, so ordinary word wrapping cannot break one:
    // without falling back to glyphs a long path runs straight off the dialog
    // however much room it is given.
    let location = column![
        text("SAVES TO").size(9).style(muted),
        text(where_to.clone())
            .size(10)
            .style(muted)
            .wrapping(text::Wrapping::WordOrGlyph),
    ]
    .spacing(3)
    .width(Fill);

    let buttons = row![
        text("").width(Fill),
        button(text("Copy path").size(11))
            .padding([4, 12])
            .style(style::toggle)
            // Nothing to copy when there is no repository to save one to.
            .on_press_maybe((!where_to.is_empty()).then_some(Message::CopyText(where_to))),
        button(text("Close").size(11))
            .padding([4, 12])
            .style(style::toggle)
            .on_press(Message::CloseSettings),
        button(text("Save").size(11))
            .padding([4, 12])
            .style(style::primary)
            .on_press(Message::SaveSettings),
    ]
    .spacing(8)
    .align_y(Center);

    container(column![location, buttons].spacing(9))
        .padding([9, 12])
        .width(Fill)
        .style(style::panel)
        .into()
}

fn scope_row(scope: Scope, has_repo: bool) -> Element<'static, Message> {
    let mut tabs = row![].spacing(4).align_y(Center);

    for option in [Scope::Global, Scope::Repo] {
        let available = option == Scope::Global || has_repo;

        tabs = tabs.push(
            button(text(option.title()).size(11))
                .padding([4, 10])
                .style(style::tab(option == scope))
                .on_press_maybe(available.then_some(Message::SettingsScope(option))),
        );
    }

    let hint = match scope {
        Scope::Global => "Applies to every repository.",
        Scope::Repo => "Overrides the global settings here. Empty means \"use the global one\".",
    };

    column![tabs, text(hint).size(10).style(muted)]
        .spacing(6)
        .into()
}

fn branching(settings: &Settings, scope: Scope) -> Element<'_, Message> {
    let flow = settings.flow();

    let mut section = Column::new()
        .spacing(8)
        .push(heading("BRANCHING"))
        .push(modes(settings, scope))
        .push(field(
            "Main branch",
            keys::FLOW_MAIN,
            "main",
            settings,
            scope,
        ));

    if flow.mode.has_develop() {
        section = section.push(field(
            "Develop branch",
            keys::FLOW_DEVELOP,
            "develop",
            settings,
            scope,
        ));
    }

    section = section.push(text(describe(&flow)).size(10).style(muted));

    section.into()
}

/// Says, in the repository's own branch names, what the chosen workflow does.
fn describe(flow: &crate::settings::Flow) -> String {
    match flow.mode {
        Mode::Simple => format!(
            "Everything branches from {} and merges back into it.",
            flow.main
        ),
        Mode::GitHub => format!(
            "Short-lived branches come off {0}, are named by what they are, and merge straight \
             back into {0}. A release is whatever {0} is at the time, so there are no release \
             branches.",
            flow.main
        ),
        Mode::GitFlow => format!(
            "Features and bugfixes branch from {} and merge back into it. Hotfixes and releases \
             branch from {} and merge back into it.",
            flow.develop, flow.main
        ),
    }
}

/// The workflow picker. Three buttons rather than a checkbox, because there
/// are three answers and one of them is not the absence of another.
fn modes(settings: &Settings, scope: Scope) -> Element<'_, Message> {
    let current = settings.layer(scope).get(keys::FLOW_MODE);
    let effective = settings.mode();

    let mut buttons = row![].spacing(4).align_y(Center);

    if scope == Scope::Repo {
        buttons = buttons.push(
            button(text("Inherit").size(11))
                .padding([4, 10])
                .style(style::option(current.is_none()))
                .on_press(Message::SettingsChanged(
                    keys::FLOW_MODE.to_owned(),
                    String::new(),
                )),
        );
    }

    for mode in Mode::ALL {
        // With nothing set in this scope the effective mode is still worth
        // showing, so the row is never a line of blanks.
        let chosen = match current {
            Some(value) => value == mode.key(),
            None => scope == Scope::Global && effective == mode,
        };

        buttons = buttons.push(
            button(text(mode.title()).size(11))
                .padding([4, 10])
                .style(style::option(chosen))
                .on_press(Message::SettingsChanged(
                    keys::FLOW_MODE.to_owned(),
                    mode.key().to_owned(),
                )),
        );
    }

    row![text("Workflow").size(11).width(LABEL), buttons]
        .spacing(10)
        .align_y(Center)
        .into()
}

fn prefixes(settings: &Settings, scope: Scope) -> Element<'_, Message> {
    let mode = settings.mode();

    if !mode.names_kinds() {
        return column![
            heading("BRANCH PREFIXES"),
            text("Pick a workflow above to name sorts of branch.")
                .size(10)
                .style(muted),
        ]
        .spacing(8)
        .into();
    }

    let mut section = Column::new().spacing(8).push(heading("BRANCH PREFIXES"));

    // Only the sorts this workflow has a use for: a release prefix under
    // GitHub Flow would be a box that never gets read.
    for kind in mode.kinds() {
        let Some((label, key, default)) = prefix_field(*kind) else {
            continue;
        };

        section = section.push(field(label, key, default, settings, scope));
    }

    section.into()
}

fn prefix_field(kind: Kind) -> Option<(&'static str, &'static str, &'static str)> {
    match kind {
        Kind::Plain => None,
        Kind::Feature => Some(("Feature", keys::PREFIX_FEATURE, "feature/")),
        Kind::Bugfix => Some(("Bugfix", keys::PREFIX_BUGFIX, "bugfix/")),
        Kind::Hotfix => Some(("Hotfix", keys::PREFIX_HOTFIX, "hotfix/")),
        Kind::Release => Some(("Release", keys::PREFIX_RELEASE, "release/")),
    }
}

fn authentication(settings: &Settings, scope: Scope) -> Element<'_, Message> {
    column![
        heading("AUTHENTICATION"),
        choice(
            "SSH agent",
            keys::USE_AGENT,
            &[("Use it", "true"), ("Skip it", "false")],
            settings,
            scope,
        ),
        choice(
            "Credential helper",
            keys::USE_HELPER,
            &[("Use it", "true"), ("Skip it", "false")],
            settings,
            scope,
        ),
        browsable(
            "SSH key",
            keys::SSH_KEY,
            "~/.ssh/id_ed25519",
            settings,
            scope,
        ),
        browsable(
            "Public key",
            keys::SSH_PUBLIC_KEY,
            "only if it is not <key>.pub",
            settings,
            scope,
        ),
        field("SSH username", keys::SSH_USER, "git", settings, scope),
        text(
            "A configured key is tried first, then the agent, then the helper. gitDruid does not \
             store passphrases — for a key that has one, add it to your ssh agent."
        )
        .size(10)
        .style(muted),
    ]
    .spacing(8)
    .into()
}

/// A labelled text box.
///
/// The box holds only what this scope sets. What it would fall back to shows
/// through as the placeholder, so an empty box is informative rather than
/// blank.
fn field<'a>(
    label: &'a str,
    key: &'static str,
    default: &'a str,
    settings: &'a Settings,
    scope: Scope,
) -> Element<'a, Message> {
    let value = settings.layer(scope).get(key).unwrap_or_default();
    let inherited = settings.inherited(scope, key).unwrap_or(default);

    row![
        text(label).size(11).width(LABEL),
        text_input(inherited, value)
            .size(11)
            .padding([4, 6])
            .style(style::input)
            .on_input(move |value| Message::SettingsChanged(key.to_owned(), value)),
    ]
    .spacing(10)
    .align_y(Center)
    .into()
}

/// A text box for a path, with a picker beside it.
///
/// Typing is still the quicker way in for anyone who knows where their keys
/// are, so the box stays: the button is an alternative to it, not a
/// replacement for it.
fn browsable<'a>(
    label: &'a str,
    key: &'static str,
    default: &'a str,
    settings: &'a Settings,
    scope: Scope,
) -> Element<'a, Message> {
    let value = settings.layer(scope).get(key).unwrap_or_default();
    let inherited = settings.inherited(scope, key).unwrap_or(default);

    row![
        text(label).size(11).width(LABEL),
        text_input(inherited, value)
            .size(11)
            .padding([4, 6])
            .style(style::input)
            .on_input(move |value| Message::SettingsChanged(key.to_owned(), value)),
        button(text("Browse…").size(11))
            .padding([4, 10])
            .style(style::toggle)
            .on_press(Message::BrowseFor(key)),
    ]
    .spacing(10)
    .align_y(Center)
    .into()
}

/// A labelled row of mutually exclusive buttons.
///
/// In the repository scope there is a third option — inheriting — because
/// "off" and "whatever the global file says" are different answers and a
/// two-state control cannot tell them apart.
fn choice<'a>(
    label: &'a str,
    key: &'static str,
    options: &'a [(&'a str, &'a str)],
    settings: &'a Settings,
    scope: Scope,
) -> Element<'a, Message> {
    let current = settings.layer(scope).get(key);

    let mut buttons = row![].spacing(4).align_y(Center);

    if scope == Scope::Repo {
        buttons = buttons.push(
            button(text("Inherit").size(11))
                .padding([4, 10])
                .style(style::option(current.is_none()))
                .on_press(Message::SettingsChanged(key.to_owned(), String::new())),
        );
    }

    for (title, value) in options {
        buttons = buttons.push(
            button(text(*title).size(11))
                .padding([4, 10])
                .style(style::option(current == Some(*value)))
                .on_press(Message::SettingsChanged(
                    key.to_owned(),
                    (*value).to_owned(),
                )),
        );
    }

    row![text(label).size(11).width(LABEL), buttons]
        .spacing(10)
        .align_y(Center)
        .into()
}

fn heading(label: &str) -> Element<'_, Message> {
    text(label.to_owned()).size(10).style(muted).into()
}

fn muted(theme: &Theme) -> text::Style {
    text::Style {
        color: Some(style::muted(theme)),
    }
}

/// The row of branch sorts, for the new-branch prompt.
///
/// Absent without git-flow: a repository branching everything off one line has
/// no sorts to choose between.
pub fn flow_picker(settings: &Settings, chosen: Kind) -> Option<Element<'static, Message>> {
    let mode = settings.mode();

    if !mode.names_kinds() {
        return None;
    }

    let mut kinds = row![].spacing(4).align_y(Center);

    for kind in mode.kinds().iter().copied() {
        kinds = kinds.push(
            button(text(kind.title()).size(11))
                .padding([3, 8])
                .style(style::option(kind == chosen))
                .on_press(Message::PromptFlow(kind)),
        );
    }

    Some(kinds.into())
}

/// What the open prompt would actually do, in the prompt's own words.
///
/// The name typed into the box is not the name that gets created — a prefix
/// goes on the front of it, and where it starts depends on its sort — so the
/// answer is shown rather than left to be worked out.
pub fn flow_preview(settings: &Settings, prompt: &Prompt) -> Option<String> {
    if prompt.kind != PromptKind::NewBranch {
        return None;
    }

    let flow = settings.flow();

    if !flow.mode.names_kinds() {
        return None;
    }

    // A commit picked out of the graph is where this starts, whatever the
    // workflow would otherwise have said.
    let start = prompt
        .at
        .clone()
        .unwrap_or_else(|| flow.start_point(prompt.flow).to_owned());

    Some(match prompt.value.trim().is_empty() {
        true => format!("→ from {start}"),
        false => format!(
            "→ {} from {start}",
            flow.branch_name(prompt.flow, &prompt.value)
        ),
    })
}
