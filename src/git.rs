//! Thin wrappers over the `git` binary.
//!
//! `aw` shells out rather than linking a git library: the host's git is the
//! one whose credential helpers, SSH configuration, and signing setup already
//! work, and matching its behaviour exactly matters more than speed here.

use anyhow::{Context, Result};
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// How long `aw doctor` waits for a remote to answer before declaring it
/// unreachable. A host that accepts the connection but never replies would
/// otherwise hang the whole run. Fixed for now; a configurable bound can come
/// later.
const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

pub fn init(dir: &Path) -> Result<()> {
    run(dir, &["init", "--quiet"]).map(|_| ())
}

pub fn is_repo(dir: &Path) -> bool {
    dir.join(".git").exists()
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
}
