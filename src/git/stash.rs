//! Putting work aside and getting it back.
//!
//! A stash is a commit that is not on any branch, holding what the working
//! tree and the index looked like at the moment it was made. Applying one
//! merges it back, which means it can conflict — and when it does, it is left
//! for the ordinary conflict pane to settle rather than being rolled back.

use std::path::Path;

use git2::{StashApplyOptions, StashFlags};

use super::refs::signature;
use super::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stash {
    /// Position in the stack. Zero is the most recent, and every index shifts
    /// down when one below it is dropped.
    pub index: usize,
    pub message: String,
    pub short_id: String,
}

/// Reads the stash stack, newest first.
pub fn stashes(repo_path: &Path) -> Result<Vec<Stash>> {
    let mut repo = super::open(repo_path)?;

    let mut found = Vec::new();

    repo.stash_foreach(|index, message, id| {
        found.push(Stash {
            index,
            message: message.to_owned(),
            short_id: format!("{:.7}", id),
        });

        true
    })?;

    Ok(found)
}

/// Puts the working tree aside, leaving it clean.
pub fn stash_save(repo_path: &Path, message: &str, untracked: bool) -> Result<String> {
    let mut repo = super::open(repo_path)?;

    if repo.state() != git2::RepositoryState::Clean {
        return Err(Error::new(
            "the repository is mid-operation — finish or abort it before stashing",
        ));
    }

    let signature = signature(&repo)?;
    let message = message.trim();

    let mut flags = StashFlags::DEFAULT;

    if untracked {
        flags |= StashFlags::INCLUDE_UNTRACKED;
    }

    let id = repo
        .stash_save2(
            &signature,
            (!message.is_empty()).then_some(message),
            Some(flags),
        )
        .map_err(|error| match error.code() {
            // libgit2 says "there is nothing to stash" in its own words; this
            // is the one case worth rewording, because it is not a failure.
            git2::ErrorCode::NotFound => Error::new("there is nothing to stash"),
            _ => Error::from(error),
        })?;

    Ok(format!("Stashed as {:.7}", id))
}

/// Puts a stash back without removing it from the stack.
pub fn stash_apply(repo_path: &Path, index: usize) -> Result<String> {
    apply(repo_path, index, false)
}

/// Puts a stash back and removes it.
pub fn stash_pop(repo_path: &Path, index: usize) -> Result<String> {
    apply(repo_path, index, true)
}

fn apply(repo_path: &Path, index: usize, drop_after: bool) -> Result<String> {
    let mut repo = super::open(repo_path)?;

    if repo.state() != git2::RepositoryState::Clean {
        return Err(Error::new(
            "the repository is mid-operation — finish or abort it first",
        ));
    }

    let mut options = StashApplyOptions::new();

    // Whatever was staged when the stash was made goes back to being staged.
    options.reinstantiate_index();

    repo.stash_apply(index, Some(&mut options))
        .map_err(|error| match error.code() {
            git2::ErrorCode::NotFound => Error::new(format!("there is no stash at {index}")),
            git2::ErrorCode::Conflict | git2::ErrorCode::MergeConflict => Error::new(
                "putting that stash back conflicts with the working tree — commit or stash what \
                 is there first",
            ),
            _ => Error::from(error),
        })?;

    if !drop_after {
        return Ok(format!("Applied stash {index}, and kept it"));
    }

    // Only once it is safely back: dropping first would lose it if the apply
    // turned out to fail.
    repo.stash_drop(index)?;

    Ok(format!("Popped stash {index}"))
}

/// Throws a stash away.
pub fn stash_drop(repo_path: &Path, index: usize) -> Result<String> {
    let mut repo = super::open(repo_path)?;

    repo.stash_drop(index)
        .map_err(|_| Error::new(format!("there is no stash at {index}")))?;

    Ok(format!("Dropped stash {index}"))
}
