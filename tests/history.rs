//! Checks on the commit graph and the ref operations.
//!
//! The graph's lane assignment is the part of gitDruid that a screenshot
//! cannot verify: a line drawn in the wrong lane still looks like a graph. So
//! the shape of the layout is asserted here, against repositories whose
//! topology is built by the `git` CLI.

use std::path::Path;
use std::process::Command;

use git_druid::git::{self, BadgeKind, Edge};

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

/// An initialised repository with an identity configured.
fn empty_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path();

    git_cli(path, &["init", "--initial-branch=main", "--quiet"]);
    git_cli(path, &["config", "user.name", "gitDruid Test"]);
    git_cli(path, &["config", "user.email", "test@example.invalid"]);

    dir
}

/// Writes `name` and commits it, returning nothing — the graph is read back
/// through gitDruid rather than by id.
fn commit(path: &Path, name: &str) {
    std::fs::write(path.join(name), format!("{name}\n")).unwrap();
    git_cli(path, &["add", name]);
    git_cli(path, &["commit", "--quiet", "-m", name]);
}

/// A repository with a linear history of three commits on `main`.
fn linear() -> tempfile::TempDir {
    let dir = empty_repo();

    for name in ["one", "two", "three"] {
        commit(dir.path(), name);
    }

    dir
}

#[test]
fn a_linear_history_uses_one_lane() {
    let dir = linear();

    let history = git::history(dir.path()).unwrap();

    assert_eq!(history.commits.len(), 3);
    assert_eq!(history.lanes, 1, "a straight history needs one lane");

    // Newest first, and every commit in lane zero.
    let summaries: Vec<&str> = history
        .commits
        .iter()
        .map(|commit| commit.summary.as_str())
        .collect();
    assert_eq!(summaries, ["three", "two", "one"]);

    for commit in &history.commits {
        assert_eq!(commit.row.node, 0, "{} should sit in lane 0", commit.summary);
    }

    // The root has no parent, so the line above it ends at the node and
    // nothing leaves the bottom of the row.
    let root = history.commits.last().unwrap();
    assert!(
        root.row
            .edges
            .iter()
            .all(|edge| matches!(edge, Edge::Into { .. })),
        "the root commit only receives lines: {:?}",
        root.row.edges
    );
    assert!(
        root.row.edges.iter().all(|edge| edge.bottom().is_none()),
        "nothing continues below the root commit: {:?}",
        root.row.edges
    );

    // And the tip is the other way round: nothing above it, one line below.
    let tip = history.commits.first().unwrap();
    assert!(
        tip.row.edges.iter().all(|edge| edge.top().is_none()),
        "nothing arrives above the newest commit: {:?}",
        tip.row.edges
    );
}

#[test]
fn a_branch_gets_a_lane_of_its_own() {
    let dir = empty_repo();
    let path = dir.path();

    commit(path, "base");
    git_cli(path, &["checkout", "--quiet", "-b", "side"]);
    commit(path, "on-side");
    git_cli(path, &["checkout", "--quiet", "main"]);
    commit(path, "on-main");

    let history = git::history(path).unwrap();

    assert_eq!(history.commits.len(), 3);
    assert_eq!(history.lanes, 2, "two tips need two lanes");

    let tips: Vec<usize> = history
        .commits
        .iter()
        .filter(|commit| commit.summary != "base")
        .map(|commit| commit.row.node)
        .collect();

    assert_eq!(
        tips.len(),
        2,
        "both branch tips should be in the graph: {tips:?}"
    );
    assert_ne!(tips[0], tips[1], "two tips must not share a lane");

    // Both tips converge on the base commit, so its row draws a line for each.
    let base = history
        .commits
        .iter()
        .find(|commit| commit.summary == "base")
        .expect("base should be in the graph");

    let arriving = base
        .row
        .edges
        .iter()
        .filter(|edge| matches!(edge, Edge::Into { .. }))
        .count();

    assert_eq!(
        arriving, 2,
        "both branches converge on base: {:?}",
        base.row.edges
    );
}

#[test]
fn a_merge_draws_a_line_to_each_parent() {
    let dir = empty_repo();
    let path = dir.path();

    commit(path, "base");
    git_cli(path, &["checkout", "--quiet", "-b", "side"]);
    commit(path, "on-side");
    git_cli(path, &["checkout", "--quiet", "main"]);
    commit(path, "on-main");
    git_cli(path, &["merge", "--quiet", "--no-ff", "-m", "merge", "side"]);

    let history = git::history(path).unwrap();

    let merge = history
        .commits
        .iter()
        .find(|commit| commit.summary == "merge")
        .expect("the merge should be in the graph");

    let lanes: Vec<usize> = merge
        .row
        .edges
        .iter()
        .filter_map(|edge| match edge {
            Edge::Out { to, .. } => Some(*to),
            _ => None,
        })
        .collect();

    assert_eq!(
        lanes.len(),
        2,
        "a merge leaves one line per parent: {:?}",
        merge.row.edges
    );
    assert!(
        lanes.contains(&merge.row.node),
        "the first parent keeps the merge's own lane: {lanes:?}"
    );
    assert_eq!(
        lanes.iter().collect::<std::collections::HashSet<_>>().len(),
        2,
        "the second parent takes a different lane: {lanes:?}"
    );
}

#[test]
fn every_row_stays_inside_the_lanes_it_reports() {
    let dir = empty_repo();
    let path = dir.path();

    // Three tips at once, so lanes are allocated, freed and reused.
    commit(path, "base");

    for branch in ["a", "b", "c"] {
        git_cli(path, &["checkout", "--quiet", "-b", branch, "main"]);
        commit(path, branch);
    }

    let history = git::history(path).unwrap();

    for commit in &history.commits {
        assert!(
            commit.row.node < commit.row.lanes,
            "{} sits in lane {} but the row reports {} lanes",
            commit.summary,
            commit.row.node,
            commit.row.lanes
        );
        assert!(
            commit.row.lanes <= history.lanes,
            "{} reports more lanes than the graph is wide",
            commit.summary
        );

        for edge in &commit.row.edges {
            for lane in [edge.top(), edge.bottom()].into_iter().flatten() {
                assert!(
                    lane < history.lanes,
                    "{} draws a line in lane {lane}, outside a {}-lane graph",
                    commit.summary,
                    history.lanes
                );
            }
        }
    }
}

#[test]
fn head_branches_and_tags_are_badged() {
    let dir = linear();
    let path = dir.path();

    git_cli(path, &["branch", "other"]);
    git_cli(path, &["tag", "v1.0"]);

    let history = git::history(path).unwrap();
    let tip = history.commits.first().expect("a tip commit");

    let kinds: Vec<BadgeKind> = tip.badges.iter().map(|badge| badge.kind).collect();

    assert!(
        kinds.contains(&BadgeKind::Head),
        "the checked-out branch is badged as HEAD: {:?}",
        tip.badges
    );
    assert!(
        kinds.contains(&BadgeKind::LocalBranch),
        "another branch on the same commit is badged too: {:?}",
        tip.badges
    );
    assert!(
        kinds.contains(&BadgeKind::Tag),
        "the tag is badged: {:?}",
        tip.badges
    );

    // HEAD reads first, so the badge that matters most is the one seen first.
    assert_eq!(tip.badges[0].kind, BadgeKind::Head);
}

#[test]
fn branches_are_created_checked_out_renamed_and_deleted() {
    let dir = linear();
    let path = dir.path();

    git::create_branch(path, "feature", None).unwrap();

    let refs = git::refs(path).unwrap();
    let names: Vec<&str> = refs.local.iter().map(|b| b.name.as_str()).collect();
    assert_eq!(names, ["feature", "main"]);
    assert!(refs.local.iter().any(|b| b.name == "main" && b.is_head));

    git::checkout_branch(path, "feature").unwrap();
    assert_eq!(git_cli(path, &["branch", "--show-current"]).trim(), "feature");

    git::rename_branch(path, "feature", "renamed").unwrap();
    assert_eq!(git_cli(path, &["branch", "--show-current"]).trim(), "renamed");

    // The branch HEAD is on cannot be deleted, whatever the force flag says.
    let refused = git::delete_branch(path, "renamed", true).unwrap_err();
    assert!(
        refused.to_string().contains("current branch"),
        "unexpected message: {refused}"
    );

    git::checkout_branch(path, "main").unwrap();
    git::delete_branch(path, "renamed", false).unwrap();

    assert!(!git_cli(path, &["branch"]).contains("renamed"));
}

#[test]
fn deleting_an_unmerged_branch_needs_forcing() {
    let dir = linear();
    let path = dir.path();

    git_cli(path, &["checkout", "--quiet", "-b", "unmerged"]);
    commit(path, "only-here");
    git_cli(path, &["checkout", "--quiet", "main"]);

    let refs = git::refs(path).unwrap();
    let branch = refs
        .local
        .iter()
        .find(|b| b.name == "unmerged")
        .expect("the branch should be listed");
    assert!(
        !branch.merged,
        "a branch with its own commits is not merged, which is what the warning keys off"
    );

    let refused = git::delete_branch(path, "unmerged", false).unwrap_err();
    assert!(
        refused.to_string().contains("not on HEAD"),
        "unexpected message: {refused}"
    );

    git::delete_branch(path, "unmerged", true).unwrap();
    assert!(!git_cli(path, &["branch"]).contains("unmerged"));
}

#[test]
fn tags_are_created_and_deleted() {
    let dir = linear();
    let path = dir.path();

    git::create_tag(path, "v1.0", None, None).unwrap();
    git::create_tag(path, "v1.1", None, Some("a release")).unwrap();

    let refs = git::refs(path).unwrap();
    let names: Vec<&str> = refs.tags.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["v1.0", "v1.1"]);

    // The annotated tag still reports the commit it points at, not the tag
    // object, which is what the graph badges against.
    assert_eq!(refs.tags[0].short_id, refs.tags[1].short_id);

    let clash = git::create_tag(path, "v1.0", None, None).unwrap_err();
    assert!(clash.to_string().contains("already exists"));

    git::delete_tag(path, "v1.0").unwrap();
    assert_eq!(git::refs(path).unwrap().tags.len(), 1);
}

#[test]
fn merging_fast_forwards_when_it_can() {
    let dir = linear();
    let path = dir.path();

    git_cli(path, &["checkout", "--quiet", "-b", "ahead"]);
    commit(path, "extra");
    git_cli(path, &["checkout", "--quiet", "main"]);

    let before = git_cli(path, &["rev-parse", "HEAD"]);
    let summary = git::merge_branch(path, "ahead").unwrap();

    assert!(
        summary.contains("Fast-forwarded"),
        "unexpected summary: {summary}"
    );
    assert_ne!(before, git_cli(path, &["rev-parse", "HEAD"]));
    assert_eq!(
        git_cli(path, &["rev-parse", "HEAD"]),
        git_cli(path, &["rev-parse", "ahead"]),
        "main should now be exactly where ahead is"
    );
}

#[test]
fn merging_divergent_branches_writes_a_merge_commit() {
    let dir = empty_repo();
    let path = dir.path();

    commit(path, "base");
    git_cli(path, &["checkout", "--quiet", "-b", "side"]);
    commit(path, "on-side");
    git_cli(path, &["checkout", "--quiet", "main"]);
    commit(path, "on-main");

    let summary = git::merge_branch(path, "side").unwrap();
    assert!(summary.starts_with("Merged side"), "unexpected: {summary}");

    // Two parents, and both branches' files present.
    let parents = git_cli(path, &["rev-list", "--parents", "-n", "1", "HEAD"]);
    assert_eq!(
        parents.split_whitespace().count(),
        3,
        "a merge commit and its two parents: {parents}"
    );
    assert!(path.join("on-side").exists());
    assert!(path.join("on-main").exists());

    // And the repository is clean again — no MERGE_HEAD left behind.
    assert_eq!(
        git::snapshot(path).unwrap().pending_operation,
        None,
        "the merge should have been finished, not left open"
    );
}

#[test]
fn a_conflicted_merge_is_left_open_and_finished_by_committing() {
    let dir = empty_repo();
    let path = dir.path();

    std::fs::write(path.join("file.txt"), "base\n").unwrap();
    git_cli(path, &["add", "file.txt"]);
    git_cli(path, &["commit", "--quiet", "-m", "base"]);

    git_cli(path, &["checkout", "--quiet", "-b", "side"]);
    std::fs::write(path.join("file.txt"), "side\n").unwrap();
    git_cli(path, &["commit", "--quiet", "-am", "side"]);

    git_cli(path, &["checkout", "--quiet", "main"]);
    std::fs::write(path.join("file.txt"), "main\n").unwrap();
    git_cli(path, &["commit", "--quiet", "-am", "main"]);

    let summary = git::merge_branch(path, "side").unwrap();
    assert!(summary.contains("conflicted"), "unexpected: {summary}");

    let snapshot = git::snapshot(path).unwrap();
    assert_eq!(snapshot.pending_operation.as_deref(), Some("merge"));

    // Committing is refused until the conflict is resolved.
    let refused = git::commit(path, "merge").unwrap_err();
    assert!(
        refused.to_string().contains("resolve"),
        "unexpected message: {refused}"
    );

    // Resolve, stage, and the commit gitDruid writes is a real merge commit.
    std::fs::write(path.join("file.txt"), "resolved\n").unwrap();
    git_cli(path, &["add", "file.txt"]);

    git::commit(path, "merge side").unwrap();

    let parents = git_cli(path, &["rev-list", "--parents", "-n", "1", "HEAD"]);
    assert_eq!(
        parents.split_whitespace().count(),
        3,
        "the finished merge has two parents: {parents}"
    );
    assert_eq!(
        git::snapshot(path).unwrap().pending_operation,
        None,
        "MERGE_HEAD should be cleaned up once the merge is committed"
    );
}

#[test]
fn a_commit_reports_what_it_changed() {
    let dir = empty_repo();
    let path = dir.path();

    std::fs::write(path.join("kept.txt"), "one\ntwo\n").unwrap();
    std::fs::write(path.join("gone.txt"), "bye\n").unwrap();
    git_cli(path, &["add", "."]);
    git_cli(path, &["commit", "--quiet", "-m", "base"]);

    std::fs::write(path.join("kept.txt"), "one\ntwo\nthree\n").unwrap();
    std::fs::remove_file(path.join("gone.txt")).unwrap();
    std::fs::write(path.join("new.txt"), "hello\n").unwrap();
    git_cli(path, &["add", "-A"]);
    git_cli(path, &["commit", "--quiet", "-m", "the change"]);

    let history = git::history(path).unwrap();
    let tip = history.commits.first().unwrap();

    let detail = git::commit_detail(path, &tip.id).unwrap();

    assert_eq!(detail.message, "the change");
    assert_eq!(detail.parents.len(), 1);

    let mut described: Vec<(String, usize, usize)> = detail
        .files
        .iter()
        .map(|file| (file.display(), file.added, file.removed))
        .collect();
    described.sort();

    assert_eq!(
        described,
        [
            ("gone.txt".to_owned(), 0, 1),
            ("kept.txt".to_owned(), 1, 0),
            ("new.txt".to_owned(), 1, 0),
        ]
    );
}

#[test]
fn a_commits_file_can_be_read_back_as_a_diff() {
    let dir = empty_repo();
    let path = dir.path();

    std::fs::write(path.join("file.txt"), "one\ntwo\nthree\n").unwrap();
    git_cli(path, &["add", "."]);
    git_cli(path, &["commit", "--quiet", "-m", "base"]);

    std::fs::write(path.join("file.txt"), "one\nTWO\nthree\n").unwrap();
    git_cli(path, &["commit", "--quiet", "-am", "shout"]);

    let history = git::history(path).unwrap();
    let tip = history.commits.first().unwrap();
    let detail = git::commit_detail(path, &tip.id).unwrap();

    let file = detail.files.first().expect("one changed file");
    let diff = git::commit_file_diff(path, &tip.id, file).unwrap();

    assert_eq!(diff.counts(), (1, 1));
    assert_eq!(diff.path, std::path::PathBuf::from("file.txt"));

    // The diff belongs to the commit, so nothing about it can be staged.
    assert_eq!(diff.source.side(), None);
    assert!(matches!(diff.source, git::Source::Commit(ref id) if *id == tip.id));

    let lines: Vec<String> = diff
        .hunks()
        .iter()
        .flat_map(|hunk| hunk.lines.iter())
        .map(|line| format!("{}{}", line.origin.sign(), line.text()))
        .collect();

    assert_eq!(lines, [" one", "-two", "+TWO", " three"]);
}

#[test]
fn staging_a_committed_hunk_is_refused() {
    let dir = empty_repo();
    let path = dir.path();

    std::fs::write(path.join("file.txt"), "one\n").unwrap();
    git_cli(path, &["add", "."]);
    git_cli(path, &["commit", "--quiet", "-m", "base"]);

    std::fs::write(path.join("file.txt"), "two\n").unwrap();
    git_cli(path, &["commit", "--quiet", "-am", "change"]);

    let history = git::history(path).unwrap();
    let tip = history.commits.first().unwrap();
    let detail = git::commit_detail(path, &tip.id).unwrap();
    let diff = git::commit_file_diff(path, &tip.id, &detail.files[0]).unwrap();

    let hunk = diff.hunks().first().expect("a hunk");

    // The UI never offers this, but the guard is what makes that safe.
    assert!(git::stage_hunk(path, &diff, hunk).is_err());
    assert!(git::unstage_hunk(path, &diff, hunk).is_err());
}

#[test]
fn a_commits_file_diff_follows_a_rename() {
    let dir = empty_repo();
    let path = dir.path();

    let body: String = (1..=20).map(|n| format!("line {n}\n")).collect();
    std::fs::write(path.join("before.txt"), &body).unwrap();
    git_cli(path, &["add", "."]);
    git_cli(path, &["commit", "--quiet", "-m", "base"]);

    std::fs::rename(path.join("before.txt"), path.join("after.txt")).unwrap();
    std::fs::write(path.join("after.txt"), body.replace("line 1\n", "LINE 1\n")).unwrap();
    git_cli(path, &["add", "-A"]);
    git_cli(path, &["commit", "--quiet", "-m", "rename"]);

    let history = git::history(path).unwrap();
    let tip = history.commits.first().unwrap();
    let detail = git::commit_detail(path, &tip.id).unwrap();

    let file = detail.files.first().expect("one changed file");
    assert_eq!(file.display(), "before.txt → after.txt");

    // The rename is what makes this worth testing: the diff has to be found
    // under the old name as well as the new one.
    let diff = git::commit_file_diff(path, &tip.id, file).unwrap();
    assert_eq!(diff.counts(), (1, 1));
}

#[test]
fn a_root_commits_files_diff_against_nothing() {
    let dir = empty_repo();
    let path = dir.path();

    std::fs::write(path.join("file.txt"), "one\ntwo\n").unwrap();
    git_cli(path, &["add", "."]);
    git_cli(path, &["commit", "--quiet", "-m", "first"]);

    let history = git::history(path).unwrap();
    let root = history.commits.last().unwrap();
    let detail = git::commit_detail(path, &root.id).unwrap();

    assert!(detail.parents.is_empty());

    let diff = git::commit_file_diff(path, &root.id, &detail.files[0]).unwrap();

    assert_eq!(diff.counts(), (2, 0), "every line of a root commit is added");
}

#[test]
fn asking_for_a_file_a_commit_did_not_touch_says_so() {
    let dir = empty_repo();
    let path = dir.path();

    std::fs::write(path.join("touched.txt"), "one\n").unwrap();
    std::fs::write(path.join("untouched.txt"), "one\n").unwrap();
    git_cli(path, &["add", "."]);
    git_cli(path, &["commit", "--quiet", "-m", "base"]);

    std::fs::write(path.join("touched.txt"), "two\n").unwrap();
    git_cli(path, &["commit", "--quiet", "-am", "change"]);

    let history = git::history(path).unwrap();
    let tip = history.commits.first().unwrap();

    let absent = git::ChangedFile {
        path: std::path::PathBuf::from("untouched.txt"),
        old_path: None,
        change: git::Change::Modified,
        added: 0,
        removed: 0,
    };

    let error = git::commit_file_diff(path, &tip.id, &absent).unwrap_err();
    assert!(
        error.to_string().contains("was not changed by"),
        "unexpected message: {error}"
    );
}

/// Reads a settings file the way gitDruid would, from text.
fn flow_from(text: &str) -> git_druid::settings::Flow {
    use git_druid::settings::{Layer, Settings};

    Settings {
        global: Layer::parse(text),
        repo: Layer::default(),
    }
    .flow()
}

#[test]
fn a_git_flow_feature_branches_from_develop_and_merges_back_into_it() {
    use git_druid::settings::Kind;

    let dir = empty_repo();
    let path = dir.path();

    commit(path, "base");
    git_cli(path, &["branch", "develop"]);

    // main moves on, so branching from the wrong one would be visible.
    commit(path, "on-main");

    let flow = flow_from("[flow]\n enabled = true\n main = main\n develop = develop\n");

    let name = flow.branch_name(Kind::Feature, "login");
    assert_eq!(name, "feature/login");

    git::create_branch(path, &name, Some(flow.start_point(Kind::Feature))).unwrap();
    git::checkout_branch(path, &name).unwrap();
    commit(path, "the-feature");

    // It really came off develop: main's commit is not reachable from here.
    assert!(
        !git_cli(path, &["log", "--oneline", &name]).contains("on-main"),
        "a feature should branch from develop, not main"
    );

    let summary = git::finish_branch(path, &name, flow.merges_into(Kind::Feature)).unwrap();
    assert!(summary.contains("on develop"), "unexpected: {summary}");

    // Finishing left us on develop, with the feature merged in.
    assert_eq!(git_cli(path, &["branch", "--show-current"]).trim(), "develop");
    assert!(git_cli(path, &["log", "--oneline", "develop"]).contains("the-feature"));

    // And main is untouched — that is what release branches are for.
    assert!(!git_cli(path, &["log", "--oneline", "main"]).contains("the-feature"));
}

#[test]
fn a_git_flow_hotfix_branches_from_main_and_merges_back_into_it() {
    use git_druid::settings::Kind;

    let dir = empty_repo();
    let path = dir.path();

    commit(path, "base");
    git_cli(path, &["branch", "develop"]);
    git_cli(path, &["checkout", "--quiet", "develop"]);
    commit(path, "on-develop");
    git_cli(path, &["checkout", "--quiet", "main"]);

    let flow = flow_from("[flow]\n enabled = true\n main = main\n develop = develop\n");

    assert_eq!(flow.start_point(Kind::Hotfix), "main");

    let name = flow.branch_name(Kind::Hotfix, "crash");
    assert_eq!(name, "hotfix/crash");

    git::create_branch(path, &name, Some(flow.start_point(Kind::Hotfix))).unwrap();
    git::checkout_branch(path, &name).unwrap();
    commit(path, "the-fix");

    // A hotfix must not carry unreleased work along with it.
    assert!(
        !git_cli(path, &["log", "--oneline", &name]).contains("on-develop"),
        "a hotfix should branch from main, not develop"
    );

    git::finish_branch(path, &name, flow.merges_into(Kind::Hotfix)).unwrap();

    assert_eq!(git_cli(path, &["branch", "--show-current"]).trim(), "main");
    assert!(git_cli(path, &["log", "--oneline", "main"]).contains("the-fix"));
}

#[test]
fn without_git_flow_a_branch_gets_no_prefix_and_comes_off_main() {
    use git_druid::settings::Kind;

    let dir = empty_repo();
    let path = dir.path();

    commit(path, "base");

    let flow = flow_from("[flow]\n main = main\n");

    let name = flow.branch_name(Kind::Feature, "login");
    assert_eq!(name, "login", "prefixes belong to git-flow");

    git::create_branch(path, &name, Some(flow.start_point(Kind::Feature))).unwrap();
    git::checkout_branch(path, "login").unwrap();
    commit(path, "the-work");

    git::finish_branch(path, "login", flow.merges_into(Kind::Feature)).unwrap();

    assert_eq!(git_cli(path, &["branch", "--show-current"]).trim(), "main");
    assert!(git_cli(path, &["log", "--oneline", "main"]).contains("the-work"));
}

#[test]
fn finishing_into_a_branch_that_does_not_exist_says_which_one() {
    let dir = empty_repo();
    let path = dir.path();

    commit(path, "base");
    git_cli(path, &["checkout", "--quiet", "-b", "feature/login"]);
    commit(path, "the-work");

    // git-flow is configured, but nobody ever made the develop branch.
    let error = git::finish_branch(path, "feature/login", "develop").unwrap_err();

    assert!(
        error.to_string().contains("develop") && error.to_string().contains("Settings"),
        "the message should name the branch and where to fix it: {error}"
    );

    // And it did not move us off the branch before failing.
    assert_eq!(
        git_cli(path, &["branch", "--show-current"]).trim(),
        "feature/login"
    );
}

#[test]
fn finishing_a_branch_into_itself_is_refused() {
    let dir = empty_repo();
    let path = dir.path();

    commit(path, "base");

    let error = git::finish_branch(path, "main", "main").unwrap_err();
    assert!(
        error.to_string().contains("nothing to finish"),
        "unexpected: {error}"
    );
}

#[test]
fn a_repository_file_overrides_the_workflow_for_that_repository_only() {
    use git_druid::settings::{Layer, Scope, Settings, keys};

    let dir = empty_repo();
    let path = dir.path();

    let mut settings = Settings {
        global: Layer::parse("[flow]\n enabled = true\n develop = develop\n"),
        repo: Layer::default(),
    };

    // This one checkout branches off a differently named line.
    settings
        .layer_mut(Scope::Repo)
        .set(keys::FLOW_DEVELOP, "integration");

    let written = git_druid::settings::save(&settings, Scope::Repo, Some(path)).unwrap();
    assert_eq!(written, path.join(".gitdruid"));

    // Read back the way the app does when it opens a repository.
    let reloaded = git_druid::settings::Settings {
        global: settings.global.clone(),
        repo: git_druid::settings::load(Some(path)).repo,
    };

    let flow = reloaded.flow();
    assert!(flow.enabled, "the global file still supplies this");
    assert_eq!(flow.develop, "integration", "the repository overrides this");

    // And the file is the plain text it claims to be.
    let text = std::fs::read_to_string(path.join(".gitdruid")).unwrap();
    assert!(text.contains("[flow]"), "unexpected file:\n{text}");
    assert!(text.contains("develop = integration"), "unexpected file:\n{text}");
}
