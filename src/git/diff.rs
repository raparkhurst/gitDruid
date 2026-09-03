//! Producing a diff for a single file, in a form the UI can render and the
//! staging code can apply.

use std::path::{Path, PathBuf};

use git2::{Diff, DiffFindOptions, DiffOptions, Patch, Repository};

use super::detail::ChangedFile;
use super::{Change, Error, FileEntry, Result, Side};

/// Which side of a diff a line came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// Unchanged; present on both sides.
    Context,
    /// Present only on the new side.
    Addition,
    /// Present only on the old side.
    Deletion,
    /// The `\ No newline at end of file` marker. Carries no file content.
    NoNewline,
}

impl Origin {
    /// The character git prints in the first column.
    pub fn sign(self) -> &'static str {
        match self {
            Origin::Context => " ",
            Origin::Addition => "+",
            Origin::Deletion => "-",
            Origin::NoNewline => "\\",
        }
    }

    /// Whether this line exists in the old side of the diff.
    fn in_old(self) -> bool {
        matches!(self, Origin::Context | Origin::Deletion)
    }

    /// Whether this line exists in the new side of the diff.
    fn in_new(self) -> bool {
        matches!(self, Origin::Context | Origin::Addition)
    }
}

/// A single line of a hunk.
///
/// `content` holds the raw bytes, including the trailing newline when the line
/// has one. Keeping bytes rather than a `String` means applying a hunk
/// reproduces the file exactly, even when it is not valid UTF-8.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub origin: Origin,
    pub old_lineno: Option<u32>,
    pub new_lineno: Option<u32>,
    pub content: Vec<u8>,
}

impl Line {
    /// The line's text for display, with the line terminator removed.
    pub fn text(&self) -> String {
        let mut bytes = self.content.as_slice();

        if let Some(rest) = bytes.strip_suffix(b"\n") {
            bytes = rest;
        }

        if let Some(rest) = bytes.strip_suffix(b"\r") {
            bytes = rest;
        }

        String::from_utf8_lossy(bytes).into_owned()
    }
}

/// A contiguous run of changed lines, with its surrounding context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    /// The `@@ -1,7 +1,9 @@` header, including any trailing function context.
    pub header: String,
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
    pub lines: Vec<Line>,
}

impl Hunk {
    /// Counts the added and removed lines, for the hunk summary.
    pub fn counts(&self) -> (usize, usize) {
        self.lines
            .iter()
            .fold((0, 0), |(added, removed), line| match line.origin {
                Origin::Addition => (added + 1, removed),
                Origin::Deletion => (added, removed + 1),
                _ => (added, removed),
            })
    }

    /// The bytes this hunk expects to find in the old side of the file.
    pub(super) fn old_side(&self) -> Vec<u8> {
        self.side_bytes(Origin::in_old)
    }

    /// The bytes this hunk produces on the new side of the file.
    pub(super) fn new_side(&self) -> Vec<u8> {
        self.side_bytes(Origin::in_new)
    }

    fn side_bytes(&self, include: fn(Origin) -> bool) -> Vec<u8> {
        let mut bytes = Vec::new();

        for line in &self.lines {
            if include(line.origin) {
                bytes.extend_from_slice(&line.content);
            }
        }

        bytes
    }
}

/// What a file's diff turned out to contain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Content {
    Text(Vec<Hunk>),
    /// git considers the file binary, so there is nothing line-based to show.
    Binary,
    /// The file is listed as changed but produced no hunks — a pure mode
    /// change, for instance.
    Empty,
}

/// Where a diff came from, and so what can be done with it.
///
/// A diff is either a change that has not landed yet — which can be moved
/// between the working tree and the index — or one that has. Carrying a bare
/// `Side` for both would mean answering "which side would staging apply to?"
/// for a commit, where the question has no answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// A change between HEAD, the index and the working tree.
    Working(Side),
    /// A change that is already committed, named by the commit's id.
    Commit(String),
}

impl Source {
    /// The side a staging action would apply to, if staging applies at all.
    pub fn side(&self) -> Option<Side> {
        match self {
            Source::Working(side) => Some(*side),
            Source::Commit(_) => None,
        }
    }
}

/// A file's diff, plus what staging needs to write it back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDiff {
    pub path: PathBuf,
    pub old_path: Option<PathBuf>,
    pub source: Source,
    pub change: Change,
    pub content: Content,
    /// File mode on the old side, or 0 when the file is absent there.
    pub old_mode: u32,
    /// File mode on the new side, or 0 when the file is absent there.
    pub new_mode: u32,
}

impl FileDiff {
    pub fn hunks(&self) -> &[Hunk] {
        match &self.content {
            Content::Text(hunks) => hunks,
            _ => &[],
        }
    }

    /// Total added and removed lines across every hunk.
    pub fn counts(&self) -> (usize, usize) {
        self.hunks().iter().fold((0, 0), |(added, removed), hunk| {
            let (a, r) = hunk.counts();
            (added + a, removed + r)
        })
    }
}

/// Builds the diff for one file on one side of the working tree.
pub fn file_diff(repo_path: &Path, entry: &FileEntry) -> Result<FileDiff> {
    let repo = super::open(repo_path)?;

    let mut options = DiffOptions::new();
    options
        .include_untracked(true)
        .recurse_untracked_dirs(true)
        .show_untracked_content(true);

    restrict(&mut options, &entry.path, entry.old_path.as_deref());

    let mut diff = build_diff(&repo, entry.side, &mut options)?;

    find_renames(&mut diff, entry.old_path.is_some())?;

    assemble(
        &diff,
        &entry.path,
        entry.old_path.clone(),
        entry.change,
        Source::Working(entry.side),
    )
}

/// Builds the diff for one file as one commit changed it.
///
/// The comparison is against the commit's first parent, matching what `git
/// show` prints and what [`super::commit_detail`] lists — so the file list and
/// the diff behind it always agree.
pub fn commit_file_diff(repo_path: &Path, id: &str, file: &ChangedFile) -> Result<FileDiff> {
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
    restrict(&mut options, &file.path, file.old_path.as_deref());

    let mut diff =
        repo.diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), Some(&mut options))?;

    find_renames(&mut diff, file.old_path.is_some())?;

    assemble(
        &diff,
        &file.path,
        file.old_path.clone(),
        file.change,
        Source::Commit(oid.to_string()),
    )
}

/// Narrows a diff to one file, and to the name it had on the other side of a
/// rename.
fn restrict(options: &mut DiffOptions, path: &Path, old_path: Option<&Path>) {
    options.context_lines(3);

    // Paths come from status or from a delta, so they are literal. Without
    // this, a file named `foo[1].txt` would be read as a glob and match
    // nothing.
    options.disable_pathspec_match(true);
    options.pathspec(path);

    if let Some(old_path) = old_path {
        options.pathspec(old_path);
    }
}

fn find_renames(diff: &mut Diff<'_>, renamed: bool) -> Result<()> {
    if !renamed {
        return Ok(());
    }

    let mut find = DiffFindOptions::new();
    find.renames(true).copies(false);
    diff.find_similar(Some(&mut find))?;

    Ok(())
}

/// Reads the one delta the diff was narrowed to.
fn assemble(
    diff: &Diff<'_>,
    path: &Path,
    old_path: Option<PathBuf>,
    change: Change,
    source: Source,
) -> Result<FileDiff> {
    let index = locate_delta(diff, path).ok_or_else(|| match &source {
        Source::Working(_) => Error::new(format!(
            "{} is no longer changed — refresh to see the current state",
            path.display()
        )),
        Source::Commit(id) => Error::new(format!(
            "{} was not changed by {:.7}",
            path.display(),
            id
        )),
    })?;

    let delta = diff
        .get_delta(index)
        .ok_or_else(|| Error::new("diff changed while it was being read"))?;

    let old_mode = u32::from(delta.old_file().mode());
    let new_mode = u32::from(delta.new_file().mode());

    let content = if delta.flags().contains(git2::DiffFlags::BINARY) {
        Content::Binary
    } else {
        match Patch::from_diff(diff, index)? {
            Some(patch) => read_hunks(&patch)?,
            // libgit2 declines to produce a patch for binary content.
            None => Content::Binary,
        }
    };

    Ok(FileDiff {
        path: path.to_path_buf(),
        old_path,
        source,
        change,
        content,
        old_mode,
        new_mode,
    })
}

fn build_diff<'repo>(
    repo: &'repo Repository,
    side: Side,
    options: &mut DiffOptions,
) -> Result<Diff<'repo>> {
    let diff = match side {
        Side::Worktree => repo.diff_index_to_workdir(None, Some(options))?,
        Side::Index => {
            // Before the first commit there is no HEAD tree; diffing against
            // `None` treats every staged file as newly added, which is exactly
            // what it is.
            let tree = match repo.head() {
                Ok(head) => Some(head.peel_to_tree()?),
                Err(_) => None,
            };

            repo.diff_tree_to_index(tree.as_ref(), None, Some(options))?
        }
    };

    Ok(diff)
}

/// Finds the delta matching the path, on either side of a rename.
fn locate_delta(diff: &Diff<'_>, path: &Path) -> Option<usize> {
    (0..diff.deltas().len()).find(|index| {
        let Some(delta) = diff.get_delta(*index) else {
            return false;
        };

        delta.new_file().path() == Some(path) || delta.old_file().path() == Some(path)
    })
}

fn read_hunks(patch: &Patch<'_>) -> Result<Content> {
    let hunk_count = patch.num_hunks();

    if hunk_count == 0 {
        return Ok(Content::Empty);
    }

    let mut hunks = Vec::with_capacity(hunk_count);

    for index in 0..hunk_count {
        let (hunk, line_count) = patch.hunk(index)?;

        let mut lines = Vec::with_capacity(line_count);

        for line_index in 0..line_count {
            let line = patch.line_in_hunk(index, line_index)?;

            lines.push(Line {
                origin: origin_of(line.origin_value()),
                old_lineno: line.old_lineno(),
                new_lineno: line.new_lineno(),
                content: line.content().to_vec(),
            });
        }

        hunks.push(Hunk {
            header: String::from_utf8_lossy(hunk.header()).trim_end().to_owned(),
            old_start: hunk.old_start(),
            old_lines: hunk.old_lines(),
            new_start: hunk.new_start(),
            new_lines: hunk.new_lines(),
            lines,
        });
    }

    Ok(Content::Text(hunks))
}

fn origin_of(origin: git2::DiffLineType) -> Origin {
    use git2::DiffLineType::*;

    match origin {
        Addition => Origin::Addition,
        Deletion => Origin::Deletion,
        // The three EOFNL variants carry the "\ No newline" marker text rather
        // than file content. Whether a line ends in a newline is already
        // encoded in the preceding line's bytes.
        ContextEOFNL | AddEOFNL | DeleteEOFNL => Origin::NoNewline,
        _ => Origin::Context,
    }
}
