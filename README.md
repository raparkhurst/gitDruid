# gitDruid

A cross-platform git GUI focused on building good commits: see what changed,
stage it a hunk at a time, and write the message — on macOS, Windows and Linux
from one binary.

## Running it

```sh
cargo run                          # opens the repository containing the current directory
cargo run -- /path/to/repo
cargo run -- /one/repo /another    # one tab each
```

There is no runtime toolchain to install: the UI is [iced], drawn with wgpu, and
git access goes through [libgit2] in-process.

## How it looks

A splash screen on the way in: `assets/splash.jpg` with the name, the version
and the author over the empty left third of it, in an undecorated window of its
own. It has to be its own window — the point of a splash is to be on screen
*before* the application, and something drawn inside the application's window
cannot be, because that window is already there. It opens alone, the
application comes up behind it five seconds later, and it goes a second after
that. Clicking it skips the wait, and **Settings → Appearance** turns it
off for good.

Monospaced throughout, on one of five palettes: `Console` on warm near-black,
`Dark` for something neutral, `Dracula`, `Matrix`, and `Parchment` on paper.
The choice is written to the global settings file as it is made — nobody picks
a palette and then expects to confirm it — so the window opens the way it was
left. A git client is mostly paths, hashes and diffs, things
that want to line up, and everything in the window shares one grid because of
it. Chrome is hairlines and flat fills rather than cards and shadows; the
accent colour marks what can be acted on, and nothing else. `src/ui/theme.rs`
holds them all, and `src/ui/style.rs` everything derived from them.

## Installing it

Prebuilt: `gitDruid.app` on macOS, and on Linux a `.deb`, an `.rpm`, an Arch
package, an AppImage, or a tarball that runs where it is unpacked and can add a
menu entry without root. See [packaging/README.md](packaging/README.md) for
which to use and how to build each.

On Linux, **Settings → Add to menu** puts a launcher and icons under
`~/.local/share` pointing at wherever gitDruid is running from — which is how
an AppImage, which integrates nothing by design, still gets an icon to click.

## What it does today

Three columns: the refs on the left, the commit graph in the middle, and the
working tree on the right. Several repositories can be open at once, one per
tab.

- Opens the repository containing the working directory, any path arguments,
  one chosen from the folder picker, or a folder dropped onto the window. Each
  opens a tab; any path inside a checkout resolves to its root, and a
  repository already open is switched to rather than opened twice.
- Draws the history of every ref as a graph, with each branch line in its own
  color, and badges for HEAD, branches and tags. A summary too long for the row
  is elided rather than wrapped; selecting the commit shows the whole message,
  its parents, and the files it changed, and selecting one of those files shows
  the diff behind it.
- Lists branches and tags, and creates, checks out, renames, deletes and merges
  branches; creates and deletes tags. Destructive actions confirm first, and
  deleting a branch whose commits HEAD cannot reach says so before it does it.
- Lists unstaged and staged changes in two tabs, each list free to use the
  pane's full height, with rename, deletion, untracked and conflict states
  called out. Several files can be selected at once — ⌘-click to add one,
  ⇧-click for a range — and staged or unstaged together from the button at the
  foot of the list.
- Shows a file's diff with line numbers, and stages or unstages it whole or one
  hunk at a time.
- Writes the commit, refusing empty messages, empty commits and unresolved
  conflicts — and finishes an open merge, writing the commit with both parents.
- Shows the branch and how far ahead of and behind its upstream it is. Which
  checkout a tab is opens on hovering it, and in the settings dialog under
  "This repository".
- Configurable, in a dialog: which workflow this repository follows — a single
  branch, GitHub Flow, or git-flow — what its branches are called, the prefixes
  for features, bugfixes, hotfixes and releases, and how to authenticate to a
  remote.
- Creates branches by workflow. Under git-flow a feature starts from develop
  and a hotfix from main; under GitHub Flow everything comes off the main line
  and goes straight back into it. Either way "Finish" merges a branch into
  wherever it belongs.
- Offers the rest on a right-click: ignoring an untracked file, its extension
  or its folder; discarding unstaged changes; cherry-picking or reverting a
  commit; copying its id; and the branch and tag actions, without going via the
  sidebar.
- Reopens the repositories that were open last time, in the same tab order.
- Fetches, pulls and pushes. The buttons carry the counts — `Pull ↓2`,
  `Push ↑1` — so they say both what they do and why they are worth pressing.
  A first push sets the branch's upstream, the way `git push -u` does.
- Follows changes it did not make. Editing a file, or running `git add` in a
  terminal, shows up on its own within a couple of seconds.

Not there yet: rebasing, cloning, managing remotes, cherry-picking or reverting
a merge commit, and staging individual lines within a hunk.

## How it is put together

```
src/
  git/          libgit2, wrapped in owned data that knows nothing about the UI
    status.rs   what changed, on which side of the index
    diff.rs     hunks and lines for one file
    stage.rs    moving changes between the working tree and the index
    commit.rs   writing the index out as a commit
    history.rs  the commit walk, and which lane each commit is drawn in
    refs.rs     listing branches and tags, and the operations on them
    merge.rs    merging a branch into the one HEAD is on
    pick.rs     cherry-picking and reverting, and aborting either
    worktree.rs ignoring a file, and discarding changes to one
    remote.rs   fetch, pull and push, and what they would act on
    detail.rs   what one commit from the graph changed
  settings.rs   the two configuration files, and what they mean
  desktop.rs    adding a Linux applications-menu entry, when asked
  app.rs        state and the update loop; every git call runs off the UI thread
  ui/           rendering, and the messages the user's clicks produce
    refs.rs     the left column: branches, tags, and what can be done to them
    graph.rs    the centre column: the commit graph
    menu.rs     the right-click menu, and what each row offers
    commit.rs   the centre column, when a commit is selected
    diff.rs     the centre column, for a file from the tree or from a commit
    files.rs    the right column: the two file lists and the commit box
    settings.rs the settings dialog
    splash.rs   the splash screen
    theme.rs    the two palettes
    style.rs    everything derived from whichever palette is active
```

Nothing holds a `git2::Repository` between calls. Each operation takes a
repository path, opens it, and returns plain `Clone`-able data, which is what
lets every git call run on a background task without sharing state with the UI.

### Drawing the graph

Lane assignment happens in `git/history.rs`, not in the widget: which lane a
commit sits in depends on every commit above it, so it is a property of the
history rather than of the row drawing it. A lane holds the commit it is
waiting for; when that commit arrives the lane is freed and handed to the
commit's first parent, which is what makes a branch continue straight down the
graph while its merges fan out sideways.

Each row then draws three kinds of line, and they are separate cases rather
than one case with different endpoints — a line that reaches a commit stops at
it, and a line that leaves one starts at it. Collapsing them draws a stray line
below every root commit. `tests/history.rs` asserts the lane layout against
repositories whose topology is built by the `git` CLI, because a line in the
wrong lane still looks like a graph.

## Settings

Two files, layered:

```
~/.config/gitDruid/config     everywhere
<repo>/.gitdruid              this repository only
```

Both are git's own config format — sections in square brackets, `key = value`
beneath them — because it is the format anyone who opens `.gitdruid` in an
editor already knows:

```
[flow]
	mode = gitflow
	main = main
	develop = develop
[prefix]
	feature = feature/
[credentials]
	sshkey = ~/.ssh/id_ed25519
```

A repository's file overrides the global one key by key, and anything neither
sets falls back to a built-in default, so an installation with no files at all
still works. In the dialog's repository scope every box may be left empty, and
an empty box shows the value it would inherit as its placeholder — which is
what makes "what happens if I leave this alone" answerable without looking
anywhere else. Toggles get a third state there for the same reason: "off" and
"whatever the global file says" are different answers.

Paths to ssh keys can be typed or picked with a file browser, and are stored
relative to `~` so a settings file copied to another machine still points at
that machine's key.

The dialog names the file it will write to, and offers to copy that path. It
wraps at glyph level rather than at word level, because a filesystem path
contains no spaces and ordinary wrapping cannot break one.

gitDruid does not store passphrases. A key that has one belongs in the ssh
agent, which is tried after any key named in the settings.

### The status strip

Questions and results share one strip under the toolbar, and it keeps its
height whether or not it has anything in it. Growing a bar into existence
pushes the whole window down and pulls it back up a moment later, which is
worse than the space it saves — and it moves whatever the user was about to
click. A question outranks a result there, because a question is blocking
something and a result is only there to be read.

### Workflows

Three, and the difference between them is where a branch starts and what it
merges back into:

| | starts from | merges into | named |
|---|---|---|---|
| Single branch | main | main | no |
| GitHub Flow | main | main | yes |
| git-flow | develop, or main for a hotfix | the one it came from | yes |

GitHub Flow has no release branches, because a release is whatever the main
line is at the time — so the settings dialog does not offer a release prefix
there, and a `release/` branch is not something it will offer to finish.

`flow.mode` is `simple`, `github` or `gitflow`. A settings file written before
there were three still reads: the old `flow.enabled` boolean is consulted when
`flow.mode` is absent.

### Selecting several files

A file list has two selections at once, and they are different questions: which
files a bulk action would touch, and whose diff is on screen. A plain click
sets both, ⌘-click adds to the first alone, and ⇧-click takes the range from
the anchor. The marked rows are tinted and the one being shown is tinted
harder, so eight selected files read as one block with the current file picked
out of it.

The button at the foot of the list says what it will do rather than what it
could — `Stage all (12)` with nothing marked, `Stage 3 selected` with three —
so nothing has to be counted by eye first. Marks are dropped for files that
leave the list, because a button labelled with a count must not act on what is
no longer there.

Both behaviours live in free functions over a list of paths rather than in
`Repo`, since a shift range off by one row is not something a screenshot would
show; `app.rs` tests them directly.

The right-click menu acts on the selection too, and says so: `Stage these 3
files` rather than `Stage this file`. Right-clicking a row *outside* the
selection narrows the selection to that row first, so the menu never offers to
act on files highlighted somewhere else in the list. Ignoring is the exception
— it stays per-file, because three files rarely want the same rule and a wrong
one is quiet until it bites.

### The right-click menu

iced has no context menu, so this one is a `stack`: a transparent sheet over
the whole window that swallows the next click — which is what makes clicking
anywhere else dismiss it — and the menu itself above the sheet, offset by
padding to sit under the pointer and clamped to stay inside the window. A
button press carries no position in iced, so the pointer is tracked as it
moves; that costs little, because hover styling already re-renders the view on
every movement.

What a menu offers depends on the state of the row it was opened on: an ignore
entry only for a file git is not tracking yet, no delete for the branch you are
standing on, no discard on the staged side because unstaging is the answer
there. That is a judgement rather than a layout, so `ui::menu::items` takes a
`Context` of plain git data rather than the application, and `tests/menu.rs`
asserts what each kind of row offers against real repositories.

Cherry-picking and reverting are the same shape as a merge — apply, then either
commit or leave the conflicts — so both finish through the ordinary commit
button, and `abort` puts a half-done one back.

### Talking to a remote

`git2` ships with no transports enabled, so `Cargo.toml` turns on `ssh` and
`https`; without them there is nothing for fetch, pull or push to connect to.
Credentials follow the ladder git itself uses — the ssh agent, then git's
credential helper — and the callback counts its attempts, because libgit2 asks
again for every method it is willing to try and a missing agent otherwise turns
into an endless retry rather than an error.

Push refuses a branch that is behind before it opens a connection, since the
server would reject it anyway and gitDruid can say why in its own words. It
also reads libgit2's per-ref rejection callback: a push whose ref the server
turned down still returns `Ok`, so without that a rejected push would report
success.

Pull is a fetch followed by the same merge code a branch merge uses, so its
four outcomes are the same ones — up to date, fast-forward, a merge commit, or
conflicts left in the working tree for the existing flow to finish.

Credentials are tried in a fixed order — a key named in the settings first,
since someone who named one meant that one; then the agent; then the helper.
The closure works down that list and refuses once it runs out, rather than
letting libgit2 ask forever.

`tests/remote.rs` runs all of this against a bare repository in a temp
directory over libgit2's local transport: real pushes and real fetches, minus
the credentials.

### One state per tab

Each tab owns its whole state — snapshot, refs, graph, selection, commit
message, and its own `busy` flag, so work in one tab does not grey out the
buttons in another. That splits messages in two, and `app.rs` says which is
which: a message the user produced can only have come from the tab on screen,
so it applies to the active repository, while a message carrying the result of
a git call names the repository it was read for. The user is free to switch
tabs while a read is in flight, and a result whose tab has since been closed is
dropped rather than landing on whichever tab happens to be in front.

Only the active repository is polled. The others are not being looked at, and a
tab comes up to date the moment it is switched to.

### Keeping up with the working tree

gitDruid re-reads the working tree every two seconds and adopts the result only
when it differs from what is on screen, so an unchanged repository costs a
status call and nothing else — no re-read of the diff, no reload of the graph,
no scroll position lost. Polling stops while the window is in the background,
and runs once immediately when it comes forward, so the list is current by the
time anyone looks at it. A poll that fails is dropped rather than reported: the
working tree gets read while other tools are writing to it, and an `index.lock`
held by someone else's `git add` is not news.

The interval is `app::POLL_INTERVAL`. It is a full `git status` each time, which
is cheap on an ordinary repository and less so on a very large one.

### Staging a single hunk

Whole-file staging maps onto libgit2's index calls. A single hunk does not, so
gitDruid rebuilds the file's index blob: it takes the content currently in the
index, and splices the hunk's other side into the line range the hunk covers.

Before writing anything it checks that the range still holds exactly what the
hunk expected. A diff that has gone stale — because the file changed on disk, or
another process touched the index — is refused with a message rather than
applied to the wrong lines. `tests/staging.rs` checks the results against the
`git` CLI itself, including files with no trailing newline.

## Tests

```sh
cargo test
```

The integration tests build real repositories in a temp directory and assert
against `git diff --cached`, so a bug in gitDruid's diff reader cannot hide a bug
in its staging.

[iced]: https://iced.rs
[libgit2]: https://libgit2.org
