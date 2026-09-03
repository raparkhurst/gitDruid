//! What the user has configured, and where it is kept.
//!
//! Two files, layered. `~/.config/gitDruid/config` holds what is true
//! everywhere; a `.gitdruid` file in a repository overrides it for that
//! repository alone. Anything neither file sets falls back to the defaults
//! below, so an installation with no files at all still works.
//!
//! Both files use git's own config format — sections in square brackets and
//! `key = value` beneath them — because it is the format anyone reaching for
//! `.gitdruid` in an editor already knows.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

/// The file inside a repository. Named without a leading dot-directory so it
/// is easy to find, and easy to add to `.gitignore` or commit as the team
/// prefers.
pub const REPO_FILE: &str = ".gitdruid";

#[derive(Debug, Clone)]
pub struct Error(String);

impl Error {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

/// Which file a value is read from or written to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// `~/.config/gitDruid/config`.
    Global,
    /// `<repo>/.gitdruid`.
    Repo,
}

impl Scope {
    pub fn title(self) -> &'static str {
        match self {
            Scope::Global => "Global",
            Scope::Repo => "This repository",
        }
    }
}

/// One file's worth of values, keyed as `section.name`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Layer {
    values: BTreeMap<String, String>,
}

impl Layer {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }

    /// Sets a value, or removes it when `value` is blank — an empty box in the
    /// dialog means "no opinion here", which for a repository layer is what
    /// makes the global value show through again.
    pub fn set(&mut self, key: &str, value: &str) {
        let value = value.trim();

        if value.is_empty() {
            self.values.remove(key);
        } else {
            self.values.insert(key.to_owned(), value.to_owned());
        }
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Parses git-config-shaped text. Unknown keys are kept as they are, so
    /// hand-editing a file gitDruid does not fully understand loses nothing.
    pub fn parse(text: &str) -> Self {
        let mut values = BTreeMap::new();
        let mut section = String::new();

        for line in text.lines() {
            let line = line.trim();

            if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                continue;
            }

            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                section = name.trim().to_ascii_lowercase();
                continue;
            }

            let Some((name, value)) = line.split_once('=') else {
                continue;
            };

            let name = name.trim().to_ascii_lowercase();
            let value = value.trim().trim_matches('"');

            if section.is_empty() || name.is_empty() {
                continue;
            }

            values.insert(format!("{section}.{name}"), value.to_owned());
        }

        Self { values }
    }

    /// Writes the values back out, grouped into their sections.
    pub fn render(&self) -> String {
        let mut text = String::from("# gitDruid settings\n");
        let mut current = String::new();

        for (key, value) in &self.values {
            let (section, name) = key.split_once('.').unwrap_or(("misc", key));

            if section != current {
                text.push_str(&format!("\n[{section}]\n"));
                current = section.to_owned();
            }

            text.push_str(&format!("\t{name} = {value}\n"));
        }

        text
    }
}

/// The two layers, and the answers they add up to.
#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub global: Layer,
    /// Empty when the repository has no `.gitdruid`, or when none is open.
    pub repo: Layer,
}

impl Settings {
    pub fn layer(&self, scope: Scope) -> &Layer {
        match scope {
            Scope::Global => &self.global,
            Scope::Repo => &self.repo,
        }
    }

    pub fn layer_mut(&mut self, scope: Scope) -> &mut Layer {
        match scope {
            Scope::Global => &mut self.global,
            Scope::Repo => &mut self.repo,
        }
    }

    /// The value in force: the repository's if it has one, else the global.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.repo.get(key).or_else(|| self.global.get(key))
    }

    /// The value in force *ignoring* `scope`, which is what the dialog shows
    /// as a placeholder: "leave this blank and you will get that".
    pub fn inherited(&self, scope: Scope, key: &str) -> Option<&str> {
        match scope {
            Scope::Global => None,
            Scope::Repo => self.global.get(key),
        }
    }

    fn flag(&self, key: &str, fallback: bool) -> bool {
        match self.get(key) {
            Some(value) => matches!(
                value.to_ascii_lowercase().as_str(),
                "true" | "yes" | "on" | "1"
            ),
            None => fallback,
        }
    }

    fn text<'a>(&'a self, key: &str, fallback: &'a str) -> &'a str {
        self.get(key).filter(|value| !value.is_empty()).unwrap_or(fallback)
    }

    pub fn flow(&self) -> Flow {
        Flow {
            enabled: self.flag(keys::FLOW_ENABLED, false),
            main: self.text(keys::FLOW_MAIN, "main").to_owned(),
            develop: self.text(keys::FLOW_DEVELOP, "develop").to_owned(),
            feature: self.text(keys::PREFIX_FEATURE, "feature/").to_owned(),
            bugfix: self.text(keys::PREFIX_BUGFIX, "bugfix/").to_owned(),
            hotfix: self.text(keys::PREFIX_HOTFIX, "hotfix/").to_owned(),
            release: self.text(keys::PREFIX_RELEASE, "release/").to_owned(),
        }
    }

    pub fn credentials(&self) -> Credentials {
        Credentials {
            ssh_key: self.get(keys::SSH_KEY).map(expand_home),
            ssh_public_key: self.get(keys::SSH_PUBLIC_KEY).map(expand_home),
            username: self.get(keys::SSH_USER).map(str::to_owned),
            use_agent: self.flag(keys::USE_AGENT, true),
            use_helper: self.flag(keys::USE_HELPER, true),
        }
    }
}

/// Every key gitDruid itself reads, in one place.
pub mod keys {
    /// The palette, remembered so the window comes back the way it was left.
    pub const THEME: &str = "ui.theme";

    pub const FLOW_ENABLED: &str = "flow.enabled";
    pub const FLOW_MAIN: &str = "flow.main";
    pub const FLOW_DEVELOP: &str = "flow.develop";

    pub const PREFIX_FEATURE: &str = "prefix.feature";
    pub const PREFIX_BUGFIX: &str = "prefix.bugfix";
    pub const PREFIX_HOTFIX: &str = "prefix.hotfix";
    pub const PREFIX_RELEASE: &str = "prefix.release";

    pub const SSH_KEY: &str = "credentials.sshkey";
    pub const SSH_PUBLIC_KEY: &str = "credentials.sshpublickey";
    pub const SSH_USER: &str = "credentials.username";
    pub const USE_AGENT: &str = "credentials.useagent";
    pub const USE_HELPER: &str = "credentials.usehelper";
}

/// The kinds of branch a workflow names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// No prefix and no opinion about where it starts.
    Plain,
    Feature,
    Bugfix,
    Hotfix,
    Release,
}

impl Kind {
    pub const ALL: [Kind; 5] = [
        Kind::Plain,
        Kind::Feature,
        Kind::Bugfix,
        Kind::Hotfix,
        Kind::Release,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Kind::Plain => "Plain",
            Kind::Feature => "Feature",
            Kind::Bugfix => "Bugfix",
            Kind::Hotfix => "Hotfix",
            Kind::Release => "Release",
        }
    }
}

/// How this repository branches, and what it merges back into.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Flow {
    /// False for a repository that branches everything off one line.
    pub enabled: bool,
    pub main: String,
    pub develop: String,
    pub feature: String,
    pub bugfix: String,
    pub hotfix: String,
    pub release: String,
}

impl Flow {
    /// The prefix a branch of this kind carries.
    ///
    /// Empty without git-flow: a repository branching everything off one line
    /// has no kinds to tell apart, so naming them would be noise.
    pub fn prefix(&self, kind: Kind) -> &str {
        if !self.enabled {
            return "";
        }

        match kind {
            Kind::Plain => "",
            Kind::Feature => &self.feature,
            Kind::Bugfix => &self.bugfix,
            Kind::Hotfix => &self.hotfix,
            Kind::Release => &self.release,
        }
    }

    /// The branch a new one of this kind starts from.
    ///
    /// Without git-flow everything comes off the main line. With it, work in
    /// progress comes off develop and a fix for what is already released comes
    /// off main — which is the whole point of running it.
    pub fn start_point(&self, kind: Kind) -> &str {
        if !self.enabled {
            return &self.main;
        }

        match kind {
            Kind::Hotfix | Kind::Release | Kind::Plain => &self.main,
            Kind::Feature | Kind::Bugfix => &self.develop,
        }
    }

    /// The branch this kind is finished by merging into.
    pub fn merges_into(&self, kind: Kind) -> &str {
        if !self.enabled {
            return &self.main;
        }

        match kind {
            Kind::Hotfix | Kind::Release | Kind::Plain => &self.main,
            Kind::Feature | Kind::Bugfix => &self.develop,
        }
    }

    /// The full branch name for `name` of this kind.
    pub fn branch_name(&self, kind: Kind, name: &str) -> String {
        let name = name.trim();
        let prefix = self.prefix(kind);

        // Someone who types the prefix themselves should not get it twice.
        if prefix.is_empty() || name.starts_with(prefix) {
            return name.to_owned();
        }

        format!("{prefix}{name}")
    }

    /// Which kind an existing branch looks like, by its prefix.
    ///
    /// Longest prefix wins, so a `feature/` that happens to start with a
    /// shorter configured prefix is still read as a feature.
    pub fn kind_of(&self, branch: &str) -> Option<Kind> {
        [Kind::Feature, Kind::Bugfix, Kind::Hotfix, Kind::Release]
            .into_iter()
            .filter(|kind| {
                let prefix = self.prefix(*kind);
                !prefix.is_empty() && branch.starts_with(prefix)
            })
            .max_by_key(|kind| self.prefix(*kind).len())
    }
}

/// How to answer a remote asking who we are.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Credentials {
    pub ssh_key: Option<PathBuf>,
    pub ssh_public_key: Option<PathBuf>,
    pub username: Option<String>,
    pub use_agent: bool,
    pub use_helper: bool,
}

/// `~/.config/gitDruid/config`.
pub fn global_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;

    Some(home.join(".config").join("gitDruid").join("config"))
}

pub fn repo_path(repo: &Path) -> PathBuf {
    repo.join(REPO_FILE)
}

/// Reads both layers. A missing file is not an error — it is the common case.
pub fn load(repo: Option<&Path>) -> Settings {
    Settings {
        global: global_path().map(|path| read(&path)).unwrap_or_default(),
        repo: repo.map(|repo| read(&repo_path(repo))).unwrap_or_default(),
    }
}

fn read(path: &Path) -> Layer {
    match std::fs::read_to_string(path) {
        Ok(text) => Layer::parse(&text),
        // Unreadable is treated as absent on purpose: settings are a
        // convenience, and failing to open a repository over one would not be.
        Err(_) => Layer::default(),
    }
}

pub fn save(settings: &Settings, scope: Scope, repo: Option<&Path>) -> Result<PathBuf> {
    let path = match scope {
        Scope::Global => global_path()
            .ok_or_else(|| Error::new("there is no HOME to put a settings file in"))?,
        Scope::Repo => {
            let repo = repo.ok_or_else(|| Error::new("no repository is open"))?;

            repo_path(repo)
        }
    };

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| Error::new(format!("{}: {error}", parent.display())))?;
    }

    std::fs::write(&path, settings.layer(scope).render())
        .map_err(|error| Error::new(format!("{}: {error}", path.display())))?;

    Ok(path)
}

/// Writes a path under the home directory back as `~/…`.
///
/// The inverse of [`expand_home`], used when a path arrives from the file
/// picker: storing the expanded form would tie the settings file to one
/// account, and these files get copied between machines.
pub fn shorten(path: &Path) -> String {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return path.display().to_string();
    };

    match path.strip_prefix(&home) {
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

/// Expands a leading `~/`, which is how anyone writes a path to a key.
fn expand_home(value: &str) -> PathBuf {
    let Some(rest) = value.strip_prefix("~/") else {
        return PathBuf::from(value);
    };

    match std::env::var_os("HOME") {
        Some(home) => PathBuf::from(home).join(rest),
        None => PathBuf::from(value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_home_path_round_trips_through_the_short_form() {
        let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
            return;
        };

        let key = home.join(".ssh").join("id_ed25519");

        let short = shorten(&key);
        assert_eq!(short, "~/.ssh/id_ed25519");
        assert_eq!(expand_home(&short), key);

        // Something outside home is left exactly as it is.
        let elsewhere = PathBuf::from("/etc/ssh/key");
        assert_eq!(shorten(&elsewhere), "/etc/ssh/key");
        assert_eq!(expand_home("/etc/ssh/key"), elsewhere);
    }

    #[test]
    fn a_value_survives_being_written_and_read_back() {
        let mut layer = Layer::default();
        layer.set(keys::FLOW_ENABLED, "true");
        layer.set(keys::FLOW_DEVELOP, "develop");
        layer.set(keys::PREFIX_FEATURE, "feature/");

        let round_tripped = Layer::parse(&layer.render());

        assert_eq!(round_tripped, layer);
        assert_eq!(round_tripped.get(keys::PREFIX_FEATURE), Some("feature/"));
    }

    #[test]
    fn blanking_a_value_removes_it() {
        let mut layer = Layer::default();
        layer.set(keys::FLOW_MAIN, "trunk");
        assert_eq!(layer.get(keys::FLOW_MAIN), Some("trunk"));

        layer.set(keys::FLOW_MAIN, "   ");
        assert_eq!(
            layer.get(keys::FLOW_MAIN),
            None,
            "an emptied box means no opinion, not an empty branch name"
        );
        assert!(layer.is_empty());
    }

    #[test]
    fn a_repository_overrides_the_global_but_only_where_it_says_so() {
        let settings = Settings {
            global: Layer::parse("[flow]\n main = main\n develop = develop\n"),
            repo: Layer::parse("[flow]\n main = trunk\n"),
        };

        assert_eq!(settings.get(keys::FLOW_MAIN), Some("trunk"));
        assert_eq!(settings.get(keys::FLOW_DEVELOP), Some("develop"));

        // And the dialog can say what blanking the override would give back.
        assert_eq!(
            settings.inherited(Scope::Repo, keys::FLOW_MAIN),
            Some("main")
        );
        assert_eq!(settings.inherited(Scope::Global, keys::FLOW_MAIN), None);
    }

    #[test]
    fn defaults_apply_when_nothing_is_configured() {
        let flow = Settings::default().flow();

        assert!(!flow.enabled, "git-flow is opt-in");
        assert_eq!(flow.main, "main");
        assert_eq!(flow.feature, "feature/");

        let credentials = Settings::default().credentials();
        assert!(credentials.use_agent);
        assert!(credentials.use_helper);
        assert_eq!(credentials.ssh_key, None);
    }

    #[test]
    fn without_flow_everything_comes_off_the_main_branch() {
        let settings = Settings {
            global: Layer::parse("[flow]\n main = trunk\n"),
            repo: Layer::default(),
        };

        let flow = settings.flow();

        for kind in Kind::ALL {
            assert_eq!(flow.start_point(kind), "trunk", "{kind:?}");
            assert_eq!(flow.merges_into(kind), "trunk", "{kind:?}");
        }

        assert_eq!(
            flow.branch_name(Kind::Feature, "login"),
            "login",
            "prefixes are a git-flow idea"
        );
    }

    #[test]
    fn with_flow_each_kind_has_its_own_line() {
        let settings = Settings {
            global: Layer::parse("[flow]\n enabled = true\n main = main\n develop = develop\n"),
            repo: Layer::default(),
        };

        let flow = settings.flow();

        // Work in progress comes off develop; a fix for what is released
        // comes off main.
        assert_eq!(flow.start_point(Kind::Feature), "develop");
        assert_eq!(flow.merges_into(Kind::Feature), "develop");
        assert_eq!(flow.start_point(Kind::Bugfix), "develop");
        assert_eq!(flow.start_point(Kind::Hotfix), "main");
        assert_eq!(flow.merges_into(Kind::Hotfix), "main");
        assert_eq!(flow.start_point(Kind::Release), "main");

        assert_eq!(flow.branch_name(Kind::Feature, "login"), "feature/login");
        assert_eq!(
            flow.branch_name(Kind::Feature, "feature/login"),
            "feature/login",
            "typing the prefix should not double it"
        );
    }

    #[test]
    fn an_existing_branch_is_recognised_by_its_prefix() {
        let settings = Settings {
            global: Layer::parse(
                "[flow]\n enabled = true\n[prefix]\n feature = feature/\n hotfix = hotfix/\n",
            ),
            repo: Layer::default(),
        };

        let flow = settings.flow();

        assert_eq!(flow.kind_of("feature/login"), Some(Kind::Feature));
        assert_eq!(flow.kind_of("hotfix/crash"), Some(Kind::Hotfix));
        assert_eq!(flow.kind_of("main"), None);
        assert_eq!(flow.kind_of("something-else"), None);
    }

    #[test]
    fn prefixes_are_matched_longest_first() {
        let settings = Settings {
            global: Layer::parse(
                "[flow]\n enabled = true\n[prefix]\n feature = f/\n bugfix = f/bug/\n",
            ),
            repo: Layer::default(),
        };

        let flow = settings.flow();

        assert_eq!(
            flow.kind_of("f/bug/crash"),
            Some(Kind::Bugfix),
            "the longer prefix is the more specific answer"
        );
        assert_eq!(flow.kind_of("f/login"), Some(Kind::Feature));
    }

    #[test]
    fn hand_written_files_are_read_forgivingly() {
        let layer = Layer::parse(
            "# a comment\n\
             ; another\n\
             [Flow]\n\
             \tEnabled = TRUE\n\
             main=\"trunk\"\n\
             nonsense without an equals\n\
             [prefix]\n\
             feature = feat/\n",
        );

        assert_eq!(layer.get("flow.enabled"), Some("TRUE"));
        assert_eq!(layer.get("flow.main"), Some("trunk"), "quotes are stripped");
        assert_eq!(layer.get("prefix.feature"), Some("feat/"));

        let settings = Settings {
            global: layer,
            repo: Layer::default(),
        };
        assert!(settings.flow().enabled, "TRUE should read as true");
    }
}
