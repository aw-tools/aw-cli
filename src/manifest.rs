//! `workspace.toml` — the single source of truth for a workspace.
//!
//! Everything `aw` does is derived from this file. Nothing else on disk is
//! authoritative: `.aw/trees.yaml` is generated from it, and skill links are
//! computed from it.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

pub const FILENAME: &str = "workspace.toml";

#[derive(Debug, Deserialize)]
pub struct Manifest {
    pub workspace: Workspace,
    #[serde(default)]
    pub identity: Option<Identity>,
    #[serde(default, rename = "repo")]
    pub repos: Vec<Repo>,
}

#[derive(Debug, Deserialize)]
pub struct Workspace {
    pub name: String,
}

/// Repository-local git identity, applied to every managed repository.
///
/// Repository-local configuration beats user and system configuration, which
/// makes a workspace portable to any machine or runner without relying on the
/// host's global git setup.
#[derive(Debug, Deserialize)]
pub struct Identity {
    pub name: Option<String>,
    pub email: Option<String>,
    pub signingkey: Option<String>,
    pub gpgsign: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct Repo {
    pub path: String,
    pub url: String,
    pub branch: Option<String>,
    /// Opt-in to skill propagation. Absent means off: skills are never loaded
    /// from a repository that has not explicitly opted in.
    #[serde(default)]
    pub skills: Option<SkillOptIn>,
    /// Prefix this repository's skills with its tree name when linking. Off by
    /// default: a skill's directory name is the name it declares about itself,
    /// and renaming it breaks both that declaration and any cross-reference
    /// between skills in the same repository.
    #[serde(default, rename = "skill-prefix")]
    pub skill_prefix: bool,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum SkillOptIn {
    All(bool),
    Named(Vec<String>),
}

impl Repo {
    /// Garden tree name for this entry.
    ///
    /// Derived from the path so it stays stable across manifest reordering,
    /// and flattened so nested checkouts cannot collide with sibling names.
    pub fn tree_name(&self) -> String {
        self.path
            .trim_matches('/')
            .replace(['/', '.'], "-")
            .trim_matches('-')
            .to_owned()
    }

    /// Which named skills this repository exposes, if it opted in at all.
    pub fn skill_filter(&self) -> Option<SkillFilter<'_>> {
        match self.skills.as_ref()? {
            SkillOptIn::All(false) => None,
            SkillOptIn::All(true) => Some(SkillFilter::All),
            SkillOptIn::Named(names) => Some(SkillFilter::Named(names)),
        }
    }
}

pub enum SkillFilter<'a> {
    All,
    Named(&'a [String]),
}

impl SkillFilter<'_> {
    pub fn admits(&self, name: &str) -> bool {
        match self {
            Self::All => true,
            Self::Named(names) => names.iter().any(|n| n == name),
        }
    }
}

impl Manifest {
    pub fn load(root: &Path) -> Result<Self> {
        let path = root.join(FILENAME);
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading manifest {}", path.display()))?;
        let manifest: Self = toml::from_str(&text)
            .with_context(|| format!("parsing manifest {}", path.display()))?;
        manifest
            .validate()
            .with_context(|| format!("in manifest {}", path.display()))?;
        Ok(manifest)
    }

    fn validate(&self) -> Result<()> {
        let mut seen = std::collections::BTreeSet::new();
        for repo in &self.repos {
            check_contained(&repo.path)?;
            let tree = repo.tree_name();
            anyhow::ensure!(
                !tree.is_empty(),
                "repo path {} has no usable tree name; use a path with at least one alphanumeric segment",
                repo.path
            );
            anyhow::ensure!(
                seen.insert(tree.clone()),
                "repo path {} collides with another entry on tree name {tree:?}; give the repositories distinct paths",
                repo.path
            );
        }
        Ok(())
    }
}

/// A workspace contains everything it manages. A checkout that escapes the root
/// makes the workspace non-portable and behaves differently depending on who
/// runs the bootstrap — it works in a terminal and fails under a sandboxed
/// agent, which is exactly the divergence a reproducible workspace exists to
/// prevent.
fn check_contained(path: &str) -> Result<()> {
    anyhow::ensure!(!path.trim().is_empty(), "repo path is empty");
    let path = Path::new(path);
    anyhow::ensure!(
        path.is_relative(),
        "repo path {} is absolute; paths must be inside the workspace",
        path.display()
    );
    anyhow::ensure!(
        !path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir)),
        "repo path {} escapes the workspace with `..`; paths must be inside the workspace",
        path.display()
    );
    Ok(())
}

/// Locate the workspace root: the given directory, or the nearest ancestor of
/// the current directory that contains a manifest.
pub fn resolve_root(explicit: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(dir) = explicit {
        let dir =
            std::fs::canonicalize(&dir).with_context(|| format!("resolving {}", dir.display()))?;
        anyhow::ensure!(
            dir.join(FILENAME).is_file(),
            "{} is not a workspace: no {FILENAME}",
            dir.display()
        );
        return Ok(dir);
    }

    let mut dir = std::env::current_dir().context("resolving current directory")?;
    loop {
        if dir.join(FILENAME).is_file() {
            return Ok(dir);
        }
        anyhow::ensure!(
            dir.pop(),
            "not inside a workspace: no {FILENAME} in any parent"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_a_nested_path() {
        check_contained("vendor/skills").expect("stays inside the workspace");
    }

    #[test]
    fn rejects_parent_traversal() {
        let err = check_contained("../skills").expect_err("escapes the workspace");
        assert!(err.to_string().contains("escapes the workspace"));
    }

    #[test]
    fn rejects_an_absolute_path() {
        let err = check_contained("/srv/skills").expect_err("is outside the workspace");
        assert!(err.to_string().contains("absolute"));
    }

    #[test]
    fn rejects_colliding_tree_names() {
        let manifest: Manifest = toml::from_str(
            "[workspace]\nname = \"w\"\n\
             [[repo]]\npath = \"vendor/alpha\"\nurl = \"u\"\n\
             [[repo]]\npath = \"vendor-alpha\"\nurl = \"u\"\n",
        )
        .expect("fixture parses");
        let err = manifest.validate().expect_err("colliding tree names");
        assert!(err.to_string().contains("collides"), "{err}");
    }

    #[test]
    fn rejects_a_path_with_no_tree_name() {
        let manifest: Manifest =
            toml::from_str("[workspace]\nname = \"w\"\n[[repo]]\npath = \".\"\nurl = \"u\"\n")
                .expect("fixture parses");
        let err = manifest.validate().expect_err("empty tree name");
        assert!(err.to_string().contains("no usable tree name"), "{err}");
    }
}
