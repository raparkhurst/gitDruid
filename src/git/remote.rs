//! Talking to a remote: fetch, pull and push.
//!
//! Authentication is the awkward part. libgit2 has no idea how the user's git
//! is configured, so [`authenticate`] walks the same ladder git itself does —
//! the credential helper, then the ssh agent — and gives up rather than
//! looping when none of them works.

use std::cell::RefCell;
use std::path::Path;

use git2::{Cred, CredentialType, FetchOptions, PushOptions, RemoteCallbacks, Repository};

use super::refs::head_branch;
use super::{Error, Result};
use crate::settings::Credentials;

/// What push and pull would act on.
///
/// A repository with no remote, or with HEAD detached, has none of this, which
/// is what lets the UI disable the buttons rather than failing on the click.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tracking {
    /// The branch HEAD is on.
    pub branch: String,
    /// The remote it would talk to.
    pub remote: String,
    /// The upstream's shorthand, once one is configured. Until then a push
    /// creates it.
    pub upstream: Option<String>,
    pub ahead: usize,
    pub behind: usize,
}

impl Tracking {
    /// Where a push would send the branch, for the confirmation to name.
    pub fn destination(&self) -> String {
        format!("{}/{}", self.remote, self.branch)
    }
}

/// Reads what a push or pull from here would do.
pub fn tracking(repo_path: &Path) -> Result<Option<Tracking>> {
    let repo = super::open(repo_path)?;

    Ok(read_tracking(&repo))
}

pub(super) fn read_tracking(repo: &Repository) -> Option<Tracking> {
    let branch = head_branch(repo)?;
    let name = branch.name().ok()??.to_owned();

    let upstream = branch.upstream().ok();

    let remote = match &upstream {
        // The remote a branch already tracks wins over any default.
        Some(_) => repo
            .branch_upstream_remote(branch.get().name().ok()?)
            .ok()
            .and_then(|name| name.as_str().ok().map(str::to_owned))?,
        None => default_remote(repo)?,
    };

    let (ahead, behind) = match (branch.get().target(), &upstream) {
        (Some(local), Some(upstream)) => upstream
            .get()
            .target()
            .and_then(|target| repo.graph_ahead_behind(local, target).ok())
            .unwrap_or((0, 0)),
        _ => (0, 0),
    };

    Some(Tracking {
        branch: name,
        remote,
        upstream: upstream
            .and_then(|upstream| upstream.name().ok().flatten().map(str::to_owned)),
        ahead,
        behind,
    })
}

/// The remote a branch with no upstream would push to.
///
/// `origin` by convention, or the only remote there is. With several to choose
/// from and nothing to choose by, gitDruid declines to guess.
fn default_remote(repo: &Repository) -> Option<String> {
    let remotes = repo.remotes().ok()?;
    let names: Vec<String> = remotes.iter().flatten().flatten().map(str::to_owned).collect();

    if names.iter().any(|name| name == "origin") {
        return Some("origin".to_owned());
    }

    match names.as_slice() {
        [only] => Some(only.clone()),
        _ => None,
    }
}

/// Updates the remote-tracking branches, touching nothing else.
pub fn fetch(repo_path: &Path, credentials: &Credentials) -> Result<String> {
    let repo = super::open(repo_path)?;

    let tracking = read_tracking(&repo).ok_or_else(no_remote)?;

    fetch_remote(&repo, &tracking.remote, credentials)?;

    // Report against the state after the fetch, not the one the UI still has.
    let after = read_tracking(&repo).ok_or_else(no_remote)?;

    Ok(match (after.ahead, after.behind) {
        (0, 0) => format!("Fetched {} — up to date", after.remote),
        (ahead, 0) => format!("Fetched {} — {ahead} to push", after.remote),
        (0, behind) => format!("Fetched {} — {behind} to pull", after.remote),
        (ahead, behind) => format!(
            "Fetched {} — {ahead} to push, {behind} to pull",
            after.remote
        ),
    })
}

/// Fetches, then merges the upstream into the current branch.
pub fn pull(repo_path: &Path, credentials: &Credentials) -> Result<String> {
    let repo = super::open(repo_path)?;

    super::merge::require_clean(&repo)?;

    let tracking = read_tracking(&repo).ok_or_else(no_remote)?;

    fetch_remote(&repo, &tracking.remote, credentials)?;

    let Some(upstream) = read_tracking(&repo).and_then(|tracking| tracking.upstream) else {
        return Err(Error::new(format!(
            "{} is not tracking a branch on {} — push it first, and pull will follow it after that",
            tracking.branch, tracking.remote
        )));
    };

    let reference = repo
        .find_branch(&upstream, git2::BranchType::Remote)
        .map_err(|_| Error::new(format!("{upstream} is not on the remote any more")))?;

    let annotated = repo.reference_to_annotated_commit(reference.get())?;

    super::merge::merge_into_head(&repo, &annotated, &upstream)
}

/// Sends the current branch to its remote.
pub fn push(repo_path: &Path, credentials: &Credentials) -> Result<String> {
    let repo = super::open(repo_path)?;

    let tracking = read_tracking(&repo).ok_or_else(no_remote)?;

    // A push that would not fast-forward is rejected by the server anyway;
    // saying so first is friendlier than relaying the transport's wording.
    if tracking.behind > 0 && tracking.upstream.is_some() {
        return Err(Error::new(format!(
            "{} is {} behind {} — pull first, then push",
            tracking.branch,
            tracking.behind,
            tracking.destination()
        )));
    }

    if tracking.upstream.is_some() && tracking.ahead == 0 {
        return Ok(format!("{} is already up to date", tracking.destination()));
    }

    // libgit2 reports a per-ref rejection through this callback rather than
    // through the return value, so a push can "succeed" having done nothing.
    let rejection = RefCell::new(None::<String>);

    {
        let mut callbacks = RemoteCallbacks::new();
        authenticate(&mut callbacks, credentials);

        callbacks.push_update_reference(|reference, status| {
            if let Some(status) = status {
                *rejection.borrow_mut() = Some(format!("{reference} was rejected: {status}"));
            }

            Ok(())
        });

        let mut options = PushOptions::new();
        options.remote_callbacks(callbacks);

        let mut remote = repo
            .find_remote(&tracking.remote)
            .map_err(|_| Error::new(format!("there is no remote named {}", tracking.remote)))?;

        let refspec = format!(
            "refs/heads/{0}:refs/heads/{0}",
            tracking.branch
        );

        remote
            .push(&[refspec.as_str()], Some(&mut options))
            .map_err(describe)?;
    }

    if let Some(message) = rejection.into_inner() {
        return Err(Error::new(format!("{message} — pull first, then push")));
    }

    // A first push is what creates the tracking relationship, the same way
    // `git push -u` does; without it the next pull would not know where to go.
    if tracking.upstream.is_none() {
        let mut branch = repo.find_branch(&tracking.branch, git2::BranchType::Local)?;
        let _ = branch.set_upstream(Some(&tracking.destination()));
    }

    Ok(match tracking.ahead {
        0 => format!("Pushed {}", tracking.destination()),
        1 => format!("Pushed 1 commit to {}", tracking.destination()),
        ahead => format!("Pushed {ahead} commits to {}", tracking.destination()),
    })
}

fn fetch_remote(repo: &Repository, name: &str, credentials: &Credentials) -> Result<()> {
    let mut callbacks = RemoteCallbacks::new();
    authenticate(&mut callbacks, credentials);

    let mut options = FetchOptions::new();
    options.remote_callbacks(callbacks);

    let mut remote = repo
        .find_remote(name)
        .map_err(|_| Error::new(format!("there is no remote named {name}")))?;

    // An empty refspec list means "whatever the remote is configured to
    // fetch", which is what `git fetch` with no arguments does.
    remote
        .fetch::<&str>(&[], Some(&mut options), None)
        .map_err(describe)?;

    Ok(())
}

/// Answers a credential request, in the order the settings ask for.
///
/// libgit2 calls this again for every method it is willing to try, so the
/// closure works down a list and refuses once it runs out: without that, a
/// wrong key or a missing agent turns into an endless retry rather than an
/// error anyone can read.
fn authenticate(callbacks: &mut RemoteCallbacks<'_>, credentials: &Credentials) {
    let credentials = credentials.clone();
    let mut tried: Vec<Method> = Vec::new();

    callbacks.credentials(move |url, username, allowed| {
        // The remote asks for a username on its own before it asks for a key.
        if allowed.contains(CredentialType::USERNAME) && !tried.contains(&Method::Username) {
            tried.push(Method::Username);

            let name = username
                .map(str::to_owned)
                .or_else(|| credentials.username.clone())
                .unwrap_or_else(|| "git".to_owned());

            return Cred::username(&name);
        }

        let name = username
            .map(str::to_owned)
            .or_else(|| credentials.username.clone())
            .unwrap_or_else(|| "git".to_owned());

        for method in Method::ORDER {
            if tried.contains(&method) || !method.offered(&credentials, allowed) {
                continue;
            }

            tried.push(method);

            return match method {
                // A configured key is tried first: someone who named one meant
                // that one, whatever else the agent happens to be holding.
                Method::Key => {
                    let private = credentials.ssh_key.as_ref().expect("offered implies set");

                    Cred::ssh_key(&name, credentials.ssh_public_key.as_deref(), private, None)
                }
                Method::Agent => Cred::ssh_key_from_agent(&name),
                Method::Helper => {
                    let config = git2::Config::open_default()?;

                    Cred::credential_helper(&config, url, username)
                }
                Method::Default => Cred::default(),
                Method::Username => unreachable!("handled above"),
            };
        }

        Err(git2::Error::from_str(&exhausted(&credentials)))
    });
}

/// The ways gitDruid knows how to answer, in the order it tries them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Method {
    Username,
    Key,
    Agent,
    Helper,
    Default,
}

impl Method {
    const ORDER: [Method; 4] = [Method::Key, Method::Agent, Method::Helper, Method::Default];

    fn offered(self, credentials: &Credentials, allowed: CredentialType) -> bool {
        match self {
            Method::Username => allowed.contains(CredentialType::USERNAME),
            Method::Key => {
                allowed.contains(CredentialType::SSH_KEY) && credentials.ssh_key.is_some()
            }
            Method::Agent => allowed.contains(CredentialType::SSH_KEY) && credentials.use_agent,
            Method::Helper => {
                allowed.contains(CredentialType::USER_PASS_PLAINTEXT) && credentials.use_helper
            }
            Method::Default => allowed.contains(CredentialType::DEFAULT),
        }
    }
}

/// Says what was tried, so the message points at the setting to change.
fn exhausted(credentials: &Credentials) -> String {
    let mut tried = Vec::new();

    if credentials.ssh_key.is_some() {
        tried.push("the configured ssh key");
    }
    if credentials.use_agent {
        tried.push("the ssh agent");
    }
    if credentials.use_helper {
        tried.push("git\'s credential helper");
    }

    match tried.as_slice() {
        [] => "no authentication method is enabled — turn one on in Settings".to_owned(),
        methods => format!(
            "the remote refused {} — check the key or the account in Settings",
            methods.join(", then ")
        ),
    }
}

/// Turns a transport failure into something worth reading.
fn describe(error: git2::Error) -> Error {
    let message = error.message();

    if error.class() == git2::ErrorClass::Ssh || message.contains("authentication") {
        return Error::new(format!(
            "{message} — gitDruid uses your ssh agent and git's credential helper, and neither \
             could answer"
        ));
    }

    Error::new(message)
}

fn no_remote() -> Error {
    Error::new("this branch has no remote to talk to")
}
