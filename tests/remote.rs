//! Checks on fetch, pull and push.
//!
//! A bare repository in a temp directory stands in for a server. libgit2 talks
//! to it over its local transport, so these are real pushes and real fetches —
//! the same code paths a network remote takes, minus the credentials.

use std::path::Path;
use std::process::Command;

use git_druid::git;
use git_druid::settings::Credentials;

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

fn commit(path: &Path, name: &str) {
    std::fs::write(path.join(name), format!("{name}\n")).unwrap();
    git_cli(path, &["add", name]);
    git_cli(path, &["commit", "--quiet", "-m", name]);
}

/// A bare "server" with one commit, and a clone of it.
///
/// Returns the temp directory holding both; the clone is at `work`.
fn cloned() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("temp dir");
    let root = dir.path();

    let origin = root.join("origin.git");
    let seed = root.join("seed");

    std::fs::create_dir(&seed).unwrap();
    git_cli(&seed, &["init", "--initial-branch=main", "--quiet"]);
    git_cli(&seed, &["config", "user.name", "gitDruid Test"]);
    git_cli(&seed, &["config", "user.email", "test@example.invalid"]);
    commit(&seed, "base");

    // Explicit: the tests run with HOME inside the temp directory, so there is
    // no init.defaultBranch to fall back on, and a bare repository whose HEAD
    // names a branch that was never pushed clones with nothing checked out.
    git_cli(
        root,
        &["init", "--bare", "--initial-branch=main", "--quiet", "origin.git"],
    );
    git_cli(&seed, &["remote", "add", "origin", origin.to_str().unwrap()]);
    git_cli(&seed, &["push", "--quiet", "-u", "origin", "main"]);

    git_cli(
        root,
        &["clone", "--quiet", origin.to_str().unwrap(), "work"],
    );

    let work = root.join("work");
    git_cli(&work, &["config", "user.name", "gitDruid Test"]);
    git_cli(&work, &["config", "user.email", "test@example.invalid"]);

    assert_eq!(
        git_cli(&work, &["branch", "--show-current"]).trim(),
        "main",
        "the clone should have checked out main; the fixture is wrong if not"
    );

    dir
}

/// A local remote needs no credentials; these tests are about the operations,
/// not about how the app answers a server.
fn anonymous() -> Credentials {
    Credentials::default()
}

fn work(dir: &tempfile::TempDir) -> std::path::PathBuf {
    dir.path().join("work")
}

fn origin(dir: &tempfile::TempDir) -> std::path::PathBuf {
    dir.path().join("origin.git")
}

#[test]
fn a_repository_with_no_remote_has_nothing_to_track() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path();

    git_cli(path, &["init", "--initial-branch=main", "--quiet"]);
    git_cli(path, &["config", "user.name", "gitDruid Test"]);
    git_cli(path, &["config", "user.email", "test@example.invalid"]);
    commit(path, "base");

    assert_eq!(
        git::tracking(path).unwrap(),
        None,
        "with no remote there is nothing for the buttons to act on"
    );

    // And the operations say so rather than failing obscurely.
    let error = git::push(path, &anonymous()).unwrap_err();
    assert!(error.to_string().contains("no remote"), "unexpected: {error}");
}

#[test]
fn a_clone_tracks_the_branch_it_came_from() {
    let dir = cloned();

    let tracking = git::tracking(&work(&dir)).unwrap().expect("a tracked branch");

    assert_eq!(tracking.branch, "main");
    assert_eq!(tracking.remote, "origin");
    assert_eq!(tracking.upstream.as_deref(), Some("origin/main"));
    assert_eq!((tracking.ahead, tracking.behind), (0, 0));
    assert_eq!(tracking.destination(), "origin/main");
}

#[test]
fn pushing_sends_commits_to_the_remote() {
    let dir = cloned();
    let work = work(&dir);

    commit(&work, "one");
    commit(&work, "two");

    let tracking = git::tracking(&work).unwrap().unwrap();
    assert_eq!((tracking.ahead, tracking.behind), (2, 0));

    let summary = git::push(&work, &anonymous()).unwrap();
    assert_eq!(summary, "Pushed 2 commits to origin/main");

    // The bare repository really has them.
    let log = git_cli(&origin(&dir), &["log", "--oneline", "main"]);
    assert!(log.contains("two"), "origin should have the commits: {log}");

    // And gitDruid now agrees there is nothing left to send.
    let after = git::tracking(&work).unwrap().unwrap();
    assert_eq!((after.ahead, after.behind), (0, 0));

    assert_eq!(git::push(&work, &anonymous()).unwrap(), "origin/main is already up to date");
}

#[test]
fn pushing_a_new_branch_sets_its_upstream() {
    let dir = cloned();
    let work = work(&dir);

    git_cli(&work, &["checkout", "--quiet", "-b", "feature"]);
    commit(&work, "only-here");

    // A brand new branch tracks nothing, but origin is the obvious remote.
    let before = git::tracking(&work).unwrap().unwrap();
    assert_eq!(before.branch, "feature");
    assert_eq!(before.remote, "origin");
    assert_eq!(before.upstream, None);

    git::push(&work, &anonymous()).unwrap();

    let after = git::tracking(&work).unwrap().unwrap();
    assert_eq!(
        after.upstream.as_deref(),
        Some("origin/feature"),
        "the first push is what creates the tracking relationship"
    );

    assert!(git_cli(&origin(&dir), &["branch"]).contains("feature"));
}

#[test]
fn fetching_notices_what_the_remote_gained() {
    let dir = cloned();
    let work = work(&dir);

    // Someone else pushes, through a second clone.
    let other = dir.path().join("other");
    git_cli(
        dir.path(),
        &["clone", "--quiet", origin(&dir).to_str().unwrap(), "other"],
    );
    git_cli(&other, &["config", "user.name", "Someone Else"]);
    git_cli(&other, &["config", "user.email", "else@example.invalid"]);
    commit(&other, "theirs");
    git_cli(&other, &["push", "--quiet"]);

    // Until we fetch, our clone has no idea.
    assert_eq!(git::tracking(&work).unwrap().unwrap().behind, 0);

    let summary = git::fetch(&work, &anonymous()).unwrap();
    assert_eq!(summary, "Fetched origin — 1 to pull");

    assert_eq!(git::tracking(&work).unwrap().unwrap().behind, 1);

    // Fetching is not merging: the working tree is untouched.
    assert!(!work.join("theirs").exists());
    assert_eq!(git::snapshot(&work).unwrap().unstaged.len(), 0);
}

#[test]
fn pulling_fast_forwards_when_nothing_diverged() {
    let dir = cloned();
    let work = work(&dir);

    let other = dir.path().join("other");
    git_cli(
        dir.path(),
        &["clone", "--quiet", origin(&dir).to_str().unwrap(), "other"],
    );
    git_cli(&other, &["config", "user.name", "Someone Else"]);
    git_cli(&other, &["config", "user.email", "else@example.invalid"]);
    commit(&other, "theirs");
    git_cli(&other, &["push", "--quiet"]);

    let summary = git::pull(&work, &anonymous()).unwrap();
    assert!(
        summary.contains("Fast-forwarded"),
        "unexpected summary: {summary}"
    );

    // Pulling did touch the working tree, which is the difference from fetch.
    assert!(work.join("theirs").exists());
    assert_eq!(git::tracking(&work).unwrap().unwrap().behind, 0);
}

#[test]
fn pulling_merges_when_both_sides_moved() {
    let dir = cloned();
    let work = work(&dir);

    let other = dir.path().join("other");
    git_cli(
        dir.path(),
        &["clone", "--quiet", origin(&dir).to_str().unwrap(), "other"],
    );
    git_cli(&other, &["config", "user.name", "Someone Else"]);
    git_cli(&other, &["config", "user.email", "else@example.invalid"]);
    commit(&other, "theirs");
    git_cli(&other, &["push", "--quiet"]);

    commit(&work, "ours");

    let summary = git::pull(&work, &anonymous()).unwrap();
    assert!(summary.starts_with("Merged"), "unexpected summary: {summary}");

    // Both sides are present, under a real merge commit.
    assert!(work.join("theirs").exists());
    assert!(work.join("ours").exists());

    let parents = git_cli(&work, &["rev-list", "--parents", "-n", "1", "HEAD"]);
    assert_eq!(
        parents.split_whitespace().count(),
        3,
        "a merge commit and its two parents: {parents}"
    );

    // The merge is finished, not left open.
    assert_eq!(git::snapshot(&work).unwrap().pending_operation, None);
}

#[test]
fn pushing_from_behind_is_refused_before_it_reaches_the_remote() {
    let dir = cloned();
    let work = work(&dir);

    let other = dir.path().join("other");
    git_cli(
        dir.path(),
        &["clone", "--quiet", origin(&dir).to_str().unwrap(), "other"],
    );
    git_cli(&other, &["config", "user.name", "Someone Else"]);
    git_cli(&other, &["config", "user.email", "else@example.invalid"]);
    commit(&other, "theirs");
    git_cli(&other, &["push", "--quiet"]);

    commit(&work, "ours");
    git::fetch(&work, &anonymous()).unwrap();

    let error = git::push(&work, &anonymous()).unwrap_err();

    assert!(
        error.to_string().contains("behind") && error.to_string().contains("pull first"),
        "unexpected message: {error}"
    );

    // And nothing reached origin.
    let log = git_cli(&origin(&dir), &["log", "--oneline", "main"]);
    assert!(!log.contains("ours"), "origin should be untouched: {log}");
}

#[test]
fn a_conflicted_pull_is_left_open_for_the_user_to_finish() {
    let dir = cloned();
    let work = work(&dir);

    let other = dir.path().join("other");
    git_cli(
        dir.path(),
        &["clone", "--quiet", origin(&dir).to_str().unwrap(), "other"],
    );
    git_cli(&other, &["config", "user.name", "Someone Else"]);
    git_cli(&other, &["config", "user.email", "else@example.invalid"]);

    std::fs::write(other.join("shared.txt"), "theirs\n").unwrap();
    git_cli(&other, &["add", "-A"]);
    git_cli(&other, &["commit", "--quiet", "-m", "theirs"]);
    git_cli(&other, &["push", "--quiet"]);

    std::fs::write(work.join("shared.txt"), "ours\n").unwrap();
    git_cli(&work, &["add", "-A"]);
    git_cli(&work, &["commit", "--quiet", "-m", "ours"]);

    let summary = git::pull(&work, &anonymous()).unwrap();
    assert!(summary.contains("conflicted"), "unexpected: {summary}");

    // Left exactly where `git pull` leaves it, for the existing merge flow to
    // finish: resolve, stage, commit.
    let snapshot = git::snapshot(&work).unwrap();
    assert_eq!(snapshot.pending_operation.as_deref(), Some("merge"));

    std::fs::write(work.join("shared.txt"), "resolved\n").unwrap();
    git_cli(&work, &["add", "shared.txt"]);
    git::commit(&work, "merge origin/main").unwrap();

    assert_eq!(git::snapshot(&work).unwrap().pending_operation, None);
}

#[test]
fn pulling_a_branch_with_no_upstream_says_to_push_it_first() {
    let dir = cloned();
    let work = work(&dir);

    git_cli(&work, &["checkout", "--quiet", "-b", "local-only"]);
    commit(&work, "only-here");

    let error = git::pull(&work, &anonymous()).unwrap_err();

    assert!(
        error.to_string().contains("push it first"),
        "unexpected message: {error}"
    );
}
