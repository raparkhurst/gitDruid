//! Applying one existing commit somewhere else: cherry-picking it, or
//! reverting it.
//!
//! Both are the same shape as a merge — apply, then either commit or leave the
//! conflicts for the user — so both end the same way [`super::merge`] does, and
//! a conflicted one is finished by the ordinary commit button.

use std::path::Path;

use git2::build::CheckoutBuilder;
use git2::{Commit, Repository};

use super::merge::require_clean;
use super::refs::{head_commit, signature};
use super::{Error, Result};

/// Applies `id`'s change on top of the current branch.
///
/// The original author is kept and the committer is whoever is running
/// gitDruid, which is what `git cherry-pick` records.
pub fn cherry_pick(repo_path: &Path, id: &str) -> Result<String> {
    let repo = super::open(repo_path)?;

    require_clean(&repo)?;

    let commit = find(&repo, id)?;

    if commit.parent_count() > 1 {
        return Err(Error::new(
            "that is a merge commit — cherry-picking one needs a parent to pick against, which \
             gitDruid cannot ask for yet",
        ));
    }

    let short = format!("{:.7}", commit.id());
    let summary = commit.summary().ok().flatten().unwrap_or("").to_owned();

    repo.cherrypick(&commit, None)?;

    let message = match summary.is_empty() {
        true => format!("Cherry-pick {short}"),
        false => summary.clone(),
    };

    finish(
        &repo,
        &message,
        Some(&commit),
        &format!("Cherry-picked {short}"),
        &format!("Cherry-picking {short}"),
    )
}

/// Records a commit that undoes `id`.
pub fn revert(repo_path: &Path, id: &str) -> Result<String> {
    let repo = super::open(repo_path)?;

    require_clean(&repo)?;

    let commit = find(&repo, id)?;

    if commit.parent_count() > 1 {
        return Err(Error::new(
            "that is a merge commit — reverting one needs a parent to revert against, which \
             gitDruid cannot ask for yet",
        ));
    }

    let short = format!("{:.7}", commit.id());
    let summary = commit.summary().ok().flatten().unwrap_or("").to_owned();

    repo.revert(&commit, None)?;

    let message = format!("Revert \"{summary}\"\n\nThis reverts commit {}.\n", commit.id());

    finish(
        &repo,
        &message,
        None,
        &format!("Reverted {short}"),
        &format!("Reverting {short}"),
    )
}

/// Writes the commit an apply produced, or reports the conflicts it left.
///
/// `author` carries the original author through a cherry-pick; a revert is
/// this user's own work, so it has none.
fn finish(
    repo: &Repository,
    message: &str,
    author: Option<&Commit<'_>>,
    done: &str,
    blocked: &str,
) -> Result<String> {
    let mut index = repo.index()?;

    if index.has_conflicts() {
        let count = index.conflicts().map(|conflicts| conflicts.count()).unwrap_or(0);

        // Left open on purpose, exactly where `git cherry-pick` leaves it:
        // resolving and committing is how it finishes, and gitDruid's commit
        // button can now do that.
        return Ok(format!(
            "{blocked} left {count} conflicted — resolve, stage, then commit"
        ));
    }

    let tree = repo.find_tree(index.write_tree()?)?;
    let committer = signature(repo)?;
    let parent = head_commit(repo)?;

    let author = match author {
        Some(commit) => commit.author(),
        None => committer.clone(),
    };

    // An apply that changed nothing is worth saying so rather than recording
    // an empty commit nobody asked for.
    if tree.id() == parent.tree_id() {
        repo.cleanup_state()?;

        return Err(Error::new(
            "that change is already present here, so there is nothing to apply",
        ));
    }

    let id = repo.commit(Some("HEAD"), &author, &committer, message, &tree, &[&parent])?;

    repo.cleanup_state()?;

    Ok(format!("{done} as {:.7}", id))
}

fn find<'repo>(repo: &'repo Repository, id: &str) -> Result<Commit<'repo>> {
    let oid =
        git2::Oid::from_str(id).map_err(|_| Error::new(format!("{id} is not a commit id")))?;

    repo.find_commit(oid)
        .map_err(|_| Error::new(format!("no commit {id} in this repository")))
}

/// Aborts an apply that was left open, putting the working tree back.
pub fn abort(repo_path: &Path) -> Result<String> {
    let repo = super::open(repo_path)?;

    let state = repo.state();

    if state == git2::RepositoryState::Clean {
        return Err(Error::new("there is nothing to abort"));
    }

    let head = head_commit(&repo)?;

    repo.reset(
        head.as_object(),
        git2::ResetType::Hard,
        Some(CheckoutBuilder::new().force()),
    )?;

    repo.cleanup_state()?;

    Ok("Aborted, and the working tree is back where it was".to_owned())
}
