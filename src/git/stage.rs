//! Moving changes between the working tree and the index.
//!
//! Whole-file operations map onto libgit2's index calls directly. Hunk-level
//! operations are done by rebuilding the file's index blob: take the content
//! currently in the index, splice the hunk's other side into the line range it
//! covers, and write the result back. The old side is verified against the
//! index before anything is written, so a diff that has gone stale is refused
//! rather than applied to the wrong lines.

use std::path::Path;

use git2::{Index, IndexEntry, IndexTime, Repository};

use super::{Change, Error, FileDiff, FileEntry, Hunk, Result, Side};

/// Stages every change in a file: `git add <path>`.
pub fn stage_file(repo_path: &Path, entry: &FileEntry) -> Result<()> {
    let repo = super::open(repo_path)?;
    let mut index = repo.index()?;

    match entry.change {
        // The file is gone from disk, so there is nothing to read and add.
        Change::Deleted => {
            index.remove_path(&entry.path)?;
        }
        Change::Renamed => {
            if let Some(old_path) = &entry.old_path {
                // A missing old entry is fine; it just means the rename was
                // already partly staged.
                let _ = index.remove_path(old_path);
            }

            index.add_path(&entry.path)?;
        }
        _ => {
            index.add_path(&entry.path)?;
        }
    }

    index.write()?;

    Ok(())
}

/// Unstages every change in a file: `git reset -- <path>`.
pub fn unstage_file(repo_path: &Path, entry: &FileEntry) -> Result<()> {
    let repo = super::open(repo_path)?;

    let Ok(head) = repo.head() else {
        // With no commits yet there is nothing to reset to, so unstaging can
        // only mean dropping the entry.
        let mut index = repo.index()?;
        index.remove_path(&entry.path)?;
        index.write()?;

        return Ok(());
    };

    let commit = head.peel_to_commit()?;

    let mut paths = vec![entry.path.clone()];

    if let Some(old_path) = &entry.old_path {
        paths.push(old_path.clone());
    }

    repo.reset_default(Some(commit.as_object()), paths.iter())?;

    Ok(())
}

/// Stages a single hunk of a file's unstaged changes.
pub fn stage_hunk(repo_path: &Path, diff: &FileDiff, hunk: &Hunk) -> Result<()> {
    if diff.source.side() != Some(Side::Worktree) {
        return Err(Error::new("that hunk cannot be staged"));
    }

    ensure_hunk_supported(diff)?;

    let repo = super::open(repo_path)?;
    let mut index = repo.index()?;

    let base = index_content(&repo, &index, &diff.path)?;

    // The unstaged diff runs index → working tree, so the hunk's old side is
    // what the index holds and its new side is what staging should produce.
    let updated = splice(
        &base,
        line_range(hunk.old_start, hunk.old_lines),
        &hunk.old_side(),
        &hunk.new_side(),
        &diff.path,
    )?;

    if diff.change == Change::Deleted && updated.is_empty() {
        index.remove_path(&diff.path)?;
    } else {
        let mode = resolve_mode(&index, &diff.path, diff.new_mode);
        write_entry(&repo, &mut index, &diff.path, &updated, mode)?;
    }

    index.write()?;

    Ok(())
}

/// Unstages a single hunk of a file's staged changes.
pub fn unstage_hunk(repo_path: &Path, diff: &FileDiff, hunk: &Hunk) -> Result<()> {
    if diff.source.side() != Some(Side::Index) {
        return Err(Error::new("that hunk is not staged"));
    }

    ensure_hunk_supported(diff)?;

    let repo = super::open(repo_path)?;
    let mut index = repo.index()?;

    let base = index_content(&repo, &index, &diff.path)?;

    // The staged diff runs HEAD → index, so here the index is the *new* side
    // and unstaging means putting the old side back.
    let updated = splice(
        &base,
        line_range(hunk.new_start, hunk.new_lines),
        &hunk.new_side(),
        &hunk.old_side(),
        &diff.path,
    )?;

    if diff.change == Change::Added && updated.is_empty() {
        // The file is not in HEAD, so an empty result means it should not be
        // in the index either.
        index.remove_path(&diff.path)?;
    } else {
        let mode = resolve_mode(&index, &diff.path, diff.old_mode);
        write_entry(&repo, &mut index, &diff.path, &updated, mode)?;
    }

    index.write()?;

    Ok(())
}

/// Rejects the cases where rewriting a single blob is not enough to express the
/// change, so the user gets a clear message instead of a half-applied index.
fn ensure_hunk_supported(diff: &FileDiff) -> Result<()> {
    let reason = match diff.change {
        Change::Renamed => "renames",
        Change::TypeChange => "type changes",
        Change::Conflicted => "conflicted files",
        _ => return Ok(()),
    };

    Err(Error::new(format!(
        "hunk-level staging does not handle {reason} — use the whole-file button instead"
    )))
}

/// The line range a hunk covers, as a zero-based start and a length.
fn line_range(start: u32, count: u32) -> (usize, usize) {
    // git line numbers are 1-based. A zero-length range is the exception: its
    // start is the line *after* which content is inserted, which is already
    // the right zero-based insertion point.
    let start = if count == 0 {
        start as usize
    } else {
        start.saturating_sub(1) as usize
    };

    (start, count as usize)
}

/// Replaces a line range of `base` with `replacement`, after checking that the
/// range currently holds `expected`.
fn splice(
    base: &[u8],
    (start, len): (usize, usize),
    expected: &[u8],
    replacement: &[u8],
    path: &Path,
) -> Result<Vec<u8>> {
    let lines = line_lengths(base);

    let end = start
        .checked_add(len)
        .filter(|end| *end <= lines.len())
        .ok_or_else(|| stale(path))?;

    let prefix: usize = lines[..start].iter().sum();
    let removed: usize = lines[start..end].iter().sum();

    if &base[prefix..prefix + removed] != expected {
        return Err(stale(path));
    }

    let mut updated = Vec::with_capacity(base.len() - removed + replacement.len());
    updated.extend_from_slice(&base[..prefix]);
    updated.extend_from_slice(replacement);
    updated.extend_from_slice(&base[prefix + removed..]);

    Ok(updated)
}

fn stale(path: &Path) -> Error {
    Error::new(format!(
        "{} changed since this diff was read — refresh and try again",
        path.display()
    ))
}

/// The byte length of each line, counting its terminator.
///
/// A trailing chunk with no newline still counts as a line, which is what makes
/// files without a final newline round-trip.
fn line_lengths(content: &[u8]) -> Vec<usize> {
    let mut lengths = Vec::new();
    let mut current = 0;

    for byte in content {
        current += 1;

        if *byte == b'\n' {
            lengths.push(current);
            current = 0;
        }
    }

    if current > 0 {
        lengths.push(current);
    }

    lengths
}

fn index_content(repo: &Repository, index: &Index, path: &Path) -> Result<Vec<u8>> {
    let Some(entry) = index.get_path(path, 0) else {
        // Untracked files have no index entry, so their "old side" is empty.
        return Ok(Vec::new());
    };

    Ok(repo.find_blob(entry.id)?.content().to_vec())
}

const REGULAR: u32 = 0o100644;
const EXECUTABLE: u32 = 0o100755;
const SYMLINK: u32 = 0o120000;

fn resolve_mode(index: &Index, path: &Path, preferred: u32) -> u32 {
    if is_blob_mode(preferred) {
        return preferred;
    }

    index
        .get_path(path, 0)
        .map(|entry| entry.mode)
        .filter(|mode| is_blob_mode(*mode))
        .unwrap_or(REGULAR)
}

fn is_blob_mode(mode: u32) -> bool {
    matches!(mode, REGULAR | EXECUTABLE | SYMLINK)
}

fn write_entry(
    repo: &Repository,
    index: &mut Index,
    path: &Path,
    content: &[u8],
    mode: u32,
) -> Result<()> {
    let id = repo.blob(content)?;

    // The stat fields are left zeroed. git uses them only to skip re-hashing
    // unchanged files, so a zeroed entry costs one extra content check and is
    // never wrong.
    let entry = IndexEntry {
        ctime: IndexTime::new(0, 0),
        mtime: IndexTime::new(0, 0),
        dev: 0,
        ino: 0,
        mode,
        uid: 0,
        gid: 0,
        file_size: content.len() as u32,
        id,
        flags: 0,
        flags_extended: 0,
        path: super::index_path(path)?,
    };

    index.add(&entry)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_lengths_counts_a_missing_final_newline() {
        assert_eq!(line_lengths(b"a\nbb\nccc"), vec![2, 3, 3]);
        assert_eq!(line_lengths(b"a\nbb\n"), vec![2, 3]);
        assert_eq!(line_lengths(b""), Vec::<usize>::new());
    }

    #[test]
    fn line_range_handles_insertions() {
        // `@@ -0,0 +1,3 @@` inserts at the very start.
        assert_eq!(line_range(0, 0), (0, 0));
        // `@@ -5,0 +6,2 @@` inserts after the fifth line.
        assert_eq!(line_range(5, 0), (5, 0));
        // `@@ -1,2 +1,3 @@` covers the first two lines.
        assert_eq!(line_range(1, 2), (0, 2));
    }

    #[test]
    fn splice_replaces_the_expected_range() {
        let base = b"one\ntwo\nthree\n";

        let updated = splice(base, (1, 1), b"two\n", b"TWO\n", Path::new("f")).unwrap();

        assert_eq!(updated, b"one\nTWO\nthree\n");
    }

    #[test]
    fn splice_inserts_without_removing() {
        let base = b"one\nthree\n";

        let updated = splice(base, (1, 0), b"", b"two\n", Path::new("f")).unwrap();

        assert_eq!(updated, b"one\ntwo\nthree\n");
    }

    #[test]
    fn splice_preserves_a_missing_final_newline() {
        let base = b"one\ntwo";

        let updated = splice(base, (1, 1), b"two", b"TWO", Path::new("f")).unwrap();

        assert_eq!(updated, b"one\nTWO");
    }

    #[test]
    fn splice_refuses_a_stale_diff() {
        let base = b"one\ntwo\nthree\n";

        // The hunk expects `2\n` where the index actually holds `two\n`.
        assert!(splice(base, (1, 1), b"2\n", b"TWO\n", Path::new("f")).is_err());
        // And a range running past the end of the file.
        assert!(splice(base, (2, 5), b"", b"", Path::new("f")).is_err());
    }
}
