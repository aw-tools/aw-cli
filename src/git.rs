//! Thin wrappers over the `git` binary.
//!
//! `aw` shells out rather than linking a git library: the host's git is the
//! one whose credential helpers, SSH configuration, and signing setup already
//! work, and matching its behaviour exactly matters more than speed here.

use anyhow::{Context, Result};
use std::io::Read;
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

/// How long `aw init` waits for a template clone to complete.
const CLONE_TIMEOUT: Duration = Duration::from_secs(120);

/// How long `aw doctor` waits for a remote to answer before declaring it
/// unreachable. A host that accepts the connection but never replies would
/// otherwise hang the whole run. Fixed for now; a configurable bound can come
/// later.
const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

pub fn init(dir: &Path) -> Result<()> {
    run(dir, &["init", "--quiet", "--initial-branch=trunk"]).map(|_| ())
}

pub fn is_repo(dir: &Path) -> bool {
    dir.join(".git").exists()
}

/// Clone a template source without inheriting any working-tree state from a
/// local checkout.
pub fn clone_template(source: &str, destination: &Path) -> Result<()> {
    let mut child = Command::new("git")
        .args(["clone", "--quiet", "--no-checkout", "--"])
        .arg(source)
        .arg(destination)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "true")
        .spawn()
        .with_context(|| format!("cloning template {source}"))?;

    let Some(status) = wait_for_clone(&mut child, CLONE_TIMEOUT)
        .with_context(|| format!("waiting for template clone {source}"))?
    else {
        anyhow::bail!(
            "cloning template {source} timed out after {}s",
            CLONE_TIMEOUT.as_secs()
        );
    };

    let mut stderr = String::new();
    if let Some(mut pipe) = child.stderr.take() {
        pipe.read_to_string(&mut stderr)
            .with_context(|| format!("reading errors from template clone {source}"))?;
    }
    anyhow::ensure!(
        status.success(),
        "cloning template {source} failed: {}",
        stderr.trim()
    );
    Ok(())
}

fn wait_for_clone(child: &mut Child, timeout: Duration) -> std::io::Result<Option<ExitStatus>> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait()? {
            Some(status) => return Ok(Some(status)),
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Ok(None);
            }
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    }
}

/// Resolve a tag, SHA, or local/remote branch to a commit in a cloned
/// repository, fetching the named ref when the clone did not bring it across.
pub fn resolve_commit(dir: &Path, reference: Option<&str>) -> Result<String> {
    let Some(reference) = reference else {
        return rev_parse_commit(dir, "HEAD");
    };

    if let Ok(sha) = rev_parse_commit(dir, reference) {
        return Ok(sha);
    }

    let remote = format!("refs/remotes/origin/{reference}");
    if let Ok(sha) = rev_parse_commit(dir, &remote) {
        return Ok(sha);
    }

    run(dir, &["fetch", "--quiet", "origin", "--", reference])
        .with_context(|| format!("fetching template ref {reference:?}"))?;
    rev_parse_commit(dir, "FETCH_HEAD")
        .with_context(|| format!("resolving template ref {reference:?}"))
}

pub fn checkout_detached(dir: &Path, sha: &str) -> Result<()> {
    run(dir, &["checkout", "--quiet", "--detach", sha]).map(|_| ())
}

/// Return tracked files selected by an exclude file. `git ls-files` gives
/// `.seedignore` gitignore-compatible globbing, directory patterns, comments,
/// and negation without duplicating that matcher in `aw`.
pub fn tracked_excluded(dir: &Path, exclude_file: &str) -> Result<Vec<String>> {
    let out = run(
        dir,
        &[
            "ls-files",
            "--cached",
            "--ignored",
            "-z",
            &format!("--exclude-from={exclude_file}"),
        ],
    )?;
    Ok(out.split_terminator('\0').map(ToOwned::to_owned).collect())
}

/// Set a repository-local configuration value, returning whether it changed.
///
/// Repository-local configuration is last-wins over user and system scope,
/// which is what makes a bootstrapped workspace behave identically on a
/// laptop and on a runner.
pub fn set_config(dir: &Path, key: &str, value: &str) -> Result<bool> {
    if run(dir, &["config", "--local", "--get", "--", key])
        .is_ok_and(|current| current.trim() == value)
    {
        return Ok(false);
    }
    run(dir, &["config", "--local", "--", key, value])?;
    Ok(true)
}

/// Whether a remote is reachable, distinguishing authentication failures from
/// everything else so `aw doctor` can suggest the right remedy.
pub fn probe_remote(url: &str) -> RemoteProbe {
    let child = Command::new("git")
        .args(["ls-remote", "--exit-code", "--", url, "HEAD"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "true")
        .spawn();

    let Ok(mut child) = child else {
        return RemoteProbe::Unreachable("could not run git".to_owned());
    };

    // Poll for exit; kill the child if it outlives the deadline. `ls-remote`
    // writes only a line or two to stderr, well under the pipe buffer, so
    // reading it after exit cannot deadlock the child.
    let deadline = Instant::now() + PROBE_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut stderr = String::new();
                if let Some(mut pipe) = child.stderr.take() {
                    let _ = pipe.read_to_string(&mut stderr);
                }
                return classify_probe(status.success(), stderr.trim());
            }
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return RemoteProbe::Unreachable(format!(
                    "no response within {}s",
                    PROBE_TIMEOUT.as_secs()
                ));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(_) => return RemoteProbe::Unreachable("could not run git".to_owned()),
        }
    }
}

/// Turn an `ls-remote` outcome into a probe result, separating an
/// authentication failure from any other unreachability so `doctor` can suggest
/// the right remedy.
fn classify_probe(success: bool, stderr: &str) -> RemoteProbe {
    if success {
        return RemoteProbe::Reachable;
    }
    let lowered = stderr.to_lowercase();
    if [
        "authentication",
        "permission denied",
        "could not read username",
        "access denied",
    ]
    .iter()
    .any(|needle| lowered.contains(needle))
    {
        RemoteProbe::Denied(stderr.to_owned())
    } else {
        RemoteProbe::Unreachable(stderr.to_owned())
    }
}

pub enum RemoteProbe {
    Reachable,
    Denied(String),
    Unreachable(String),
}

pub fn version() -> Result<String> {
    let out = Command::new("git")
        .arg("--version")
        .output()
        .context("running `git --version`")?;
    anyhow::ensure!(out.status.success(), "`git --version` failed");
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

fn run(dir: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .with_context(|| format!("running `git {}`", args.join(" ")))?;
    anyhow::ensure!(
        out.status.success(),
        "`git {}` failed in {}: {}",
        args.join(" "),
        dir.display(),
        String::from_utf8_lossy(&out.stderr).trim()
    );
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn rev_parse_commit(dir: &Path, reference: &str) -> Result<String> {
    let commit = format!("{reference}^{{commit}}");
    run(dir, &["rev-parse", "--verify", "--end-of-options", &commit])
        .map(|sha| sha.trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn success_is_reachable() {
        assert!(matches!(classify_probe(true, ""), RemoteProbe::Reachable));
    }

    #[test]
    fn auth_failure_is_denied() {
        assert!(matches!(
            classify_probe(false, "fatal: Authentication failed for 'https://x'"),
            RemoteProbe::Denied(_)
        ));
    }

    #[test]
    fn other_failure_is_unreachable() {
        assert!(matches!(
            classify_probe(false, "fatal: unable to access: Could not resolve host"),
            RemoteProbe::Unreachable(_)
        ));
    }

    #[test]
    fn clone_wait_times_out_and_kills_the_child() {
        let mut child = Command::new("sleep").arg("10").spawn().unwrap();
        let started = Instant::now();

        let status = wait_for_clone(&mut child, Duration::from_millis(10)).unwrap();

        assert!(status.is_none());
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(child.try_wait().unwrap().is_some());
    }
}
