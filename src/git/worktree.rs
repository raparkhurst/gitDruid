//! Edits to the working tree itself: ignoring a file, and throwing changes
//! away.
//!
//! These are the two operations here that git cannot undo. Staging can be
//! unstaged and a commit can be reset, but a discarded edit was never written
//! anywhere, so the callers of [`discard`] are expected to have asked first.

use std::path::{Path, PathBuf};

use git2::build::CheckoutBuilder;

use super::status::{Change, FileEntry, Side};
use super::{Error, Result};

/// What a generated `.gitignore` line should cover.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ignore {
    /// This one file, and nothing else with the same name elsewhere.
    File,
    /// Everything with this extension, anywhere.
    Extension,
    /// The folder the file is in, and everything under it.
    Folder,
}

/// The line that would be added to `.gitignore` for `path`.
///
/// File and folder patterns are anchored with a leading `/` so they mean the
/// path they were generated from and not every file that happens to share its
/// name. Extension patterns are deliberately not anchored: an extension worth
/// ignoring is worth ignoring everywhere.
pub fn pattern(path: &Path, scope: Ignore) -> Option<String> {
    let text = path.to_str()?.replace('\\', "/");

    if text.is_empty() {
        return None;
    }

    match scope {
        Ignore::File => Some(format!("/{text}")),
        Ignore::Extension => {
            let extension = path.extension()?.to_str()?;

            (!extension.is_empty()).then(|| format!("*.{extension}"))
        }
        Ignore::Folder => {
            let parent = path.parent()?.to_str()?.replace('\\', "/");

            (!parent.is_empty()).then(|| format!("/{parent}/"))
        }
    }
}

/// Adds a line to the repository's `.gitignore`, creating it if need be.
pub fn ignore(repo_path: &Path, pattern: &str) -> Result<String> {
    let pattern = pattern.trim();

    if pattern.is_empty() {
        return Err(Error::new("there is nothing to ignore"));
    }

    let repo = super::open(repo_path)?;
    let workdir = workdir(&repo)?;
    let file = workdir.join(".gitignore");

    let existing = std::fs::read_to_string(&file).unwrap_or_default();

    if existing
        .lines()
        .any(|line| line.trim() == pattern)
    {
        return Ok(format!("{pattern} is already in .gitignore"));
    }

    let mut updated = existing;

    // Append on a line of its own, whether or not the file ended with one.
    if !updated.is_empty() && !updated.ends_with('\n') {
        updated.push('\n');
    }

    updated.push_str(pattern);
    updated.push('\n');

    std::fs::write(&file, updated)?;

    Ok(format!("Added {pattern} to .gitignore"))
}

/// Throws away the unstaged changes to one file.
///
/// Only the working-tree side: discarding something staged would mean deciding
/// whether the user meant "unstage it" or "lose it", and unstaging is already
/// a button.
pub fn discard(repo_path: &Path, entry: &FileEntry) -> Result<String> {
    if entry.side != Side::Worktree {
        return Err(Error::new(
            "only unstaged changes can be discarded — unstage it first",
        ));
    }

    let repo = super::open(repo_path)?;
    let workdir = workdir(&repo)?;

    if entry.change == Change::Untracked {
        let full = workdir.join(&entry.path);

        // An untracked path is not in the index, so there is nothing to
        // restore it from: discarding it means deleting it.
        if full.is_dir() {
            std::fs::remove_dir_all(&full)?;
        } else {
            std::fs::remove_file(&full)?;
        }

        return Ok(format!("Deleted {}", entry.path.display()));
    }

    let mut checkout = CheckoutBuilder::new();
    checkout.force().path(&entry.path);

    // A rename shows up under its new name; restoring it means bringing the
    // old name back as well.
    if let Some(old_path) = &entry.old_path {
        checkout.path(old_path);
    }

    repo.checkout_index(None, Some(&mut checkout))?;

    if entry.change == Change::Renamed
        && let Some(old_path) = &entry.old_path
        && old_path != &entry.path
    {
        // The new name is not in the index, so the checkout above left it
        // behind; without this the file would exist under both names.
        let _ = std::fs::remove_file(workdir.join(&entry.path));
    }

    Ok(format!("Discarded changes to {}", entry.path.display()))
}

fn workdir(repo: &git2::Repository) -> Result<PathBuf> {
    repo.workdir()
        .map(Path::to_path_buf)
        .ok_or_else(|| Error::new("this repository has no working tree"))
}
