//! Reading and resolving a conflicted file.
//!
//! A conflict lives in two places at once. The index holds up to three stages
//! of the file — the common ancestor, ours, and theirs — and the working tree
//! holds one file with markers in it. Resolving means agreeing on one content
//! and putting it in both: the working tree gets the text, and the index gets
//! it at stage zero, which is what makes the conflict go away.
//!
//! The regions come from parsing the markers rather than from re-merging the
//! stages, because the file on disk is what the user sees and may already have
//! edited by hand. Re-merging would quietly throw that away.

use std::path::{Path, PathBuf};

use git2::Repository;

use super::{Error, Result};

/// Which side of a conflicted region to keep.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Ours,
    Theirs,
    /// Both, ours first — the answer more often than either alone when two
    /// people added different things in the same place.
    Both,
}

/// A run of lines in a conflicted file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Region {
    /// Text both sides agree on.
    Common(Vec<String>),
    /// Text they do not.
    Split {
        ours: Vec<String>,
        theirs: Vec<String>,
        /// The common ancestor's version, when the file was written in diff3
        /// style. Empty otherwise, which is the default.
        base: Vec<String>,
        /// What the markers called each side, usually a branch name.
        ours_label: String,
        theirs_label: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    pub path: PathBuf,
    pub regions: Vec<Region>,
    /// True when the file is not text, and so cannot be resolved a region at a
    /// time — one whole side has to be chosen.
    pub binary: bool,
}

impl Conflict {
    /// How many regions are still to be decided.
    pub fn unresolved(&self) -> usize {
        self.regions
            .iter()
            .filter(|region| matches!(region, Region::Split { .. }))
            .count()
    }

    /// True when nothing is left with markers around it.
    pub fn is_settled(&self) -> bool {
        !self.binary && self.unresolved() == 0
    }
}

const OURS: &str = "<<<<<<<";
const BASE: &str = "|||||||";
const SPLIT: &str = "=======";
const THEIRS: &str = ">>>>>>>";

/// Reads a conflicted file as it currently stands on disk.
pub fn conflict(repo_path: &Path, path: &Path) -> Result<Conflict> {
    let repo = super::open(repo_path)?;
    let workdir = workdir(&repo)?;

    let bytes = std::fs::read(workdir.join(path)).map_err(|error| {
        Error::new(format!("{}: {error}", path.display()))
    })?;

    let Ok(text) = String::from_utf8(bytes) else {
        return Ok(Conflict {
            path: path.to_path_buf(),
            regions: Vec::new(),
            binary: true,
        });
    };

    Ok(Conflict {
        path: path.to_path_buf(),
        regions: parse(&text),
        binary: false,
    })
}

/// Splits marked-up text into regions.
///
/// Anything that is not a well-formed conflict is common text: a file with a
/// stray `<<<<<<<` in it is not a reason to refuse to show the file.
fn parse(text: &str) -> Vec<Region> {
    let mut regions = Vec::new();
    let mut common: Vec<String> = Vec::new();

    let mut lines = text.lines().peekable();

    while let Some(line) = lines.next() {
        if !line.starts_with(OURS) {
            common.push(line.to_owned());
            continue;
        }

        let ours_label = label(line, OURS);

        let mut ours = Vec::new();
        let mut base = Vec::new();
        let mut theirs = Vec::new();
        let mut theirs_label = String::new();

        let mut section = Section::Ours;
        let mut closed = false;

        for line in lines.by_ref() {
            if line.starts_with(BASE) {
                section = Section::Base;
                continue;
            }

            if line.starts_with(SPLIT) {
                section = Section::Theirs;
                continue;
            }

            if line.starts_with(THEIRS) {
                theirs_label = label(line, THEIRS);
                closed = true;
                break;
            }

            match section {
                Section::Ours => ours.push(line.to_owned()),
                Section::Base => base.push(line.to_owned()),
                Section::Theirs => theirs.push(line.to_owned()),
            }
        }

        if !closed {
            // An opening marker with no closing one is just text; putting it
            // back keeps the file readable rather than swallowing the rest.
            common.push(line.to_owned());
            common.extend(ours);
            common.extend(base);
            common.extend(theirs);
            continue;
        }

        if !common.is_empty() {
            regions.push(Region::Common(std::mem::take(&mut common)));
        }

        regions.push(Region::Split {
            ours,
            theirs,
            base,
            ours_label,
            theirs_label,
        });
    }

    if !common.is_empty() {
        regions.push(Region::Common(common));
    }

    regions
}

enum Section {
    Ours,
    Base,
    Theirs,
}

fn label(line: &str, marker: &str) -> String {
    line.trim_start_matches(marker).trim().to_owned()
}

/// Settles one region, leaving the rest as they are.
///
/// `index` counts conflicted regions only, so it matches what the UI numbers.
pub fn resolve(repo_path: &Path, path: &Path, index: usize, side: Side) -> Result<String> {
    let repo = super::open(repo_path)?;
    let workdir = workdir(&repo)?;

    let file = workdir.join(path);

    let text = std::fs::read_to_string(&file)
        .map_err(|error| Error::new(format!("{}: {error}", path.display())))?;

    let regions = parse(&text);

    let mut seen = 0;
    let mut found = false;
    let mut out: Vec<String> = Vec::new();

    for region in regions {
        match region {
            Region::Common(lines) => out.extend(lines),
            Region::Split { ours, theirs, .. } => {
                if seen != index {
                    // Left as it was, markers and all.
                    out.extend(rebuild(&ours, &theirs));
                    seen += 1;
                    continue;
                }

                found = true;
                seen += 1;

                match side {
                    Side::Ours => out.extend(ours),
                    Side::Theirs => out.extend(theirs),
                    Side::Both => {
                        out.extend(ours);
                        out.extend(theirs);
                    }
                }
            }
        }
    }

    if !found {
        return Err(Error::new(
            "that conflict is not there any more — the file changed underneath",
        ));
    }

    write_lines(&file, &out, text.ends_with('\n'))?;

    Ok(match side {
        Side::Ours => "Kept ours".to_owned(),
        Side::Theirs => "Took theirs".to_owned(),
        Side::Both => "Kept both".to_owned(),
    })
}

/// Puts a region back the way it was, for the ones not being resolved.
fn rebuild(ours: &[String], theirs: &[String]) -> Vec<String> {
    let mut lines = vec![OURS.to_owned()];

    lines.extend(ours.iter().cloned());
    lines.push(SPLIT.to_owned());
    lines.extend(theirs.iter().cloned());
    lines.push(THEIRS.to_owned());

    lines
}

/// Takes one whole side of a conflicted file, from the index rather than from
/// the markers: the stages are what git recorded, before anyone edited them.
pub fn take(repo_path: &Path, path: &Path, side: Side) -> Result<String> {
    let repo = super::open(repo_path)?;
    let workdir = workdir(&repo)?;

    let mut index = repo.index()?;

    let entry = index
        .conflicts()?
        .flatten()
        .find(|conflict| {
            let stage = conflict.our.as_ref().or(conflict.their.as_ref());

            stage.is_some_and(|entry| {
                String::from_utf8_lossy(&entry.path).as_ref() == path.to_string_lossy()
            })
        })
        .ok_or_else(|| Error::new(format!("{} is not conflicted", path.display())))?;

    let chosen = match side {
        Side::Ours => entry.our,
        Side::Theirs => entry.their,
        Side::Both => {
            return Err(Error::new(
                "a whole file cannot be both — resolve the regions one at a time",
            ));
        }
    };

    let file = workdir.join(path);

    match chosen {
        Some(stage) => {
            let blob = repo.find_blob(stage.id)?;

            std::fs::write(&file, blob.content())
                .map_err(|error| Error::new(format!("{}: {error}", path.display())))?;

            index.add_path(path)?;
        }
        None => {
            // The side being taken deleted the file, so taking it means the
            // file goes.
            let _ = std::fs::remove_file(&file);

            index.remove_path(path)?;
        }
    }

    index.write()?;

    Ok(match side {
        Side::Ours => format!("Kept our {}", path.display()),
        Side::Theirs => format!("Took their {}", path.display()),
        Side::Both => unreachable!("rejected above"),
    })
}

/// Stages a file whose conflicts have been settled.
pub fn mark_resolved(repo_path: &Path, path: &Path) -> Result<String> {
    let repo = super::open(repo_path)?;
    let workdir = workdir(&repo)?;

    let file = workdir.join(path);

    // Staging a file that still has markers in it would commit them, and the
    // markers are exactly the thing nobody notices until it is pushed.
    if let Ok(text) = std::fs::read_to_string(&file)
        && text.lines().any(|line| {
            line.starts_with(OURS) || line.starts_with(SPLIT) || line.starts_with(THEIRS)
        })
    {
        return Err(Error::new(format!(
            "{} still has conflict markers in it",
            path.display()
        )));
    }

    let mut index = repo.index()?;

    if file.exists() {
        index.add_path(path)?;
    } else {
        index.remove_path(path)?;
    }

    index.write()?;

    Ok(format!("Resolved {}", path.display()))
}

fn write_lines(file: &Path, lines: &[String], trailing_newline: bool) -> Result<()> {
    let mut text = lines.join("\n");

    if trailing_newline && !text.is_empty() {
        text.push('\n');
    }

    std::fs::write(file, text).map_err(|error| Error::new(format!("{}: {error}", file.display())))
}

fn workdir(repo: &Repository) -> Result<PathBuf> {
    repo.workdir()
        .map(Path::to_path_buf)
        .ok_or_else(|| Error::new("this repository has no working tree"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MARKED: &str = "\
keep me
<<<<<<< HEAD
ours
=======
theirs
>>>>>>> other
and me
";

    #[test]
    fn a_conflicted_file_reads_as_common_text_around_the_disputed_part() {
        let regions = parse(MARKED);

        assert_eq!(
            regions,
            vec![
                Region::Common(vec!["keep me".to_owned()]),
                Region::Split {
                    ours: vec!["ours".to_owned()],
                    theirs: vec!["theirs".to_owned()],
                    base: Vec::new(),
                    ours_label: "HEAD".to_owned(),
                    theirs_label: "other".to_owned(),
                },
                Region::Common(vec!["and me".to_owned()]),
            ]
        );
    }

    #[test]
    fn diff3_style_carries_the_ancestor_too() {
        let regions = parse(
            "<<<<<<< HEAD\nours\n||||||| base\nwas\n=======\ntheirs\n>>>>>>> other\n",
        );

        let Region::Split { base, ours, theirs, .. } = &regions[0] else {
            panic!("expected a split: {regions:?}");
        };

        assert_eq!(ours, &["ours"]);
        assert_eq!(base, &["was"], "the ancestor is worth showing when it is there");
        assert_eq!(theirs, &["theirs"]);
    }

    #[test]
    fn an_unclosed_marker_is_just_text() {
        // A file can contain the characters without being conflicted, and
        // swallowing the rest of it would be worse than showing them.
        let regions = parse("before\n<<<<<<< not really\nafter\n");

        assert_eq!(
            regions,
            vec![Region::Common(vec![
                "before".to_owned(),
                "<<<<<<< not really".to_owned(),
                "after".to_owned(),
            ])]
        );
    }

    #[test]
    fn a_file_with_no_markers_is_one_common_region() {
        let regions = parse("one\ntwo\n");

        assert_eq!(regions, vec![Region::Common(vec!["one".to_owned(), "two".to_owned()])]);
    }

    #[test]
    fn several_conflicts_are_counted_separately() {
        let text = "<<<<<<< a\n1\n=======\n2\n>>>>>>> b\nmiddle\n<<<<<<< a\n3\n=======\n4\n>>>>>>> b\n";

        let conflict = Conflict {
            path: PathBuf::from("f"),
            regions: parse(text),
            binary: false,
        };

        assert_eq!(conflict.unresolved(), 2);
        assert!(!conflict.is_settled());
    }

    #[test]
    fn a_settled_file_says_so() {
        let conflict = Conflict {
            path: PathBuf::from("f"),
            regions: parse("all agreed\n"),
            binary: false,
        };

        assert_eq!(conflict.unresolved(), 0);
        assert!(conflict.is_settled());
    }

    #[test]
    fn a_binary_conflict_is_never_settled_by_regions() {
        let conflict = Conflict {
            path: PathBuf::from("f"),
            regions: Vec::new(),
            binary: true,
        };

        assert!(
            !conflict.is_settled(),
            "a binary file has to have a whole side chosen"
        );
    }
}
