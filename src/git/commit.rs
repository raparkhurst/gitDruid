//! Writing the index out as a commit.

use std::path::Path;

use super::{Error, Result};

/// Where a summary stops being comfortable.
///
/// git enforces nothing: a commit message is bytes, and the "subject" is just
/// everything before the first blank line. These are the conventional lengths
/// — fifty is what `git log --oneline` and most forges show without cutting,
/// seventy-two is where they start truncating — so they are advice here, not a
/// rule. A summary that needs fifty-five characters is better than one that
/// has been mangled to fit.
pub const SUMMARY_IDEAL: usize = 50;
pub const SUMMARY_LIMIT: usize = 72;

/// Cleans a typed summary: one line, and no longer than [`SUMMARY_LIMIT`].
///
/// Counted in characters rather than bytes. Cutting a string at a byte offset
/// splits whatever multi-byte character straddles it, and a commit message is
/// exactly the place someone writes a name with an accent in it.
pub fn trim_summary(text: &str) -> String {
    text.replace(['\n', '\r'], " ")
        .chars()
        .take(SUMMARY_LIMIT)
        .collect()
}

/// Joins a summary and a description the way git reads them back: the subject,
/// a blank line, then the body.
pub fn compose(summary: &str, description: &str) -> String {
    let summary = summary.trim();
    let description = description.trim();

    match description.is_empty() {
        true => summary.to_owned(),
        false => format!("{summary}\n\n{description}"),
    }
}

/// Splits a message back into the two boxes it was written in.
pub fn split(message: &str) -> (String, String) {
    let mut lines = message.trim().lines();

    let summary = lines.next().unwrap_or_default().trim().to_owned();
    let description = lines.collect::<Vec<_>>().join("\n").trim().to_owned();

    (summary, description)
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_summary_stops_at_the_limit() {
        let long = "x".repeat(200);

        assert_eq!(trim_summary(&long).chars().count(), SUMMARY_LIMIT);
        assert_eq!(trim_summary("short").chars().count(), 5);
    }

    #[test]
    fn a_summary_is_cut_between_characters_not_between_bytes() {
        // Every one of these is three bytes, so a byte-wise cut at 72 would
        // land in the middle of one.
        let accented = "é".repeat(100);
        let trimmed = trim_summary(&accented);

        assert_eq!(trimmed.chars().count(), SUMMARY_LIMIT);
        assert!(trimmed.chars().all(|c| c == 'é'), "a character was split");
    }

    #[test]
    fn a_pasted_newline_does_not_make_the_summary_two_lines() {
        assert_eq!(trim_summary("one\ntwo"), "one two");
        assert_eq!(trim_summary("one\r\ntwo"), "one  two");
    }

    #[test]
    fn a_summary_on_its_own_is_the_whole_message() {
        assert_eq!(compose("Add a lexer", ""), "Add a lexer");
        assert_eq!(compose("  Add a lexer  ", "   "), "Add a lexer");
    }

    #[test]
    fn a_description_is_separated_by_a_blank_line() {
        // The blank line is what makes git read the first line as the subject.
        assert_eq!(
            compose("Add a lexer", "It reads one token at a time."),
            "Add a lexer\n\nIt reads one token at a time."
        );
    }

    #[test]
    fn a_message_splits_back_into_the_boxes_it_came_from() {
        let (summary, description) = split("Add a lexer\n\nIt reads one token at a time.");

        assert_eq!(summary, "Add a lexer");
        assert_eq!(description, "It reads one token at a time.");
    }

    #[test]
    fn composing_and_splitting_are_the_same_operation_backwards() {
        for (summary, description) in [
            ("Short", ""),
            ("Short", "A body."),
            ("Short", "Two\n\nparagraphs."),
        ] {
            let (back, body) = split(&compose(summary, description));

            assert_eq!(back, summary);
            assert_eq!(body, description);
        }
    }

    #[test]
    fn a_message_with_no_blank_line_keeps_its_first_line_as_the_summary() {
        // git would call all of this the subject; a one-line box cannot hold
        // it, and the first line is what every other tool shows.
        let (summary, description) = split("First line\nsecond line");

        assert_eq!(summary, "First line");
        assert_eq!(description, "second line");
    }

    #[test]
    fn an_empty_message_splits_into_nothing() {
        assert_eq!(split(""), (String::new(), String::new()));
        assert_eq!(split("   \n\n  "), (String::new(), String::new()));
    }
}
