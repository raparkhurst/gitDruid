//! Everything gitDruid knows about git.
//!
//! The whole layer is a set of free functions that take a repository path and
//! return owned, `Clone`-able data. Nothing here holds a `git2::Repository`
//! across calls, which keeps the types `Send` and lets the UI run every
//! operation on a background thread without sharing state.

mod commit;
mod detail;
mod diff;
mod history;
mod merge;
mod pick;
mod refs;
mod remote;
mod stage;
mod status;
mod worktree;

pub use commit::commit;
pub use detail::{ChangedFile, CommitDetail, commit_detail};
pub use diff::{Content, FileDiff, Hunk, Line, Origin, Source, commit_file_diff, file_diff};
pub use history::{Badge, BadgeKind, Commit, Edge, History, Row, history};
pub use merge::{finish_branch, merge_branch};
pub use pick::{abort, cherry_pick, revert};
pub use remote::{Tracking, fetch, pull, push, tracking};
pub use refs::{
    Branch, Refs, Tag, checkout_branch, create_branch, create_tag, delete_branch, delete_tag,
    refs, rename_branch,
};
pub use stage::{stage_file, stage_hunk, unstage_file, unstage_hunk};
pub use status::{Change, FileEntry, Side, Snapshot, snapshot};
pub use worktree::{Ignore, discard, ignore, pattern};

use std::fmt;
use std::path::{Path, PathBuf};

/// A git failure, flattened to a message so it can cross a `Task` boundary.
///
/// `git2::Error` is neither `Clone` nor cheap to move through iced messages,
/// so errors are stringified at the point they occur.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(String);

impl Error {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

impl From<git2::Error> for Error {
    fn from(error: git2::Error) -> Self {
        Self(error.message().to_owned())
    }
}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self(error.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// Finds the repository containing `start`, walking up through parents.
///
/// `start` may be a file — one dropped onto the window, say — in which case the
/// search begins at the directory holding it.
pub fn discover(start: &Path) -> Option<PathBuf> {
    let from = if start.is_file() {
        start.parent()?
    } else {
        start
    };

    let repo = git2::Repository::discover(from).ok()?;
    repo.workdir().map(Path::to_path_buf)
}

/// Opens the repository rooted at `path`, rejecting bare repositories.
///
/// gitDruid stages and diffs against a working tree, so a bare repository has
/// nothing for it to show.
fn open(path: &Path) -> Result<git2::Repository> {
    let repo = git2::Repository::open(path)?;

    if repo.is_bare() {
        return Err(Error::new(format!(
            "{} is a bare repository — it has no working tree to stage from",
            path.display()
        )));
    }

    Ok(repo)
}

/// Converts a repo-relative path into the `/`-separated bytes git stores.
fn index_path(path: &Path) -> Result<Vec<u8>> {
    let text = path
        .to_str()
        .ok_or_else(|| Error::new(format!("{} is not valid UTF-8", path.display())))?;

    Ok(text.replace('\\', "/").into_bytes())
}
