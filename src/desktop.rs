//! Adding gitDruid to the desktop's applications menu.
//!
//! An AppImage does not integrate itself. The format deliberately writes
//! nothing outside the file it is, and the tools that do integrate it —
//! AppImageLauncher, appimaged — are packages someone has to install first,
//! which is exactly what is not allowed on the machines this matters for.
//!
//! So the application does it, when asked. Everything written lands under
//! `~/.local/share`, which every desktop reads without being told to, and the
//! launcher points at wherever this binary is actually running from: the
//! AppImage itself when there is one, and the executable otherwise.

use std::fmt;
use std::path::{Path, PathBuf};

/// The icon, carried in the binary so that a single file is genuinely a single
/// file. Seventeen kilobytes, against the eleven megabytes around them.
const ICONS: [(u32, &[u8]); 7] = [
    (16, include_bytes!("../packaging/icons/16.png")),
    (32, include_bytes!("../packaging/icons/32.png")),
    (48, include_bytes!("../packaging/icons/48.png")),
    (64, include_bytes!("../packaging/icons/64.png")),
    (128, include_bytes!("../packaging/icons/128.png")),
    (256, include_bytes!("../packaging/icons/256.png")),
    (512, include_bytes!("../packaging/icons/512.png")),
];

/// Must match the application id the window sets, or the desktop will not
/// connect the two and the dock shows a blank square.
const ID: &str = "gitdruid";

#[derive(Debug, Clone)]
pub struct Error(String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

type Result<T> = std::result::Result<T, Error>;

/// What the settings dialog needs to know to offer this.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Menu {
    /// What the launcher would run.
    pub target: PathBuf,
    /// True when this is an AppImage rather than a plain executable, which is
    /// worth saying: it is the case where nothing else will do it.
    pub packaged: bool,
    /// Where the launcher is, if one is already there.
    pub installed: Option<PathBuf>,
}

/// Whether adding a menu entry makes sense here.
///
/// Only on Linux: macOS has bundles and Windows has shortcuts, and neither is
/// a `.desktop` file in a well-known directory.
pub fn menu() -> Option<Menu> {
    if !cfg!(target_os = "linux") {
        return None;
    }

    let entry = applications().map(|dir| dir.join(format!("{ID}.desktop")));

    // The AppImage runtime sets this to the path of the file itself. The
    // executable inside is in a temporary mount that will not exist next time.
    let (target, packaged) = match std::env::var_os("APPIMAGE") {
        Some(path) => (PathBuf::from(path), true),
        None => (std::env::current_exe().ok()?, false),
    };

    Some(Menu {
        target,
        packaged,
        installed: entry.filter(|path| path.exists()),
    })
}

/// Writes the launcher and the icons, and returns where the launcher went.
pub fn install() -> Result<PathBuf> {
    let menu = menu().ok_or_else(|| Error("this is only a thing on Linux".to_owned()))?;

    let applications =
        applications().ok_or_else(|| Error("there is no HOME to install into".to_owned()))?;

    std::fs::create_dir_all(&applications).map_err(|error| wrap(&applications, error))?;

    let icons = data()
        .ok_or_else(|| Error("there is no HOME to install into".to_owned()))?
        .join("icons/hicolor");

    for (size, bytes) in ICONS {
        let dir = icons.join(format!("{size}x{size}/apps"));

        std::fs::create_dir_all(&dir).map_err(|error| wrap(&dir, error))?;

        let path = dir.join(format!("{ID}.png"));

        std::fs::write(&path, bytes).map_err(|error| wrap(&path, error))?;
    }

    let entry = applications.join(format!("{ID}.desktop"));

    std::fs::write(&entry, launcher(&menu.target)).map_err(|error| wrap(&entry, error))?;

    refresh(&applications);

    Ok(entry)
}

/// Takes the launcher and icons back out, leaving settings alone.
pub fn remove() -> Result<()> {
    let Some(applications) = applications() else {
        return Ok(());
    };

    let entry = applications.join(format!("{ID}.desktop"));

    if entry.exists() {
        std::fs::remove_file(&entry).map_err(|error| wrap(&entry, error))?;
    }

    if let Some(data) = data() {
        for (size, _) in ICONS {
            let path = data.join(format!("icons/hicolor/{size}x{size}/apps/{ID}.png"));

            let _ = std::fs::remove_file(path);
        }
    }

    refresh(&applications);

    Ok(())
}

fn launcher(target: &Path) -> String {
    // Exec is a quoted string when it has to be: a path with a space in it,
    // unquoted, is read as a command and an argument.
    let exec = target.display().to_string();

    let exec = match exec.contains(' ') {
        true => format!("\"{exec}\""),
        false => exec,
    };

    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=gitDruid\n\
         GenericName=Git Client\n\
         Comment=See what changed, stage it a hunk at a time, and write the message\n\
         Exec={exec} %f\n\
         Icon={ID}\n\
         Terminal=false\n\
         Categories=Development;RevisionControl;\n\
         Keywords=git;vcs;version control;commit;diff;branch;\n\
         StartupWMClass={ID}\n\
         MimeType=inode/directory;\n"
    )
}

/// Nudges the desktop into noticing. Most read the directory themselves, so a
/// missing tool is not a failure — the entry is already written either way.
fn refresh(applications: &Path) {
    let _ = std::process::Command::new("update-desktop-database")
        .arg(applications)
        .status();
}

fn data() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("XDG_DATA_HOME").filter(|dir| !dir.is_empty()) {
        return Some(PathBuf::from(dir));
    }

    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
}

fn applications() -> Option<PathBuf> {
    data().map(|dir| dir.join("applications"))
}

fn wrap(path: &Path, error: std::io::Error) -> Error {
    Error(format!("{}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_launcher_points_at_whatever_is_running() {
        let text = launcher(Path::new("/home/someone/Apps/gitDruid.AppImage"));

        assert!(
            text.contains("Exec=/home/someone/Apps/gitDruid.AppImage %f"),
            "{text}"
        );

        // %f is what lets a folder be dropped on the launcher, or opened with
        // it from a file manager.
        assert!(text.contains("MimeType=inode/directory;"), "{text}");
    }

    #[test]
    fn a_path_with_a_space_is_quoted() {
        let text = launcher(Path::new("/home/someone/My Apps/gitDruid.AppImage"));

        assert!(
            text.contains("Exec=\"/home/someone/My Apps/gitDruid.AppImage\" %f"),
            "an unquoted path with a space reads as a command and an argument: {text}"
        );
    }

    #[test]
    fn the_launcher_and_the_window_agree_on_the_application_id() {
        let text = launcher(Path::new("/usr/bin/git-druid"));

        // The desktop matches a window to its launcher by this, and shows a
        // blank square in the dock when they disagree.
        assert!(text.contains(&format!("StartupWMClass={ID}")), "{text}");
        assert!(text.contains(&format!("Icon={ID}")), "{text}");
        assert_eq!(ID, "gitdruid", "this has to match src/main.rs");
    }

    #[test]
    fn every_embedded_icon_is_a_png_of_the_size_it_claims() {
        for (size, bytes) in ICONS {
            assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "{size} is not a PNG");

            // IHDR carries the dimensions, big-endian, right after the header
            // and the chunk length and tag.
            let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
            let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap());

            assert_eq!((width, height), (size, size));
        }
    }
}
