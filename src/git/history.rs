//! The commit graph: which commits exist, and how to draw the lines between
//! them.
//!
//! The walk covers every ref, not just HEAD, so a branch you are not on still
//! shows up. Layout happens here rather than in the UI because the lane a
//! commit sits in depends on every commit above it — it is a property of the
//! history, not of the widget drawing it.

use std::collections::HashMap;
use std::path::Path;

use git2::{Oid, Repository, Sort};

use super::Result;

/// How many commits to read. A graph long enough to scroll for a minute is
/// already more than anyone reads; walking a kernel-sized history to draw it
/// is not.
pub const LIMIT: usize = 500;

/// A ref that points at a particular commit, drawn as a badge beside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Badge {
    pub name: String,
    pub kind: BadgeKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BadgeKind {
    /// The branch HEAD is on.
    Head,
    LocalBranch,
    RemoteBranch,
    Tag,
}

/// One line drawn in a row.
///
/// The three cases are genuinely different shapes, not one shape with
/// different endpoints: a line that reaches the commit stops at it, and a line
/// that leaves the commit starts at it. Collapsing them into a single
/// `from → to` pair draws a root commit's line straight through the bottom of
/// its own row, into nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    /// A line for another branch, passing the row untouched.
    Through { lane: usize, color: usize },
    /// A line arriving from the row above and ending at this commit.
    Into { from: usize, color: usize },
    /// A line leaving this commit for one of its parents.
    Out { to: usize, color: usize },
}

impl Edge {
    /// The lane the line occupies at the top edge of the row, if any.
    pub fn top(self) -> Option<usize> {
        match self {
            Edge::Through { lane, .. } => Some(lane),
            Edge::Into { from, .. } => Some(from),
            Edge::Out { .. } => None,
        }
    }

    /// The lane the line occupies at the bottom edge of the row, if any.
    pub fn bottom(self) -> Option<usize> {
        match self {
            Edge::Through { lane, .. } => Some(lane),
            Edge::Into { .. } => None,
            Edge::Out { to, .. } => Some(to),
        }
    }

    pub fn color(self) -> usize {
        match self {
            Edge::Through { color, .. } | Edge::Into { color, .. } | Edge::Out { color, .. } => {
                color
            }
        }
    }
}

/// Everything the graph gutter needs to draw one row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// The lane the commit's node sits in.
    pub node: usize,
    pub color: usize,
    pub edges: Vec<Edge>,
    /// One past the highest lane this row touches, for sizing the gutter.
    pub lanes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    pub id: String,
    /// The abbreviated id, which is what gets shown.
    pub short_id: String,
    pub summary: String,
    /// True when the message carries more than its summary line, so a row can
    /// say there is something to open even when the summary itself fits.
    pub has_body: bool,
    pub author: String,
    /// Author time, already formatted — the UI does no date arithmetic.
    pub when: String,
    pub badges: Vec<Badge>,
    pub row: Row,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct History {
    pub commits: Vec<Commit>,
    /// True when the walk stopped at [`LIMIT`] rather than at the root.
    pub truncated: bool,
    /// The widest row, so every gutter in the list is drawn the same width.
    pub lanes: usize,
}

impl History {
    pub fn empty() -> Self {
        Self {
            commits: Vec::new(),
            truncated: false,
            lanes: 0,
        }
    }

    pub fn find(&self, id: &str) -> Option<&Commit> {
        self.commits.iter().find(|commit| commit.id == id)
    }
}

/// Reads the graph of every ref in the repository.
pub fn history(repo_path: &Path) -> Result<History> {
    let repo = super::open(repo_path)?;

    let badges = collect_badges(&repo);

    let mut walk = repo.revwalk()?;
    // Topological order keeps a branch's commits together instead of
    // interleaving them by date; the time sort breaks ties so the result still
    // reads newest-first.
    walk.set_sorting(Sort::TOPOLOGICAL | Sort::TIME)?;

    if walk.push_glob("refs/heads/*").is_err() {
        // An unborn branch has no refs to push, which is not an error: the
        // repository simply has no history yet.
    }

    let _ = walk.push_glob("refs/remotes/*");
    let _ = walk.push_glob("refs/tags/*");
    let _ = walk.push_head();

    let mut layout = Layout::default();
    let mut commits = Vec::new();
    let mut truncated = false;

    for (index, oid) in walk.enumerate() {
        if index >= LIMIT {
            truncated = true;
            break;
        }

        let oid = oid?;
        let commit = repo.find_commit(oid)?;

        let parents: Vec<Oid> = commit.parent_ids().collect();
        let row = layout.place(oid, &parents);

        commits.push(Commit {
            id: oid.to_string(),
            short_id: format!("{:.7}", oid),
            summary: commit
                .summary()
                .ok()
                .flatten()
                .unwrap_or("(no message)")
                .to_owned(),
            has_body: commit
                .body()
                .ok()
                .flatten()
                .is_some_and(|body| !body.trim().is_empty()),
            author: commit
                .author()
                .name()
                .unwrap_or("(unknown author)")
                .to_owned(),
            when: format_time(commit.time()),
            badges: badges.get(&oid).cloned().unwrap_or_default(),
            row,
        });
    }

    let lanes = commits.iter().map(|commit| commit.row.lanes).max().unwrap_or(0);

    Ok(History {
        commits,
        truncated,
        lanes,
    })
}

/// Assigns commits to lanes, one row at a time.
///
/// A lane is a vertical track holding the oid it is waiting to draw. When that
/// commit arrives the lane is freed and handed to the commit's first parent,
/// which is what makes a branch's line continue straight down the graph.
#[derive(Default)]
struct Layout {
    lanes: Vec<Option<Oid>>,
}

impl Layout {
    fn place(&mut self, oid: Oid, parents: &[Oid]) -> Row {
        // Snapshot the lanes before claiming one: a lane this commit starts is
        // not a line arriving from the row above, and counting it as one draws
        // a stray line above every branch tip.
        let before = self.lanes.clone();

        let incoming: Vec<usize> = self
            .lanes
            .iter()
            .enumerate()
            .filter(|(_, waiting)| **waiting == Some(oid))
            .map(|(lane, _)| lane)
            .collect();

        // A commit nothing is waiting for is the tip of a branch, so it starts
        // a lane of its own.
        let node = match incoming.first() {
            Some(lane) => *lane,
            None => self.claim(oid),
        };

        // Every lane waiting for this commit has now reached it; they all
        // converge on the node and only the node's lane carries on.
        for lane in &incoming {
            self.lanes[*lane] = None;
        }
        self.lanes[node] = None;

        let mut targets = Vec::new();

        for (position, parent) in parents.iter().enumerate() {
            // The first parent inherits the node's lane: the mainline of this
            // branch keeps going straight down.
            let lane = if position == 0 {
                self.lanes[node] = Some(*parent);
                node
            } else {
                match self.lanes.iter().position(|waiting| *waiting == Some(*parent)) {
                    Some(existing) => existing,
                    None => self.claim(*parent),
                }
            };

            targets.push(lane);
        }

        let mut edges = Vec::new();

        for (lane, waiting) in before.iter().enumerate() {
            let Some(waiting) = waiting else { continue };

            if *waiting == oid {
                edges.push(Edge::Into {
                    from: lane,
                    color: lane,
                });
            } else if self.lanes.get(lane) == Some(&Some(*waiting)) {
                edges.push(Edge::Through {
                    lane,
                    color: lane,
                });
            }
        }

        for lane in targets {
            edges.push(Edge::Out {
                to: lane,
                color: lane,
            });
        }

        let lanes = self
            .lanes
            .iter()
            .rposition(Option::is_some)
            .map(|last| last + 1)
            .unwrap_or(0)
            .max(before.iter().rposition(Option::is_some).map(|l| l + 1).unwrap_or(0))
            .max(node + 1);

        Row {
            node,
            color: node,
            edges,
            lanes,
        }
    }

    /// Takes the leftmost free lane, widening the graph only when none is free.
    fn claim(&mut self, oid: Oid) -> usize {
        match self.lanes.iter().position(Option::is_none) {
            Some(lane) => {
                self.lanes[lane] = Some(oid);
                lane
            }
            None => {
                self.lanes.push(Some(oid));
                self.lanes.len() - 1
            }
        }
    }
}

/// Maps every commit that a ref points at to the badges it should carry.
fn collect_badges(repo: &Repository) -> HashMap<Oid, Vec<Badge>> {
    let mut badges: HashMap<Oid, Vec<Badge>> = HashMap::new();

    let head = repo.head().ok();
    let head_name = head
        .as_ref()
        .filter(|reference| reference.is_branch())
        .and_then(|reference| reference.shorthand().ok())
        .map(str::to_owned);

    if let Ok(branches) = repo.branches(None) {
        for (branch, kind) in branches.flatten() {
            let Some(oid) = branch.get().target() else {
                continue;
            };

            let Ok(Some(name)) = branch.name() else {
                continue;
            };

            let kind = match kind {
                git2::BranchType::Remote => BadgeKind::RemoteBranch,
                git2::BranchType::Local if head_name.as_deref() == Some(name) => BadgeKind::Head,
                git2::BranchType::Local => BadgeKind::LocalBranch,
            };

            badges.entry(oid).or_default().push(Badge {
                name: name.to_owned(),
                kind,
            });
        }
    }

    if let Ok(names) = repo.tag_names(None) {
        for name in names.iter().flatten().flatten() {
            // An annotated tag points at a tag object, so peel it to reach the
            // commit the badge belongs beside.
            let Ok(object) = repo.revparse_single(&format!("refs/tags/{name}")) else {
                continue;
            };

            let Ok(commit) = object.peel_to_commit() else {
                continue;
            };

            badges.entry(commit.id()).or_default().push(Badge {
                name: name.to_owned(),
                kind: BadgeKind::Tag,
            });
        }
    }

    // HEAD first, then branches, then tags, so the badge that matters most
    // reads first.
    for list in badges.values_mut() {
        list.sort_by_key(|badge| match badge.kind {
            BadgeKind::Head => 0,
            BadgeKind::LocalBranch => 1,
            BadgeKind::RemoteBranch => 2,
            BadgeKind::Tag => 3,
        });
    }

    badges
}

/// Formats a git timestamp as `YYYY-MM-DD HH:MM` in the commit's own zone.
///
/// git stores the offset the author was at, so showing it in that zone matches
/// what `git log` prints without pulling in a date library.
pub(super) fn format_time(time: git2::Time) -> String {
    let offset = i64::from(time.offset_minutes()) * 60;
    let local = time.seconds() + offset;

    let days = local.div_euclid(86_400);
    let seconds = local.rem_euclid(86_400);

    let (year, month, day) = civil_from_days(days);

    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}",
        seconds / 3600,
        (seconds % 3600) / 60
    )
}

/// Days since the Unix epoch to a calendar date, by Howard Hinnant's
/// `civil_from_days`.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);

    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;

    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;

    let day = (day_of_year - (153 * shifted_month + 2) / 5 + 1) as u32;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    } as u32;

    (if month <= 2 { year + 1 } else { year }, month, day)
}
