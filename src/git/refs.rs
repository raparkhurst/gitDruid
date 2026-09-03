//! Listing branches and tags, and the operations that change them.
//!
//! Every operation returns the sentence the UI shows on success, so the caller
//! never has to reconstruct what happened from the arguments it passed in.

use std::path::Path;

use git2::build::CheckoutBuilder;
use git2::{BranchType, Repository};

use super::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Branch {
    pub name: String,
    /// True for the branch HEAD is currently on.
    pub is_head: bool,
    /// The upstream's shorthand, when the branch tracks one.
    pub upstream: Option<String>,
    pub ahead: usize,
    pub behind: usize,
    /// True when every commit on the branch is already reachable from HEAD, so
    /// deleting it loses nothing. The UI needs this before it can warn.
    pub merged: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tag {
    pub name: String,
    /// The commit the tag resolves to, abbreviated.
    pub short_id: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Refs {
    pub local: Vec<Branch>,
    pub remote: Vec<Branch>,
    pub tags: Vec<Tag>,
    /// What a push or pull would act on, or `None` when there is nothing to
    /// talk to. Read here because it is a fact about branches and remotes, and
    /// this is already read whenever either might have changed.
    pub tracking: Option<super::Tracking>,
}

impl Refs {
    pub fn is_empty(&self) -> bool {
        self.local.is_empty() && self.remote.is_empty() && self.tags.is_empty()
    }
}

/// Reads every branch and tag in the repository.
pub fn refs(repo_path: &Path) -> Result<Refs> {
    let repo = super::open(repo_path)?;

    let mut local = Vec::new();
    let mut remote = Vec::new();

    for (branch, kind) in repo.branches(None)?.flatten() {
        let Ok(Some(name)) = branch.name() else {
            continue;
        };

        let name = name.to_owned();

        let entry = match kind {
            BranchType::Local => {
                let (ahead, behind) = divergence(&repo, &branch);

                Branch {
                    is_head: branch.is_head(),
                    upstream: branch
                        .upstream()
                        .ok()
                        .and_then(|upstream| upstream.name().ok().flatten().map(str::to_owned)),
                    ahead,
                    behind,
                    merged: is_merged(&repo, &branch).unwrap_or(false),
                    name,
                }
            }
            BranchType::Remote => Branch {
                name,
                is_head: false,
                upstream: None,
                ahead: 0,
                behind: 0,
                merged: true,
            },
        };

        match kind {
            BranchType::Local => local.push(entry),
            BranchType::Remote => remote.push(entry),
        }
    }

    let mut tags = Vec::new();

    for name in repo.tag_names(None)?.iter().flatten().flatten() {
        let short_id = repo
            .revparse_single(&format!("refs/tags/{name}"))
            .and_then(|object| object.peel_to_commit())
            .map(|commit| format!("{:.7}", commit.id()))
            .unwrap_or_default();

        tags.push(Tag {
            name: name.to_owned(),
            short_id,
        });
    }

    local.sort_by(|a, b| a.name.cmp(&b.name));
    remote.sort_by(|a, b| a.name.cmp(&b.name));
    tags.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(Refs {
        local,
        remote,
        tags,
        tracking: super::remote::read_tracking(&repo),
    })
}

/// How far a branch has diverged from its upstream, or `(0, 0)` without one.
fn divergence(repo: &Repository, branch: &git2::Branch<'_>) -> (usize, usize) {
    let Some(local) = branch.get().target() else {
        return (0, 0);
    };

    let Some(upstream) = branch
        .upstream()
        .ok()
        .and_then(|upstream| upstream.get().target())
    else {
        return (0, 0);
    };

    repo.graph_ahead_behind(local, upstream).unwrap_or((0, 0))
}

/// Creates a branch at `start`, or at HEAD when `start` is `None`.
pub fn create_branch(repo_path: &Path, name: &str, start: Option<&str>) -> Result<String> {
    let name = validate_name(name, "branch")?;

    let repo = super::open(repo_path)?;

    let target = match start {
        Some(revision) => repo.revparse_single(revision)?.peel_to_commit()?,
        None => head_commit(&repo)?,
    };

    if repo.find_branch(&name, BranchType::Local).is_ok() {
        return Err(Error::new(format!("a branch named {name} already exists")));
    }

    repo.branch(&name, &target, false)?;

    Ok(format!("Created {name} at {:.7}", target.id()))
}

/// Moves HEAD onto `name` and updates the working tree to match.
///
/// The checkout is a safe one: git refuses rather than overwriting work that
/// has not been committed, which is the behaviour `git switch` has.
pub fn checkout_branch(repo_path: &Path, name: &str) -> Result<String> {
    let repo = super::open(repo_path)?;

    if repo.state() != git2::RepositoryState::Clean {
        return Err(Error::new(
            "the repository is mid-operation — finish or abort it before switching branches",
        ));
    }

    checkout(&repo, name)
}

pub(super) fn checkout(repo: &Repository, name: &str) -> Result<String> {
    let branch = repo
        .find_branch(name, BranchType::Local)
        .map_err(|_| Error::new(format!("there is no local branch named {name}")))?;

    let reference = branch
        .get()
        .name()
        .map(str::to_owned)
        .map_err(|_| Error::new(format!("{name} is not a name git can use")))?;

    let tree = branch.get().peel(git2::ObjectType::Tree)?;

    repo.checkout_tree(&tree, Some(CheckoutBuilder::new().safe()))
        .map_err(|error| {
            Error::new(format!(
                "cannot switch to {name} without losing uncommitted work: {}",
                error.message()
            ))
        })?;

    repo.set_head(&reference)?;

    Ok(format!("Switched to {name}"))
}

/// Deletes a branch, refusing the one HEAD is on.
///
/// `force` is required for a branch whose commits are not reachable from HEAD,
/// since deleting it is the one way to lose them.
pub fn delete_branch(repo_path: &Path, name: &str, force: bool) -> Result<String> {
    let repo = super::open(repo_path)?;

    let mut branch = repo
        .find_branch(name, BranchType::Local)
        .map_err(|_| Error::new(format!("there is no local branch named {name}")))?;

    if branch.is_head() {
        return Err(Error::new(format!(
            "{name} is the current branch — switch to another one before deleting it"
        )));
    }

    if !force && !is_merged(&repo, &branch)? {
        return Err(Error::new(format!(
            "{name} has commits that are not on HEAD — deleting it would lose them"
        )));
    }

    branch.delete()?;

    Ok(format!("Deleted {name}"))
}

/// True when everything on `branch` is already reachable from HEAD.
fn is_merged(repo: &Repository, branch: &git2::Branch<'_>) -> Result<bool> {
    let Some(tip) = branch.get().target() else {
        return Ok(true);
    };

    let Ok(head) = repo.head() else {
        return Ok(false);
    };

    let Some(head) = head.target() else {
        return Ok(false);
    };

    let (ahead, _) = repo.graph_ahead_behind(tip, head)?;

    Ok(ahead == 0)
}

pub fn rename_branch(repo_path: &Path, from: &str, to: &str) -> Result<String> {
    let to = validate_name(to, "branch")?;

    let repo = super::open(repo_path)?;

    let mut branch = repo
        .find_branch(from, BranchType::Local)
        .map_err(|_| Error::new(format!("there is no local branch named {from}")))?;

    if repo.find_branch(&to, BranchType::Local).is_ok() {
        return Err(Error::new(format!("a branch named {to} already exists")));
    }

    branch.rename(&to, false)?;

    Ok(format!("Renamed {from} to {to}"))
}

/// Tags `start`, or HEAD when `start` is `None`.
///
/// The tag is annotated when a message is given and lightweight otherwise,
/// matching what `git tag` does with and without `-m`.
pub fn create_tag(
    repo_path: &Path,
    name: &str,
    start: Option<&str>,
    message: Option<&str>,
) -> Result<String> {
    let name = validate_name(name, "tag")?;

    let repo = super::open(repo_path)?;

    let target = match start {
        Some(revision) => repo.revparse_single(revision)?,
        None => head_commit(&repo)?.into_object(),
    };

    if repo.find_reference(&format!("refs/tags/{name}")).is_ok() {
        return Err(Error::new(format!("a tag named {name} already exists")));
    }

    match message.map(str::trim).filter(|text| !text.is_empty()) {
        Some(message) => {
            let signature = signature(&repo)?;
            repo.tag(&name, &target, &signature, message, false)?;
        }
        None => {
            repo.tag_lightweight(&name, &target, false)?;
        }
    }

    Ok(format!("Tagged {:.7} as {name}", target.id()))
}

pub fn delete_tag(repo_path: &Path, name: &str) -> Result<String> {
    let repo = super::open(repo_path)?;

    repo.find_reference(&format!("refs/tags/{name}"))
        .map_err(|_| Error::new(format!("there is no tag named {name}")))?;

    repo.tag_delete(name)?;

    Ok(format!("Deleted tag {name}"))
}

/// Rejects names git itself would reject, before anything is written.
fn validate_name(name: &str, kind: &str) -> Result<String> {
    let name = name.trim();

    if name.is_empty() {
        return Err(Error::new(format!("a {kind} needs a name")));
    }

    if !git2::Reference::is_valid_name(&format!("refs/heads/{name}")) {
        return Err(Error::new(format!("{name} is not a valid {kind} name")));
    }

    Ok(name.to_owned())
}

/// The branch HEAD is on, or `None` when it is detached or unborn.
pub(super) fn head_branch(repo: &Repository) -> Option<git2::Branch<'_>> {
    let head = repo.head().ok()?;

    if !head.is_branch() {
        return None;
    }

    Some(git2::Branch::wrap(head))
}

pub(super) fn head_commit(repo: &Repository) -> Result<git2::Commit<'_>> {
    repo.head()
        .and_then(|head| head.peel_to_commit())
        .map_err(|_| Error::new("this repository has no commits yet"))
}

pub(super) fn signature(repo: &Repository) -> Result<git2::Signature<'static>> {
    repo.signature().map_err(|_| {
        Error::new("git has no author configured — set user.name and user.email, then try again")
    })
}
