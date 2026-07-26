//! `aw` — provision and report on reproducible multi-repository agentic
//! workspaces.
//!
//! A thin orchestrator. Cloning and per-repository git configuration are
//! garden's job; `aw` owns the manifest, skill linking, and reporting.

mod garden;
mod git;
mod manifest;
mod skills;
mod template;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use manifest::Manifest;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "aw", version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Verb,
}

#[derive(Subcommand)]
enum Verb {
    /// Create a new workspace from a cloned template.
    Init {
        /// Directory to create. Defaults to the current directory.
        dir: Option<PathBuf>,
        /// Workspace name recorded in the manifest. Defaults to the directory
        /// name with any `.workspace` suffix removed.
        #[arg(long)]
        name: Option<String>,
        /// Template URL or local path, optionally followed by `@ref`.
        #[arg(long)]
        template: Option<String>,
    },
    /// Clone missing repositories, converge configuration, link skills.
    Bootstrap {
        /// Workspace root. Defaults to the nearest ancestor with a manifest.
        dir: Option<PathBuf>,
    },
    /// Check the environment, remotes, and link health.
    Doctor {
        /// Workspace root. Defaults to the nearest ancestor with a manifest.
        dir: Option<PathBuf>,
    },
}

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(err) => {
            eprintln!("aw: {err:#}");
            ExitCode::FAILURE
        }
    }
}

/// Returns whether the command succeeded. `doctor` reports failures through
/// this rather than an error, because a failed check is a finding, not a crash.
fn run() -> Result<bool> {
    match Cli::parse().command {
        Verb::Init {
            dir,
            name,
            template,
        } => init(dir, name, template.as_deref()).map(|()| true),
        Verb::Bootstrap { dir } => bootstrap(&manifest::resolve_root(dir)?),
        Verb::Doctor { dir } => doctor(&manifest::resolve_root(dir)?),
    }
}

fn init(dir: Option<PathBuf>, name: Option<String>, template: Option<&str>) -> Result<()> {
    let dir = dir.unwrap_or_else(|| PathBuf::from("."));
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let root =
        std::fs::canonicalize(&dir).with_context(|| format!("resolving {}", dir.display()))?;

    let source = template::Source::parse(template)?;
    let prepared = template::prepare(&source)?;
    manifest::validate_template(
        &root,
        &source.url,
        source.reference.as_deref().unwrap_or(""),
        &prepared.sha,
    )?;
    let name = name.unwrap_or_else(|| default_name(&root));
    validate_name(&name)?;
    let created = template::materialise(&root, &prepared)?;
    set_workspace_name(&root, &name)?;
    let manifest = manifest::record_template(
        &root,
        &source.url,
        source.reference.as_deref().unwrap_or(""),
        &prepared.sha,
    )?;

    if git::is_repo(&root) {
        println!("repo      already a git repository");
    } else {
        git::init(&root)?;
        println!("repo      git init");
    }

    for (key, value) in garden::identity_pairs(manifest.identity.as_ref()) {
        git::set_config(&root, key, &value)?;
    }

    println!(
        "source    {}@{} ({})",
        source.url,
        source.reference.as_deref().unwrap_or(""),
        prepared.sha
    );
    if created.is_empty() {
        println!("template  no changes — every file already present");
    } else {
        for path in &created {
            println!("template  + {path}");
        }
    }
    println!("name      {name}");

    eprintln!();
    eprintln!("Next: edit workspace.toml, then run bin/bootstrap.");
    eprintln!("No commit was made — review `git status` first.");
    Ok(())
}

/// Workspace name from the directory, dropping the `.workspace` suffix the
/// naming convention adds.
fn default_name(root: &Path) -> String {
    let dir = root.file_name().map_or_else(
        || "workspace".to_owned(),
        |n| n.to_string_lossy().into_owned(),
    );
    dir.strip_suffix(".workspace")
        .map(ToOwned::to_owned)
        .unwrap_or(dir)
}

/// Reject a name that could not survive being written into the manifest as a
/// TOML basic string. Without this, a directory or `--name` containing a quote
/// or backslash would produce an unparseable `workspace.toml`.
fn validate_name(name: &str) -> Result<()> {
    anyhow::ensure!(!name.trim().is_empty(), "workspace name is empty");
    anyhow::ensure!(
        !name.contains(['"', '\\']) && !name.contains(char::is_control),
        "workspace name {name:?} contains characters that cannot appear in the manifest; \
         use letters, digits, dots, or dashes"
    );
    Ok(())
}

/// Rewrite only the placeholder name in the freshly copied template, leaving a
/// manifest the human has already edited untouched.
fn set_workspace_name(root: &Path, name: &str) -> Result<()> {
    let path = root.join(manifest::FILENAME);
    let metadata = std::fs::symlink_metadata(&path)
        .with_context(|| format!("reading metadata for {}", path.display()))?;
    anyhow::ensure!(
        metadata.file_type().is_file(),
        "{} must be a regular file",
        path.display()
    );
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let updated = text.replacen("name = \"CHANGEME\"", &format!("name = \"{name}\""), 1);
    if updated == text {
        return Ok(());
    }
    std::fs::write(&path, updated).with_context(|| format!("writing {}", path.display()))
}

/// Returns whether every managed repository is present. A repo still missing
/// after garden reports success is a failure the exit code must reflect, so a
/// script or agent driving `aw` does not read it as a clean run.
fn bootstrap(root: &Path) -> Result<bool> {
    let manifest = Manifest::load(root)?;
    println!("workspace {}", manifest.workspace.name);

    // Phase 1 — repositories.
    let regenerated = garden::write_trees(root, &manifest)?;
    println!(
        "trees     {} ({} entries)",
        if regenerated {
            "regenerated"
        } else {
            "unchanged"
        },
        manifest.repos.len()
    );
    garden::grow(root, &manifest)?;
    for repo in &manifest.repos {
        let path = garden::checkout_path(root, &repo.path);
        println!(
            "repo      {:<24} {}",
            repo.path,
            if git::is_repo(&path) {
                "present"
            } else {
                "MISSING"
            }
        );
    }

    // Phase 2 — configuration. Garden applies this to the trees it manages;
    // the workspace repository itself is not a tree, so `aw` owns it.
    let pairs = garden::identity_pairs(manifest.identity.as_ref());
    let mut changed = 0;
    for (key, value) in &pairs {
        if git::set_config(root, key, value)? {
            changed += 1;
        }
    }
    println!(
        "config    workspace repo: {} of {} keys updated",
        changed,
        pairs.len()
    );

    // Phase 3 — skills.
    let resolution = skills::resolve(root, &manifest)?;
    for (harness, changed) in skills::link(root, &resolution)? {
        println!("skills    {harness:<12} {changed} link(s) changed");
    }
    for skill in &resolution.linked {
        println!("skill     {:<24} {}", skill.name, skill.origin.label());
    }
    for (loser, winner) in &resolution.shadowed {
        println!(
            "shadowed  {:<24} {} shadowed by {}",
            loser.name,
            loser.origin.label(),
            winner.label()
        );
    }

    // Phase 4 — verify.
    let missing = manifest
        .repos
        .iter()
        .filter(|r| !git::is_repo(&garden::checkout_path(root, &r.path)))
        .count();
    println!(
        "verify    {} repo(s), {} missing, {} skill(s) linked, {} shadowed",
        manifest.repos.len(),
        missing,
        resolution.linked.len(),
        resolution.shadowed.len()
    );
    Ok(missing == 0)
}

fn doctor(root: &Path) -> Result<bool> {
    let mut ok = true;
    let mut check = |pass: bool, label: &str, detail: &str, remedy: &str| {
        if pass {
            println!("PASS  {label:<22} {detail}");
        } else {
            println!("FAIL  {label:<22} {detail}");
            println!("      {:<22} remedy: {remedy}", "");
            ok = false;
        }
    };

    match git::version() {
        Ok(v) => check(true, "git", &v, ""),
        Err(e) => check(false, "git", &format!("{e:#}"), "install git"),
    }

    match garden::version() {
        Ok((major, minor)) => {
            let (want_major, want_minor) = garden::MIN_VERSION;
            let recent = (major, minor) >= (want_major, want_minor);
            check(
                recent,
                "garden",
                &format!("{major}.{minor} (minimum {want_major}.{want_minor})"),
                "brew upgrade garden",
            );
        }
        Err(e) => check(false, "garden", &format!("{e:#}"), "brew install garden"),
    }

    let config = root.join(garden::CONFIG_FILE);
    check(
        config.is_file(),
        "garden.yaml",
        &config.display().to_string(),
        "run `aw init` in this directory to restore it",
    );

    // A garden config whose include is missing resolves to zero trees and
    // exits successfully, so an absent generated file is silent at the garden
    // layer and has to be caught here.
    let generated = root.join(garden::GENERATED_FILE);
    check(
        generated.is_file(),
        "generated trees",
        &generated.display().to_string(),
        "run `aw bootstrap`",
    );

    let manifest = Manifest::load(root)?;
    for repo in &manifest.repos {
        let (pass, detail, remedy) = match git::probe_remote(&repo.url) {
            git::RemoteProbe::Reachable => (true, "reachable".to_owned(), ""),
            git::RemoteProbe::Denied(why) => (
                false,
                format!("access denied — {why}"),
                "check credentials for this host",
            ),
            git::RemoteProbe::Unreachable(why) => (
                false,
                format!("unreachable — {why}"),
                "check the URL and network",
            ),
        };
        check(
            pass,
            "remote",
            &format!("{:<24} {detail}", repo.path),
            remedy,
        );
    }

    for harness in skills::HARNESSES {
        let dir = root.join(harness.dir);
        check(
            dir.is_dir(),
            "discovery dir",
            &format!("{:<12} {}", harness.name, dir.display()),
            "run `aw bootstrap`",
        );
    }

    let dangling = skills::dangling(root)?;
    check(
        dangling.is_empty(),
        "skill links",
        &format!("{} dangling", dangling.len()),
        "run `aw bootstrap` to re-link, or remove the stale entry",
    );
    for path in &dangling {
        println!("      {:<22} {}", "", path.display());
    }

    Ok(ok)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_name_strips_the_workspace_suffix() {
        assert_eq!(default_name(Path::new("/x/demo.workspace")), "demo");
        assert_eq!(default_name(Path::new("/x/plain")), "plain");
    }

    #[test]
    fn validate_name_rejects_manifest_breaking_characters() {
        assert!(validate_name("client-alpha").is_ok());
        assert!(validate_name("bad\"quote").is_err());
        assert!(validate_name("bad\\slash").is_err());
        assert!(validate_name("bad\nnewline").is_err());
        assert!(validate_name("   ").is_err());
    }
}
