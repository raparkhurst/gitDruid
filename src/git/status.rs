//! Working-tree and index status: what changed, and on which side.

use std::path::{Path, PathBuf};

use git2::{Repository, Status, StatusOptions};

use super::{Error, Result};

/// Which of the two diffs a file entry belongs to.
///
/// git compares three trees: HEAD, the index, and the working tree. Every
/// change gitDruid shows lives in one of the two gaps between them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    /// Index → working tree. The "unstaged" list.
    Worktree,
    /// HEAD → index. The "staged" list.
    Index,
}

impl Side {
    pub fn title(self) -> &'static str {
        match self {
            Side::Worktree => "Unstaged",
            Side::Index => "Staged",
        }
    }
}

/// How a single file changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    Added,
    Modified,
    Deleted,
    Renamed,
    TypeChange,
    Untracked,
    Conflicted,
}

impl Change {
    /// The single-letter badge git itself uses.
    pub fn badge(self) -> &'static str {
        match self {
            Change::Added => "A",
            Change::Modified => "M",
            Change::Deleted => "D",
            Change::Renamed => "R",
            Change::TypeChange => "T",
            Change::Untracked => "?",
            Change::Conflicted => "!",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Change::Added => "added",
            Change::Modified => "modified",
            Change::Deleted => "deleted",
            Change::Renamed => "renamed",
            Change::TypeChange => "type changed",
            Change::Untracked => "untracked",
            Change::Conflicted => "conflicted",
        }
    }
}

/// One changed file on one side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    pub path: PathBuf,
    /// Set only for renames; the path the file had on the other side.
    pub old_path: Option<PathBuf>,
    pub change: Change,
    pub side: Side,
}

impl FileEntry {
    /// The path as it should read in the file list, showing renames as `old → new`.
    pub fn display(&self) -> String {
        match &self.old_path {
            Some(old) if old != &self.path => {
                format!("{} → {}", old.display(), self.path.display())
            }
            _ => self.path.display().to_string(),
        }
    }
}

/// Where HEAD points, and how it compares to its upstream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Head {
    /// Branch name, or a short commit id when detached.
    pub label: String,
    pub detached: bool,
    /// True before the first commit, when HEAD points at a branch that does not exist yet.
    pub unborn: bool,
    /// Commits ahead of / behind the upstream branch, when there is one.
    pub upstream: Option<Upstream>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upstream {
    pub name: String,
    pub ahead: usize,
    pub behind: usize,
}

/// A complete picture of the repository at one moment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub path: PathBuf,
    pub head: Head,
    pub unstaged: Vec<FileEntry>,
    pub staged: Vec<FileEntry>,
    /// Set when the repository is mid-merge, mid-rebase, etc.
    pub pending_operation: Option<String>,
}

impl Snapshot {
    pub fn entries(&self, side: Side) -> &[FileEntry] {
        match side {
            Side::Worktree => &self.unstaged,
            Side::Index => &self.staged,
        }
    }

    pub fn find(&self, side: Side, path: &Path) -> Option<&FileEntry> {
        self.entries(side).iter().find(|entry| entry.path == path)
    }

    /// The repository directory name, for the window title.
    pub fn name(&self) -> String {
        self.path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.path.display().to_string())
    }
}

/// Reads the full status of the repository at `path`.
pub fn snapshot(path: &Path) -> Result<Snapshot> {
    let repo = super::open(path)?;

    let workdir = repo
        .workdir()
        .ok_or_else(|| Error::new("repository has no working tree"))?
        .to_path_buf();

    let head = read_head(&repo)?;

    let mut options = StatusOptions::new();
    options
        .include_untracked(true)
        .recurse_untracked_dirs(true)
        .include_ignored(false)
        .include_unmodified(false)
        .renames_head_to_index(true)
        .renames_index_to_workdir(true);

    let statuses = repo.statuses(Some(&mut options))?;

    let mut unstaged = Vec::new();
    let mut staged = Vec::new();

    for entry in statuses.iter() {
        let status = entry.status();

        // A conflicted path is neither staged nor unstaged — it needs
        // resolving first, so it is surfaced in the working-tree list where
        // the user is already looking for work to do.
        if status.is_conflicted() {
            let path = entry
                .path()
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from("<non-utf8 path>"));

            unstaged.push(FileEntry {
                path,
                old_path: None,
                change: Change::Conflicted,
                side: Side::Worktree,
            });

            continue;
        }

        if let Some(change) = index_change(status) {
            let delta = entry.head_to_index();

            staged.push(FileEntry {
                path: delta_path(delta.as_ref(), entry.path().ok(), true),
                old_path: rename_source(delta.as_ref(), change),
                change,
                side: Side::Index,
            });
        }

        if let Some(change) = worktree_change(status) {
            let delta = entry.index_to_workdir();

            unstaged.push(FileEntry {
                path: delta_path(delta.as_ref(), entry.path().ok(), true),
                old_path: rename_source(delta.as_ref(), change),
                change,
                side: Side::Worktree,
            });
        }
    }

    unstaged.sort_by(|a, b| a.path.cmp(&b.path));
    staged.sort_by(|a, b| a.path.cmp(&b.path));

    Ok(Snapshot {
        path: workdir,
        head,
        unstaged,
        staged,
        pending_operation: pending_operation(&repo),
    })
}

fn index_change(status: Status) -> Option<Change> {
    if status.contains(Status::INDEX_NEW) {
        Some(Change::Added)
    } else if status.contains(Status::INDEX_DELETED) {
        Some(Change::Deleted)
    } else if status.contains(Status::INDEX_RENAMED) {
        Some(Change::Renamed)
    } else if status.contains(Status::INDEX_TYPECHANGE) {
        Some(Change::TypeChange)
    } else if status.contains(Status::INDEX_MODIFIED) {
        Some(Change::Modified)
    } else {
        None
    }
}

fn worktree_change(status: Status) -> Option<Change> {
    if status.contains(Status::WT_NEW) {
        Some(Change::Untracked)
    } else if status.contains(Status::WT_DELETED) {
        Some(Change::Deleted)
    } else if status.contains(Status::WT_RENAMED) {
        Some(Change::Renamed)
    } else if status.contains(Status::WT_TYPECHANGE) {
        Some(Change::TypeChange)
    } else if status.contains(Status::WT_MODIFIED) {
        Some(Change::Modified)
    } else {
        None
    }
}

/// Picks the path to show for a delta.
///
/// A deleted file has no new-side path, so it falls back to the old side; the
/// status entry's own path is the last resort.
fn delta_path(
    delta: Option<&git2::DiffDelta<'_>>,
    fallback: Option<&str>,
    prefer_new: bool,
) -> PathBuf {
    let from_delta = delta.and_then(|delta| {
        let (first, second) = if prefer_new {
            (delta.new_file().path(), delta.old_file().path())
        } else {
            (delta.old_file().path(), delta.new_file().path())
        };

        first.or(second).map(Path::to_path_buf)
    });

    from_delta
        .or_else(|| fallback.map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("<non-utf8 path>"))
}

fn rename_source(delta: Option<&git2::DiffDelta<'_>>, change: Change) -> Option<PathBuf> {
    if change != Change::Renamed {
        return None;
    }

    delta
        .and_then(|delta| delta.old_file().path())
        .map(Path::to_path_buf)
}

fn read_head(repo: &Repository) -> Result<Head> {
    if repo.head_detached().unwrap_or(false) {
        let label = repo
            .head()
            .ok()
            .and_then(|head| head.target())
            .map(|oid| format!("{:.7}", oid))
            .unwrap_or_else(|| "unknown".to_owned());

        return Ok(Head {
            label,
            detached: true,
            unborn: false,
            upstream: None,
        });
    }

    let head = match repo.head() {
        Ok(head) => head,
        // An unborn HEAD is the state of a fresh `git init`: the ref exists in
        // .git/HEAD but points at a branch with no commits.
        Err(_) => {
            let label = repo
                .find_reference("HEAD")
                .ok()
                .and_then(|reference| {
                    reference
                        .symbolic_target()
                        .ok()
                        .flatten()
                        .map(str::to_owned)
                })
                .map(|target| target.trim_start_matches("refs/heads/").to_owned())
                .unwrap_or_else(|| "main".to_owned());

            return Ok(Head {
                label,
                detached: false,
                unborn: true,
                upstream: None,
            });
        }
    };

    let label = head.shorthand().unwrap_or("HEAD").to_owned();
    let upstream = read_upstream(repo, &head);

    Ok(Head {
        label,
        detached: false,
        unborn: false,
        upstream,
    })
}

fn read_upstream(repo: &Repository, head: &git2::Reference<'_>) -> Option<Upstream> {
    let name = head.shorthand().ok()?;
    let branch = repo.find_branch(name, git2::BranchType::Local).ok()?;
    let upstream = branch.upstream().ok()?;

    let upstream_name = upstream
        .name()
        .ok()
        .flatten()
        .unwrap_or("upstream")
        .to_owned();

    let local = head.target()?;
    let remote = upstream.get().target()?;
    let (ahead, behind) = repo.graph_ahead_behind(local, remote).ok()?;

    Some(Upstream {
        name: upstream_name,
        ahead,
        behind,
    })
}

/// Describes an in-progress multi-step operation, so the UI can warn instead of
/// letting the user commit into a half-finished merge or rebase.
fn pending_operation(repo: &Repository) -> Option<String> {
    let label = match repo.state() {
        git2::RepositoryState::Clean => return None,
        git2::RepositoryState::Merge => "merge",
        git2::RepositoryState::Revert | git2::RepositoryState::RevertSequence => "revert",
        git2::RepositoryState::CherryPick | git2::RepositoryState::CherryPickSequence => {
            "cherry-pick"
        }
        git2::RepositoryState::Bisect => "bisect",
        git2::RepositoryState::Rebase
        | git2::RepositoryState::RebaseInteractive
        | git2::RepositoryState::RebaseMerge => "rebase",
        git2::RepositoryState::ApplyMailbox | git2::RepositoryState::ApplyMailboxOrRebase => "am",
    };

    Some(label.to_owned())
}
