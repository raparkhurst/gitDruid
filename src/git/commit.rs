//! Writing the index out as a commit.

use std::path::Path;

use super::{Error, Result};

/// Commits whatever is currently staged, returning the new commit's short id.
pub fn commit(repo_path: &Path, message: &str) -> Result<String> {
    let message = message.trim();

    if message.is_empty() {
        return Err(Error::new("a commit needs a message"));
    }

    let mut repo = super::open(repo_path)?;

    // A merge, cherry-pick or revert left open is finished by committing, so
    // those states are allowed through. Only a merge contributes extra
    // parents; the other two record an ordinary commit on top of HEAD.
    let state = repo.state();
    let merging = state == git2::RepositoryState::Merge;

    let resumable = matches!(
        state,
        git2::RepositoryState::Clean
            | git2::RepositoryState::Merge
            | git2::RepositoryState::CherryPick
            | git2::RepositoryState::Revert
    );

    if !resumable {
        return Err(Error::new(
            "the repository is mid-operation — finish or abort it on the command line first",
        ));
    }

    let applying = state != git2::RepositoryState::Clean;

    // MERGE_HEAD is read through a callback that needs the repository mutably,
    // so it happens before anything else borrows it.
    let merge_head_ids = if merging {
        merge_head_ids(&mut repo)?
    } else {
        Vec::new()
    };

    let signature = repo.signature().map_err(|_| {
        Error::new("git has no author configured — set user.name and user.email, then try again")
    })?;

    let mut index = repo.index()?;

    if index.has_conflicts() {
        return Err(Error::new("resolve the conflicted files before committing"));
    }

    let tree_id = index.write_tree()?;
    let tree = repo.find_tree(tree_id)?;

    let parent = match repo.head() {
        Ok(head) => Some(head.peel_to_commit()?),
        // No HEAD yet: this is the repository's first commit, so it has no parent.
        Err(_) => None,
    };

    // A commit finishing an apply legitimately carries its parent's tree —
    // merging a branch whose work is already present changes nothing — so the
    // empty check only applies to ordinary commits.
    if !applying {
        if let Some(parent) = &parent
            && parent.tree_id() == tree_id
        {
            return Err(Error::new("nothing is staged to commit"));
        }

        if parent.is_none() && tree.is_empty() {
            return Err(Error::new("nothing is staged to commit"));
        }
    }

    let merge_heads = merge_head_ids
        .iter()
        .map(|oid| repo.find_commit(*oid).map_err(Error::from))
        .collect::<Result<Vec<_>>>()?;

    let mut parents: Vec<&git2::Commit<'_>> = parent.iter().collect();
    parents.extend(merge_heads.iter());

    let id = repo.commit(
        Some("HEAD"),
        &signature,
        &signature,
        message,
        &tree,
        &parents,
    )?;

    // The operation is over once its commit exists; leaving MERGE_HEAD or
    // CHERRY_PICK_HEAD behind would make the next commit part of it too.
    if applying {
        repo.cleanup_state()?;
    }

    Ok(format!("{:.7}", id))
}

/// The commits MERGE_HEAD names, which become the extra parents of the commit
/// that finishes an open merge.
fn merge_head_ids(repo: &mut git2::Repository) -> Result<Vec<git2::Oid>> {
    let mut ids = Vec::new();

    repo.mergehead_foreach(|oid| {
        ids.push(*oid);
        true
    })?;

    Ok(ids)
}
