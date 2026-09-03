//! Replacing the last commit.
//!
//! Amending is not committing again: the new commit takes the old one's place
//! and its parents, so the branch does not grow. That makes it the right tool
//! for a message with a typo in it and the wrong one for anything already
//! pushed, which is a judgement the caller has more information about than
//! this does.

use std::path::Path;

use super::refs::signature;
use super::{Error, Result};

/// The message of the commit that would be amended, for filling the box in.
pub fn head_message(repo_path: &Path) -> Result<String> {
    let repo = super::open(repo_path)?;

    let head = repo
        .head()
        .and_then(|head| head.peel_to_commit())
        .map_err(|_| Error::new("there is no commit to amend"))?;

    Ok(head.message().unwrap_or_default().trim().to_owned())
}

/// Replaces the last commit with what is staged now, under `message`.
pub fn amend(repo_path: &Path, message: &str) -> Result<String> {
    let message = message.trim();

    if message.is_empty() {
        return Err(Error::new("a commit needs a message"));
    }

    let repo = super::open(repo_path)?;

    // Amending during a merge would rewrite the commit the merge started from,
    // which is never what anyone means by it.
    if repo.state() != git2::RepositoryState::Clean {
        return Err(Error::new(
            "the repository is mid-operation — finish or abort it before amending",
        ));
    }

    let head = repo
        .head()
        .and_then(|head| head.peel_to_commit())
        .map_err(|_| Error::new("there is no commit to amend"))?;

    let mut index = repo.index()?;

    if index.has_conflicts() {
        return Err(Error::new("resolve the conflicted files before amending"));
    }

    let tree = repo.find_tree(index.write_tree()?)?;

    // The author stays with the commit and the committer becomes whoever is
    // amending, which is what `git commit --amend` records.
    let committer = signature(&repo)?;

    let id = head.amend(
        Some("HEAD"),
        None,
        Some(&committer),
        None,
        Some(message),
        Some(&tree),
    )?;

    Ok(format!("Amended {:.7}", id))
}
