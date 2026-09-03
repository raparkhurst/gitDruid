//! Application state and the update loop.
//!
//! Every git call is blocking, so it is wrapped in a `Task` and runs off the
//! UI thread. Nothing but plain data crosses back, which keeps the repository
//! handle out of the state entirely: each task opens the repository it needs.
//!
//! Several repositories can be open at once, one per tab. That splits messages
//! in two. A message the user produced — a click, a keystroke — can only have
//! come from the tab on screen, so it applies to the active repository. A
//! message carrying the result of a git call cannot: the user is free to
//! switch tabs while it runs, so every such message names the repository it
//! belongs to and is dropped if that repository has since been closed.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use iced::futures::channel::mpsc;
use iced::widget::text_editor;
use iced::{Event, Point, Size, Subscription, Task, Theme, keyboard, mouse, window};

use crate::git::{self, FileEntry, Side};
use crate::settings::{self, Scope, Settings};

pub struct GitDruid {
    pub repos: Vec<Repo>,
    /// Index into `repos`. Meaningful only while `repos` is not empty, and
    /// kept in range by every path that removes one.
    pub active: usize,
    /// Repositories whose first snapshot has not arrived yet. They hold a tab
    /// so that opening a slow repository shows something immediately.
    pub opening: Vec<PathBuf>,
    /// The repository to bring to the front once it has finished opening.
    ///
    /// Snapshots come back in whatever order git finishes them, so "the one
    /// that arrived last" is not the one the user asked for. This records
    /// which one that was.
    activate: Option<PathBuf>,
    pub notice: Option<Notice>,
    /// The global layer, and the baseline the dialog edits when no repository
    /// is open. Each open repository carries its own resolved copy.
    pub settings: Settings,
    pub settings_open: bool,
    /// True while the splash screen is up, which is only ever at startup.
    pub splash: bool,
    /// When the process started, used only to give up on a splash that the
    /// window never told us had appeared.
    started: Instant,
    /// Where the pointer is, so a right-click can put a menu under it. iced
    /// hands out no position with a button press, so it is tracked as it moves.
    pub cursor: Point,
    /// The window's size, for keeping a menu inside it.
    pub window: Size,
    pub menu: Option<Menu>,
    /// Which modifiers are held. A click carries none in iced, so the last
    /// state they were reported in is what a click has to be read against.
    pub modifiers: keyboard::Modifiers,
    /// Which file the dialog is editing.
    pub settings_scope: Scope,
    /// Polling pauses while the window is in the background: nobody is looking
    /// at a stale list, and a status read is not free on a large repository.
    pub focused: bool,
    pub theme: Theme,
}

/// How often the working tree is re-read to catch changes gitDruid did not
/// make itself — an editor saving a file, or `git add` from a terminal.
const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// How long to leave a splash that nothing has dismissed before assuming the
/// window never announced itself and taking it down anyway.
const STUCK_SPLASH: Duration = Duration::from_secs(15);

pub struct Repo {
    pub snapshot: git::Snapshot,
    pub refs: git::Refs,
    pub history: git::History,
    /// The graph is read separately from the file lists and takes longer, so
    /// the centre column can say so rather than flashing an empty list.
    pub loading_history: bool,
    /// True while a git task for *this* repository is in flight. Per-tab, so
    /// work in one does not grey out the buttons in another.
    pub busy: bool,
    /// Which of the two file lists the working-tree pane is showing.
    pub tab: Side,
    /// What the centre column is showing.
    pub focus: Focus,
    pub selection: Option<Selection>,
    /// Every file a bulk action would apply to, on the visible side.
    ///
    /// Separate from `selection`, which is only about whose diff is on screen:
    /// a plain click sets both, and a modifier-click adds to this alone.
    pub marked: BTreeSet<PathBuf>,
    /// Where a shift-click measures its range from. Held apart from
    /// `selection` so that extending a range does not move the far end.
    pub anchor: Option<PathBuf>,
    /// The diff for `selection`. `None` while one is being read.
    pub diff: Option<git::FileDiff>,
    /// The commit the graph selection points at. `None` while one is read.
    pub detail: Option<git::CommitDetail>,
    /// The file picked out of `detail`, and its diff within that commit.
    pub detail_file: Option<git::ChangedFile>,
    pub detail_diff: Option<git::FileDiff>,
    pub message: text_editor::Content,
    /// The global layer plus this repository's `.gitdruid`.
    pub settings: Settings,
    /// The ref the sidebar's action bar applies to.
    pub selected_ref: Option<RefTarget>,
    /// An open name prompt or confirmation, shown above the ref list.
    pub prompt: Option<Prompt>,
}

/// The centre column shows one thing at a time: the graph, or whatever was
/// picked out of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    History,
    /// A working-tree file, which can be staged from.
    File,
    /// One commit out of the graph, read-only.
    Commit,
    /// One file as that commit changed it, read-only.
    CommitFile,
}

impl Repo {
    fn new(snapshot: git::Snapshot, global: settings::Layer) -> Self {
        // Open on whichever list has something to show, preferring unstaged
        // work: reviewing it is what leads to a commit.
        let tab = if snapshot.unstaged.is_empty() && !snapshot.staged.is_empty() {
            Side::Index
        } else {
            Side::Worktree
        };

        let settings = Settings {
            global,
            repo: settings::load(Some(&snapshot.path)).repo,
        };

        Self {
            settings,
            snapshot,
            refs: git::Refs::default(),
            history: git::History::empty(),
            loading_history: true,
            busy: false,
            tab,
            focus: Focus::History,
            selection: None,
            marked: BTreeSet::new(),
            anchor: None,
            diff: None,
            detail: None,
            detail_file: None,
            detail_diff: None,
            message: text_editor::Content::new(),
            selected_ref: None,
            prompt: None,
        }
    }

    pub fn path(&self) -> &Path {
        &self.snapshot.path
    }

    /// The files a bulk action applies to.
    pub fn bulk_paths(&self, side: Side) -> Vec<PathBuf> {
        let order: Vec<PathBuf> = self
            .snapshot
            .entries(side)
            .iter()
            .map(|entry| entry.path.clone())
            .collect();

        bulk_selection(&order, &self.marked)
    }

    /// Applies a click to the multi-selection.
    fn mark(&mut self, side: Side, path: &Path, modifiers: keyboard::Modifiers) {
        let order: Vec<PathBuf> = self
            .snapshot
            .entries(side)
            .iter()
            .map(|entry| entry.path.clone())
            .collect();

        select(&order, &mut self.marked, &mut self.anchor, path, modifiers);
    }

    /// Drops marks for files that are no longer on the visible side.
    fn prune_marks(&mut self) {
        let present: BTreeSet<PathBuf> = self
            .snapshot
            .entries(self.tab)
            .iter()
            .map(|entry| entry.path.clone())
            .collect();

        self.marked.retain(|path| present.contains(path));

        if self.anchor.as_ref().is_some_and(|path| !present.contains(path)) {
            self.anchor = None;
        }
    }

    /// The name on the tab.
    pub fn name(&self) -> String {
        self.snapshot.name()
    }
}

/// An open context menu, and what it is about.
#[derive(Debug, Clone)]
pub struct Menu {
    /// Where the click happened, in window coordinates.
    pub at: Point,
    pub target: Target,
}

/// What was right-clicked. Each kind of row offers a different menu, and the
/// row already knows which it is, so it says so rather than the menu guessing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    File(Side, PathBuf),
    Commit(String),
    Ref(RefTarget),
}

/// A ref the sidebar can act on. Which list it came from decides what the
/// action bar offers, so the kind travels with the name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefTarget {
    Local(String),
    Remote(String),
    Tag(String),
}

impl RefTarget {
    pub fn name(&self) -> &str {
        match self {
            RefTarget::Local(name) | RefTarget::Remote(name) | RefTarget::Tag(name) => name,
        }
    }
}

/// Which file the diff pane is showing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    pub side: Side,
    pub path: PathBuf,
}

/// A question asked in the bar above the ref list.
///
/// Naming and confirming share one bar because they are the same interaction:
/// the app stops, asks, and does nothing until it is answered.
#[derive(Debug, Clone)]
pub struct Prompt {
    pub kind: PromptKind,
    /// Which sort of branch is being made, when the workflow names sorts.
    pub flow: settings::Kind,
    /// What has been typed, for the kinds that take a name.
    pub value: String,
    /// The ref the prompt is about, empty for the kinds that create one.
    pub subject: String,
    /// The commit to act at, when the prompt was opened from the graph.
    pub at: Option<String>,
    /// The files the answer applies to, captured when the question was asked.
    ///
    /// Held rather than looked up again on the way out: between asking and
    /// answering, a background poll can change what is selected, and a
    /// confirmation has to be about what was on screen when it was read.
    pub paths: Vec<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptKind {
    NewBranch,
    RenameBranch,
    NewTag,
    /// `force` is set when the branch has commits HEAD cannot reach, so the
    /// confirmation has to say what is about to be lost.
    DeleteBranch {
        force: bool,
    },
    DeleteTag,
    Merge,
    /// Throwing away unstaged changes: the one action here git cannot undo.
    Discard {
        side: Side,
    },
    /// Merge a workflow branch back into the branch it belongs to.
    Finish,
    /// Pull and push are confirmed for opposite reasons: a pull can leave a
    /// merge half-done in the working tree, and a push is the one action here
    /// that other people can see.
    Pull,
    Push,
}

impl PromptKind {
    /// True for the prompts that need something typed before they can run.
    pub fn needs_name(self) -> bool {
        matches!(
            self,
            PromptKind::NewBranch | PromptKind::RenameBranch | PromptKind::NewTag
        )
    }
}

/// A one-line result banner.
#[derive(Debug, Clone)]
pub struct Notice {
    pub text: String,
    pub is_error: bool,
}

#[derive(Debug, Clone)]
pub enum Message {
    // Tabs.
    PickRepo,
    RepoPicked(Option<PathBuf>),
    SelectRepo(PathBuf),
    CloseRepo(PathBuf),

    // Acting on the active repository.
    Refresh,
    SelectTab(Side),
    Select(Side, PathBuf),
    ShowHistory,
    SelectCommit(String),
    ShowCommit,
    SelectCommitFile(git::ChangedFile),
    ToggleFile(Side, PathBuf),
    /// Stage or unstage the marked files, or all of them when none are marked.
    ToggleMany(Side),
    /// Drop the multi-selection without touching the index.
    ClearMarks,
    ToggleHunk(usize),
    Checkout(String),
    Fetch,
    OpenSettings,
    CloseSettings,
    /// The window is on screen, which is when the splash's clock starts.
    WindowOpened,
    DismissSplash,
    CursorMoved(Point),
    ModifiersChanged(keyboard::Modifiers),
    WindowResized(Size),
    OpenMenu(Target),
    CloseMenu,
    Ignore(String),
    CherryPick(String),
    Revert(String),
    Abort,
    CopyText(String),
    SettingsScope(Scope),
    SettingsChanged(String, String),
    SaveSettings,
    /// Add or remove this application's entry in the desktop menu.
    InstallDesktopEntry,
    RemoveDesktopEntry,
    /// Open a file picker for the setting named by the key.
    BrowseFor(&'static str),
    Browsed(&'static str, Option<PathBuf>),
    /// The sort of branch the open prompt is making.
    PromptFlow(settings::Kind),
    SelectRef(RefTarget),
    /// Opens the bar above the ref list.
    Ask(PromptKind, String, Option<String>),
    PromptChanged(String),
    PromptSubmit,
    PromptCancel,
    EditMessage(text_editor::Action),
    Commit,

    // Results, each naming the repository it was read for.
    Refreshed(PathBuf, Result<git::Snapshot, git::Error>),
    RefsRead(PathBuf, Result<git::Refs, git::Error>),
    HistoryRead(PathBuf, Result<git::History, git::Error>),
    Diffed(PathBuf, Selection, Result<git::FileDiff, git::Error>),
    Detailed(PathBuf, String, Result<git::CommitDetail, git::Error>),
    CommitFileDiffed(PathBuf, String, PathBuf, Result<git::FileDiff, git::Error>),
    /// A staging or ref action finished; `Ok` carries what to report.
    Applied(PathBuf, Result<String, git::Error>),
    Committed(PathBuf, Result<String, git::Error>),
    Polled(PathBuf, Result<git::Snapshot, git::Error>),

    // Whole-window.
    Poll,
    WindowFocused(bool),
    ThemeChanged(Theme),
    DismissNotice,
}

/// Builds the initial state, opening whatever repositories were named.
///
/// Every path argument opens a tab; without any, the current directory is
/// searched upwards, so launching from inside a checkout just works.
pub fn boot() -> (GitDruid, Task<Message>) {
    let arguments: Vec<PathBuf> = std::env::args().skip(1).map(PathBuf::from).collect();

    // With nothing named and nothing remembered, the directory gitDruid was
    // launched from is the obvious thing to open.
    let starts: Vec<PathBuf> = match arguments.is_empty() {
        true => match settings::load(None).session().is_empty() {
            true => std::env::current_dir().into_iter().collect(),
            false => Vec::new(),
        },
        false => arguments,
    };

    let settings = settings::load(None);

    // The palette is read before anything is drawn, so the window opens the
    // way it was last left rather than flickering to it.
    let theme = settings
        .get(settings::keys::THEME)
        .and_then(crate::ui::theme::by_name)
        .unwrap_or_else(crate::ui::theme::default);

    let splash = settings.splash();

    let mut state = GitDruid {
        repos: Vec::new(),
        active: 0,
        opening: Vec::new(),
        activate: None,
        notice: None,
        settings,
        splash,
        started: Instant::now(),
        settings_open: false,
        settings_scope: Scope::Global,
        cursor: Point::ORIGIN,
        window: Size::new(1400.0, 840.0),
        menu: None,
        modifiers: keyboard::Modifiers::default(),
        focused: true,
        theme,
    };

    // The tabs that were open last time come back first, then whatever was
    // asked for on the command line — `open` switches to a repository already
    // there rather than opening it twice, so naming one of them just brings it
    // to the front.
    let remembered = state.settings.session();
    let previous = state.settings.active_repo();

    let mut tasks: Vec<Task<Message>> = remembered
        .iter()
        .filter_map(|start| git::discover(start))
        .map(|root| state.open(root))
        .collect();

    let named: Vec<PathBuf> = starts.iter().filter_map(|start| git::discover(start)).collect();

    tasks.extend(named.iter().map(|root| state.open(root.clone())));

    // A repository named on the command line is what to look at; failing
    // that, whichever tab was in front last time.
    state.activate = named
        .first()
        .cloned()
        .or(previous)
        .or_else(|| state.opening.first().cloned());

    // The repositories are read while the splash is up, so the time it takes
    // is not time added to starting: by the time it goes, the tabs are there.
    // The splash's own clock does not start until the window opens.
    (state, Task::batch(tasks))
}

pub fn title(state: &GitDruid) -> String {
    match state.active() {
        Some(repo) => format!("{} — gitDruid", repo.name()),
        None => "gitDruid".to_owned(),
    }
}

pub fn theme(state: &GitDruid) -> Theme {
    state.theme.clone()
}

pub fn subscription(_state: &GitDruid) -> Subscription<Message> {
    Subscription::batch([
        // Dropping a folder onto the window opens the repository it belongs
        // to, which is the same path the Open… picker takes — a drop is just
        // another way to name a directory. Dropping a file works too; the
        // search starts at its parent.
        iced::event::listen_with(|event, _status, _window| match event {
            Event::Window(window::Event::FileDropped(path)) => {
                Some(Message::RepoPicked(Some(path)))
            }
            Event::Window(window::Event::Focused) => Some(Message::WindowFocused(true)),
            Event::Window(window::Event::Unfocused) => Some(Message::WindowFocused(false)),
            Event::Window(window::Event::Resized(size)) => Some(Message::WindowResized(size)),
            Event::Window(window::Event::Opened { .. }) => Some(Message::WindowOpened),
            // A button press carries no position, so the last one the pointer
            // moved to is where a menu has to open.
            Event::Mouse(mouse::Event::CursorMoved { position }) => {
                Some(Message::CursorMoved(position))
            }
            Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) => {
                Some(Message::ModifiersChanged(modifiers))
            }
            _ => None,
        }),
        Subscription::run(ticks).map(|_| Message::Poll),
    ])
}

/// A tick every [`POLL_INTERVAL`], from a thread that spends its life asleep.
///
/// iced's timers need an async runtime feature gitDruid does not otherwise
/// want: every git call here is blocking and runs on the thread pool. One
/// sleeping thread is cheaper than changing the executor under all of them.
fn ticks() -> mpsc::Receiver<()> {
    let (mut sender, receiver) = mpsc::channel(1);

    std::thread::spawn(move || {
        loop {
            std::thread::sleep(POLL_INTERVAL);

            // A full channel just means the last tick is still unhandled, and
            // the next one will do instead. A closed one means the window has
            // gone, and so should this thread.
            if let Err(error) = sender.try_send(())
                && error.is_disconnected()
            {
                break;
            }
        }
    });

    receiver
}

pub fn update(state: &mut GitDruid, message: Message) -> Task<Message> {
    // An open menu is dismissed by anything the user does next. Pointer
    // movement and results arriving in the background are not that: a menu
    // that vanished because a poll came back two seconds later would be a bug.
    let transient = matches!(
        message,
        Message::CursorMoved(_)
            | Message::ModifiersChanged(_)
            | Message::DismissSplash
            | Message::WindowOpened
            | Message::WindowResized(_)
            | Message::OpenMenu(_)
            | Message::Poll
            | Message::Polled(..)
            | Message::Refreshed(..)
            | Message::RefsRead(..)
            | Message::HistoryRead(..)
            | Message::Diffed(..)
            | Message::Detailed(..)
            | Message::CommitFileDiffed(..)
    );

    if !transient {
        state.menu = None;
    }

    match message {
        Message::PickRepo => Task::perform(pick_folder(), Message::RepoPicked),

        Message::RepoPicked(None) => Task::none(),

        Message::RepoPicked(Some(path)) => {
            // Accept a path anywhere inside a checkout, not just its root.
            let Some(root) = git::discover(&path) else {
                return state.report(Err(git::Error::new(format!(
                    "{} is not inside a git repository",
                    path.display()
                ))));
            };

            state.notice = None;
            state.open(root)
        }

        Message::SelectRepo(path) => {
            let Some(index) = state.index_of(&path) else {
                return Task::none();
            };

            state.active = index;
            state.remember_session();

            // Whatever changed while this tab was in the background should be
            // on screen by the time it has finished being looked at.
            state.poll()
        }

        Message::CloseRepo(path) => {
            state.opening.retain(|opening| opening != &path);

            if state.activate.as_deref() == Some(path.as_path()) {
                state.activate = None;
            }

            let Some(index) = state.index_of(&path) else {
                return Task::none();
            };

            state.repos.remove(index);

            // Keep the tab to the left, which is where the eye already is.
            state.active = state.active.min(state.repos.len().saturating_sub(1));
            state.remember_session();

            Task::none()
        }

        Message::Refresh => state.refresh(),

        Message::Refreshed(path, Ok(snapshot)) => state.adopt(&path, snapshot),

        Message::Refreshed(path, Err(error)) => {
            // A repository that never opened leaves no tab behind.
            state.opening.retain(|opening| opening != &path);

            if let Some(repo) = state.repo_mut(&path) {
                repo.busy = false;
            }

            state.report(Err(error))
        }

        Message::RefsRead(path, result) => {
            let Some(repo) = state.repo_mut(&path) else {
                return Task::none();
            };

            match result {
                Ok(refs) => {
                    repo.refs = refs;

                    // The action bar must never point at a ref that has just
                    // been deleted or renamed out from under it.
                    if repo
                        .selected_ref
                        .as_ref()
                        .is_some_and(|target| !exists(&repo.refs, target))
                    {
                        repo.selected_ref = None;
                    }

                    Task::none()
                }
                Err(error) => state.report(Err(error)),
            }
        }

        Message::HistoryRead(path, result) => {
            let Some(repo) = state.repo_mut(&path) else {
                return Task::none();
            };

            repo.loading_history = false;

            match result {
                Ok(history) => {
                    repo.history = history;
                    Task::none()
                }
                Err(error) => state.report(Err(error)),
            }
        }

        Message::SelectTab(side) => {
            let Some(repo) = state.active_mut() else {
                return Task::none();
            };

            repo.tab = side;
            repo.marked.clear();
            repo.anchor = None;

            // The pane only ever shows a file the list is showing, so the
            // selection moves to the newly visible side with it.
            repo.selection = next_selection(&repo.snapshot, side);
            repo.diff = None;
            repo.focus = if repo.selection.is_some() {
                Focus::File
            } else {
                Focus::History
            };

            select_task(repo)
        }

        Message::Select(side, path) => {
            let modifiers = state.modifiers;

            let Some(repo) = state.active_mut() else {
                return Task::none();
            };

            let Some(entry) = repo.snapshot.find(side, &path).cloned() else {
                return Task::none();
            };

            if repo.tab != side {
                repo.marked.clear();
                repo.anchor = None;
            }

            repo.tab = side;
            repo.mark(side, &path, modifiers);

            // Whatever else a click does to the selection, the diff shows the
            // file that was clicked: that is the one the pointer is on.
            repo.selection = Some(Selection { side, path });
            repo.diff = None;
            repo.focus = Focus::File;

            diff_task(repo.snapshot.path.clone(), entry)
        }

        Message::Diffed(path, selection, result) => {
            let Some(repo) = state.repo_mut(&path) else {
                return Task::none();
            };

            // A slower diff for a file the user has already navigated away
            // from must not overwrite the current one.
            if repo.selection.as_ref() != Some(&selection) {
                return Task::none();
            }

            match result {
                Ok(diff) => {
                    repo.diff = Some(diff);
                    Task::none()
                }
                Err(error) => {
                    repo.selection = None;
                    repo.focus = Focus::History;
                    state.report(Err(error))
                }
            }
        }

        Message::ShowHistory => {
            if let Some(repo) = state.active_mut() {
                repo.focus = Focus::History;
            }

            Task::none()
        }

        Message::SelectCommit(id) => {
            let Some(repo) = state.active_mut() else {
                return Task::none();
            };

            // A different commit is a different file list, so whatever was
            // open from the last one goes with it.
            let same = repo.detail.as_ref().is_some_and(|detail| detail.id == id);

            repo.focus = Focus::Commit;

            if same {
                return Task::none();
            }

            repo.detail = None;
            repo.detail_file = None;
            repo.detail_diff = None;

            detail_task(repo.snapshot.path.clone(), id)
        }

        Message::Detailed(path, id, result) => {
            let Some(repo) = state.repo_mut(&path) else {
                return Task::none();
            };

            match result {
                Ok(detail) => {
                    // A slower read for a commit the user has moved on from
                    // must not replace what is on screen.
                    if detail.id == id {
                        repo.detail = Some(detail);
                    }

                    Task::none()
                }
                Err(error) => {
                    repo.focus = Focus::History;
                    state.report(Err(error))
                }
            }
        }

        Message::ShowCommit => {
            if let Some(repo) = state.active_mut() {
                repo.focus = Focus::Commit;
            }

            Task::none()
        }

        Message::SelectCommitFile(file) => {
            let Some(repo) = state.active_mut() else {
                return Task::none();
            };

            let Some(detail) = &repo.detail else {
                return Task::none();
            };

            let id = detail.id.clone();
            let path = repo.snapshot.path.clone();

            repo.focus = Focus::CommitFile;
            repo.detail_file = Some(file.clone());
            repo.detail_diff = None;

            commit_file_task(path, id, file)
        }

        Message::CommitFileDiffed(repo_path, id, path, result) => {
            let Some(repo) = state.repo_mut(&repo_path) else {
                return Task::none();
            };

            // A slower read for a file the user has already moved on from must
            // not replace what is on screen.
            let wanted = repo.detail.as_ref().is_some_and(|detail| detail.id == id)
                && repo
                    .detail_file
                    .as_ref()
                    .is_some_and(|file| file.path == path);

            if !wanted {
                return Task::none();
            }

            match result {
                Ok(diff) => {
                    repo.detail_diff = Some(diff);
                    Task::none()
                }
                Err(error) => {
                    repo.focus = Focus::Commit;
                    repo.detail_file = None;
                    state.report(Err(error))
                }
            }
        }

        Message::ToggleFile(side, path) => state.toggle_files(side, vec![path]),

        Message::ToggleMany(side) => {
            let Some(repo) = state.active() else {
                return Task::none();
            };

            let paths = repo.bulk_paths(side);

            state.toggle_files(side, paths)
        }

        Message::ClearMarks => {
            if let Some(repo) = state.active_mut() {
                repo.marked.clear();
                repo.anchor = None;
            }

            Task::none()
        }

        Message::ToggleHunk(index) => state.toggle_hunk(index),

        Message::Applied(path, result) => match result {
            Ok(summary) => {
                state.notice = Some(Notice {
                    text: summary,
                    is_error: false,
                });

                state.refresh_path(&path)
            }
            Err(error) => {
                if let Some(repo) = state.repo_mut(&path) {
                    repo.busy = false;
                }

                state.report(Err(error))
            }
        },

        Message::Checkout(name) => {
            let Some(repo) = state.active_mut() else {
                return Task::none();
            };

            let path = repo.snapshot.path.clone();
            repo.busy = true;

            let wanted = path.clone();

            Task::perform(
                async move { git::checkout_branch(&path, &name) },
                move |result| Message::Applied(wanted.clone(), result),
            )
        }

        Message::Fetch => {
            let Some(repo) = state.active_mut() else {
                return Task::none();
            };

            let path = repo.snapshot.path.clone();
            let credentials = repo.settings.credentials();
            repo.busy = true;

            let wanted = path.clone();

            Task::perform(
                async move { git::fetch(&path, &credentials) },
                move |result| Message::Applied(wanted.clone(), result),
            )
        }

        Message::WindowOpened => {
            if !state.splash {
                return Task::none();
            }

            Task::perform(
                async {
                    std::thread::sleep(crate::ui::splash::DURATION);
                },
                |()| Message::DismissSplash,
            )
        }

        Message::DismissSplash => {
            state.splash = false;
            Task::none()
        }

        Message::CursorMoved(position) => {
            state.cursor = position;
            Task::none()
        }

        Message::ModifiersChanged(modifiers) => {
            state.modifiers = modifiers;
            Task::none()
        }

        Message::WindowResized(size) => {
            state.window = size;
            Task::none()
        }

        Message::OpenMenu(target) => {
            if state.active().is_none() {
                return Task::none();
            }

            // Right-clicking a row outside the selection makes that row the
            // selection. Without this the menu would offer to act on files
            // that are highlighted somewhere else in the list, which is the
            // one thing a menu must never do.
            if let Target::File(side, path) = &target
                && let Some(repo) = state.active_mut()
                && (repo.tab != *side || !repo.marked.contains(path))
            {
                repo.tab = *side;
                repo.marked = BTreeSet::from([path.clone()]);
                repo.anchor = Some(path.clone());
            }

            state.menu = Some(Menu {
                at: state.cursor,
                target,
            });

            Task::none()
        }

        Message::CloseMenu => {
            state.menu = None;
            Task::none()
        }

        Message::CopyText(text) => iced::clipboard::write(text),

        Message::Ignore(pattern) => {
            let Some(repo) = state.active_mut() else {
                return Task::none();
            };

            let path = repo.snapshot.path.clone();
            repo.busy = true;

            let wanted = path.clone();

            Task::perform(
                async move { git::ignore(&path, &pattern) },
                move |result| Message::Applied(wanted.clone(), result),
            )
        }

        Message::CherryPick(id) => state.apply(id, true),

        Message::Revert(id) => state.apply(id, false),

        Message::Abort => {
            let Some(repo) = state.active_mut() else {
                return Task::none();
            };

            let path = repo.snapshot.path.clone();
            repo.busy = true;

            let wanted = path.clone();

            Task::perform(async move { git::abort(&path) }, move |result| {
                Message::Applied(wanted.clone(), result)
            })
        }

        Message::OpenSettings => {
            state.settings_open = true;

            // A repository is the only place the per-repository file can go,
            // so without one the dialog can only be about the global file.
            if state.active().is_none() {
                state.settings_scope = Scope::Global;
            }

            Task::none()
        }

        Message::CloseSettings => {
            state.settings_open = false;
            Task::none()
        }

        Message::SettingsScope(scope) => {
            if scope == Scope::Repo && state.active().is_none() {
                return Task::none();
            }

            state.settings_scope = scope;
            Task::none()
        }

        Message::SettingsChanged(key, value) => {
            let scope = state.settings_scope;

            state.settings_mut().layer_mut(scope).set(&key, &value);

            Task::none()
        }

        Message::SaveSettings => state.save_settings(),

        Message::InstallDesktopEntry => {
            state.notice = Some(match crate::desktop::install() {
                Ok(path) => Notice {
                    text: format!("Added to the applications menu ({})", path.display()),
                    is_error: false,
                },
                Err(error) => Notice {
                    text: error.to_string(),
                    is_error: true,
                },
            });

            Task::none()
        }

        Message::RemoveDesktopEntry => {
            state.notice = Some(match crate::desktop::remove() {
                Ok(()) => Notice {
                    text: "Removed from the applications menu".to_owned(),
                    is_error: false,
                },
                Err(error) => Notice {
                    text: error.to_string(),
                    is_error: true,
                },
            });

            Task::none()
        }

        Message::BrowseFor(key) => {
            Task::perform(pick_key(), move |path| Message::Browsed(key, path))
        }

        Message::Browsed(_, None) => Task::none(),

        Message::Browsed(key, Some(path)) => {
            let scope = state.settings_scope;

            // Stored short, so a settings file copied to another machine still
            // points at that machine's key.
            state
                .settings_mut()
                .layer_mut(scope)
                .set(key, &settings::shorten(&path));

            Task::none()
        }

        Message::PromptFlow(kind) => {
            if let Some(repo) = state.active_mut()
                && let Some(prompt) = &mut repo.prompt
            {
                prompt.flow = kind;
            }

            Task::none()
        }

        Message::SelectRef(target) => {
            if let Some(repo) = state.active_mut() {
                // A prompt was asked about the previous selection, so it stops
                // making sense the moment the selection moves.
                repo.prompt = None;
                repo.selected_ref = Some(target);
            }

            Task::none()
        }

        Message::Ask(kind, subject, at) => {
            if let Some(repo) = state.active_mut() {
                let flow = repo.settings.flow();

                let paths = match kind {
                    PromptKind::Discard { .. } => match repo.marked.is_empty() {
                        true => vec![PathBuf::from(&subject)],
                        false => repo.marked.iter().cloned().collect(),
                    },
                    _ => Vec::new(),
                };

                repo.prompt = Some(Prompt {
                    paths,
                    // Renaming starts from the current name, which is what the
                    // user is usually editing rather than replacing.
                    value: match kind {
                        PromptKind::RenameBranch => subject.clone(),
                        _ => String::new(),
                    },
                    // A workflow that names sorts of branch has to start on
                    // one of them; a feature is the everyday case.
                    flow: match flow.mode.names_kinds() {
                        true => settings::Kind::Feature,
                        false => settings::Kind::Plain,
                    },
                    kind,
                    subject,
                    at,
                });
            }

            Task::none()
        }

        Message::PromptChanged(value) => {
            if let Some(repo) = state.active_mut()
                && let Some(prompt) = &mut repo.prompt
            {
                prompt.value = value;
            }

            Task::none()
        }

        Message::PromptCancel => {
            if let Some(repo) = state.active_mut() {
                repo.prompt = None;
            }

            Task::none()
        }

        Message::PromptSubmit => state.run_prompt(),

        Message::EditMessage(action) => {
            if let Some(repo) = state.active_mut() {
                repo.message.perform(action);
            }

            Task::none()
        }

        Message::Commit => {
            let Some(repo) = state.active_mut() else {
                return Task::none();
            };

            let path = repo.snapshot.path.clone();
            let message = repo.message.text();

            repo.busy = true;

            let wanted = path.clone();

            Task::perform(
                async move { git::commit(&path, &message) },
                move |result| Message::Committed(wanted.clone(), result),
            )
        }

        Message::Committed(path, Ok(id)) => {
            if let Some(repo) = state.repo_mut(&path) {
                repo.message = text_editor::Content::new();
            }

            state.notice = Some(Notice {
                text: format!("Committed {id}"),
                is_error: false,
            });

            state.refresh_path(&path)
        }

        Message::Committed(path, Err(error)) => {
            if let Some(repo) = state.repo_mut(&path) {
                repo.busy = false;
            }

            state.report(Err(error))
        }

        Message::WindowFocused(focused) => {
            state.focused = focused;

            // Coming back to the window should show what changed while it was
            // away, rather than waiting out the rest of the interval.
            match focused {
                true => state.poll(),
                false => Task::none(),
            }
        }

        Message::Poll => {
            // If the window never reported opening, the splash would sit there
            // until it was clicked. This is the way out of that.
            if state.splash && state.started.elapsed() > STUCK_SPLASH {
                state.splash = false;
            }

            state.poll()
        }

        Message::Polled(path, Ok(snapshot)) => {
            let Some(repo) = state.repo(&path) else {
                return Task::none();
            };

            // Nothing changed on disk, so nothing should change on screen:
            // adopting an identical snapshot would re-read the diff and the
            // graph for no reason. An action started while the read was in
            // flight supersedes it.
            if repo.busy || repo.snapshot == snapshot {
                return Task::none();
            }

            state.adopt(&path, snapshot)
        }

        // A failed poll is not worth a banner. The working tree gets read
        // while other tools are writing to it, so a transient failure — an
        // index.lock held by a `git add` in progress — means try again in two
        // seconds, which is exactly what happens.
        Message::Polled(_, Err(_)) => Task::none(),

        Message::ThemeChanged(theme) => {
            // Written straight away rather than on some later Save: nobody
            // picks a palette and then expects to confirm it.
            state.set_global(settings::keys::THEME, &theme.to_string());
            state.theme = theme;

            Task::none()
        }

        Message::DismissNotice => {
            state.notice = None;
            Task::none()
        }
    }
}

impl GitDruid {
    pub fn active(&self) -> Option<&Repo> {
        self.repos.get(self.active)
    }

    fn active_mut(&mut self) -> Option<&mut Repo> {
        self.repos.get_mut(self.active)
    }

    pub fn index_of(&self, path: &Path) -> Option<usize> {
        self.repos.iter().position(|repo| repo.path() == path)
    }

    fn repo(&self, path: &Path) -> Option<&Repo> {
        self.repos.iter().find(|repo| repo.path() == path)
    }

    fn repo_mut(&mut self, path: &Path) -> Option<&mut Repo> {
        self.repos.iter_mut().find(|repo| repo.path() == path)
    }

    /// Opens `path` in a tab, or switches to it when it is already open.
    ///
    /// Two tabs on one checkout would poll each other's writes and disagree
    /// about what is staged, so the same repository is never opened twice.
    fn open(&mut self, path: PathBuf) -> Task<Message> {
        if let Some(index) = self.index_of(&path) {
            self.active = index;
            return self.poll();
        }

        if !self.opening.contains(&path) {
            self.opening.push(path.clone());
        }

        self.activate = Some(path.clone());

        snapshot_task(path)
    }

    /// Reads the active repository's working tree in the background.
    ///
    /// This must never flip a tab to "Working…" and never race a task that is
    /// already writing to the repository, so unlike [`Self::refresh`] it
    /// leaves `busy` alone and stands down while anything else is running.
    /// Only the active repository is polled: nobody is looking at the others.
    fn poll(&self) -> Task<Message> {
        let Some(repo) = self.active() else {
            return Task::none();
        };

        if repo.busy || !self.focused {
            return Task::none();
        }

        let path = repo.snapshot.path.clone();
        let wanted = path.clone();

        Task::perform(async move { git::snapshot(&path) }, move |result| {
            Message::Polled(wanted.clone(), result)
        })
    }

    fn refresh(&mut self) -> Task<Message> {
        let Some(path) = self.active().map(|repo| repo.snapshot.path.clone()) else {
            return Task::none();
        };

        self.refresh_path(&path)
    }

    fn refresh_path(&mut self, path: &Path) -> Task<Message> {
        let Some(repo) = self.repo_mut(path) else {
            return Task::none();
        };

        repo.busy = true;

        snapshot_task(path.to_path_buf())
    }

    /// Installs a new snapshot, keeping the commit message and — where it still
    /// makes sense — the current selection, then re-reads everything the
    /// snapshot does not carry.
    fn adopt(&mut self, path: &Path, snapshot: git::Snapshot) -> Task<Message> {
        match self.index_of(path) {
            Some(index) => {
                self.repos[index].snapshot = snapshot;
                self.repos[index].busy = false;
            }
            None => {
                // First snapshot for a repository that was still opening. If
                // its tab was closed while it loaded, the result is dropped.
                if !self.opening.iter().any(|opening| opening == path) {
                    return Task::none();
                }

                self.opening.retain(|opening| opening != path);
                self.repos
                    .push(Repo::new(snapshot, self.settings.global.clone()));

                // Come to the front only if this is the tab that was asked
                // for, or if it is the only one there is.
                if self.activate.as_deref() == Some(path) || self.repos.len() == 1 {
                    self.active = self.repos.len() - 1;
                }

                if self.activate.as_deref() == Some(path) {
                    self.activate = None;
                }

                self.remember_session();
            }
        }

        let index = self
            .index_of(path)
            .expect("the repository was just installed");
        let repo = &mut self.repos[index];

        let path = repo.snapshot.path.clone();

        repo.loading_history = true;

        let still_valid = repo.selection.as_ref().is_some_and(|selection| {
            selection.side == repo.tab
                && repo
                    .snapshot
                    .find(selection.side, &selection.path)
                    .is_some()
        });

        if !still_valid {
            repo.selection = None;
            repo.diff = None;
        }

        if repo.selection.is_none() {
            repo.selection = next_selection(&repo.snapshot, repo.tab);
        }

        // Files that were staged, unstaged or committed have left the list the
        // marks were made in.
        repo.prune_marks();

        // A file that has gone — staged in full, or committed — takes the
        // centre column back to the graph rather than leaving a stale diff.
        if repo.focus == Focus::File && repo.selection.is_none() {
            repo.focus = Focus::History;
        }

        let centre = match repo.focus {
            // Always re-read the diff: staging a hunk changes what the rest of
            // the file's diff looks like.
            Focus::File => select_task(repo),
            // A commit's diff cannot change, so nothing behind `CommitFile`
            // needs re-reading; the detail is refreshed for its badges.
            Focus::Commit | Focus::CommitFile => match &repo.detail {
                Some(detail) => detail_task(path.clone(), detail.id.clone()),
                None => Task::none(),
            },
            Focus::History => Task::none(),
        };

        Task::batch([centre, refs_task(path.clone()), history_task(path)])
    }

    fn toggle_files(&mut self, side: Side, paths: Vec<PathBuf>) -> Task<Message> {
        let Some(repo) = self.active_mut() else {
            return Task::none();
        };

        let entries: Vec<FileEntry> = paths
            .iter()
            .filter_map(|path| repo.snapshot.find(side, path).cloned())
            .collect();

        if entries.is_empty() {
            return Task::none();
        }

        let path = repo.snapshot.path.clone();
        let summary = describe(side, &entries);

        repo.busy = true;

        let wanted = path.clone();

        Task::perform(
            async move {
                for entry in &entries {
                    match side {
                        Side::Worktree => git::stage_file(&path, entry)?,
                        Side::Index => git::unstage_file(&path, entry)?,
                    }
                }

                Ok(summary)
            },
            move |result| Message::Applied(wanted.clone(), result),
        )
    }

    fn toggle_hunk(&mut self, index: usize) -> Task<Message> {
        let Some(repo) = self.active_mut() else {
            return Task::none();
        };

        let Some(diff) = repo.diff.clone() else {
            return Task::none();
        };

        let Some(hunk) = diff.hunks().get(index).cloned() else {
            return Task::none();
        };

        // A committed diff has no side to move a hunk to, and the UI does not
        // offer one; this is the guard for a message arriving anyway.
        let Some(side) = diff.source.side() else {
            return Task::none();
        };

        let path = repo.snapshot.path.clone();

        let summary = format!(
            "{} one hunk of {}",
            match side {
                Side::Worktree => "Staged",
                Side::Index => "Unstaged",
            },
            diff.path.display()
        );

        repo.busy = true;

        let wanted = path.clone();

        Task::perform(
            async move {
                match side {
                    Side::Worktree => git::stage_hunk(&path, &diff, &hunk)?,
                    Side::Index => git::unstage_hunk(&path, &diff, &hunk)?,
                }

                Ok(summary)
            },
            move |result| Message::Applied(wanted.clone(), result),
        )
    }

    /// Runs whatever the open prompt was asking about, and closes it.
    fn run_prompt(&mut self) -> Task<Message> {
        let Some(repo) = self.active_mut() else {
            return Task::none();
        };

        let Some(prompt) = repo.prompt.take() else {
            return Task::none();
        };

        let path = repo.snapshot.path.clone();
        let flow = repo.settings.flow();
        let credentials = repo.settings.credentials();
        let typed = prompt.value.trim().to_owned();

        if prompt.kind.needs_name() && typed.is_empty() {
            // Put the prompt back rather than silently doing nothing.
            repo.prompt = Some(prompt);
            return Task::none();
        }

        repo.busy = true;

        let subject = prompt.subject;
        let targets = prompt.paths;
        let wanted = path.clone();
        let reply = move |result| Message::Applied(wanted.clone(), result);

        // A commit picked out of the graph is an explicit instruction and
        // outranks the workflow's idea of where this sort of branch starts.
        let start = prompt.at.clone().or_else(|| match prompt.kind {
            PromptKind::NewBranch if flow.mode.has_develop() => {
                Some(flow.start_point(prompt.flow).to_owned())
            }
            _ => None,
        });

        let name = match prompt.kind {
            PromptKind::NewBranch => flow.branch_name(prompt.flow, &typed),
            _ => typed,
        };

        match prompt.kind {
            PromptKind::NewBranch => Task::perform(
                async move { git::create_branch(&path, &name, start.as_deref()) },
                reply,
            ),
            PromptKind::RenameBranch => Task::perform(
                async move { git::rename_branch(&path, &subject, &name) },
                reply,
            ),
            PromptKind::NewTag => Task::perform(
                async move { git::create_tag(&path, &name, start.as_deref(), None) },
                reply,
            ),
            PromptKind::DeleteBranch { force } => Task::perform(
                async move { git::delete_branch(&path, &subject, force) },
                reply,
            ),
            PromptKind::DeleteTag => {
                Task::perform(async move { git::delete_tag(&path, &subject) }, reply)
            }
            PromptKind::Merge => {
                Task::perform(async move { git::merge_branch(&path, &subject) }, reply)
            }
            PromptKind::Discard { side } => {
                let entries: Vec<FileEntry> = self
                    .active()
                    .map(|repo| {
                        targets
                            .iter()
                            .filter_map(|target| repo.snapshot.find(side, target).cloned())
                            .collect()
                    })
                    .unwrap_or_default();

                if entries.is_empty() {
                    return self.report(Err(git::Error::new(
                        "those files are not changed any more",
                    )));
                }

                let summary = match entries.as_slice() {
                    [entry] => format!("Discarded changes to {}", entry.path.display()),
                    many => format!("Discarded changes to {} files", many.len()),
                };

                Task::perform(
                    async move {
                        for entry in &entries {
                            git::discard(&path, entry)?;
                        }

                        Ok(summary)
                    },
                    reply,
                )
            }

            PromptKind::Finish => {
                // Which branch this merges back into is the workflow's answer,
                // read from the branch's own prefix.
                let target = flow
                    .kind_of(&subject)
                    .map(|kind| flow.merges_into(kind).to_owned())
                    .unwrap_or_else(|| flow.main.clone());

                Task::perform(
                    async move { git::finish_branch(&path, &subject, &target) },
                    reply,
                )
            }
            PromptKind::Pull => {
                Task::perform(async move { git::pull(&path, &credentials) }, reply)
            }
            PromptKind::Push => {
                Task::perform(async move { git::push(&path, &credentials) }, reply)
            }
        }
    }

    /// Cherry-picks or reverts a commit onto the current branch.
    fn apply(&mut self, id: String, pick: bool) -> Task<Message> {
        let Some(repo) = self.active_mut() else {
            return Task::none();
        };

        let path = repo.snapshot.path.clone();
        repo.busy = true;

        let wanted = path.clone();

        Task::perform(
            async move {
                match pick {
                    true => git::cherry_pick(&path, &id),
                    false => git::revert(&path, &id),
                }
            },
            move |result| Message::Applied(wanted.clone(), result),
        )
    }

    /// The settings the dialog is editing: the open repository's, or the
    /// global-only baseline when there is no repository to have any.
    pub fn settings(&self) -> &Settings {
        match self.repos.get(self.active) {
            Some(repo) => &repo.settings,
            None => &self.settings,
        }
    }

    fn settings_mut(&mut self) -> &mut Settings {
        match self.repos.get_mut(self.active) {
            Some(repo) => &mut repo.settings,
            None => &mut self.settings,
        }
    }

    /// Writes the open tabs to the global settings, so they come back.
    ///
    /// Only repositories that actually opened are recorded: one still being
    /// read might turn out not to be a repository at all, and remembering it
    /// would make that failure repeat on every launch.
    fn remember_session(&mut self) {
        let open: Vec<PathBuf> = self.repos.iter().map(|repo| repo.path().to_path_buf()).collect();
        let active = self.active().map(|repo| repo.path().display().to_string());

        self.settings
            .global
            .set_list(settings::keys::SESSION_PREFIX, &open);

        self.settings
            .global
            .set(settings::keys::SESSION_ACTIVE, active.as_deref().unwrap_or(""));

        for repo in &mut self.repos {
            repo.settings.global = self.settings.global.clone();
        }

        self.write_global();
    }

    /// Sets a global value and writes it out, without a banner.
    ///
    /// For settings that are their own confirmation — the palette changes as
    /// it is picked — so a "Saved" notice would only be in the way. A failure
    /// still gets one, because a setting that silently did not stick is worse
    /// than a noisy one.
    fn set_global(&mut self, key: &str, value: &str) {
        self.settings.global.set(key, value);

        // Every open tab holds a resolved copy of the global layer.
        for repo in &mut self.repos {
            repo.settings.global.set(key, value);
        }

        self.write_global();
    }

    fn write_global(&mut self) {
        let settings = Settings {
            global: self.settings.global.clone(),
            repo: settings::Layer::default(),
        };

        if let Err(error) = settings::save(&settings, Scope::Global, None) {
            self.notice = Some(Notice {
                text: error.to_string(),
                is_error: true,
            });
        }
    }

    /// Writes the layer being edited to its file.
    ///
    /// Saving the global layer pushes it into every open tab: they each hold a
    /// resolved copy, and one of them going stale would mean two tabs
    /// disagreeing about a setting that is meant to be shared.
    fn save_settings(&mut self) -> Task<Message> {
        let scope = self.settings_scope;
        let repo = self.active().map(|repo| repo.snapshot.path.clone());

        let settings = self.settings().clone();

        match settings::save(&settings, scope, repo.as_deref()) {
            Ok(path) => {
                if scope == Scope::Global {
                    self.settings.global = settings.global.clone();

                    for open in &mut self.repos {
                        open.settings.global = settings.global.clone();
                    }
                }

                self.notice = Some(Notice {
                    text: format!("Saved {}", path.display()),
                    is_error: false,
                });
            }
            Err(error) => {
                self.notice = Some(Notice {
                    text: error.to_string(),
                    is_error: true,
                });
            }
        }

        Task::none()
    }

    fn report(&mut self, result: Result<String, git::Error>) -> Task<Message> {
        self.notice = Some(match result {
            Ok(text) => Notice {
                text,
                is_error: false,
            },
            Err(error) => Notice {
                text: error.to_string(),
                is_error: true,
            },
        });

        Task::none()
    }
}

/// The files a bulk action applies to: whatever is marked, or the whole list
/// when nothing is.
///
/// The result follows the list's order rather than the marks', and a mark for
/// something no longer listed is dropped — staging a file the user cannot see
/// would be the worst kind of surprise from a button labelled with a count.
fn bulk_selection(order: &[PathBuf], marked: &BTreeSet<PathBuf>) -> Vec<PathBuf> {
    if marked.is_empty() {
        return order.to_vec();
    }

    order
        .iter()
        .filter(|path| marked.contains(*path))
        .cloned()
        .collect()
}

/// Applies a click to a multi-selection.
///
/// The three behaviours are the ones every file list has: a plain click
/// replaces the selection, the command key adds to or removes from it, and
/// shift takes everything between the anchor and here. Kept as a free function
/// over the list of paths so that it can be checked without an application
/// around it — a shift range off by one row is not something a screenshot
/// would show.
fn select(
    order: &[PathBuf],
    marked: &mut BTreeSet<PathBuf>,
    anchor: &mut Option<PathBuf>,
    path: &Path,
    modifiers: keyboard::Modifiers,
) {
    if modifiers.command() || modifiers.control() {
        if !marked.remove(path) {
            marked.insert(path.to_path_buf());
        }

        *anchor = Some(path.to_path_buf());
        return;
    }

    if modifiers.shift()
        && let Some(from) = anchor
            .as_deref()
            .and_then(|anchor| order.iter().position(|entry| entry == anchor))
        && let Some(to) = order.iter().position(|entry| entry == path)
    {
        let (first, last) = if from <= to { (from, to) } else { (to, from) };

        *marked = order[first..=last].iter().cloned().collect();

        // The anchor stays put, so widening and narrowing a range both
        // measure from the same end.
        return;
    }

    *marked = BTreeSet::from([path.to_path_buf()]);
    *anchor = Some(path.to_path_buf());
}

/// True while `target` still names something in the repository.
fn exists(refs: &git::Refs, target: &RefTarget) -> bool {
    match target {
        RefTarget::Local(name) => refs.local.iter().any(|branch| branch.name == *name),
        RefTarget::Remote(name) => refs.remote.iter().any(|branch| branch.name == *name),
        RefTarget::Tag(name) => refs.tags.iter().any(|tag| tag.name == *name),
    }
}

/// Picks the file to show when the previous selection is gone: the first one
/// on the visible side, since that is the only list the user can see.
fn next_selection(snapshot: &git::Snapshot, side: Side) -> Option<Selection> {
    let entry = snapshot.entries(side).first()?;

    Some(Selection {
        side: entry.side,
        path: entry.path.clone(),
    })
}

/// Reads the diff for whatever `repo.selection` now points at.
fn select_task(repo: &Repo) -> Task<Message> {
    let Some(selection) = &repo.selection else {
        return Task::none();
    };

    let Some(entry) = repo.snapshot.find(selection.side, &selection.path).cloned() else {
        return Task::none();
    };

    diff_task(repo.snapshot.path.clone(), entry)
}

fn describe(side: Side, entries: &[FileEntry]) -> String {
    let verb = match side {
        Side::Worktree => "Staged",
        Side::Index => "Unstaged",
    };

    match entries {
        [entry] => format!("{verb} {}", entry.path.display()),
        _ => format!("{verb} {} files", entries.len()),
    }
}

fn snapshot_task(path: PathBuf) -> Task<Message> {
    let wanted = path.clone();

    Task::perform(async move { git::snapshot(&path) }, move |result| {
        Message::Refreshed(wanted.clone(), result)
    })
}

fn refs_task(path: PathBuf) -> Task<Message> {
    let wanted = path.clone();

    Task::perform(async move { git::refs(&path) }, move |result| {
        Message::RefsRead(wanted.clone(), result)
    })
}

fn history_task(path: PathBuf) -> Task<Message> {
    let wanted = path.clone();

    Task::perform(async move { git::history(&path) }, move |result| {
        Message::HistoryRead(wanted.clone(), result)
    })
}

fn detail_task(path: PathBuf, id: String) -> Task<Message> {
    let wanted = (path.clone(), id.clone());

    Task::perform(
        async move { git::commit_detail(&path, &id) },
        move |result| {
            let (path, id) = wanted.clone();
            Message::Detailed(path, id, result)
        },
    )
}

fn commit_file_task(path: PathBuf, id: String, file: git::ChangedFile) -> Task<Message> {
    let wanted = (path.clone(), id.clone(), file.path.clone());

    Task::perform(
        async move { git::commit_file_diff(&path, &id, &file) },
        move |result| {
            let (repo, id, path) = wanted.clone();
            Message::CommitFileDiffed(repo, id, path, result)
        },
    )
}

fn diff_task(path: PathBuf, entry: FileEntry) -> Task<Message> {
    let selection = Selection {
        side: entry.side,
        path: entry.path.clone(),
    };

    let wanted = (path.clone(), selection);

    Task::perform(async move { git::file_diff(&path, &entry) }, move |result| {
        let (path, selection) = wanted.clone();
        Message::Diffed(path, selection, result)
    })
}

/// Asks for a private key, starting where ssh keeps them.
async fn pick_key() -> Option<PathBuf> {
    let mut dialog = rfd::AsyncFileDialog::new().set_title("Choose an SSH key");

    if let Some(home) = std::env::var_os("HOME") {
        let ssh = PathBuf::from(home).join(".ssh");

        // A missing directory would leave the dialog somewhere arbitrary.
        if ssh.is_dir() {
            dialog = dialog.set_directory(ssh);
        }
    }

    dialog.pick_file().await.map(|handle| handle.path().to_path_buf())
}

async fn pick_folder() -> Option<PathBuf> {
    rfd::AsyncFileDialog::new()
        .set_title("Open a git repository")
        .pick_folder()
        .await
        .map(|handle| handle.path().to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(names: &[&str]) -> Vec<PathBuf> {
        names.iter().map(PathBuf::from).collect()
    }

    fn names(marked: &BTreeSet<PathBuf>) -> Vec<String> {
        marked
            .iter()
            .map(|path| path.display().to_string())
            .collect()
    }

    const PLAIN: keyboard::Modifiers = keyboard::Modifiers::empty();
    const EXTEND: keyboard::Modifiers = keyboard::Modifiers::SHIFT;
    const TOGGLE: keyboard::Modifiers = keyboard::Modifiers::CTRL;

    fn click(
        order: &[PathBuf],
        marked: &mut BTreeSet<PathBuf>,
        anchor: &mut Option<PathBuf>,
        name: &str,
        modifiers: keyboard::Modifiers,
    ) {
        select(order, marked, anchor, Path::new(name), modifiers);
    }

    #[test]
    fn a_bulk_action_with_nothing_marked_takes_the_whole_list() {
        let order = paths(&["a", "b", "c"]);

        assert_eq!(bulk_selection(&order, &BTreeSet::new()), order);
    }

    #[test]
    fn a_bulk_action_follows_the_list_order_not_the_marks() {
        let order = paths(&["zebra.txt", "apple.txt", "mango.txt"]);
        let marked = BTreeSet::from([
            PathBuf::from("mango.txt"),
            PathBuf::from("zebra.txt"),
        ]);

        assert_eq!(
            bulk_selection(&order, &marked),
            paths(&["zebra.txt", "mango.txt"]),
            "the order on screen is the order things happen in"
        );
    }

    #[test]
    fn a_mark_for_something_no_longer_listed_is_ignored() {
        let order = paths(&["a", "b"]);
        let marked = BTreeSet::from([PathBuf::from("a"), PathBuf::from("gone")]);

        assert_eq!(
            bulk_selection(&order, &marked),
            paths(&["a"]),
            "a button labelled with a count must not act on what is not there"
        );
    }

    #[test]
    fn a_plain_click_replaces_the_selection() {
        let order = paths(&["a", "b", "c"]);
        let mut marked = BTreeSet::new();
        let mut anchor = None;

        click(&order, &mut marked, &mut anchor, "a", PLAIN);
        assert_eq!(names(&marked), ["a"]);

        click(&order, &mut marked, &mut anchor, "c", PLAIN);
        assert_eq!(names(&marked), ["c"], "a plain click is not additive");
        assert_eq!(anchor.as_deref(), Some(Path::new("c")));
    }

    #[test]
    fn the_command_key_adds_and_removes_one_at_a_time() {
        let order = paths(&["a", "b", "c"]);
        let mut marked = BTreeSet::new();
        let mut anchor = None;

        click(&order, &mut marked, &mut anchor, "a", PLAIN);
        click(&order, &mut marked, &mut anchor, "c", TOGGLE);
        assert_eq!(names(&marked), ["a", "c"]);

        // The same click again takes it back out.
        click(&order, &mut marked, &mut anchor, "c", TOGGLE);
        assert_eq!(names(&marked), ["a"]);
    }

    #[test]
    fn shift_takes_everything_between_the_anchor_and_the_click() {
        let order = paths(&["a", "b", "c", "d", "e"]);
        let mut marked = BTreeSet::new();
        let mut anchor = None;

        click(&order, &mut marked, &mut anchor, "b", PLAIN);
        click(&order, &mut marked, &mut anchor, "d", EXTEND);

        assert_eq!(names(&marked), ["b", "c", "d"], "the range includes both ends");
    }

    #[test]
    fn a_range_can_be_drawn_backwards() {
        let order = paths(&["a", "b", "c", "d", "e"]);
        let mut marked = BTreeSet::new();
        let mut anchor = None;

        click(&order, &mut marked, &mut anchor, "d", PLAIN);
        click(&order, &mut marked, &mut anchor, "b", EXTEND);

        assert_eq!(names(&marked), ["b", "c", "d"]);
    }

    #[test]
    fn the_anchor_stays_put_while_a_range_is_adjusted() {
        let order = paths(&["a", "b", "c", "d", "e"]);
        let mut marked = BTreeSet::new();
        let mut anchor = None;

        click(&order, &mut marked, &mut anchor, "b", PLAIN);
        click(&order, &mut marked, &mut anchor, "e", EXTEND);
        assert_eq!(names(&marked), ["b", "c", "d", "e"]);

        // Pulling the range back in should shrink it, not start a new one from
        // where it last ended.
        click(&order, &mut marked, &mut anchor, "c", EXTEND);
        assert_eq!(names(&marked), ["b", "c"]);
        assert_eq!(anchor.as_deref(), Some(Path::new("b")));
    }

    #[test]
    fn shift_with_nothing_to_measure_from_selects_one() {
        let order = paths(&["a", "b", "c"]);
        let mut marked = BTreeSet::new();
        let mut anchor = None;

        click(&order, &mut marked, &mut anchor, "b", EXTEND);

        assert_eq!(names(&marked), ["b"]);
        assert_eq!(anchor.as_deref(), Some(Path::new("b")));
    }

    #[test]
    fn a_range_from_an_anchor_that_has_gone_selects_one() {
        let order = paths(&["a", "b", "c"]);
        let mut marked = BTreeSet::new();

        // The anchor named a file that has since been staged away.
        let mut anchor = Some(PathBuf::from("removed"));

        click(&order, &mut marked, &mut anchor, "b", EXTEND);

        assert_eq!(names(&marked), ["b"]);
        assert_eq!(anchor.as_deref(), Some(Path::new("b")));
    }
}
