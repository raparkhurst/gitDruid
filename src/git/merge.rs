//! Merging one branch into the branch HEAD is on.
//!
//! A merge has three outcomes and they are not variations on each other: it can
//! be a no-op, it can move the branch pointer forward without writing anything,
//! or it can write a commit with two parents. Conflicts make a fourth, and that
//! one deliberately leaves the repository mid-merge for the user to resolve —
//! the same place `git merge` leaves it.

use std::path::Path;

use git2::build::CheckoutBuilder;
use git2::{BranchType, Repository};

use super::refs::{head_commit, signature};
use super::{Error, Result};

/// Merges `name` into the current branch, returning what happened.
pub fn merge_branch(repo_path: &Path, name: &str) -> Result<String> {
    let repo = super::open(repo_path)?;

    require_clean(&repo)?;

    let branch = repo
        .find_branch(name, BranchType::Local)
        .map_err(|_| Error::new(format!("there is no local branch named {name}")))?;

    if branch.is_head() {
        return Err(Error::new(format!("{name} is already the current branch")));
    }

    let annotated = repo.reference_to_annotated_commit(branch.get())?;

    merge_into_head(&repo, &annotated, name)
}

/// Merges a workflow branch into the branch it belongs to.
///
/// This is the second half of git-flow: a feature is finished by merging it
/// into develop, a hotfix by merging it into main. Which branch that is comes
/// from the settings, so the caller has already worked it out.
pub fn finish_branch(repo_path: &Path, branch: &str, target: &str) -> Result<String> {
    let repo = super::open(repo_path)?;

    require_clean(&repo)?;

    if branch == target {
        return Err(Error::new(format!(
            "{branch} is where this sort of branch is merged to, so there is nothing to finish"
        )));
    }

    // Look the source up before switching, so a typo is caught while the
    // working tree is still where the user left it.
    repo.find_branch(branch, BranchType::Local)
        .map_err(|_| Error::new(format!("there is no local branch named {branch}")))?;

    repo.find_branch(target, BranchType::Local).map_err(|_| {
        Error::new(format!(
            "there is no local branch named {target} to merge {branch} into — check the workflow \
             branches in Settings"
        ))
    })?;

    super::refs::checkout(&repo, target)?;

    let source = repo.find_branch(branch, BranchType::Local)?;
    let annotated = repo.reference_to_annotated_commit(source.get())?;

    let summary = merge_into_head(&repo, &annotated, branch)?;

    Ok(format!("{summary}, on {target}"))
}

/// Refuses to start anything while another operation is half-finished.
pub(super) fn require_clean(repo: &Repository) -> Result<()> {
    if repo.state() != git2::RepositoryState::Clean {
        return Err(Error::new(
            "the repository is already mid-operation — finish or abort it first",
        ));
    }

    Ok(())
}

/// Merges an already-resolved commit into HEAD.
///
/// Shared by [`merge_branch`] and by pulling, which merges the upstream: the
/// four outcomes are the same whichever side the commit came from.
pub(super) fn merge_into_head(
    repo: &Repository,
    annotated: &git2::AnnotatedCommit<'_>,
    name: &str,
) -> Result<String> {
    let head = repo.head().map_err(|_| {
        Error::new("this repository has no commits yet, so there is nothing to merge into")
    })?;

    if !head.is_branch() {
        return Err(Error::new(
            "HEAD is detached — check out a branch before merging",
        ));
    }

    let (analysis, _) = repo.merge_analysis(&[annotated])?;

    if analysis.is_up_to_date() {
        return Ok(format!("Already up to date with {name}"));
    }

    if analysis.is_fast_forward() {
        return fast_forward(repo, name, annotated.id());
    }

    if !analysis.is_normal() {
        return Err(Error::new(format!("{name} cannot be merged into HEAD")));
    }

    repo.merge(&[annotated], None, Some(CheckoutBuilder::new().safe()))?;

    let mut index = repo.index()?;

    if index.has_conflicts() {
        let conflicted = conflict_count(&index);

        // The merge stays open on purpose: resolving the files and committing
        // is how it finishes, and gitDruid can now write that commit.
        return Ok(format!(
            "Merging {name} left {conflicted} conflicted — resolve, stage, then commit"
        ));
    }

    let tree = repo.find_tree(index.write_tree()?)?;
    let signature = signature(repo)?;
    let ours = head_commit(repo)?;
    let theirs = repo.find_commit(annotated.id())?;

    let id = repo.commit(
        Some("HEAD"),
        &signature,
        &signature,
        &format!("Merge '{name}'"),
        &tree,
        &[&ours, &theirs],
    )?;

    repo.cleanup_state()?;

    Ok(format!("Merged {name} as {:.7}", id))
}

/// Moves the current branch forward to `target` without writing a commit.
fn fast_forward(repo: &Repository, name: &str, target: git2::Oid) -> Result<String> {
    let commit = repo.find_commit(target)?;

    repo.checkout_tree(commit.as_object(), Some(CheckoutBuilder::new().safe()))
        .map_err(|error| {
            Error::new(format!(
                "cannot fast-forward to {name} without losing uncommitted work: {}",
                error.message()
            ))
        })?;

    let mut head = repo.head()?;
    head.set_target(target, &format!("merge {name}: fast-forward"))?;

    Ok(format!("Fast-forwarded to {name} ({:.7})", target))
}

fn conflict_count(index: &git2::Index) -> String {
    let count = index
        .conflicts()
        .map(|conflicts| conflicts.count())
        .unwrap_or(0);

    match count {
        1 => "1 file".to_owned(),
        count => format!("{count} files"),
    }
}
