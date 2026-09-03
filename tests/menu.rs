//! Checks on what the right-click menu offers.
//!
//! Which lines appear is a judgement about the state of the repository — an
//! ignore entry for a file git is not yet tracking, no delete for the branch
//! you are standing on — so it is asserted against real repositories rather
//! than read off the screen.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use git_druid::app::{RefTarget, Target};
use git_druid::git;
use git_druid::settings::{Layer, Settings};
use git_druid::ui::menu::{self, Context};

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

    std::fs::write(path.join("kept.txt"), "one\n").unwrap();
    git_cli(path, &["add", "-A"]);
    git_cli(path, &["commit", "--quiet", "-m", "base"]);

    dir
}

/// Everything the menu reads, gathered the way the app gathers it.
struct Bundle {
    snapshot: git::Snapshot,
    refs: git::Refs,
    history: git::History,
    flow: git_druid::settings::Flow,
    marked: BTreeSet<PathBuf>,
}

fn bundle(path: &Path, config: &str) -> Bundle {
    Bundle {
        snapshot: git::snapshot(path).unwrap(),
        refs: git::refs(path).unwrap(),
        history: git::history(path).unwrap(),
        flow: Settings {
            global: Layer::parse(config),
            repo: Layer::default(),
        }
        .flow(),
        marked: BTreeSet::new(),
    }
}

/// The same bundle with a multi-selection, as a right-click over one leaves it.
fn with_marks(mut bundle: Bundle, names: &[&str]) -> Bundle {
    bundle.marked = names.iter().map(PathBuf::from).collect();
    bundle
}

fn labels(bundle: &Bundle, target: &Target) -> Vec<String> {
    menu::items(&context(bundle), target)
        .iter()
        .map(|item| item.label().to_owned())
        .filter(|label| !label.is_empty())
        .collect()
}

fn context(bundle: &Bundle) -> Context<'_> {
    Context {
        snapshot: &bundle.snapshot,
        refs: &bundle.refs,
        history: &bundle.history,
        flow: &bundle.flow,
        marked: &bundle.marked,
    }
}

fn mentions(labels: &[String], needle: &str) -> bool {
    labels.iter().any(|label| label.contains(needle))
}

#[test]
fn an_untracked_file_can_be_ignored_three_ways() {
    let dir = repo();
    let path = dir.path();

    std::fs::create_dir_all(path.join("logs")).unwrap();
    std::fs::write(path.join("logs/server.log"), "noise\n").unwrap();

    let bundle = bundle(path, "");
    let target = Target::File(git::Side::Worktree, PathBuf::from("logs/server.log"));
    let labels = labels(&bundle, &target);

    // The generated pattern is on the line, so what it will write is visible
    // before it is clicked.
    assert!(mentions(&labels, "Ignore this file  (/logs/server.log)"), "{labels:?}");
    assert!(mentions(&labels, "Ignore this extension  (*.log)"), "{labels:?}");
    assert!(mentions(&labels, "Ignore this folder  (/logs/)"), "{labels:?}");

    // And an untracked file is deleted rather than reverted.
    assert!(mentions(&labels, "Delete this file"), "{labels:?}");
}

#[test]
fn a_tracked_file_is_not_offered_an_ignore() {
    let dir = repo();
    let path = dir.path();

    std::fs::write(path.join("kept.txt"), "one\ntwo\n").unwrap();

    let bundle = bundle(path, "");
    let labels = labels(
        &bundle,
        &Target::File(git::Side::Worktree, PathBuf::from("kept.txt")),
    );

    // Ignoring something already in the index does nothing until it is removed
    // from it, so offering it would be a lie.
    assert!(!mentions(&labels, "Ignore"), "{labels:?}");
    assert!(mentions(&labels, "Discard these changes"), "{labels:?}");
    assert!(mentions(&labels, "Stage this file"), "{labels:?}");
}

#[test]
fn a_staged_file_offers_no_discard() {
    let dir = repo();
    let path = dir.path();

    std::fs::write(path.join("kept.txt"), "one\ntwo\n").unwrap();
    git_cli(path, &["add", "-A"]);

    let bundle = bundle(path, "");
    let labels = labels(
        &bundle,
        &Target::File(git::Side::Index, PathBuf::from("kept.txt")),
    );

    assert!(mentions(&labels, "Unstage this file"), "{labels:?}");
    assert!(
        !mentions(&labels, "Discard"),
        "unstaging is the answer on this side: {labels:?}"
    );
}

#[test]
fn a_commit_offers_the_things_you_do_to_one() {
    let dir = repo();
    let path = dir.path();

    let bundle = bundle(path, "");
    let id = bundle.history.commits.first().unwrap().id.clone();

    let labels = labels(&bundle, &Target::Commit(id.clone()));

    assert_eq!(labels[0], "base", "the summary names what was clicked");
    assert!(mentions(&labels, "Cherry-pick onto this branch"), "{labels:?}");
    assert!(mentions(&labels, "Revert this commit"), "{labels:?}");
    assert!(mentions(&labels, "Branch here"), "{labels:?}");
    assert!(mentions(&labels, "Tag here"), "{labels:?}");
    assert!(mentions(&labels, &format!("Copy id  ({id:.7})")), "{labels:?}");
}

#[test]
fn the_branch_you_are_on_cannot_be_checked_out_or_deleted_from_the_menu() {
    let dir = repo();
    let path = dir.path();

    git_cli(path, &["branch", "other"]);

    let bundle = bundle(path, "");

    let current = labels(&bundle, &Target::Ref(RefTarget::Local("main".to_owned())));
    assert!(!mentions(&current, "Check out"), "{current:?}");
    assert!(!mentions(&current, "Delete"), "{current:?}");
    assert!(mentions(&current, "Rename"), "{current:?}");

    let other = labels(&bundle, &Target::Ref(RefTarget::Local("other".to_owned())));
    assert!(mentions(&other, "Check out"), "{other:?}");
    assert!(mentions(&other, "Delete"), "{other:?}");
    assert!(mentions(&other, "Merge into this branch"), "{other:?}");
}

#[test]
fn a_workflow_branch_offers_to_finish_into_the_right_place() {
    let dir = repo();
    let path = dir.path();

    git_cli(path, &["branch", "develop"]);
    git_cli(path, &["branch", "feature/login"]);
    git_cli(path, &["branch", "hotfix/crash"]);

    let config = "[flow]\n enabled = true\n main = main\n develop = develop\n";
    let bundle = bundle(path, config);

    let feature = labels(
        &bundle,
        &Target::Ref(RefTarget::Local("feature/login".to_owned())),
    );
    assert!(mentions(&feature, "Finish into develop"), "{feature:?}");

    let hotfix = labels(
        &bundle,
        &Target::Ref(RefTarget::Local("hotfix/crash".to_owned())),
    );
    assert!(mentions(&hotfix, "Finish into main"), "{hotfix:?}");

    // A branch the workflow does not recognise has nowhere obvious to go.
    let plain = labels(&bundle, &Target::Ref(RefTarget::Local("develop".to_owned())));
    assert!(!mentions(&plain, "Finish"), "{plain:?}");
}

#[test]
fn without_a_workflow_nothing_offers_to_finish() {
    let dir = repo();
    let path = dir.path();

    git_cli(path, &["branch", "feature/login"]);

    let bundle = bundle(path, "");
    let labels = labels(
        &bundle,
        &Target::Ref(RefTarget::Local("feature/login".to_owned())),
    );

    assert!(!mentions(&labels, "Finish"), "{labels:?}");
}

#[test]
fn a_tag_offers_only_what_can_be_done_to_a_tag() {
    let dir = repo();
    let path = dir.path();

    git_cli(path, &["tag", "v1.0"]);

    let bundle = bundle(path, "");
    let labels = labels(&bundle, &Target::Ref(RefTarget::Tag("v1.0".to_owned())));

    assert_eq!(labels, ["v1.0", "Delete tag"]);
}

#[test]
fn destructive_lines_are_marked_as_such() {
    let dir = repo();
    let path = dir.path();

    std::fs::write(path.join("scratch.txt"), "junk\n").unwrap();

    let bundle = bundle(path, "");

    let items = menu::items(
        &context(&bundle),
        &Target::File(git::Side::Worktree, PathBuf::from("scratch.txt")),
    );

    let marked: Vec<&str> = items
        .iter()
        .filter(|item| item.is_destructive())
        .map(|item| item.label())
        .collect();

    assert_eq!(
        marked,
        ["Delete this file"],
        "only the line that loses work should be styled as dangerous"
    );
}

#[test]
fn a_menu_for_a_file_that_is_gone_is_empty() {
    let dir = repo();
    let path = dir.path();

    let bundle = bundle(path, "");
    let labels = labels(
        &bundle,
        &Target::File(git::Side::Worktree, PathBuf::from("never-existed.txt")),
    );

    assert!(
        labels.is_empty(),
        "a stale target should offer nothing rather than a menu of no-ops: {labels:?}"
    );
}

/// Three untracked files, so a selection has something to be made of.
fn three_files(path: &Path) {
    for name in ["one.txt", "two.txt", "three.txt"] {
        std::fs::write(path.join(name), "x\n").unwrap();
    }
}

#[test]
fn a_menu_over_a_selection_acts_on_the_whole_selection() {
    let dir = repo();
    let path = dir.path();

    three_files(path);

    let bundle = with_marks(bundle(path, ""), &["one.txt", "two.txt", "three.txt"]);
    let labels = labels(
        &bundle,
        &Target::File(git::Side::Worktree, PathBuf::from("two.txt")),
    );

    assert_eq!(
        labels[0], "3 files selected",
        "the heading should name the selection, not the row: {labels:?}"
    );
    assert!(
        mentions(&labels, "Stage these 3 files"),
        "the line must say how many it will stage: {labels:?}"
    );
    assert!(
        !mentions(&labels, "Stage this file"),
        "offering to stage one while three are selected is the bug: {labels:?}"
    );
    assert!(
        mentions(&labels, "Discard changes to these 3 files"),
        "{labels:?}"
    );
}

#[test]
fn a_selection_of_one_still_reads_as_one_file() {
    let dir = repo();
    let path = dir.path();

    three_files(path);

    let bundle = with_marks(bundle(path, ""), &["two.txt"]);
    let labels = labels(
        &bundle,
        &Target::File(git::Side::Worktree, PathBuf::from("two.txt")),
    );

    assert_eq!(labels[0], "two.txt");
    assert!(mentions(&labels, "Stage this file"), "{labels:?}");
    assert!(!mentions(&labels, "these"), "{labels:?}");
}

#[test]
fn a_staged_selection_offers_to_unstage_all_of_it() {
    let dir = repo();
    let path = dir.path();

    three_files(path);
    git_cli(path, &["add", "-A"]);

    let bundle = with_marks(bundle(path, ""), &["one.txt", "two.txt"]);
    let labels = labels(
        &bundle,
        &Target::File(git::Side::Index, PathBuf::from("one.txt")),
    );

    assert!(mentions(&labels, "Unstage these 2 files"), "{labels:?}");
    assert!(!mentions(&labels, "Discard"), "{labels:?}");
}

#[test]
fn marks_for_files_on_the_other_side_do_not_inflate_the_count() {
    let dir = repo();
    let path = dir.path();

    three_files(path);
    git_cli(path, &["add", "one.txt"]);

    // one.txt is staged now, so a mark on it is not part of the unstaged list.
    let bundle = with_marks(bundle(path, ""), &["one.txt", "two.txt"]);
    let labels = labels(
        &bundle,
        &Target::File(git::Side::Worktree, PathBuf::from("two.txt")),
    );

    assert!(
        mentions(&labels, "Stage this file"),
        "only one of the marks is on this side: {labels:?}"
    );
}

#[test]
fn ignoring_is_not_offered_for_a_selection() {
    let dir = repo();
    let path = dir.path();

    three_files(path);

    let target = Target::File(git::Side::Worktree, PathBuf::from("one.txt"));

    let selection = with_marks(bundle(path, ""), &["one.txt", "two.txt"]);
    let many = labels(&selection, &target);

    // Three files rarely want the same rule, and a wrong ignore is quiet
    // until it bites.
    assert!(!mentions(&many, "Ignore"), "{many:?}");

    // On its own, the same file is offered one.
    let alone = with_marks(bundle(path, ""), &["one.txt"]);
    assert!(
        mentions(&labels(&alone, &target), "Ignore this file"),
        "a single file should still be ignorable"
    );
}
