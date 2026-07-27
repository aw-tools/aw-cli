//! Adopt an existing checkout into the workspace manifest.

use crate::git;
use crate::manifest::{self, Manifest, Repo};
use anyhow::{Context, Result};
use std::path::Path;

pub fn run(path: &Path) -> Result<()> {
    let root = manifest::resolve_root(None)?;
    let root = std::fs::canonicalize(&root)
        .with_context(|| format!("resolving workspace root {}", root.display()))?;
    let (checkout, relative) = manifest::canonicalise_member(&root, path)?;
    anyhow::ensure!(
        git::is_repo_checked(&checkout)?,
        "{} is not a git repository",
        checkout.display()
    );
    let worktree_root = git::worktree_root(&checkout)?;
    anyhow::ensure!(
        worktree_root == checkout,
        "checkout {} is not the git worktree root {}",
        checkout.display(),
        worktree_root.display()
    );
    ensure_metadata_contained(
        &root,
        &git::per_worktree_git_dir(&checkout)?,
        "git directory",
    )?;
    ensure_metadata_contained(
        &root,
        &git::common_git_dir(&checkout)?,
        "git common directory",
    )?;

    let manifest = Manifest::load(&root)?;
    let mut repo = Repo {
        path: relative,
        url: String::new(),
        branch: None,
        skills: None,
        skill_prefix: false,
    };
    let tree_name = repo.tree_name();
    anyhow::ensure!(
        !tree_name.is_empty(),
        "repo path {} has no usable tree name; use a path with at least one alphanumeric segment",
        repo.path
    );
    for existing in &manifest.repos {
        if let Ok(existing_checkout) = std::fs::canonicalize(root.join(&existing.path)) {
            anyhow::ensure!(
                existing_checkout != checkout,
                "repo path {} is already in the manifest as {}",
                repo.path,
                existing.path
            );
        }
        anyhow::ensure!(
            existing.path != repo.path,
            "repo path {} is already in the manifest",
            repo.path
        );
        anyhow::ensure!(
            existing.tree_name() != tree_name,
            "repo path {} collides with manifest entry {} on tree name {tree_name:?}",
            repo.path,
            existing.path
        );
    }

    repo.url = git::config_value(&checkout, "remote.origin.url")?
        .filter(|url| !url.trim().is_empty())
        .with_context(|| format!("checkout {} has no origin remote", checkout.display()))?;
    let branch = git::checked_out_branch(&checkout)
        .with_context(|| format!("checkout {} has no checked-out branch", checkout.display()))?;
    anyhow::ensure!(
        !branch.is_empty(),
        "checkout {} has no checked-out branch",
        checkout.display()
    );
    repo.branch = Some(branch);
    manifest::append_repo(&root, &repo)?;
    println!(
        "added repo  path={}  url={}  branch={}",
        repo.path,
        repo.url,
        repo.branch.as_deref().unwrap_or_default()
    );
    eprintln!("Review workspace.toml with `git diff`, then commit it; `aw adopt` made no commit.");
    Ok(())
}

fn ensure_metadata_contained(root: &Path, metadata: &Path, description: &str) -> Result<()> {
    anyhow::ensure!(
        metadata.starts_with(root),
        "{description} {} is outside the workspace {}",
        metadata.display(),
        root.display()
    );
    Ok(())
}
