//! Human-readable command output.
//!
//! Command bodies keep control flow and print each line at the point where its
//! values become available. This module owns only the text and its layout.

use std::fmt::Display;
use std::path::Path;

pub fn init_existing_repo() -> String {
    "repo      already a git repository".to_owned()
}

pub fn init_new_repo() -> String {
    "repo      git init".to_owned()
}

pub fn init_source(url: &str, reference: &str, sha: &str) -> String {
    format!("source    {url}@{reference} ({sha})")
}

pub fn init_unchanged_template() -> String {
    "template  no changes — every file already present".to_owned()
}

pub fn init_created_template_path(path: &str) -> String {
    format!("template  + {path}")
}

pub fn init_name(name: &str) -> String {
    format!("name      {name}")
}

pub fn init_next_step() -> String {
    "Next: edit workspace.toml, then run bin/bootstrap.".to_owned()
}

pub fn init_no_commit() -> String {
    "No commit was made — review `git status` first.".to_owned()
}

pub fn bootstrap_workspace(name: &str) -> String {
    format!("workspace {name}")
}

pub fn bootstrap_trees(regenerated: bool, entries: usize) -> String {
    let state = if regenerated {
        "regenerated"
    } else {
        "unchanged"
    };
    format!("trees     {state} ({entries} entries)")
}

pub fn bootstrap_repo(path: &str, present: bool) -> String {
    let state = if present { "present" } else { "MISSING" };
    format!("repo      {path:<24} {state}")
}

pub fn bootstrap_config(changed: usize, total: usize) -> String {
    format!("config    workspace repo: {changed} of {total} keys updated")
}

pub fn bootstrap_hooks(changed: bool, value: &str) -> String {
    let state = if changed { "set to" } else { "unchanged at" };
    format!("hooks     {} {state} {value}", crate::HOOKS_CONFIG_KEY)
}

pub fn bootstrap_skills(harness: &str, changed: usize) -> String {
    format!("skills    {harness:<12} {changed} link(s) changed")
}

pub fn bootstrap_skill(name: &str, origin: &str) -> String {
    format!("skill     {name:<24} {origin}")
}

pub fn bootstrap_shadowed(name: &str, path: &str, winner: &str) -> String {
    skill_shadowed(name, path, winner)
}

pub fn bootstrap_missing_skill_dir(repo: &str, dir: &str) -> String {
    skill_missing_dir(repo, dir)
}

pub fn bootstrap_unmatched_only(repo: &str, entry: &str) -> String {
    skill_unmatched_only(repo, entry)
}

pub fn bootstrap_agents(changed: usize) -> String {
    format!("agents    {changed} link(s) changed")
}

pub fn bootstrap_agent(name: &str, repo: &str) -> String {
    format!("agent     {name:<24} {repo}")
}

/// Report an agent definition that lost a first-appearance collision. The
/// winning name was already sourced from `winner_repo`; this one at `path` is
/// ignored.
pub fn bootstrap_agent_shadowed(name: &str, path: &str, winner_repo: &str) -> String {
    format!("shadowed  {name} at {path} is ignored because {winner_repo} is already sourced")
}

/// Report an explicitly configured agents source directory that does not
/// exist. A typo in the manifest surfaces here rather than silently
/// discovering nothing.
pub fn bootstrap_missing_agent_dir(repo: &str, dir: &str) -> String {
    format!("agents    {repo} configured agents directory {dir} does not exist")
}

/// Report an `only` allowlist entry that matched no discovered agent
/// definition. A typo in the manifest surfaces here rather than silently
/// narrowing to nothing.
pub fn bootstrap_unmatched_agent_only(repo: &str, entry: &str) -> String {
    format!("agents    {repo} `only` entry {entry} matched no agent definition")
}

pub fn bootstrap_verify(repos: usize, missing: usize, linked: usize, shadowed: usize) -> String {
    format!(
        "verify    {repos} repo(s), {missing} missing, {linked} skill(s) linked, {shadowed} shadowed"
    )
}

pub fn sync_workspace(name: &str) -> String {
    format!("workspace {name}")
}

pub fn sync_repo_fetched(path: &str) -> String {
    format!("repo      {path:<24} fetched")
}

pub fn sync_repo_failed(path: &str, why: &str) -> String {
    format!("repo      {path:<24} FAILED — {why}")
}

pub fn sync_repo_skipped(path: &str) -> String {
    format!("repo      {path:<24} not present, skipped")
}

// The workspace layer reports under its own prefix rather than `repo`, so a
// member may carry any path — including `workspace` — without producing a row
// indistinguishable from the layer's.

pub fn sync_layer_fetched(label: &str) -> String {
    format!("layer     {label:<24} fetched")
}

pub fn sync_layer_failed(label: &str, why: &str) -> String {
    format!("layer     {label:<24} FAILED — {why}")
}

pub fn sync_layer_absent(label: &str) -> String {
    format!("layer     {label:<24} not a repository, skipped")
}

pub fn sync_layer_no_origin(label: &str) -> String {
    format!("layer     {label:<24} no origin, skipped")
}

pub fn sync_skills(harness: &str, changed: usize) -> String {
    format!("skills    {harness:<12} {changed} link(s) changed")
}

pub fn sync_skill(name: &str, origin: &str) -> String {
    format!("skill     {name:<24} {origin}")
}

pub fn sync_shadowed(name: &str, path: &str, winner: &str) -> String {
    skill_shadowed(name, path, winner)
}

pub fn sync_missing_skill_dir(repo: &str, dir: &str) -> String {
    skill_missing_dir(repo, dir)
}

pub fn sync_unmatched_only(repo: &str, entry: &str) -> String {
    skill_unmatched_only(repo, entry)
}

/// Report a skill that lost a first-appearance collision. The winning name was
/// already sourced from `winner`; this one at `path` is ignored.
fn skill_shadowed(name: &str, path: &str, winner: &str) -> String {
    format!("shadowed  {name} at {path} is ignored because {winner} is already sourced")
}

/// Report an explicitly configured source directory that does not exist. A
/// typo in the manifest surfaces here rather than silently discovering nothing.
fn skill_missing_dir(repo: &str, dir: &str) -> String {
    format!("skills    {repo} configured skills directory {dir} does not exist")
}

/// Report an `only` allowlist entry that matched no discovered skill. A typo in
/// the manifest surfaces here rather than silently narrowing to nothing.
fn skill_unmatched_only(repo: &str, entry: &str) -> String {
    format!("skills    {repo} `only` entry {entry} matched no skill")
}

pub fn sync_verify(
    repos: usize,
    skipped: usize,
    failed: usize,
    linked: usize,
    shadowed: usize,
) -> String {
    format!(
        "verify    {repos} repo(s), {skipped} skipped, {failed} failed, \
         {linked} skill(s) linked, {shadowed} shadowed"
    )
}

pub struct DoctorCheck {
    label: &'static str,
    detail: String,
    remedy: &'static str,
}

impl DoctorCheck {
    const fn new(label: &'static str, detail: String, remedy: &'static str) -> Self {
        Self {
            label,
            detail,
            remedy,
        }
    }
}

pub fn doctor_check(pass: bool, check: &DoctorCheck) -> String {
    let status = if pass { "PASS" } else { "FAIL" };
    format!("{status}  {:<22} {}", check.label, check.detail)
}

pub fn doctor_remedy(check: &DoctorCheck) -> String {
    format!("      {:<22} remedy: {}", "", check.remedy)
}

pub fn doctor_git(version: &str) -> DoctorCheck {
    DoctorCheck::new("git", version.to_owned(), "")
}

pub fn doctor_git_error(error: &impl Display) -> DoctorCheck {
    DoctorCheck::new("git", format!("{error:#}"), "install git")
}

pub fn doctor_garden(major: u32, minor: u32, want_major: u32, want_minor: u32) -> DoctorCheck {
    DoctorCheck::new(
        "garden",
        format!("{major}.{minor} (minimum {want_major}.{want_minor})"),
        "brew upgrade garden",
    )
}

pub fn doctor_garden_error(error: &impl Display) -> DoctorCheck {
    DoctorCheck::new("garden", format!("{error:#}"), "brew install garden")
}

pub fn doctor_garden_config(path: &Path) -> DoctorCheck {
    DoctorCheck::new(
        "garden.yaml",
        path.display().to_string(),
        "run `aw init` in this directory to restore it",
    )
}

pub fn doctor_generated_trees(path: &Path) -> DoctorCheck {
    DoctorCheck::new(
        "generated trees",
        path.display().to_string(),
        "run `aw bootstrap`",
    )
}

pub fn doctor_pre_commit_hook(configured: Option<&str>) -> DoctorCheck {
    let detail = configured.map_or_else(
        || format!("{} is unset", crate::HOOKS_CONFIG_KEY),
        |value| format!("{} = {value}", crate::HOOKS_CONFIG_KEY),
    );
    let remedy = if configured.is_some() {
        "unset core.hooksPath, then run `aw bootstrap`"
    } else {
        "run `aw bootstrap`"
    };
    DoctorCheck::new("pre-commit hook", detail, remedy)
}

pub fn doctor_hook_live(hook: &Path, live: bool) -> DoctorCheck {
    let detail = if live {
        format!("{} is executable", hook.display())
    } else {
        format!("{} is missing or not executable", hook.display())
    };
    DoctorCheck::new(
        "hook script",
        detail,
        "restore the hook or `chmod +x` it, then run `bin/install-hooks`",
    )
}

pub fn doctor_remote_reachable(path: &str) -> DoctorCheck {
    DoctorCheck::new("remote", format!("{path:<24} reachable"), "")
}

pub fn doctor_remote_denied(path: &str, why: &str) -> DoctorCheck {
    DoctorCheck::new(
        "remote",
        format!("{path:<24} access denied — {why}"),
        "check credentials for this host",
    )
}

pub fn doctor_remote_unreachable(path: &str, why: &str) -> DoctorCheck {
    DoctorCheck::new(
        "remote",
        format!("{path:<24} unreachable — {why}"),
        "check the URL and network",
    )
}

pub fn doctor_remote_missing_branch(path: &str, branch: &str) -> DoctorCheck {
    DoctorCheck::new(
        "remote",
        format!("{path:<24} pinned branch {branch} not found on remote"),
        "push the branch to the remote, or correct the manifest `branch`",
    )
}

pub fn doctor_discovery_dir(harness: &str, path: &Path) -> DoctorCheck {
    DoctorCheck::new(
        "discovery dir",
        format!("{harness:<12} {}", path.display()),
        "run `aw bootstrap`",
    )
}

pub fn doctor_skill_dir(repo: &str, dir: &str) -> DoctorCheck {
    DoctorCheck::new(
        "skills dir",
        format!("{repo} configured skills directory {dir} does not exist"),
        "create the directory or correct the manifest `dirs`",
    )
}

pub fn doctor_skill_unmatched_only(repo: &str, entry: &str) -> DoctorCheck {
    DoctorCheck::new(
        "skills only",
        format!("{repo} `only` entry {entry} matched no skill"),
        "correct the manifest `only` or drop the entry",
    )
}

pub fn doctor_skill_links(dangling: usize) -> DoctorCheck {
    DoctorCheck::new(
        "skill links",
        format!("{dangling} dangling"),
        "run `aw bootstrap` to re-link, or remove the stale entry",
    )
}

pub fn doctor_dangling_link(path: &Path) -> String {
    format!("      {:<22} {}", "", path.display())
}

/// Report a member's delivery model. Informational only: a member that declares
/// none is not a fault, it is the conservative default, so this renders no
/// PASS/FAIL marker and prints outside `doctor`'s pass/fail channel.
pub fn doctor_delivery_model(path: &str, declared: Option<&str>) -> String {
    let state = match declared {
        Some(file) => format!("declared ({file})"),
        None => "none — conservative default applies".to_owned(),
    };
    format!("      {:<22} {path:<24} {state}", "delivery")
}

/// The delivery-model notice printed to stderr after an adoption, alongside the
/// review-and-commit guidance. A freshly adopted member that declares none takes
/// the conservative default; that is informational, not an error.
pub fn adopt_delivery_model(declared: Option<&str>) -> String {
    match declared {
        Some(file) => format!("Delivery model declared in {file}; honour it for this repository."),
        None => "No delivery model declared; the conservative default applies.".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pre_commit_hook_unset_points_at_bootstrap() {
        let check = doctor_pre_commit_hook(None);
        assert_eq!(check.detail, "core.hooksPath is unset");
        assert_eq!(check.remedy, "run `aw bootstrap`");
    }

    #[test]
    fn pre_commit_hook_redirected_points_at_unset() {
        let check = doctor_pre_commit_hook(Some("hooks"));
        assert_eq!(check.detail, "core.hooksPath = hooks");
        assert_eq!(
            check.remedy,
            "unset core.hooksPath, then run `aw bootstrap`"
        );
    }

    #[test]
    fn hook_live_renders_fail_with_remedy_when_dead() {
        let check = doctor_hook_live(Path::new("/w/.githooks/pre-commit"), false);
        assert!(doctor_check(false, &check).starts_with("FAIL"));
        assert!(check.detail.contains("missing or not executable"));
        assert!(!check.remedy.is_empty());
    }

    #[test]
    fn hook_live_renders_pass_when_live() {
        let check = doctor_hook_live(Path::new("/w/.githooks/pre-commit"), true);
        assert!(doctor_check(true, &check).starts_with("PASS"));
    }

    #[test]
    fn delivery_model_declared_names_the_file() {
        let line = doctor_delivery_model("member", Some("AGENTS.md"));
        assert!(line.contains("delivery"));
        assert!(line.contains("member"));
        assert!(line.contains("declared (AGENTS.md)"));
    }

    #[test]
    fn delivery_model_absent_states_the_conservative_default() {
        let line = doctor_delivery_model("member", None);
        assert!(line.contains("none — conservative default applies"));
        // Informational only: never a PASS/FAIL marker that would read as a
        // check and never fail the doctor exit code.
        assert!(!line.contains("PASS") && !line.contains("FAIL"));
    }

    #[test]
    fn adopt_delivery_model_declared_names_the_file() {
        let line = adopt_delivery_model(Some("AGENTS.md"));
        assert!(line.contains("declared in AGENTS.md"));
    }

    #[test]
    fn adopt_delivery_model_absent_states_the_conservative_default() {
        let line = adopt_delivery_model(None);
        assert!(line.contains("No delivery model declared"));
        assert!(line.contains("conservative default"));
    }
}
