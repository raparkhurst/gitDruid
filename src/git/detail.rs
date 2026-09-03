//! What one commit from the graph did.
//!
//! This is the read-only counterpart to [`super::status`]: the same shape of
//! answer — which files changed, and how — but for a commit that already
//! exists rather than for the working tree.

use std::path::{Path, PathBuf};

use git2::{Diff, DiffOptions, Patch};

use super::status::Change;
use super::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedFile {
    pub path: PathBuf,
    /// Set only for renames; the path the file had before.
    pub old_path: Option<PathBuf>,
    pub change: Change,
    pub added: usize,
    pub removed: usize,
}

impl ChangedFile {
    pub fn display(&self) -> String {
        match &self.old_path {
            Some(old) if old != &self.path => {
                format!("{} → {}", old.display(), self.path.display())
            }
            _ => self.path.display().to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitDetail {
    pub id: String,
    pub short_id: String,
    pub author: String,
    pub email: String,
    pub when: String,
    /// The whole message, not just the summary line.
    pub message: String,
    /// Abbreviated ids of the parents, so a merge is visible as one.
    pub parents: Vec<String>,
    pub files: Vec<ChangedFile>,
    /// True when the file list was cut short; huge commits are not worth
    /// rendering in full.
    pub truncated: bool,
}

/// How many changed files to describe. A commit touching more than this is
/// almost always a bulk move, and the list stops being informative long before
/// it stops being long.
const FILE_LIMIT: usize = 400;

/// Reads one commit and the diff against its first parent.
///
/// A merge is shown against its first parent, which is the convention `git
/// show` follows: the second parent's changes are the ones being merged in and
/// are already described by the branch they came from.
pub fn commit_detail(repo_path: &Path, id: &str) -> Result<CommitDetail> {
    let repo = super::open(repo_path)?;

    let oid = git2::Oid::from_str(id).map_err(|_| Error::new(format!("{id} is not a commit id")))?;
    let commit = repo
        .find_commit(oid)
        .map_err(|_| Error::new(format!("no commit {id} in this repository")))?;

    let tree = commit.tree()?;
    let parent = commit.parent(0).ok();
    let parent_tree = match &parent {
        Some(parent) => Some(parent.tree()?),
        None => None,
    };

    let mut options = DiffOptions::new();
    options.include_typechange(true);

    let mut diff =
        repo.diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), Some(&mut options))?;

    diff.find_similar(None)?;

    let (files, truncated) = changed_files(&diff)?;

    let author = commit.author();

    Ok(CommitDetail {
        id: oid.to_string(),
        short_id: format!("{:.7}", oid),
        author: author.name().unwrap_or("(unknown author)").to_owned(),
        email: author.email().unwrap_or_default().to_owned(),
        when: super::history::format_time(commit.time()),
        message: commit.message().unwrap_or("(no message)").trim().to_owned(),
        parents: commit
            .parent_ids()
            .map(|parent| format!("{:.7}", parent))
            .collect(),
        files,
        truncated,
    })
}

fn changed_files(diff: &Diff<'_>) -> Result<(Vec<ChangedFile>, bool)> {
    let total = diff.deltas().len();
    let mut files = Vec::new();

    for (index, delta) in diff.deltas().enumerate().take(FILE_LIMIT) {
        let Some(change) = describe(delta.status()) else {
            continue;
        };

        let path = delta
            .new_file()
            .path()
            .or_else(|| delta.old_file().path())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("<unknown path>"));

        let old_path = match change {
            Change::Renamed => delta.old_file().path().map(Path::to_path_buf),
            _ => None,
        };

        // A binary file has no line counts, and asking for a patch for one
        // costs a read that produces nothing.
        let (added, removed) = match Patch::from_diff(diff, index)? {
            Some(patch) => {
                let (_, added, removed) = patch.line_stats()?;
                (added, removed)
            }
            None => (0, 0),
        };

        files.push(ChangedFile {
            path,
            old_path,
            change,
            added,
            removed,
        });
    }

    Ok((files, total > FILE_LIMIT))
}

fn describe(status: git2::Delta) -> Option<Change> {
    match status {
        git2::Delta::Added => Some(Change::Added),
        git2::Delta::Deleted => Some(Change::Deleted),
        git2::Delta::Modified => Some(Change::Modified),
        git2::Delta::Renamed | git2::Delta::Copied => Some(Change::Renamed),
        git2::Delta::Typechange => Some(Change::TypeChange),
        git2::Delta::Conflicted => Some(Change::Conflicted),
        _ => None,
    }
}
