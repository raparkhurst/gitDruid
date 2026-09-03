//! Resolving conflicts, against conflicts git itself produced.
//!
//! The parser is unit-tested on text; this is about the other half — the index
//! stages, and whether git agrees the file is resolved afterwards. `git status`
//! is the judge, not gitDruid's own reader.

use std::path::Path;
use std::process::Command;

use git_druid::git::{self, ConflictSide, Region};

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

/// A repository sitting in a conflicted merge of `file.txt`.
fn conflicted(ours: &str, theirs: &str, base: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path();

    git_cli(path, &["init", "--initial-branch=main", "--quiet"]);
    git_cli(path, &["config", "user.name", "gitDruid Test"]);
    git_cli(path, &["config", "user.email", "test@example.invalid"]);

    std::fs::write(path.join("file.txt"), base).unwrap();
    git_cli(path, &["add", "-A"]);
    git_cli(path, &["commit", "--quiet", "-m", "base"]);

    git_cli(path, &["checkout", "--quiet", "-b", "other"]);
    std::fs::write(path.join("file.txt"), theirs).unwrap();
    git_cli(path, &["commit", "--quiet", "-am", "theirs"]);

    git_cli(path, &["checkout", "--quiet", "main"]);
    std::fs::write(path.join("file.txt"), ours).unwrap();
    git_cli(path, &["commit", "--quiet", "-am", "ours"]);

    let summary = git::merge_branch(path, "other").unwrap();
    assert!(summary.contains("conflicted"), "expected a conflict: {summary}");

    dir
}

fn text(path: &Path) -> String {
    std::fs::read_to_string(path.join("file.txt")).unwrap()
}

/// `git status --short` for the file: `UU` while conflicted, `M ` once staged.
fn status(path: &Path) -> String {
    git_cli(path, &["status", "--short", "file.txt"])
        .chars()
        .take(2)
        .collect()
}

#[test]
fn a_conflicted_file_reads_back_with_both_sides() {
    let dir = conflicted("one\nOURS\nthree\n", "one\nTHEIRS\nthree\n", "one\ntwo\nthree\n");
    let path = dir.path();

    let conflict = git::conflict(path, Path::new("file.txt")).unwrap();

    assert!(!conflict.binary);
    assert_eq!(conflict.unresolved(), 1);
    assert!(!conflict.is_settled());

    let split = conflict
        .regions
        .iter()
        .find_map(|region| match region {
            Region::Split { ours, theirs, .. } => Some((ours.clone(), theirs.clone())),
            _ => None,
        })
        .expect("a disputed region");

    assert_eq!(split.0, ["OURS"]);
    assert_eq!(split.1, ["THEIRS"]);
}

#[test]
fn resolving_a_region_rewrites_the_file_without_its_markers() {
    let dir = conflicted("one\nOURS\nthree\n", "one\nTHEIRS\nthree\n", "one\ntwo\nthree\n");
    let path = dir.path();

    git::resolve(path, Path::new("file.txt"), 0, ConflictSide::Theirs).unwrap();

    assert_eq!(text(path), "one\nTHEIRS\nthree\n");

    // The file is settled, but git still calls it conflicted: the index has
    // not been told yet, which is what "mark resolved" is for.
    assert!(git::conflict(path, Path::new("file.txt")).unwrap().is_settled());
    assert_eq!(status(path), "UU");

    git::mark_resolved(path, Path::new("file.txt")).unwrap();
    assert_eq!(status(path), "M ");
    assert_eq!(git::snapshot(path).unwrap().unstaged.len(), 0);
}

#[test]
fn keeping_both_sides_keeps_ours_first() {
    let dir = conflicted("one\nOURS\nthree\n", "one\nTHEIRS\nthree\n", "one\ntwo\nthree\n");
    let path = dir.path();

    git::resolve(path, Path::new("file.txt"), 0, ConflictSide::Both).unwrap();

    assert_eq!(text(path), "one\nOURS\nTHEIRS\nthree\n");
}

/// Two edits far enough apart that git treats them as separate conflicts —
/// three lines of context is not enough to keep them apart.
fn two_apart(first: &str, second: &str) -> String {
    let mut lines = vec![first.to_owned()];

    lines.extend((1..=8).map(|n| format!("line {n}")));
    lines.push(second.to_owned());
    lines.push(String::new());

    lines.join("\n")
}

#[test]
fn regions_are_resolved_one_at_a_time_and_the_rest_are_left_alone() {
    let dir = conflicted(
        &two_apart("OURS1", "OURS2"),
        &two_apart("THEIRS1", "THEIRS2"),
        &two_apart("first", "second"),
    );
    let path = dir.path();

    assert_eq!(git::conflict(path, Path::new("file.txt")).unwrap().unresolved(), 2);

    // Take ours for the first; the second must still be marked up.
    git::resolve(path, Path::new("file.txt"), 0, ConflictSide::Ours).unwrap();

    let conflict = git::conflict(path, Path::new("file.txt")).unwrap();
    assert_eq!(conflict.unresolved(), 1, "only one should have gone");
    assert!(text(path).contains("OURS1"));
    assert!(text(path).contains("<<<<<<<"), "the other is still disputed");

    // Now the second, which is index 0 again once the first is gone.
    git::resolve(path, Path::new("file.txt"), 0, ConflictSide::Theirs).unwrap();

    assert_eq!(text(path), two_apart("OURS1", "THEIRS2"));
    assert!(git::conflict(path, Path::new("file.txt")).unwrap().is_settled());
}

#[test]
fn taking_a_whole_side_uses_what_git_recorded() {
    let dir = conflicted("one\nOURS\nthree\n", "one\nTHEIRS\nthree\n", "one\ntwo\nthree\n");
    let path = dir.path();

    // Even after the working tree has been scribbled on, the stages are what
    // each side actually committed.
    std::fs::write(path.join("file.txt"), "nonsense\n").unwrap();

    git::take(path, Path::new("file.txt"), ConflictSide::Theirs).unwrap();

    assert_eq!(text(path), "one\nTHEIRS\nthree\n");
    assert_eq!(status(path), "M ", "taking a side stages it as well");
}

#[test]
fn taking_ours_keeps_our_version() {
    let dir = conflicted("one\nOURS\nthree\n", "one\nTHEIRS\nthree\n", "one\ntwo\nthree\n");
    let path = dir.path();

    git::take(path, Path::new("file.txt"), ConflictSide::Ours).unwrap();

    assert_eq!(text(path), "one\nOURS\nthree\n");

    // Our side is what HEAD already had, so once it is staged there is nothing
    // to report: an empty status is the file being settled, not untouched.
    assert_eq!(status(path), "");
    assert!(
        git::snapshot(path).unwrap().unstaged.is_empty(),
        "nothing should be left conflicted"
    );
}

#[test]
fn a_file_with_markers_still_in_it_cannot_be_marked_resolved() {
    let dir = conflicted("one\nOURS\nthree\n", "one\nTHEIRS\nthree\n", "one\ntwo\nthree\n");
    let path = dir.path();

    let error = git::mark_resolved(path, Path::new("file.txt")).unwrap_err();

    assert!(
        error.to_string().contains("still has conflict markers"),
        "unexpected: {error}"
    );

    // Committing markers is the mistake nobody notices until it is pushed, so
    // the file had better still be conflicted.
    assert_eq!(status(path), "UU");
}

#[test]
fn a_resolved_merge_can_be_committed_and_leaves_nothing_open() {
    let dir = conflicted("one\nOURS\nthree\n", "one\nTHEIRS\nthree\n", "one\ntwo\nthree\n");
    let path = dir.path();

    git::resolve(path, Path::new("file.txt"), 0, ConflictSide::Ours).unwrap();
    git::mark_resolved(path, Path::new("file.txt")).unwrap();

    git::commit(path, "Merge other").unwrap();

    assert_eq!(git::snapshot(path).unwrap().pending_operation, None);
    assert_eq!(
        git_cli(path, &["rev-list", "--parents", "-n", "1", "HEAD"])
            .split_whitespace()
            .count(),
        3,
        "the merge commit keeps both parents"
    );
}

#[test]
fn a_conflict_that_has_moved_on_is_refused_rather_than_guessed_at() {
    let dir = conflicted("one\nOURS\nthree\n", "one\nTHEIRS\nthree\n", "one\ntwo\nthree\n");
    let path = dir.path();

    // Someone resolved it in an editor while the pane was open.
    std::fs::write(path.join("file.txt"), "settled\n").unwrap();

    let error = git::resolve(path, Path::new("file.txt"), 0, ConflictSide::Ours).unwrap_err();

    assert!(
        error.to_string().contains("not there any more"),
        "unexpected: {error}"
    );
    assert_eq!(text(path), "settled\n", "and the file was not touched");
}

#[test]
fn a_file_one_side_deleted_can_be_settled_either_way() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path();

    git_cli(path, &["init", "--initial-branch=main", "--quiet"]);
    git_cli(path, &["config", "user.name", "gitDruid Test"]);
    git_cli(path, &["config", "user.email", "test@example.invalid"]);

    std::fs::write(path.join("file.txt"), "original\n").unwrap();
    git_cli(path, &["add", "-A"]);
    git_cli(path, &["commit", "--quiet", "-m", "base"]);

    git_cli(path, &["checkout", "--quiet", "-b", "other"]);
    std::fs::remove_file(path.join("file.txt")).unwrap();
    git_cli(path, &["commit", "--quiet", "-am", "deleted"]);

    git_cli(path, &["checkout", "--quiet", "main"]);
    std::fs::write(path.join("file.txt"), "edited\n").unwrap();
    git_cli(path, &["commit", "--quiet", "-am", "edited"]);

    git::merge_branch(path, "other").unwrap();

    // Their side deleted it, so taking theirs means the file goes.
    git::take(path, Path::new("file.txt"), ConflictSide::Theirs).unwrap();

    assert!(!path.join("file.txt").exists(), "taking a deletion deletes");
    assert!(
        git::snapshot(path).unwrap().unstaged.is_empty(),
        "and nothing is left conflicted"
    );
}
