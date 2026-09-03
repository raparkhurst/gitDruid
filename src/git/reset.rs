//! Moving the current branch to another commit.
//!
//! The three modes differ only in how much they take with them, and the
//! difference is the whole point: one keeps the work staged, one keeps it in
//! the working tree, and one throws it away.

use std::path::Path;

use git2::build::CheckoutBuilder;
use git2::ResetType;

use super::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reset {
    /// Move the branch. The index and the working tree keep everything, so the
    /// commits that were undone come back as staged changes.
    Soft,
    /// Move the branch and the index. The work stays in the working tree,
    /// unstaged.
    Mixed,
    /// Move everything. Anything not committed is gone.
    Hard,
}

impl Reset {
    pub fn title(self) -> &'static str {
        match self {
            Reset::Soft => "keeping the changes staged",
            Reset::Mixed => "keeping the changes",
            Reset::Hard => "discarding the changes",
        }
    }

    fn kind(self) -> ResetType {
        match self {
            Reset::Soft => ResetType::Soft,
            Reset::Mixed => ResetType::Mixed,
            Reset::Hard => ResetType::Hard,
        }
    }
}

/// Moves the current branch to `target`.
pub fn reset(repo_path: &Path, target: &str, mode: Reset) -> Result<String> {
    let repo = super::open(repo_path)?;

    if repo.state() != git2::RepositoryState::Clean {
        return Err(Error::new(
            "the repository is mid-operation — finish or abort it before resetting",
        ));
    }

    let object = repo
        .revparse_single(target)
        .map_err(|_| Error::new(format!("there is no commit {target}")))?;

    let commit = object
        .peel_to_commit()
        .map_err(|_| Error::new(format!("{target} is not a commit")))?;

    // A forced checkout, because a hard reset is being asked to overwrite the
    // working tree and a safe one would refuse.
    let mut checkout = CheckoutBuilder::new();
    checkout.force();

    repo.reset(commit.as_object(), mode.kind(), Some(&mut checkout))?;

    Ok(format!(
        "Reset to {:.7}, {}",
        commit.id(),
        mode.title()
    ))
}
