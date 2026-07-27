//! Refresh remote-tracking refs and workspace skill links.

use crate::git::FetchOutcome;
use crate::{garden, git, manifest::Manifest, reporting, skills};
use anyhow::Result;
use std::path::Path;

/// Fetch every present member and re-run skill linking without changing a
/// member's working tree or checked-out commit.
pub fn run(root: &Path) -> Result<bool> {
    let manifest = Manifest::load(root)?;
    println!("{}", reporting::sync_workspace(&manifest.workspace.name));

    let mut present = 0;
    let mut skipped = 0;
    let mut failed = 0;
    for repo in &manifest.repos {
        let path = garden::checkout_path(root, &repo.path);
        if !git::is_repo_checked(&path)? {
            skipped += 1;
            println!("{}", reporting::sync_repo_skipped(&repo.path));
            continue;
        }
        present += 1;
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
        println!(
            "{}",
            reporting::sync_shadowed(&loser.name, loser.origin.label(), winner.label())
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
    Ok(failed == 0)
}
