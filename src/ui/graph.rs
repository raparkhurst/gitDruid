//! The commit graph.
//!
//! Each row is a normal button — so selection, hover and scrolling behave like
//! the rest of the app — with a small canvas at its left edge drawing the lane
//! lines that cross it. The layout those lines follow is computed in
//! [`crate::git::history`]; nothing here decides where a commit sits.

use iced::mouse::Cursor;
use iced::widget::canvas::{self, Frame, Geometry, Path, Stroke};
use iced::widget::{
    Column, button, canvas as canvas_widget, container, mouse_area, responsive, row, scrollable,
    text,
};
use iced::{Center, Element, Fill, Font, Point, Rectangle, Renderer, Theme};

use crate::app::{Focus, Message, Repo, Target};
use crate::git::{Badge, BadgeKind, Commit, Edge, Row};
use crate::ui::style;

/// Height of one commit row. The gutter canvas is drawn to exactly this, so
/// the lines meet across the boundary between one row and the next.
const ROW_HEIGHT: f32 = 26.0;

/// Horizontal distance between lanes.
const LANE_STEP: f32 = 14.0;

/// Padding either side of the lanes inside the gutter.
const LANE_INSET: f32 = 10.0;

const NODE_RADIUS: f32 = 4.0;

/// Widths the summary has to share the row with: the author and date columns,
/// the short id, and the padding and spacing between them all.
const AUTHOR_WIDTH: f32 = 110.0;
const DATE_WIDTH: f32 = 112.0;
const ID_WIDTH: f32 = 48.0;
const ROW_CHROME: f32 = 16.0 + 8.0 * 4.0;

const SUMMARY_SIZE: f32 = 13.0;

/// Width of one character at [`SUMMARY_SIZE`], as a fraction of it.
///
/// The window is monospaced throughout, so every glyph is the same width and
/// this is the real advance rather than an average — 0.6em is what the usual
/// terminal faces use. `Wrapping::None` still clips anything that overruns,
/// which covers a face that turns out to be a little wider.
const CHARACTER_WIDTH: f32 = 0.6;

/// Never elide below this, however narrow the pane gets.
const MIN_SUMMARY: usize = 12;

pub fn view(repo: &Repo) -> Element<'_, Message> {
    if repo.loading_history && repo.history.commits.is_empty() {
        return hint("Reading history…");
    }

    if repo.history.commits.is_empty() {
        return hint("No commits yet. Stage something and write the first one.");
    }

    // The width the summaries have to fit in is only known once the pane has
    // been laid out, so the list is built from it rather than from a guess
    // that would be wrong at every window size but one.
    container(responsive(move |size| list(repo, size.width)))
        .width(Fill)
        .height(Fill)
        .style(style::canvas)
        .into()
}

fn list(repo: &Repo, width: f32) -> Element<'_, Message> {
    // No spacing between rows: each row's gutter is drawn to its own edges,
    // so a gap between them breaks every lane line into dashes.
    let mut rows = Column::new().padding([6, 8]);

    for commit in &repo.history.commits {
        rows = rows.push(commit_row(repo, commit, width));
    }

    if repo.history.truncated {
        rows = rows.push(
            container(
                text(format!(
                    "Showing the most recent {} commits.",
                    repo.history.commits.len()
                ))
                .size(11)
                .style(muted),
            )
            .padding([8, 10]),
        );
    }

    scrollable(rows).width(Fill).height(Fill).into()
}

fn commit_row<'a>(repo: &'a Repo, commit: &'a Commit, width: f32) -> Element<'a, Message> {
    let selected = repo.focus == Focus::Commit
        && repo
            .detail
            .as_ref()
            .is_some_and(|detail| detail.id == commit.id);

    let gutter = canvas_widget(Gutter {
        row: commit.row.clone(),
    })
    .width(gutter_width(repo.history.lanes))
    .height(ROW_HEIGHT);

    let mut line = row![gutter].spacing(8).align_y(Center);

    for badge in &commit.badges {
        line = line.push(badge_chip(badge));
    }

    line = line
        .push(
            text(summary(commit, width, repo.history.lanes))
                .size(SUMMARY_SIZE)
                .width(Fill)
                .wrapping(text::Wrapping::None),
        )
        .push(
            text(&commit.author)
                .size(11)
                .style(muted)
                .width(110)
                .wrapping(text::Wrapping::None),
        )
        .push(text(&commit.when).size(11).style(muted).width(112))
        .push(
            text(&commit.short_id)
                .size(11)
                .font(Font::MONOSPACE)
                .style(muted),
        );

    mouse_area(
        button(line)
            .padding([0, 8])
            .width(Fill)
            .height(ROW_HEIGHT)
            .style(style::row(selected))
            .on_press(Message::SelectCommit(commit.id.clone())),
    )
    .on_right_press(Message::OpenMenu(Target::Commit(commit.id.clone())))
    .into()
}

/// The summary as it should read in a row that is `width` wide.
///
/// A summary that does not fit is cut with an ellipsis rather than wrapped: a
/// row is one line high, so wrapping only hides the overflow. The ellipsis
/// doubles as the affordance — clicking the row opens the whole message, which
/// is also where a commit with a body says so.
fn summary(commit: &Commit, width: f32, lanes: usize) -> String {
    let badges: f32 = commit
        .badges
        .iter()
        .map(|badge| badge_width(&badge.name))
        .sum();

    let available =
        width - gutter_width(lanes) - badges - AUTHOR_WIDTH - DATE_WIDTH - ID_WIDTH - ROW_CHROME;

    let budget = (available / (SUMMARY_SIZE * CHARACTER_WIDTH)).max(0.0) as usize;
    let budget = budget.max(MIN_SUMMARY);

    let characters = commit.summary.chars().count();

    if characters <= budget {
        // The line fits, but the message may still go on past it.
        return match commit.has_body {
            true => format!("{} …", commit.summary),
            false => commit.summary.clone(),
        };
    }

    // Cut on a word boundary where there is one close to the limit, so the
    // ellipsis follows a whole word rather than half of one.
    let cut: String = commit.summary.chars().take(budget.saturating_sub(1)).collect();

    let trimmed = match cut.rsplit_once(char::is_whitespace) {
        Some((head, _)) if head.chars().count() + 8 >= budget => head,
        _ => cut.trim_end(),
    };

    format!("{trimmed}…")
}

/// What a badge chip costs the summary, including the gap after it.
fn badge_width(name: &str) -> f32 {
    // Badge text is 10pt monospace in a chip padded by 5 either side and
    // outlined, plus the row's 8px spacing. Glyph icons on HEAD and tag badges
    // add two characters.
    name.chars().count() as f32 * 6.0 + 12.0 + 12.0 + 8.0
}

fn badge_chip(badge: &Badge) -> Element<'_, Message> {
    let label = match badge.kind {
        BadgeKind::Head => format!("⌂ {}", badge.name),
        BadgeKind::Tag => format!("⌖ {}", badge.name),
        _ => badge.name.clone(),
    };

    let kind = badge.kind;

    container(
        text(label)
            .size(10)
            .wrapping(text::Wrapping::None)
            .style(move |theme: &Theme| text::Style {
                color: Some(badge_color(theme, kind)),
            }),
    )
    .padding([1, 5])
    .style(move |theme: &Theme| style::badge(badge_color(theme, kind))(theme))
    .into()
}

fn badge_color(theme: &Theme, kind: BadgeKind) -> iced::Color {
    let palette = theme.extended_palette();

    match kind {
        BadgeKind::Head => palette.success.base.color,
        BadgeKind::LocalBranch => palette.primary.base.color,
        BadgeKind::RemoteBranch => palette.secondary.base.color,
        BadgeKind::Tag => palette.warning.base.color,
    }
}

fn hint(message: &str) -> Element<'_, Message> {
    container(text(message.to_owned()).size(13).style(muted))
        .center_x(Fill)
        .center_y(Fill)
        .style(style::canvas)
        .into()
}

fn muted(theme: &Theme) -> text::Style {
    text::Style {
        color: Some(style::muted(theme)),
    }
}

fn gutter_width(lanes: usize) -> f32 {
    LANE_INSET * 2.0 + lanes.max(1).saturating_sub(1) as f32 * LANE_STEP
}

fn lane_x(lane: usize) -> f32 {
    LANE_INSET + lane as f32 * LANE_STEP
}

/// Draws the lane lines crossing one row, and the commit's node.
///
/// The gutter's width is set on the widget from the graph's widest row, so
/// every row shares one origin and the lines line up down the column.
struct Gutter {
    row: Row,
}

impl<Message> canvas::Program<Message> for Gutter {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());

        let height = bounds.height;
        let middle = height / 2.0;

        let node_x = lane_x(self.row.node);

        for edge in &self.row.edges {
            // A line into the commit runs from the top edge to the node; one
            // out of it runs from the node to the bottom edge; one passing by
            // crosses the whole row.
            let (from, to) = match *edge {
                Edge::Through { lane, .. } => (
                    Point::new(lane_x(lane), 0.0),
                    Point::new(lane_x(lane), height),
                ),
                Edge::Into { from, .. } => (
                    Point::new(lane_x(from), 0.0),
                    Point::new(node_x, middle),
                ),
                Edge::Out { to, .. } => (
                    Point::new(node_x, middle),
                    Point::new(lane_x(to), height),
                ),
            };

            let path = if (from.x - to.x).abs() < f32::EPSILON {
                Path::line(from, to)
            } else {
                // A curve rather than a diagonal: where several lines converge
                // on one node, straight diagonals overlap into a wedge that is
                // hard to follow, and curves stay separable.
                let bend = (from.y + to.y) / 2.0;

                Path::new(|builder| {
                    builder.move_to(from);
                    builder.bezier_curve_to(
                        Point::new(from.x, bend),
                        Point::new(to.x, bend),
                        to,
                    );
                })
            };

            frame.stroke(
                &path,
                Stroke::default()
                    .with_color(style::lane(theme, edge.color()))
                    .with_width(1.6),
            );
        }

        let centre = Point::new(node_x, middle);

        // The ring punches the lines out from under the node so the node reads
        // as sitting on the lane rather than being crossed by it.
        frame.fill(
            &Path::circle(centre, NODE_RADIUS + 1.5),
            style::graph_background(theme),
        );
        frame.fill(
            &Path::circle(centre, NODE_RADIUS),
            style::lane(theme, self.row.color),
        );

        vec![frame.into_geometry()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::Row;

    /// A commit with `summary`, no badges, and nothing below the summary line.
    fn commit(summary: &str, has_body: bool) -> Commit {
        Commit {
            id: "0".repeat(40),
            short_id: "0000000".to_owned(),
            summary: summary.to_owned(),
            has_body,
            author: "Ada".to_owned(),
            when: "2026-01-01 00:00".to_owned(),
            badges: Vec::new(),
            row: Row {
                node: 0,
                color: 0,
                edges: Vec::new(),
                lanes: 1,
            },
        }
    }

    /// Wide enough that a normal summary has room to spare.
    const WIDE: f32 = 900.0;

    #[test]
    fn a_summary_that_fits_is_left_alone() {
        let commit = commit("Add a lexer", false);

        assert_eq!(summary(&commit, WIDE, 1), "Add a lexer");
    }

    #[test]
    fn a_body_is_advertised_even_when_the_summary_fits() {
        let commit = commit("Add a lexer", true);

        assert_eq!(
            summary(&commit, WIDE, 1),
            "Add a lexer …",
            "the ellipsis is the only sign there is more to read"
        );
    }

    #[test]
    fn a_long_summary_is_cut_rather_than_wrapped() {
        let long = "Rework the staging path so a hunk splices into the index blob \
                    instead of round-tripping through a temporary file on disk";
        let commit = commit(long, false);

        let shown = summary(&commit, WIDE, 1);

        assert!(shown.ends_with('…'), "should be elided: {shown}");
        assert!(
            shown.chars().count() < long.chars().count(),
            "should be shorter than the original: {shown}"
        );
        assert!(
            long.starts_with(shown.trim_end_matches(['…', ' '])),
            "the kept part should be a prefix of the summary: {shown}"
        );
    }

    #[test]
    fn eliding_cuts_on_a_word_boundary() {
        let commit = commit(
            "Rework the staging path so a hunk splices into the index blob directly",
            false,
        );

        let shown = summary(&commit, 520.0, 1);
        let kept = shown.trim_end_matches('…');

        assert!(shown.ends_with('…'), "should be elided: {shown}");
        assert!(
            !kept.ends_with(char::is_whitespace),
            "no trailing space before the ellipsis: {shown:?}"
        );
        assert!(
            kept.split_whitespace().count() > 1,
            "should keep several words: {shown:?}"
        );
    }

    #[test]
    fn a_narrow_pane_still_shows_something() {
        let commit = commit("Rework the staging path entirely", false);

        // Narrower than the fixed columns alone, so the budget would go
        // negative without a floor.
        let shown = summary(&commit, 50.0, 6);
        let kept = shown.trim_end_matches('…');

        assert!(shown.ends_with('…'), "should be elided: {shown:?}");
        assert!(
            !kept.is_empty() && kept.split_whitespace().count() >= 1,
            "should still show a word rather than a bare ellipsis: {shown:?}"
        );
        assert!(
            commit.summary.starts_with(kept),
            "and it should still be the start of the real summary: {shown:?}"
        );
    }

    #[test]
    fn badges_take_room_away_from_the_summary() {
        let plain = commit("Merge branch 'feature/parser' into the mainline now", false);

        let mut badged = plain.clone();
        badged.badges = vec![
            Badge {
                name: "feature/parser".to_owned(),
                kind: BadgeKind::LocalBranch,
            },
            Badge {
                name: "v1.0.0".to_owned(),
                kind: BadgeKind::Tag,
            },
        ];

        let narrow = 560.0;

        assert!(
            summary(&badged, narrow, 1).chars().count()
                < summary(&plain, narrow, 1).chars().count(),
            "badges push the ellipsis earlier"
        );
    }
}
