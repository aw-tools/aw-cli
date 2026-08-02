//! Refresh remote-tracking refs and workspace skill links.

use crate::git::FetchOutcome;
use crate::{
    garden, git,
    manifest::{self, Manifest},
    reporting, skills,
};
use anyhow::Result;
use std::path::Path;

/// Name the workspace root reports under, matching `aw status`'s own label for
/// it.
const WORKSPACE_LABEL: &str = "workspace";

/// Fetch the workspace layer and every present member, and re-run skill
/// linking, without changing any working tree or checked-out commit.
pub fn run(root: &Path) -> Result<bool> {
    let manifest = Manifest::load(root)?;
    println!("{}", reporting::sync_workspace(&manifest.workspace.name));

    let mut present = 0;
    let mut skipped = 0;
    let mut failed = 0;

    // The workspace root is a repository too, and `aw status` reports its
    // ahead/behind alongside the members'. Fetch it here so that report reads a
    // ref something refreshes. Guard on the checkout the same way the member
    // loop does: the root is a workspace because it carries the manifest, not
    // because it is a repository, and `git config --local` in a directory that
    // is not one either fails outright or answers for an enclosing repository.
    //
    // The layer stays out of the three tallies below, which count members only,
    // and carries its own failure flag into the exit status instead.
    let mut layer_failed = false;
    if !git::is_repo_checked(root)? {
        println!("{}", reporting::sync_layer_absent(WORKSPACE_LABEL));
    } else if git::has_origin(root)? {
        match git::fetch(root)? {
            FetchOutcome::Fetched => println!("{}", reporting::sync_layer_fetched(WORKSPACE_LABEL)),
            FetchOutcome::Failed(why) => {
                layer_failed = true;
                println!("{}", reporting::sync_layer_failed(WORKSPACE_LABEL, &why));
            }
        }
    } else {
        println!("{}", reporting::sync_layer_no_origin(WORKSPACE_LABEL));
    }

    for repo in &manifest.repos {
        let path = garden::checkout_path(root, &repo.path);
        if !git::is_repo_checked(&path)? {
            skipped += 1;
            println!("{}", reporting::sync_repo_skipped(&repo.path));
            continue;
        }
        present += 1;
        let path = match manifest::canonicalise_member(root, &path) {
            Ok((path, _)) => path,
            Err(error) => {
                failed += 1;
                println!(
                    "{}",
                    reporting::sync_repo_failed(&repo.path, &format!("unsafe checkout: {error}"))
                );
                continue;
            }
        };
        match git::fetch(&path)? {
            FetchOutcome::Fetched => println!("{}", reporting::sync_repo_fetched(&repo.path)),
            FetchOutcome::Failed(why) => {
                failed += 1;
                println!("{}", reporting::sync_repo_failed(&repo.path, &why));
            }
        }
    }

    let resolution = skills::resolve(root, &manifest)?;
    for (harness, changed) in skills::link(root, &resolution)? {
        println!("{}", reporting::sync_skills(harness, changed));
    }
    for skill in &resolution.linked {
        println!(
            "{}",
            reporting::sync_skill(&skill.name, skill.origin.label())
        );
    }
    for (loser, winner) in &resolution.shadowed {
        let path = loser.target.strip_prefix(root).unwrap_or(&loser.target);
        println!(
            "{}",
            reporting::sync_shadowed(&loser.name, &path.display().to_string(), winner.label())
        );
    }
    for missing in &resolution.missing_dirs {
        println!(
            "{}",
            reporting::sync_missing_skill_dir(&missing.repo, &missing.dir)
        );
    }
    for unmatched in &resolution.unmatched_only {
        println!(
            "{}",
            reporting::sync_unmatched_only(&unmatched.repo, &unmatched.entry)
        );
    }

    println!(
        "{}",
        reporting::sync_verify(
            present,
            skipped,
            failed,
            resolution.linked.len(),
            resolution.shadowed.len()
        )
    );
    Ok(failed == 0 && !layer_failed)
}
