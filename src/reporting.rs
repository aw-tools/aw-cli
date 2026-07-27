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

pub fn bootstrap_shadowed(name: &str, origin: &str, winner: &str) -> String {
    format!("shadowed  {name:<24} {origin} shadowed by {winner}")
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

pub fn sync_skills(harness: &str, changed: usize) -> String {
    format!("skills    {harness:<12} {changed} link(s) changed")
}

pub fn sync_skill(name: &str, origin: &str) -> String {
    format!("skill     {name:<24} {origin}")
}

pub fn sync_shadowed(name: &str, origin: &str, winner: &str) -> String {
    format!("shadowed  {name:<24} {origin} shadowed by {winner}")
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
    fn new(label: &'static str, detail: String, remedy: &'static str) -> Self {
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

pub fn doctor_discovery_dir(harness: &str, path: &Path) -> DoctorCheck {
    DoctorCheck::new(
        "discovery dir",
        format!("{harness:<12} {}", path.display()),
        "run `aw bootstrap`",
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
