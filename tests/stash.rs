//! Stashing and resetting, judged by the `git` CLI.

use std::path::Path;
use std::process::Command;

use git_druid::git::{self, Reset};

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

    std::fs::write(path.join("file.txt"), "one\n").unwrap();
    git_cli(path, &["add", "-A"]);
    git_cli(path, &["commit", "--quiet", "-m", "base"]);

    dir
}

fn text(path: &Path) -> String {
    std::fs::read_to_string(path.join("file.txt")).unwrap()
}

#[test]
fn stashing_puts_the_work_aside_and_leaves_the_tree_clean() {
    let dir = repo();
    let path = dir.path();

    std::fs::write(path.join("file.txt"), "one\nin progress\n").unwrap();

    let summary = git::stash_save(path, "half a thought", false).unwrap();
    assert!(summary.starts_with("Stashed as"), "unexpected: {summary}");

    assert_eq!(text(path), "one\n", "the working tree goes back");
    assert!(git::snapshot(path).unwrap().unstaged.is_empty());

    let stashes = git::stashes(path).unwrap();
    assert_eq!(stashes.len(), 1);
    assert_eq!(stashes[0].index, 0);
    assert!(
        stashes[0].message.contains("half a thought"),
        "the message should be kept: {:?}",
        stashes[0].message
    );
}

#[test]
fn popping_brings_it_back_and_takes_it_off_the_stack() {
    let dir = repo();
    let path = dir.path();

    std::fs::write(path.join("file.txt"), "one\nin progress\n").unwrap();
    git::stash_save(path, "", false).unwrap();

    git::stash_pop(path, 0).unwrap();

    assert_eq!(text(path), "one\nin progress\n");
    assert!(git::stashes(path).unwrap().is_empty(), "popping removes it");
}

#[test]
fn applying_brings_it_back_and_leaves_it_on_the_stack() {
    let dir = repo();
    let path = dir.path();

    std::fs::write(path.join("file.txt"), "one\nin progress\n").unwrap();
    git::stash_save(path, "", false).unwrap();

    git::stash_apply(path, 0).unwrap();

    assert_eq!(text(path), "one\nin progress\n");
    assert_eq!(git::stashes(path).unwrap().len(), 1, "applying keeps it");
}

#[test]
fn what_was_staged_goes_back_to_being_staged() {
    let dir = repo();
    let path = dir.path();

    std::fs::write(path.join("file.txt"), "one\nstaged\n").unwrap();
    git_cli(path, &["add", "file.txt"]);
    std::fs::write(path.join("other.txt"), "unstaged\n").unwrap();
    git_cli(path, &["add", "other.txt"]);
    git_cli(path, &["commit", "--quiet", "-m", "second"]);

    std::fs::write(path.join("file.txt"), "one\nstaged again\n").unwrap();
    git_cli(path, &["add", "file.txt"]);

    git::stash_save(path, "", false).unwrap();
    git::stash_pop(path, 0).unwrap();

    let snapshot = git::snapshot(path).unwrap();
    assert_eq!(
        snapshot.staged.len(),
        1,
        "the index is restored, not folded into the working tree: {snapshot:?}"
    );
}

#[test]
fn untracked_files_are_only_stashed_when_asked_for() {
    let dir = repo();
    let path = dir.path();

    std::fs::write(path.join("scratch.txt"), "junk\n").unwrap();

    // Without the flag there is nothing tracked to stash at all.
    let error = git::stash_save(path, "", false).unwrap_err();
    assert!(error.to_string().contains("nothing to stash"), "{error}");
    assert!(path.join("scratch.txt").exists());

    git::stash_save(path, "", true).unwrap();
    assert!(!path.join("scratch.txt").exists(), "it should have gone with it");

    git::stash_pop(path, 0).unwrap();
    assert!(path.join("scratch.txt").exists(), "and come back");
}

#[test]
fn several_stashes_stack_newest_first() {
    let dir = repo();
    let path = dir.path();

    for n in 1..=3 {
        std::fs::write(path.join("file.txt"), format!("one\nchange {n}\n")).unwrap();
        git::stash_save(path, &format!("change {n}"), false).unwrap();
    }

    let stashes = git::stashes(path).unwrap();

    assert_eq!(stashes.len(), 3);
    assert_eq!(stashes[0].index, 0);
    assert!(
        stashes[0].message.contains("change 3"),
        "index zero is the most recent: {:?}",
        stashes[0].message
    );

    // Dropping the middle one shifts the one under it up.
    git::stash_drop(path, 1).unwrap();

    let stashes = git::stashes(path).unwrap();
    assert_eq!(stashes.len(), 2);
    assert!(stashes[0].message.contains("change 3"));
    assert!(stashes[1].message.contains("change 1"));
}

#[test]
fn dropping_a_stash_that_is_not_there_says_so() {
    let dir = repo();
    let path = dir.path();

    let error = git::stash_drop(path, 4).unwrap_err();
    assert!(error.to_string().contains("no stash at 4"), "{error}");
}

#[test]
fn a_soft_reset_brings_the_commits_back_as_staged_changes() {
    let dir = repo();
    let path = dir.path();

    std::fs::write(path.join("file.txt"), "one\ntwo\n").unwrap();
    git_cli(path, &["commit", "--quiet", "-am", "second"]);

    git::reset(path, "HEAD~1", Reset::Soft).unwrap();

    assert_eq!(git_cli(path, &["rev-list", "--count", "HEAD"]).trim(), "1");
    assert_eq!(text(path), "one\ntwo\n", "the work is still there");

    let snapshot = git::snapshot(path).unwrap();
    assert_eq!(snapshot.staged.len(), 1, "and still staged");
    assert!(snapshot.unstaged.is_empty());
}

#[test]
fn a_mixed_reset_leaves_the_work_unstaged() {
    let dir = repo();
    let path = dir.path();

    std::fs::write(path.join("file.txt"), "one\ntwo\n").unwrap();
    git_cli(path, &["commit", "--quiet", "-am", "second"]);

    git::reset(path, "HEAD~1", Reset::Mixed).unwrap();

    assert_eq!(git_cli(path, &["rev-list", "--count", "HEAD"]).trim(), "1");
    assert_eq!(text(path), "one\ntwo\n");

    let snapshot = git::snapshot(path).unwrap();
    assert!(snapshot.staged.is_empty());
    assert_eq!(snapshot.unstaged.len(), 1);
}

#[test]
fn a_hard_reset_takes_the_work_with_it() {
    let dir = repo();
    let path = dir.path();

    std::fs::write(path.join("file.txt"), "one\ntwo\n").unwrap();
    git_cli(path, &["commit", "--quiet", "-am", "second"]);

    // Something uncommitted as well, which a hard reset also throws away.
    std::fs::write(path.join("file.txt"), "one\ntwo\nthree\n").unwrap();

    git::reset(path, "HEAD~1", Reset::Hard).unwrap();

    assert_eq!(git_cli(path, &["rev-list", "--count", "HEAD"]).trim(), "1");
    assert_eq!(text(path), "one\n", "everything after the target is gone");
    assert!(git::snapshot(path).unwrap().unstaged.is_empty());
}

#[test]
fn resetting_to_a_commit_that_is_not_there_says_so() {
    let dir = repo();
    let path = dir.path();

    let error = git::reset(path, "nonsense", Reset::Mixed).unwrap_err();
    assert!(error.to_string().contains("no commit nonsense"), "{error}");
}

#[test]
fn neither_stashing_nor_resetting_happens_mid_operation() {
    let dir = repo();
    let path = dir.path();

    git_cli(path, &["checkout", "--quiet", "-b", "other"]);
    std::fs::write(path.join("file.txt"), "theirs\n").unwrap();
    git_cli(path, &["commit", "--quiet", "-am", "theirs"]);

    git_cli(path, &["checkout", "--quiet", "main"]);
    std::fs::write(path.join("file.txt"), "ours\n").unwrap();
    git_cli(path, &["commit", "--quiet", "-am", "ours"]);

    git::merge_branch(path, "other").unwrap();

    for error in [
        git::stash_save(path, "", false).unwrap_err(),
        git::reset(path, "HEAD", Reset::Mixed).unwrap_err(),
    ] {
        assert!(error.to_string().contains("mid-operation"), "{error}");
    }
}
