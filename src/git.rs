//! Thin wrappers over the `git` binary.
//!
//! `aw` shells out rather than linking a git library: the host's git is the
//! one whose credential helpers, SSH configuration, and signing setup already
//! work, and matching its behaviour exactly matters more than speed here.

use anyhow::{Context, Result};
use std::io::{self, Read};
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

/// How long `aw init` waits for each template Git operation to complete.
const TEMPLATE_GIT_TIMEOUT: Duration = Duration::from_secs(120);
const STDERR_LIMIT: usize = 64 * 1024;

/// How long `aw doctor` waits for a remote to answer before declaring it
/// unreachable. A host that accepts the connection but never replies would
/// otherwise hang the whole run. Fixed for now; a configurable bound can come
/// later.
const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// How long `aw sync` allows a member fetch to transfer data.
const FETCH_TIMEOUT: Duration = Duration::from_secs(120);

/// Result of refreshing a repository's origin remote.
pub enum FetchOutcome {
    Fetched,
    Failed(String),
}

pub fn init(dir: &Path) -> Result<()> {
    run(dir, &["init", "--quiet", "--initial-branch=trunk"]).map(|_| ())
}

pub fn is_repo(dir: &Path) -> bool {
    dir.join(".git").exists()
}

/// Whether repository metadata is present, preserving inspection failures.
pub fn is_repo_checked(dir: &Path) -> Result<bool> {
    match std::fs::metadata(dir.join(".git")) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error)
            .with_context(|| format!("inspecting repository metadata in {}", dir.display())),
    }
}

/// Whether a repository has tracked or untracked working-tree changes.
pub fn is_dirty(dir: &Path) -> Result<bool> {
    run(dir, &["status", "--porcelain"]).map(|output| !output.is_empty())
}

pub struct UpstreamComparison {
    pub ahead: u64,
    pub behind: u64,
    pub upstream: String,
}

/// Compare `HEAD` with its tracked upstream using local refs only.
pub fn upstream_comparison(dir: &Path) -> Result<Option<UpstreamComparison>> {
    let head = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["symbolic-ref", "--quiet", "HEAD"])
        .output()
        .context("reading the current branch")?;
    if head.status.code() == Some(1) {
        return Ok(None);
    }
    anyhow::ensure!(
        head.status.success(),
        "reading the current branch failed in {}: {}",
        dir.display(),
        String::from_utf8_lossy(&head.stderr).trim()
    );
    let head = String::from_utf8_lossy(&head.stdout).trim().to_owned();
    let upstream = run(
        dir,
        &["for-each-ref", "--format=%(upstream)", "--count=1", &head],
    )?;
    let upstream = upstream.trim();
    if upstream.is_empty() {
        return Ok(None);
    }
    if !ref_exists(dir, upstream)? {
        return Ok(None);
    }

    let range = format!("{upstream}...HEAD");
    let counts = run(dir, &["rev-list", "--left-right", "--count", &range])?;
    let mut counts = counts.split_whitespace();
    let behind = parse_count(counts.next(), "behind", dir)?;
    let ahead = parse_count(counts.next(), "ahead", dir)?;
    anyhow::ensure!(
        counts.next().is_none(),
        "reading ahead/behind counts failed in {}: unexpected output",
        dir.display()
    );
    Ok(Some(UpstreamComparison {
        ahead,
        behind,
        upstream: upstream.to_owned(),
    }))
}

/// Return the newest reflog timestamp for a remote-tracking ref, if retained.
pub fn newest_remote_reflog_timestamp(dir: &Path, reference: &str) -> Result<Option<u64>> {
    if !reference.starts_with("refs/remotes/") {
        return Ok(None);
    }
    let output = run(
        dir,
        &[
            "reflog",
            "show",
            "-1",
            "--date=unix",
            "--format=%gD",
            reference,
        ],
    )?;
    parse_optional_reflog_timestamp(output.trim(), reference, dir)
}

/// Return the timestamp of the clone entry in `HEAD`'s reflog, if retained.
pub fn clone_reflog_timestamp(dir: &Path) -> Result<Option<u64>> {
    let reflog = run(dir, &["rev-parse", "--git-path", "logs/HEAD"])?;
    let reflog = Path::new(reflog.trim());
    let reflog = if reflog.is_absolute() {
        reflog.to_owned()
    } else {
        dir.join(reflog)
    };
    match std::fs::metadata(&reflog) {
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("inspecting HEAD reflog in {}", dir.display()));
        }
    }

    let output = run(
        dir,
        &[
            "reflog",
            "show",
            "--date=unix",
            "--format=%gD%x09%gs",
            "HEAD",
        ],
    )?;
    for line in output.lines() {
        let Some((selector, subject)) = line.split_once('\t') else {
            anyhow::bail!(
                "reading HEAD reflog failed in {}: unexpected output",
                dir.display()
            );
        };
        if subject.starts_with("clone:") {
            return parse_optional_reflog_timestamp(selector, "HEAD clone entry", dir);
        }
    }
    Ok(None)
}

fn ref_exists(dir: &Path, reference: &str) -> Result<bool> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["show-ref", "--verify", "--quiet", reference])
        .output()
        .with_context(|| format!("checking upstream ref in {}", dir.display()))?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => anyhow::bail!(
            "checking upstream ref failed in {}: {}",
            dir.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ),
    }
}

fn parse_count(value: Option<&str>, label: &str, dir: &Path) -> Result<u64> {
    value
        .context("missing count")?
        .parse()
        .with_context(|| format!("parsing {label} count in {}", dir.display()))
}

fn parse_optional_reflog_timestamp(value: &str, source: &str, dir: &Path) -> Result<Option<u64>> {
    if value.is_empty() {
        return Ok(None);
    }
    let timestamp = value
        .rsplit_once("@{")
        .map(|(_, timestamp)| timestamp)
        .and_then(|timestamp| timestamp.strip_suffix('}'))
        .with_context(|| {
            format!(
                "parsing {source} reflog selector in {}: unexpected output",
                dir.display()
            )
        })?;
    timestamp
        .parse()
        .map(Some)
        .with_context(|| format!("parsing {source} reflog timestamp in {}", dir.display()))
}

/// Clone a template source without inheriting any working-tree state from a
/// local checkout.
pub fn clone_template(source: &str, destination: &Path) -> Result<()> {
    let mut command = Command::new("git");
    command
        .args(["clone", "--quiet", "--no-checkout", "--"])
        .arg(source)
        .arg(destination)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "true");

    let Some(output) = run_bounded(&mut command, TEMPLATE_GIT_TIMEOUT)
        .with_context(|| format!("waiting for template clone {source}"))?
    else {
        anyhow::bail!(
            "cloning template {source} timed out after {}s",
            TEMPLATE_GIT_TIMEOUT.as_secs()
        );
    };
    anyhow::ensure!(
        output.status.success(),
        "cloning template {source} failed: {}",
        output.stderr.trim()
    );
    Ok(())
}

struct BoundedOutput {
    status: ExitStatus,
    stderr: String,
}

/// Run a child with a deadline, continuously draining but retaining only a
/// bounded diagnostic.
fn run_bounded(command: &mut Command, timeout: Duration) -> io::Result<Option<BoundedOutput>> {
    let (mut stderr, stderr_writer) = UnixStream::pair()?;
    stderr.set_nonblocking(true)?;
    let stderr_writer: OwnedFd = stderr_writer.into();
    let mut child = command.stderr(Stdio::from(stderr_writer)).spawn()?;
    let mut bytes = Vec::new();
    let mut truncated = false;
    let status = wait_for_child(&mut child, &mut stderr, &mut bytes, &mut truncated, timeout)?;
    drain_stderr(&mut stderr, &mut bytes, &mut truncated)?;
    let mut stderr = String::from_utf8_lossy(&bytes).into_owned();
    if truncated {
        stderr.push_str("\n[stderr truncated]");
    }
    Ok(status.map(|status| BoundedOutput { status, stderr }))
}

fn drain_stderr(
    stderr: &mut UnixStream,
    bytes: &mut Vec<u8>,
    truncated: &mut bool,
) -> io::Result<()> {
    let mut chunk = [0_u8; 8192];
    loop {
        match stderr.read(&mut chunk) {
            Ok(0) => return Ok(()),
            Ok(count) => {
                let retained = count.min(STDERR_LIMIT.saturating_sub(bytes.len()));
                bytes.extend_from_slice(&chunk[..retained]);
                *truncated |= retained < count;
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(()),
            Err(error) => return Err(error),
        }
    }
}

fn wait_for_child(
    child: &mut Child,
    stderr: &mut UnixStream,
    bytes: &mut Vec<u8>,
    truncated: &mut bool,
    timeout: Duration,
) -> io::Result<Option<ExitStatus>> {
    let deadline = Instant::now() + timeout;
    loop {
        drain_stderr(stderr, bytes, truncated)?;
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

    if reference.starts_with("refs/") {
        if let Ok(sha) = rev_parse_commit(dir, reference) {
            return Ok(sha);
        }
        fetch_ref(dir, reference)?;
        return rev_parse_commit(dir, "FETCH_HEAD")
            .with_context(|| format!("resolving template ref {reference:?}"));
    }

    let is_hex_object_id = (4..=64).contains(&reference.len())
        && reference
            .chars()
            .all(|character| character.is_ascii_hexdigit());
    if is_hex_object_id {
        if let Ok(sha) = rev_parse_commit(dir, reference) {
            return Ok(sha);
        }
    }

    let tag = format!("refs/tags/{reference}");
    let remote = format!("refs/remotes/origin/{reference}");
    let tag_sha = rev_parse_commit(dir, &tag).ok();
    let remote_sha = rev_parse_commit(dir, &remote).ok();
    match (tag_sha, remote_sha) {
        (Some(tag_sha), Some(remote_sha)) if tag_sha != remote_sha => {
            anyhow::bail!(
                "template ref {reference:?} is ambiguous: tag and branch resolve to different commits; \
                 use refs/tags/{reference} or refs/heads/{reference}"
            );
        }
        (Some(sha), _) | (_, Some(sha)) => return Ok(sha),
        (None, None) => {}
    }

    fetch_ref(dir, reference)?;
    rev_parse_commit(dir, "FETCH_HEAD")
        .with_context(|| format!("resolving template ref {reference:?}"))
}

fn fetch_ref(dir: &Path, reference: &str) -> Result<()> {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(dir)
        .args(["fetch", "--quiet", "origin", "--", reference])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "true");
    let Some(output) = run_bounded(&mut command, TEMPLATE_GIT_TIMEOUT)
        .with_context(|| format!("waiting while fetching template ref {reference:?}"))?
    else {
        anyhow::bail!(
            "fetching template ref {reference:?} timed out after {}s",
            TEMPLATE_GIT_TIMEOUT.as_secs()
        );
    };
    anyhow::ensure!(
        output.status.success(),
        "fetching template ref {reference:?} failed: {}",
        output.stderr.trim()
    );
    Ok(())
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
    if config_value(dir, key)?.as_deref() == Some(value) {
        return Ok(false);
    }
    run(dir, &["config", "--local", "--", key, value])?;
    Ok(true)
}

/// Return a repository-local configuration value, or `None` when it is unset.
pub fn config_value(dir: &Path, key: &str) -> Result<Option<String>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["config", "--null", "--local", "--get", "--", key])
        .output()
        .with_context(|| format!("reading local git config {key}"))?;
    if output.status.success() {
        let value = String::from_utf8_lossy(&output.stdout);
        return Ok(Some(value.strip_suffix('\0').unwrap_or(&value).to_owned()));
    }
    if output.status.code() == Some(1) {
        return Ok(None);
    }
    anyhow::bail!(
        "reading local git config {key} failed in {}: {}",
        dir.display(),
        String::from_utf8_lossy(&output.stderr).trim()
    );
}

/// Set a repository-local configuration value only when the key is unset.
pub fn set_config_if_unset(dir: &Path, key: &str, value: &str) -> Result<bool> {
    if config_value(dir, key)?.is_some() {
        return Ok(false);
    }
    run(dir, &["config", "--local", "--", key, value])?;
    Ok(true)
}

/// Fetch origin branches into remote-tracking refs without changing the
/// repository's working tree or checked-out commit.
pub fn fetch(dir: &Path) -> Result<FetchOutcome> {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(dir)
        .args([
            "fetch",
            "--no-tags",
            "--no-recurse-submodules",
            "--refmap=",
            "origin",
            "+refs/heads/*:refs/remotes/origin/*",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "true");

    let Some(output) = run_bounded(&mut command, FETCH_TIMEOUT)
        .with_context(|| format!("waiting for git fetch in {}", dir.display()))?
    else {
        return Ok(FetchOutcome::Failed(format!(
            "fetch did not complete within {}s",
            FETCH_TIMEOUT.as_secs()
        )));
    };
    if output.status.success() {
        return Ok(FetchOutcome::Fetched);
    }
    let detail = output
        .stderr
        .lines()
        .find(|line| !line.trim().is_empty())
        .map_or_else(
            || format!("git fetch exited with {}", output.status),
            |line| line.trim().to_owned(),
        );
    Ok(FetchOutcome::Failed(detail))
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

pub fn checked_out_branch(dir: &Path) -> Result<String> {
    run(dir, &["symbolic-ref", "--quiet", "--short", "HEAD"]).map(|branch| branch.trim().to_owned())
}

pub fn worktree_root(dir: &Path) -> Result<PathBuf> {
    resolved_git_path(dir, "--show-toplevel", "git worktree root")
}

pub fn per_worktree_git_dir(dir: &Path) -> Result<PathBuf> {
    resolved_git_path(dir, "--git-dir", "per-worktree git directory")
}

pub fn common_git_dir(dir: &Path) -> Result<PathBuf> {
    resolved_git_path(dir, "--git-common-dir", "common git directory")
}

fn resolved_git_path(dir: &Path, option: &str, description: &str) -> Result<PathBuf> {
    let output = run(dir, &["rev-parse", "--path-format=absolute", option])?;
    let path = PathBuf::from(output.trim());
    anyhow::ensure!(
        !path.as_os_str().is_empty(),
        "`git rev-parse {option}` returned an empty {description} in {}",
        dir.display()
    );
    std::fs::canonicalize(&path)
        .with_context(|| format!("resolving {description} {}", path.display()))
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
    fn bounded_command_times_out_and_kills_the_child() {
        let mut command = Command::new("sleep");
        command.arg("10");
        let started = Instant::now();

        let output = run_bounded(&mut command, Duration::from_millis(10)).unwrap();

        assert!(output.is_none());
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn bounded_command_caps_stderr_while_the_child_runs() {
        let mut command = Command::new("sh");
        command.args([
            "-c",
            "{ dd if=/dev/zero bs=1024 count=70 2>/dev/null; } >&2; exit 7",
        ]);

        let output = run_bounded(&mut command, Duration::from_secs(1))
            .unwrap()
            .expect("command finishes");

        assert!(!output.status.success());
        assert!(output.stderr.ends_with("\n[stderr truncated]"));
        assert_eq!(
            output.stderr.len(),
            STDERR_LIMIT + "\n[stderr truncated]".len()
        );
    }
}
