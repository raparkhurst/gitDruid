//! End-to-end checks against real repositories.
//!
//! Every assertion is made against the `git` CLI rather than against gitDruid's
//! own reader, so a bug in the diff layer cannot mask a bug in the staging
//! layer.

use std::fs;
use std::path::Path;
use std::process::Command;

use git_druid::git::{self, Change, Side};

/// Runs git in `repo` and returns its stdout.
fn git_cli(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("HOME", repo)
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

/// A repository with one committed file and an identity configured.
fn repo_with(contents: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path();

    git_cli(path, &["init", "--initial-branch=main", "--quiet"]);
    git_cli(path, &["config", "user.name", "gitDruid Test"]);
    git_cli(path, &["config", "user.email", "test@example.invalid"]);

    fs::write(path.join("file.txt"), contents).unwrap();

    git_cli(path, &["add", "file.txt"]);
    git_cli(path, &["commit", "--quiet", "-m", "initial"]);

    dir
}

fn numbered(lines: std::ops::Range<u32>) -> String {
    lines.map(|n| format!("line {n}\n")).collect()
}

fn entry(snapshot: &git::Snapshot, side: Side, name: &str) -> git::FileEntry {
    snapshot
        .find(side, Path::new(name))
        .unwrap_or_else(|| panic!("{name} should be listed under {:?}", side))
        .clone()
}

#[test]
fn stages_one_hunk_and_leaves_the_other_alone() {
    let dir = repo_with(&numbered(1..11));
    let path = dir.path();

    // Two edits far enough apart that three lines of context cannot join them.
    let mut lines: Vec<String> = numbered(1..11).lines().map(str::to_owned).collect();
    lines[1] = "line 2 CHANGED".to_owned();
    lines[9] = "line 10 CHANGED".to_owned();
    fs::write(path.join("file.txt"), lines.join("\n") + "\n").unwrap();

    let snapshot = git::snapshot(path).unwrap();
    let file = entry(&snapshot, Side::Worktree, "file.txt");
    assert_eq!(file.change, Change::Modified);

    let diff = git::file_diff(path, &file).unwrap();
    assert_eq!(
        diff.hunks().len(),
        2,
        "the two edits should be separate hunks"
    );

    git::stage_hunk(path, &diff, &diff.hunks()[0]).unwrap();

    // Real git must agree: the first edit is staged, the second is not.
    let staged = git_cli(path, &["diff", "--cached"]);
    assert!(
        staged.contains("+line 2 CHANGED"),
        "staged diff was:\n{staged}"
    );
    assert!(
        !staged.contains("line 10 CHANGED"),
        "staged diff was:\n{staged}"
    );

    let unstaged = git_cli(path, &["diff"]);
    assert!(
        unstaged.contains("+line 10 CHANGED"),
        "unstaged diff was:\n{unstaged}"
    );
    assert!(
        !unstaged.contains("line 2 CHANGED"),
        "unstaged diff was:\n{unstaged}"
    );

    // The working tree itself must be untouched by staging.
    let on_disk = fs::read_to_string(path.join("file.txt")).unwrap();
    assert!(on_disk.contains("line 2 CHANGED") && on_disk.contains("line 10 CHANGED"));
}

#[test]
fn unstaging_a_hunk_reverses_staging_it() {
    let dir = repo_with(&numbered(1..11));
    let path = dir.path();

    let mut lines: Vec<String> = numbered(1..11).lines().map(str::to_owned).collect();
    lines[1] = "line 2 CHANGED".to_owned();
    lines[9] = "line 10 CHANGED".to_owned();
    fs::write(path.join("file.txt"), lines.join("\n") + "\n").unwrap();

    let snapshot = git::snapshot(path).unwrap();
    let file = entry(&snapshot, Side::Worktree, "file.txt");
    let diff = git::file_diff(path, &file).unwrap();

    git::stage_hunk(path, &diff, &diff.hunks()[0]).unwrap();
    git::stage_hunk(path, &diff, &diff.hunks()[1]).unwrap();
    assert_eq!(git_cli(path, &["diff", "--name-only"]).trim(), "");

    // Now walk it back one hunk at a time.
    let snapshot = git::snapshot(path).unwrap();
    let staged_file = entry(&snapshot, Side::Index, "file.txt");
    let staged_diff = git::file_diff(path, &staged_file).unwrap();
    assert_eq!(staged_diff.hunks().len(), 2);

    git::unstage_hunk(path, &staged_diff, &staged_diff.hunks()[1]).unwrap();

    let staged = git_cli(path, &["diff", "--cached"]);
    assert!(
        staged.contains("+line 2 CHANGED"),
        "staged diff was:\n{staged}"
    );
    assert!(
        !staged.contains("line 10 CHANGED"),
        "staged diff was:\n{staged}"
    );
}

#[test]
fn stages_an_untracked_file() {
    let dir = repo_with("committed\n");
    let path = dir.path();

    fs::write(path.join("new.txt"), "fresh\ncontent\n").unwrap();

    let snapshot = git::snapshot(path).unwrap();
    let file = entry(&snapshot, Side::Worktree, "new.txt");
    assert_eq!(file.change, Change::Untracked);

    let diff = git::file_diff(path, &file).unwrap();
    assert_eq!(diff.hunks().len(), 1);

    git::stage_hunk(path, &diff, &diff.hunks()[0]).unwrap();

    assert_eq!(
        git_cli(path, &["diff", "--cached", "--name-only"]).trim(),
        "new.txt"
    );
}

#[test]
fn stages_a_deletion() {
    let dir = repo_with("gone\nsoon\n");
    let path = dir.path();

    fs::remove_file(path.join("file.txt")).unwrap();

    let snapshot = git::snapshot(path).unwrap();
    let file = entry(&snapshot, Side::Worktree, "file.txt");
    assert_eq!(file.change, Change::Deleted);

    let diff = git::file_diff(path, &file).unwrap();
    git::stage_hunk(path, &diff, &diff.hunks()[0]).unwrap();

    assert_eq!(
        git_cli(path, &["diff", "--cached", "--name-status"]).trim(),
        "D\tfile.txt"
    );
}

#[test]
fn preserves_a_file_with_no_trailing_newline() {
    // A missing final newline is the classic way a naive patcher corrupts a
    // file, so it gets its own test.
    let dir = repo_with("alpha\nbeta");
    let path = dir.path();

    fs::write(path.join("file.txt"), "alpha\nBETA").unwrap();

    let snapshot = git::snapshot(path).unwrap();
    let file = entry(&snapshot, Side::Worktree, "file.txt");
    let diff = git::file_diff(path, &file).unwrap();

    git::stage_hunk(path, &diff, &diff.hunks()[0]).unwrap();

    let staged = git_cli(path, &["show", ":file.txt"]);
    assert_eq!(staged, "alpha\nBETA");
    assert_eq!(git_cli(path, &["diff", "--name-only"]).trim(), "");
}

#[test]
fn refuses_a_stale_hunk() {
    let dir = repo_with(&numbered(1..11));
    let path = dir.path();

    let mut lines: Vec<String> = numbered(1..11).lines().map(str::to_owned).collect();
    lines[1] = "line 2 CHANGED".to_owned();
    fs::write(path.join("file.txt"), lines.join("\n") + "\n").unwrap();

    let snapshot = git::snapshot(path).unwrap();
    let file = entry(&snapshot, Side::Worktree, "file.txt");
    let diff = git::file_diff(path, &file).unwrap();

    // Someone else stages a different edit; the diff in hand is now stale.
    fs::write(path.join("file.txt"), "completely\ndifferent\n").unwrap();
    git_cli(path, &["add", "file.txt"]);

    let error = git::stage_hunk(path, &diff, &diff.hunks()[0])
        .expect_err("a stale hunk must not be applied");

    assert!(
        error.to_string().contains("refresh"),
        "unhelpful error: {error}"
    );
}

#[test]
fn stages_a_whole_file_and_commits_it() {
    let dir = repo_with("one\n");
    let path = dir.path();

    fs::write(path.join("file.txt"), "one\ntwo\n").unwrap();

    let snapshot = git::snapshot(path).unwrap();
    let file = entry(&snapshot, Side::Worktree, "file.txt");
    git::stage_file(path, &file).unwrap();

    let id = git::commit(path, "add a second line").unwrap();
    assert_eq!(id.len(), 7);

    assert_eq!(
        git_cli(path, &["log", "-1", "--format=%s"]).trim(),
        "add a second line"
    );
    assert_eq!(git_cli(path, &["status", "--porcelain"]).trim(), "");
}

#[test]
fn refuses_to_commit_nothing() {
    let dir = repo_with("stable\n");
    let path = dir.path();

    let error = git::commit(path, "empty").expect_err("an empty commit must be refused");
    assert!(
        error.to_string().contains("nothing is staged"),
        "got: {error}"
    );

    let error = git::commit(path, "   ").expect_err("a blank message must be refused");
    assert!(error.to_string().contains("message"), "got: {error}");
}

#[test]
fn unstages_a_whole_file() {
    let dir = repo_with("one\n");
    let path = dir.path();

    fs::write(path.join("file.txt"), "one\ntwo\n").unwrap();
    git_cli(path, &["add", "file.txt"]);

    let snapshot = git::snapshot(path).unwrap();
    let file = entry(&snapshot, Side::Index, "file.txt");
    git::unstage_file(path, &file).unwrap();

    assert_eq!(
        git_cli(path, &["diff", "--cached", "--name-only"]).trim(),
        ""
    );
    assert_eq!(git_cli(path, &["diff", "--name-only"]).trim(), "file.txt");
}

#[test]
fn reads_head_and_reports_an_unborn_branch() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();

    git_cli(path, &["init", "--initial-branch=main", "--quiet"]);
    fs::write(path.join("first.txt"), "hello\n").unwrap();

    let snapshot = git::snapshot(path).unwrap();

    assert!(snapshot.head.unborn, "a fresh repo has no commits yet");
    assert_eq!(snapshot.head.label, "main");
    assert_eq!(snapshot.unstaged.len(), 1);
    assert_eq!(snapshot.staged.len(), 0);
}

#[test]
fn opens_a_repository_from_any_path_inside_it() {
    let dir = repo_with(&numbered(1..4));
    let path = dir.path();

    fs::create_dir(path.join("nested")).unwrap();
    fs::write(path.join("nested/deep.txt"), "deep\n").unwrap();

    // The working directory libgit2 reports keeps a trailing separator, and on
    // macOS the temp dir is a symlink, so compare canonical paths.
    let expected = path.canonicalize().unwrap();

    let found = |start: &Path| {
        git::discover(start)
            .unwrap_or_else(|| panic!("{} should resolve to the repository", start.display()))
            .canonicalize()
            .unwrap()
    };

    assert_eq!(found(path), expected, "the root itself");
    assert_eq!(found(&path.join("nested")), expected, "a subdirectory");
    assert_eq!(found(&path.join("nested/deep.txt")), expected, "a file");
}

#[test]
fn refuses_a_directory_that_is_not_a_repository() {
    let dir = tempfile::tempdir().expect("temp dir");

    assert_eq!(git::discover(dir.path()), None);
}
