//! Checks on ignoring, discarding, cherry-picking and reverting.
//!
//! These are the operations reachable from the right-click menu. Two of them
//! lose work if they are wrong — discarding, and an apply that leaves the
//! repository half-changed — so they are asserted against the `git` CLI rather
//! than against gitDruid's own reader.

use std::path::Path;
use std::process::Command;

use git_druid::git::{self, Change, Ignore, Side};

fn git_cli(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("HOME", repo)
        .env("GIT_AUTHOR_NAME", "gitDruid Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
        .env("GIT_COMMITTER_NAME", "gitDruid Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
        .output()
        .expect("git should be installed");

    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );

    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path();

    git_cli(path, &["init", "--initial-branch=main", "--quiet"]);
    git_cli(path, &["config", "user.name", "gitDruid Test"]);
    git_cli(path, &["config", "user.email", "test@example.invalid"]);

    std::fs::write(path.join("kept.txt"), "one\ntwo\nthree\n").unwrap();
    git_cli(path, &["add", "-A"]);
    git_cli(path, &["commit", "--quiet", "-m", "base"]);

    dir
}

fn entry(path: &Path, side: Side, name: &str) -> git_druid::git::FileEntry {
    git::snapshot(path)
        .unwrap()
        .find(side, Path::new(name))
        .unwrap_or_else(|| panic!("{name} should be listed under {side:?}"))
        .clone()
}

#[test]
fn ignore_patterns_are_anchored_where_it_matters() {
    let path = Path::new("logs/today/server.log");

    // A file and a folder mean *that* one, so they are anchored to the root.
    assert_eq!(
        git::pattern(path, Ignore::File).as_deref(),
        Some("/logs/today/server.log")
    );
    assert_eq!(
        git::pattern(path, Ignore::Folder).as_deref(),
        Some("/logs/today/")
    );

    // An extension worth ignoring is worth ignoring anywhere, so it is not.
    assert_eq!(git::pattern(path, Ignore::Extension).as_deref(), Some("*.log"));

    // Nothing sensible to say about these.
    assert_eq!(git::pattern(Path::new("README"), Ignore::Extension), None);
    assert_eq!(git::pattern(Path::new("README"), Ignore::Folder), None);
}

#[test]
fn ignoring_a_file_makes_git_stop_listing_it() {
    let dir = repo();
    let path = dir.path();

    std::fs::create_dir_all(path.join("logs")).unwrap();
    std::fs::write(path.join("logs/server.log"), "noise\n").unwrap();

    assert_eq!(
        entry(path, Side::Worktree, "logs/server.log").change,
        Change::Untracked
    );

    let pattern = git::pattern(Path::new("logs/server.log"), Ignore::Extension).unwrap();
    let summary = git::ignore(path, &pattern).unwrap();
    assert_eq!(summary, "Added *.log to .gitignore");

    // The file is gone from the unstaged list, and .gitignore is there instead.
    let snapshot = git::snapshot(path).unwrap();
    assert!(
        snapshot.find(Side::Worktree, Path::new("logs/server.log")).is_none(),
        "an ignored file should stop being listed"
    );
    assert!(snapshot.find(Side::Worktree, Path::new(".gitignore")).is_some());

    assert_eq!(
        std::fs::read_to_string(path.join(".gitignore")).unwrap(),
        "*.log\n"
    );
}

#[test]
fn ignoring_the_same_thing_twice_does_not_repeat_it() {
    let dir = repo();
    let path = dir.path();

    std::fs::write(path.join(".gitignore"), "target\n").unwrap();

    git::ignore(path, "*.log").unwrap();
    let again = git::ignore(path, "*.log").unwrap();

    assert_eq!(again, "*.log is already in .gitignore");
    assert_eq!(
        std::fs::read_to_string(path.join(".gitignore")).unwrap(),
        "target\n*.log\n",
        "the existing contents should be kept, and the line added once"
    );
}

#[test]
fn ignoring_appends_to_a_file_with_no_trailing_newline() {
    let dir = repo();
    let path = dir.path();

    std::fs::write(path.join(".gitignore"), "target").unwrap();

    git::ignore(path, "*.log").unwrap();

    assert_eq!(
        std::fs::read_to_string(path.join(".gitignore")).unwrap(),
        "target\n*.log\n",
        "the new line must not be glued onto the last one"
    );
}

#[test]
fn discarding_restores_a_modified_file_from_the_index() {
    let dir = repo();
    let path = dir.path();

    std::fs::write(path.join("kept.txt"), "one\nCHANGED\nthree\n").unwrap();

    let file = entry(path, Side::Worktree, "kept.txt");
    let summary = git::discard(path, &file).unwrap();
    assert_eq!(summary, "Discarded changes to kept.txt");

    assert_eq!(
        std::fs::read_to_string(path.join("kept.txt")).unwrap(),
        "one\ntwo\nthree\n"
    );
    assert!(git::snapshot(path).unwrap().unstaged.is_empty());
}

#[test]
fn discarding_an_untracked_file_deletes_it() {
    let dir = repo();
    let path = dir.path();

    std::fs::write(path.join("scratch.txt"), "junk\n").unwrap();

    let file = entry(path, Side::Worktree, "scratch.txt");
    assert_eq!(file.change, Change::Untracked);

    let summary = git::discard(path, &file).unwrap();
    assert_eq!(summary, "Deleted scratch.txt");

    assert!(!path.join("scratch.txt").exists());
}

#[test]
fn discarding_leaves_the_staged_side_alone() {
    let dir = repo();
    let path = dir.path();

    // Stage one change, then make another on top of it.
    std::fs::write(path.join("kept.txt"), "one\nSTAGED\nthree\n").unwrap();
    git_cli(path, &["add", "kept.txt"]);
    std::fs::write(path.join("kept.txt"), "one\nSTAGED\nUNSTAGED\n").unwrap();

    let file = entry(path, Side::Worktree, "kept.txt");
    git::discard(path, &file).unwrap();

    assert_eq!(
        std::fs::read_to_string(path.join("kept.txt")).unwrap(),
        "one\nSTAGED\nthree\n",
        "discarding should go back to the index, not to HEAD"
    );
    assert_eq!(git::snapshot(path).unwrap().staged.len(), 1);
}

#[test]
fn discarding_something_staged_is_refused() {
    let dir = repo();
    let path = dir.path();

    std::fs::write(path.join("kept.txt"), "one\nCHANGED\nthree\n").unwrap();
    git_cli(path, &["add", "kept.txt"]);

    let file = entry(path, Side::Index, "kept.txt");
    let error = git::discard(path, &file).unwrap_err();

    assert!(
        error.to_string().contains("unstage it first"),
        "unexpected message: {error}"
    );
    assert_eq!(
        std::fs::read_to_string(path.join("kept.txt")).unwrap(),
        "one\nCHANGED\nthree\n",
        "a refused discard must not touch the file"
    );
}

#[test]
fn cherry_picking_brings_one_commit_across() {
    let dir = repo();
    let path = dir.path();

    git_cli(path, &["checkout", "--quiet", "-b", "side"]);
    std::fs::write(path.join("only-here.txt"), "from the side branch\n").unwrap();
    git_cli(path, &["add", "-A"]);
    git_cli(path, &["commit", "--quiet", "-m", "Add the side file"]);

    let wanted = git_cli(path, &["rev-parse", "HEAD"]).trim().to_owned();

    git_cli(path, &["checkout", "--quiet", "main"]);
    assert!(!path.join("only-here.txt").exists());

    let summary = git::cherry_pick(path, &wanted).unwrap();
    assert!(summary.starts_with("Cherry-picked"), "unexpected: {summary}");

    // The change is here, under a new commit that keeps the old message.
    assert!(path.join("only-here.txt").exists());
    assert_eq!(
        git_cli(path, &["log", "-1", "--format=%s"]).trim(),
        "Add the side file"
    );
    assert_ne!(
        git_cli(path, &["rev-parse", "HEAD"]).trim(),
        wanted,
        "a cherry-pick makes a new commit, it does not move the old one"
    );

    // Nothing left half-done.
    assert_eq!(git::snapshot(path).unwrap().pending_operation, None);
}

#[test]
fn cherry_picking_keeps_the_original_author() {
    let dir = repo();
    let path = dir.path();

    git_cli(path, &["checkout", "--quiet", "-b", "side"]);
    std::fs::write(path.join("theirs.txt"), "theirs\n").unwrap();
    git_cli(path, &["add", "-A"]);

    let output = Command::new("git")
        .args(["commit", "--quiet", "-m", "Their work"])
        .current_dir(path)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("HOME", path)
        .env("GIT_AUTHOR_NAME", "Someone Else")
        .env("GIT_AUTHOR_EMAIL", "else@example.invalid")
        .env("GIT_COMMITTER_NAME", "gitDruid Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
        .output()
        .unwrap();
    assert!(output.status.success());

    let wanted = git_cli(path, &["rev-parse", "HEAD"]).trim().to_owned();
    git_cli(path, &["checkout", "--quiet", "main"]);

    git::cherry_pick(path, &wanted).unwrap();

    assert_eq!(
        git_cli(path, &["log", "-1", "--format=%an"]).trim(),
        "Someone Else",
        "the author travels with the change"
    );
    assert_eq!(
        git_cli(path, &["log", "-1", "--format=%cn"]).trim(),
        "gitDruid Test",
        "the committer is whoever pressed the button"
    );
}

#[test]
fn a_conflicted_cherry_pick_is_left_open_and_finished_by_committing() {
    let dir = repo();
    let path = dir.path();

    git_cli(path, &["checkout", "--quiet", "-b", "side"]);
    std::fs::write(path.join("kept.txt"), "one\nTHEIRS\nthree\n").unwrap();
    git_cli(path, &["commit", "--quiet", "-am", "Their edit"]);
    let wanted = git_cli(path, &["rev-parse", "HEAD"]).trim().to_owned();

    git_cli(path, &["checkout", "--quiet", "main"]);
    std::fs::write(path.join("kept.txt"), "one\nOURS\nthree\n").unwrap();
    git_cli(path, &["commit", "--quiet", "-am", "Our edit"]);

    let summary = git::cherry_pick(path, &wanted).unwrap();
    assert!(summary.contains("conflicted"), "unexpected: {summary}");

    let snapshot = git::snapshot(path).unwrap();
    assert_eq!(snapshot.pending_operation.as_deref(), Some("cherry-pick"));

    // Committing is refused until it is resolved, then it finishes the pick.
    assert!(git::commit(path, "resolve").is_err());

    std::fs::write(path.join("kept.txt"), "one\nRESOLVED\nthree\n").unwrap();
    git_cli(path, &["add", "kept.txt"]);
    git::commit(path, "Take theirs").unwrap();

    assert_eq!(
        git::snapshot(path).unwrap().pending_operation,
        None,
        "CHERRY_PICK_HEAD should be cleaned up once it is committed"
    );
    assert_eq!(
        git_cli(path, &["rev-list", "--parents", "-n", "1", "HEAD"])
            .split_whitespace()
            .count(),
        2,
        "a cherry-pick records an ordinary commit, not a merge"
    );
}

#[test]
fn cherry_picking_something_already_here_says_so_and_cleans_up() {
    let dir = repo();
    let path = dir.path();

    std::fs::write(path.join("kept.txt"), "one\ntwo\nthree\nfour\n").unwrap();
    git_cli(path, &["commit", "--quiet", "-am", "Add four"]);

    let wanted = git_cli(path, &["rev-parse", "HEAD"]).trim().to_owned();

    let error = git::cherry_pick(path, &wanted).unwrap_err();
    assert!(
        error.to_string().contains("already present"),
        "unexpected: {error}"
    );

    assert_eq!(
        git::snapshot(path).unwrap().pending_operation,
        None,
        "a refused pick must not leave the repository mid-operation"
    );
}

#[test]
fn reverting_undoes_a_commit_with_a_new_one() {
    let dir = repo();
    let path = dir.path();

    std::fs::write(path.join("kept.txt"), "one\ntwo\nthree\nfour\n").unwrap();
    git_cli(path, &["commit", "--quiet", "-am", "Add four"]);

    let wanted = git_cli(path, &["rev-parse", "HEAD"]).trim().to_owned();

    let summary = git::revert(path, &wanted).unwrap();
    assert!(summary.starts_with("Reverted"), "unexpected: {summary}");

    assert_eq!(
        std::fs::read_to_string(path.join("kept.txt")).unwrap(),
        "one\ntwo\nthree\n",
        "the change should be undone"
    );

    // The original commit is still there; history moved forward, not back.
    assert!(git_cli(path, &["log", "--oneline"]).contains("Add four"));
    assert!(
        git_cli(path, &["log", "-1", "--format=%s"])
            .trim()
            .starts_with("Revert"),
        "the undo is itself a commit"
    );
    assert_eq!(git::snapshot(path).unwrap().pending_operation, None);
}

#[test]
fn a_merge_commit_cannot_be_picked_or_reverted_yet() {
    let dir = repo();
    let path = dir.path();

    git_cli(path, &["checkout", "--quiet", "-b", "side"]);
    std::fs::write(path.join("theirs.txt"), "theirs\n").unwrap();
    git_cli(path, &["add", "-A"]);
    git_cli(path, &["commit", "--quiet", "-m", "theirs"]);

    git_cli(path, &["checkout", "--quiet", "main"]);
    std::fs::write(path.join("ours.txt"), "ours\n").unwrap();
    git_cli(path, &["add", "-A"]);
    git_cli(path, &["commit", "--quiet", "-m", "ours"]);
    git_cli(path, &["merge", "--quiet", "--no-ff", "-m", "merge", "side"]);

    let merge = git_cli(path, &["rev-parse", "HEAD"]).trim().to_owned();

    for error in [
        git::cherry_pick(path, &merge).unwrap_err(),
        git::revert(path, &merge).unwrap_err(),
    ] {
        assert!(
            error.to_string().contains("merge commit"),
            "unexpected message: {error}"
        );
    }

    assert_eq!(git::snapshot(path).unwrap().pending_operation, None);
}

#[test]
fn aborting_puts_a_half_done_apply_back() {
    let dir = repo();
    let path = dir.path();

    git_cli(path, &["checkout", "--quiet", "-b", "side"]);
    std::fs::write(path.join("kept.txt"), "one\nTHEIRS\nthree\n").unwrap();
    git_cli(path, &["commit", "--quiet", "-am", "Their edit"]);
    let wanted = git_cli(path, &["rev-parse", "HEAD"]).trim().to_owned();

    git_cli(path, &["checkout", "--quiet", "main"]);
    std::fs::write(path.join("kept.txt"), "one\nOURS\nthree\n").unwrap();
    git_cli(path, &["commit", "--quiet", "-am", "Our edit"]);

    git::cherry_pick(path, &wanted).unwrap();
    assert_eq!(
        git::snapshot(path).unwrap().pending_operation.as_deref(),
        Some("cherry-pick")
    );

    git::abort(path).unwrap();

    assert_eq!(git::snapshot(path).unwrap().pending_operation, None);
    assert_eq!(
        std::fs::read_to_string(path.join("kept.txt")).unwrap(),
        "one\nOURS\nthree\n",
        "the working tree should be back where it was"
    );
    assert!(git::snapshot(path).unwrap().unstaged.is_empty());
}

#[test]
fn amending_replaces_the_last_commit_rather_than_adding_one() {
    let dir = repo();
    let path = dir.path();

    let before = git_cli(path, &["rev-parse", "HEAD"]).trim().to_owned();
    let count = |path: &Path| git_cli(path, &["rev-list", "--count", "HEAD"]).trim().to_owned();

    assert_eq!(count(path), "1");
    assert_eq!(git::head_message(path).unwrap(), "base");

    let summary = git::amend(path, "a better message").unwrap();
    assert!(summary.starts_with("Amended"), "unexpected: {summary}");

    assert_eq!(count(path), "1", "the branch must not grow");
    assert_eq!(git_cli(path, &["log", "-1", "--format=%s"]).trim(), "a better message");
    assert_ne!(
        git_cli(path, &["rev-parse", "HEAD"]).trim(),
        before,
        "it is a new commit in the old one's place"
    );
}

#[test]
fn amending_takes_whatever_is_staged_with_it() {
    let dir = repo();
    let path = dir.path();

    std::fs::write(path.join("kept.txt"), "one\ntwo\nthree\nforgotten\n").unwrap();
    git_cli(path, &["add", "kept.txt"]);

    git::amend(path, "base, with the bit I forgot").unwrap();

    assert_eq!(git_cli(path, &["rev-list", "--count", "HEAD"]).trim(), "1");
    assert!(
        git_cli(path, &["show", "--format=", "--name-only", "HEAD"]).contains("kept.txt")
    );
    assert!(git::snapshot(path).unwrap().staged.is_empty());
}

#[test]
fn amending_keeps_the_original_author_and_the_parents() {
    let dir = repo();
    let path = dir.path();

    // A second commit by someone else, so there is a parent to keep.
    std::fs::write(path.join("kept.txt"), "one\ntwo\n").unwrap();

    let output = Command::new("git")
        .args(["commit", "--quiet", "-am", "theirs"])
        .current_dir(path)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("HOME", path)
        .env("GIT_AUTHOR_NAME", "Someone Else")
        .env("GIT_AUTHOR_EMAIL", "else@example.invalid")
        .env("GIT_COMMITTER_NAME", "gitDruid Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
        .output()
        .unwrap();
    assert!(output.status.success());

    let parent = git_cli(path, &["rev-parse", "HEAD~1"]).trim().to_owned();

    git::amend(path, "reworded").unwrap();

    assert_eq!(
        git_cli(path, &["log", "-1", "--format=%an"]).trim(),
        "Someone Else",
        "the author belongs to the commit, not to whoever reworded it"
    );
    assert_eq!(
        git_cli(path, &["rev-parse", "HEAD~1"]).trim(),
        parent,
        "and it hangs where it did"
    );
}

#[test]
fn amending_needs_a_commit_a_message_and_a_settled_repository() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path();

    git_cli(path, &["init", "--initial-branch=main", "--quiet"]);
    git_cli(path, &["config", "user.name", "gitDruid Test"]);
    git_cli(path, &["config", "user.email", "test@example.invalid"]);

    // Nothing to amend yet.
    let error = git::amend(path, "anything").unwrap_err();
    assert!(error.to_string().contains("no commit to amend"), "{error}");
    assert!(git::head_message(path).is_err());

    std::fs::write(path.join("f"), "x\n").unwrap();
    git_cli(path, &["add", "-A"]);
    git_cli(path, &["commit", "--quiet", "-m", "base"]);

    let error = git::amend(path, "   ").unwrap_err();
    assert!(error.to_string().contains("needs a message"), "{error}");

    // And not while something else is half-done.
    git_cli(path, &["checkout", "--quiet", "-b", "other"]);
    std::fs::write(path.join("f"), "theirs\n").unwrap();
    git_cli(path, &["commit", "--quiet", "-am", "theirs"]);
    git_cli(path, &["checkout", "--quiet", "main"]);
    std::fs::write(path.join("f"), "ours\n").unwrap();
    git_cli(path, &["commit", "--quiet", "-am", "ours"]);
    git::merge_branch(path, "other").unwrap();

    let error = git::amend(path, "during a merge").unwrap_err();
    assert!(error.to_string().contains("mid-operation"), "{error}");
}

#[test]
fn a_root_commit_can_be_amended() {
    let dir = repo();
    let path = dir.path();

    git::amend(path, "the first commit, reworded").unwrap();

    assert_eq!(git_cli(path, &["rev-list", "--count", "HEAD"]).trim(), "1");
    assert_eq!(
        git_cli(path, &["log", "-1", "--format=%s"]).trim(),
        "the first commit, reworded"
    );
}
