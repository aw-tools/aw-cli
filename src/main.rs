//! `aw` — provision and report on reproducible multi-repository agentic
//! workspaces.
//!
//! A thin orchestrator. Cloning and per-repository git configuration are
//! garden's job; `aw` owns the manifest, skill linking, and reporting.

mod adopt;
mod delivery;
mod garden;
mod git;
mod manifest;
mod reporting;
mod skills;
mod status;
mod sync;
mod template;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use manifest::Manifest;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const HOOKS_DIR: &str = ".githooks";
const HOOKS_CONFIG_KEY: &str = "core.hooksPath";
const HOOKS_PRE_COMMIT: &str = "pre-commit";

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
    /// Report repository presence and working-tree state.
    Status {
        /// Emit JSON (unstable during incubation).
        #[arg(long)]
        json: bool,
        /// Exit non-zero when bootstrap-convergeable findings are present.
        #[arg(long)]
        exit_code: bool,
        /// Workspace root. Defaults to the nearest ancestor with a manifest.
        dir: Option<PathBuf>,
    },
    /// Fetch managed repositories and report changes.
    Sync {
        /// Workspace root. Defaults to the nearest ancestor with a manifest.
        dir: Option<PathBuf>,
    },
    /// Add an existing checkout to the workspace manifest.
    Adopt {
        /// Existing checkout to add.
        path: PathBuf,
    },
}

fn main() -> ExitCode {
    // A Ctrl-C during a bounded git operation must tear down the child's
    // process group, which `process_group(0)` has moved out of the terminal's
    // foreground group; without this the child and its helpers would orphan.
    git::install_interrupt_forwarder();
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
        Verb::Status {
            dir,
            json,
            exit_code,
        } => status(&manifest::resolve_root(dir)?, json, exit_code),
        Verb::Sync { dir } => sync::run(&manifest::resolve_root(dir)?),
        Verb::Adopt { path } => adopt::run(&path).map(|()| true),
    }
}

fn status(root: &Path, json: bool, exit_code: bool) -> Result<bool> {
    let report = status::build(root)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!("{}", report.render_human());
    }
    Ok(!exit_code || !report.has_findings())
}

fn init(dir: Option<PathBuf>, name: Option<String>, template: Option<&str>) -> Result<()> {
    // Validate an explicit name before any filesystem or template side effect,
    // so a rejected name leaves no created directory behind. The derived
    // default is validated below, where the resolved root is available.
    if let Some(name) = name.as_deref() {
        validate_name(name)?;
    }

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
    let name = if let Some(name) = name {
        name
    } else {
        let name = default_name(&root);
        validate_name(&name)?;
        name
    };
    let created = template::materialise(&root, &prepared)?;
    if created.iter().any(|path| path == manifest::FILENAME) {
        set_workspace_name(&root, &name)?;
    }
    let manifest = manifest::record_template(
        &root,
        &source.url,
        source.reference.as_deref().unwrap_or(""),
        &prepared.sha,
    )?;

    if git::is_repo(&root) {
        println!("{}", reporting::init_existing_repo());
    } else {
        git::init(&root)?;
        println!("{}", reporting::init_new_repo());
    }

    for (key, value) in garden::identity_pairs(manifest.identity.as_ref()) {
        git::set_config(&root, key, &value)?;
    }

    println!(
        "{}",
        reporting::init_source(
            &source.url,
            source.reference.as_deref().unwrap_or(""),
            &prepared.sha
        )
    );
    if created.is_empty() {
        println!("{}", reporting::init_unchanged_template());
    } else {
        for path in &created {
            println!("{}", reporting::init_created_template_path(path));
        }
    }
    println!("{}", reporting::init_name(&name));

    eprintln!();
    eprintln!("{}", reporting::init_next_step());
    eprintln!("{}", reporting::init_no_commit());
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
    let updated = manifest::replace_workspace_name(&text, name)?;
    std::fs::write(&path, updated).with_context(|| format!("writing {}", path.display()))
}

/// Returns whether every managed repository is present. A repo still missing
/// after garden reports success is a failure the exit code must reflect, so a
/// script or agent driving `aw` does not read it as a clean run.
fn bootstrap(root: &Path) -> Result<bool> {
    let manifest = Manifest::load(root)?;
    println!(
        "{}",
        reporting::bootstrap_workspace(&manifest.workspace.name)
    );

    // Phase 1 — repositories.
    let regenerated = garden::write_trees(root, &manifest)?;
    println!(
        "{}",
        reporting::bootstrap_trees(regenerated, manifest.repos.len())
    );
    garden::grow(root, &manifest)?;
    for repo in &manifest.repos {
        let path = garden::checkout_path(root, &repo.path);
        println!(
            "{}",
            reporting::bootstrap_repo(&repo.path, git::is_repo(&path))
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
    println!("{}", reporting::bootstrap_config(changed, pairs.len()));
    if root.join(HOOKS_DIR).is_dir() {
        let changed = git::set_config_if_unset(root, HOOKS_CONFIG_KEY, HOOKS_DIR)?;
        let value = git::config_value(root, HOOKS_CONFIG_KEY)?.unwrap_or_default();
        println!("{}", reporting::bootstrap_hooks(changed, &value));
    }

    // Phase 3 — skills.
    let resolution = skills::resolve(root, &manifest)?;
    for (harness, changed) in skills::link(root, &resolution)? {
        println!("{}", reporting::bootstrap_skills(harness, changed));
    }
    for skill in &resolution.linked {
        println!(
            "{}",
            reporting::bootstrap_skill(&skill.name, skill.origin.label())
        );
    }
    for (loser, winner) in &resolution.shadowed {
        let path = loser.target.strip_prefix(root).unwrap_or(&loser.target);
        println!(
            "{}",
            reporting::bootstrap_shadowed(&loser.name, &path.display().to_string(), winner.label())
        );
    }
    for missing in &resolution.missing_dirs {
        println!(
            "{}",
            reporting::bootstrap_missing_skill_dir(&missing.repo, &missing.dir)
        );
    }
    for unmatched in &resolution.unmatched_only {
        println!(
            "{}",
            reporting::bootstrap_unmatched_only(&unmatched.repo, &unmatched.entry)
        );
    }

    // Phase 4 — verify.
    let missing = manifest
        .repos
        .iter()
        .filter(|r| !git::is_repo(&garden::checkout_path(root, &r.path)))
        .count();
    println!(
        "{}",
        reporting::bootstrap_verify(
            manifest.repos.len(),
            missing,
            resolution.linked.len(),
            resolution.shadowed.len()
        )
    );
    Ok(missing == 0)
}

/// Whether `<hooks_dir>/pre-commit` would actually run: a regular file with the
/// owner-execute bit set. A proxy for git's own `access(X_OK)` gate — an absent
/// file, a broken symlink, or a directory named `pre-commit` all read as dead —
/// matched to the common single-owner `0755` workspace hook rather than to
/// git's full permission logic. `metadata` follows symlinks, so a live link to
/// an executable target reads live and a dangling one reads dead.
fn hook_is_live(hooks_dir: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(hooks_dir.join(HOOKS_PRE_COMMIT))
        .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o100 != 0)
}

fn doctor(root: &Path) -> Result<bool> {
    let mut ok = true;
    let mut check = |pass: bool, report: reporting::DoctorCheck| {
        println!("{}", reporting::doctor_check(pass, &report));
        if !pass {
            println!("{}", reporting::doctor_remedy(&report));
            ok = false;
        }
    };

    match git::version() {
        Ok(v) => check(true, reporting::doctor_git(&v)),
        Err(e) => check(false, reporting::doctor_git_error(&e)),
    }

    match garden::version() {
        Ok((major, minor)) => {
            let (want_major, want_minor) = garden::MIN_VERSION;
            let recent = (major, minor) >= (want_major, want_minor);
            check(
                recent,
                reporting::doctor_garden(major, minor, want_major, want_minor),
            );
        }
        Err(e) => check(false, reporting::doctor_garden_error(&e)),
    }

    let config = root.join(garden::CONFIG_FILE);
    check(config.is_file(), reporting::doctor_garden_config(&config));

    // A garden config whose include is missing resolves to zero trees and
    // exits successfully, so an absent generated file is silent at the garden
    // layer and has to be caught here.
    let generated = root.join(garden::GENERATED_FILE);
    check(
        generated.is_file(),
        reporting::doctor_generated_trees(&generated),
    );

    let hooks_dir = root.join(HOOKS_DIR);
    if hooks_dir.is_dir() {
        let configured = git::config_value(root, HOOKS_CONFIG_KEY)?;
        let pass = configured.as_deref() == Some(HOOKS_DIR);
        check(
            pass,
            reporting::doctor_pre_commit_hook(configured.as_deref()),
        );
        // Only when the redirect is correct does a dead hook become the live
        // concern; an unset or redirected `hooksPath` already carries its own
        // actionable remedy above.
        if pass {
            let live = hook_is_live(&hooks_dir);
            check(
                live,
                reporting::doctor_hook_live(&hooks_dir.join(HOOKS_PRE_COMMIT), live),
            );
        }
    }

    let manifest = Manifest::load(root)?;
    for repo in &manifest.repos {
        match git::probe_remote(&repo.url, repo.branch.as_deref()) {
            git::RemoteProbe::Reachable => {
                check(true, reporting::doctor_remote_reachable(&repo.path));
            }
            git::RemoteProbe::Denied(why) => {
                check(false, reporting::doctor_remote_denied(&repo.path, &why));
            }
            git::RemoteProbe::Unreachable(why) => {
                check(
                    false,
                    reporting::doctor_remote_unreachable(&repo.path, &why),
                );
            }
            git::RemoteProbe::MissingBranch(branch) => {
                check(
                    false,
                    reporting::doctor_remote_missing_branch(&repo.path, &branch),
                );
            }
        }
        // A member's delivery model is authoritative in its own instruction
        // files, read live (decision 45). Surface whether one is declared as a
        // plain informational line, outside the `check` closure: an absent model
        // is the conservative default, never a fault, so it must not touch `ok`.
        let declared = delivery::declared_in(&root.join(&repo.path));
        println!("{}", reporting::doctor_delivery_model(&repo.path, declared));
    }

    for harness in skills::HARNESSES {
        let dir = root.join(harness.dir);
        check(
            dir.is_dir(),
            reporting::doctor_discovery_dir(harness.name, &dir),
        );
    }

    let resolution = skills::resolve(root, &manifest)?;
    for missing in &resolution.missing_dirs {
        check(
            false,
            reporting::doctor_skill_dir(&missing.repo, &missing.dir),
        );
    }
    for unmatched in &resolution.unmatched_only {
        check(
            false,
            reporting::doctor_skill_unmatched_only(&unmatched.repo, &unmatched.entry),
        );
    }

    let dangling = skills::dangling(root)?;
    check(
        dangling.is_empty(),
        reporting::doctor_skill_links(dangling.len()),
    );
    for path in &dangling {
        println!("{}", reporting::doctor_dangling_link(path));
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

    #[test]
    fn init_rejects_invalid_explicit_name_without_creating_the_directory() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("workspace");
        let result = init(Some(target.clone()), Some("bad\"quote".to_owned()), None);
        assert!(result.is_err(), "an invalid explicit name must fail init");
        assert!(
            !target.exists(),
            "an invalid explicit name must not create the target directory"
        );
    }

    fn write_hook(dir: &Path, mode: u32) {
        use std::os::unix::fs::PermissionsExt;
        let hook = dir.join(HOOKS_PRE_COMMIT);
        std::fs::write(&hook, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(mode)).unwrap();
    }

    #[test]
    fn hook_is_live_for_an_executable_regular_file() {
        let temp = tempfile::tempdir().unwrap();
        write_hook(temp.path(), 0o755);
        assert!(hook_is_live(temp.path()));
    }

    #[test]
    fn hook_is_dead_when_not_executable() {
        let temp = tempfile::tempdir().unwrap();
        write_hook(temp.path(), 0o644);
        assert!(!hook_is_live(temp.path()));
    }

    #[test]
    fn hook_is_dead_when_absent() {
        let temp = tempfile::tempdir().unwrap();
        assert!(!hook_is_live(temp.path()));
    }

    #[test]
    fn hook_is_dead_for_a_broken_symlink() {
        let temp = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(
            temp.path().join("nowhere"),
            temp.path().join(HOOKS_PRE_COMMIT),
        )
        .unwrap();
        assert!(!hook_is_live(temp.path()));
    }

    #[test]
    fn workspace_name_rewrite_does_not_touch_identity() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        std::fs::write(
            root.join(manifest::FILENAME),
            "[identity]\nname = \"CHANGEME\"\n\n[workspace]\nname = \"CHANGEME\"\n",
        )
        .unwrap();

        set_workspace_name(root, "demo").unwrap();

        let text = std::fs::read_to_string(root.join(manifest::FILENAME)).unwrap();
        assert!(text.contains("[identity]\nname = \"CHANGEME\""));
        assert!(text.contains("[workspace]\nname = \"demo\""));
    }
}
