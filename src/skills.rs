//! Skill discovery and linking.
//!
//! Agent Skills is an open standard: a skill is a directory containing
//! `SKILL.md`. Harnesses differ only in where they look for that directory, so
//! consumption is a symlink from each harness's discovery path to one canonical
//! skill directory. There is no package manager and no registry.

use crate::manifest::Manifest;
use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

/// Harness discovery paths, relative to the workspace root.
///
/// This table is the one place that knows about specific agent harnesses.
/// Adding support for another harness means adding a row here.
pub const HARNESSES: &[Harness] = &[
    Harness {
        name: "Claude Code",
        dir: ".claude/skills",
    },
    Harness {
        name: "Codex",
        dir: ".agents/skills",
    },
];

pub struct Harness {
    pub name: &'static str,
    pub dir: &'static str,
}

/// Workspace-local skills; highest precedence.
pub const LOCAL_DIR: &str = ".skills";

/// Directories inside a member repository that may expose skills, relative to
/// its checkout root.
///
/// `.claude/skills` is an ordinary project's discovery path. `public` and
/// `private` are the split a dedicated skills repository uses — which is itself
/// just a member repository that happens to contain nothing else. There is
/// deliberately no repository-root fallback: it would sweep up arbitrary
/// directories that merely happen to hold a `SKILL.md`.
const MEMBER_DIRS: &[&str] = &[".claude/skills", "public", "private"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Origin {
    Workspace,
    Member,
}

impl Origin {
    pub fn label(self) -> &'static str {
        match self {
            Self::Workspace => "workspace",
            Self::Member => "member repo",
        }
    }
}

#[derive(Debug)]
pub struct Skill {
    pub name: String,
    pub target: PathBuf,
    pub origin: Origin,
}

#[derive(Debug)]
pub struct Resolution {
    pub linked: Vec<Skill>,
    /// Skills that lost a name collision. Shadowing is always reported, never
    /// silent.
    pub shadowed: Vec<(Skill, Origin)>,
}

/// Resolve every skill the manifest makes available, applying precedence:
/// workspace-local beats opted-in member repositories, which are considered in
/// manifest order. First claim wins; every loser is reported.
pub fn resolve(root: &Path, manifest: &Manifest) -> Result<Resolution> {
    let mut candidates = Vec::new();

    for target in scan_dir(&root.join(LOCAL_DIR))? {
        let name = dir_name(&target)?;
        candidates.push(Skill {
            name,
            target,
            origin: Origin::Workspace,
        });
    }

    for repo in &manifest.repos {
        let Some(filter) = repo.skill_filter() else {
            continue;
        };
        let checkout = root.join(&repo.path);
        for target in scan_member(&checkout)? {
            let skill = dir_name(&target)?;
            if !filter.admits(&skill) {
                continue;
            }
            let name = if repo.skill_prefix {
                format!("{}--{skill}", repo.tree_name())
            } else {
                skill
            };
            candidates.push(Skill {
                name,
                target,
                origin: Origin::Member,
            });
        }
    }

    let mut claimed: BTreeMap<String, Skill> = BTreeMap::new();
    let mut shadowed = Vec::new();
    for skill in candidates {
        match claimed.get(&skill.name) {
            Some(winner) => shadowed.push((skill, winner.origin)),
            None => {
                claimed.insert(skill.name.clone(), skill);
            }
        }
    }

    Ok(Resolution {
        linked: claimed.into_values().collect(),
        shadowed,
    })
}

/// Create the symlinks for every harness whose discovery directory exists or
/// can be created inside the workspace. Returns a per-harness change count.
///
/// Managed symlinks that no longer correspond to a resolved skill are removed;
/// real directories are never touched, only reported by the caller.
pub fn link(root: &Path, resolution: &Resolution) -> Result<Vec<(&'static str, usize)>> {
    let mut report = Vec::new();
    for harness in HARNESSES {
        let dir = root.join(harness.dir);
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;

        let mut changed = 0;
        for skill in &resolution.linked {
            let link = dir.join(&skill.name);
            let want = relative_from(&dir, &skill.target)?;
            if std::fs::read_link(&link).is_ok_and(|existing| existing == want) {
                continue;
            }
            if link.is_symlink() {
                std::fs::remove_file(&link)
                    .with_context(|| format!("replacing stale link {}", link.display()))?;
            } else if link.exists() {
                // A real directory here is the user's own skill. Leave it.
                continue;
            }
            std::os::unix::fs::symlink(&want, &link)
                .with_context(|| format!("linking {}", link.display()))?;
            changed += 1;
        }

        changed += prune(root, &dir, resolution)?;
        report.push((harness.name, changed));
    }
    Ok(report)
}

/// Remove symlinks in a discovery directory that no longer name a resolved
/// skill. Only symlinks are removed — never real files or directories — and only
/// those pointing at a skill source inside the workspace. `aw` links skills
/// from `.skills/` and member checkouts, both inside `root`; a link whose target
/// escapes the workspace was placed by the user, so it is left untouched.
fn prune(root: &Path, dir: &Path, resolution: &Resolution) -> Result<usize> {
    let mut removed = 0;
    for entry in read_dir(dir)? {
        let path = entry.path();
        if !path.is_symlink() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if resolution.linked.iter().any(|s| s.name == name) {
            continue;
        }
        let Ok(target) = std::fs::read_link(&path) else {
            continue;
        };
        if !normalise(&dir.join(target)).starts_with(root) {
            continue;
        }
        std::fs::remove_file(&path)
            .with_context(|| format!("removing stale link {}", path.display()))?;
        removed += 1;
    }
    Ok(removed)
}

/// Symlinks in harness discovery directories whose target no longer resolves.
pub fn dangling(root: &Path) -> Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    for harness in HARNESSES {
        for entry in read_dir(&root.join(harness.dir))? {
            let path = entry.path();
            if path.is_symlink() && !path.exists() {
                found.push(path);
            }
        }
    }
    Ok(found)
}

/// Immediate sub-directories that are skills, sorted for deterministic output.
fn scan_dir(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut found: Vec<PathBuf> = read_dir(dir)?
        .into_iter()
        .map(|entry| entry.path())
        .filter(|path| path.join("SKILL.md").is_file())
        .collect();
    found.sort();
    Ok(found)
}

/// Every skill an opted-in member repository exposes, across all the layouts a
/// member may use. Absent directories are simply skipped.
fn scan_member(checkout: &Path) -> Result<Vec<PathBuf>> {
    let mut all = Vec::new();
    for dir in MEMBER_DIRS {
        all.extend(scan_dir(&checkout.join(dir))?);
    }
    Ok(all)
}

/// Directory entries, treating a missing directory as empty rather than an
/// error — an unprovisioned workspace is a normal state, not a failure.
fn read_dir(dir: &Path) -> Result<Vec<std::fs::DirEntry>> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    std::fs::read_dir(dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .collect::<std::io::Result<Vec<_>>>()
        .with_context(|| format!("reading entries of {}", dir.display()))
}

fn dir_name(path: &Path) -> Result<String> {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .with_context(|| format!("skill directory has no name: {}", path.display()))
}

/// Relative path from `from` to `to`, so links survive the workspace being
/// moved or cloned to a different absolute location.
fn relative_from(from: &Path, to: &Path) -> Result<PathBuf> {
    let from = normalise(from);
    let to = normalise(to);
    let shared = from
        .components()
        .zip(to.components())
        .take_while(|(a, b)| a == b)
        .count();
    anyhow::ensure!(
        shared > 0,
        "cannot relate {} to {}: no common ancestor",
        from.display(),
        to.display()
    );
    let mut path = PathBuf::new();
    for _ in from.components().skip(shared) {
        path.push("..");
    }
    path.extend(to.components().skip(shared));
    Ok(path)
}

/// Resolve `.` and `..` textually. The targets may not exist yet, so this
/// cannot go through `canonicalize`.
fn normalise(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relates_sibling_directories() {
        let rel = relative_from(
            Path::new("/ws/.claude/skills"),
            Path::new("/ws/.skills/review"),
        )
        .expect("shares /ws");
        assert_eq!(rel, Path::new("../../.skills/review"));
    }

    #[test]
    fn relates_a_member_repository_skill() {
        let rel = relative_from(
            Path::new("/home/me/ws/.agents/skills"),
            Path::new("/home/me/ws/skills/public/release"),
        )
        .expect("shares /home/me/ws");
        assert_eq!(rel, Path::new("../../skills/public/release"));
    }

    #[test]
    fn normalises_parent_traversal() {
        assert_eq!(normalise(Path::new("/a/b/../c/./d")), Path::new("/a/c/d"));
    }

    #[test]
    fn prune_removes_stale_workspace_links_but_keeps_external_ones() {
        let base = std::env::temp_dir().join("aw-test-prune");
        let _ = std::fs::remove_dir_all(&base);
        let root = base.join("ws");
        let dir = root.join(".claude/skills");
        std::fs::create_dir_all(&dir).expect("discovery dir");
        let outside = base.join("outside");
        std::fs::create_dir_all(&outside).expect("outside dir");

        // Stale link into the workspace (target now gone): prunable.
        std::os::unix::fs::symlink("../../.skills/gone", dir.join("stale")).expect("stale link");
        // User link escaping the workspace: must survive.
        std::os::unix::fs::symlink(&outside, dir.join("mine")).expect("user link");

        let resolution = Resolution {
            linked: Vec::new(),
            shadowed: Vec::new(),
        };
        let removed = prune(&root, &dir, &resolution).expect("prune");

        assert_eq!(removed, 1, "only the in-workspace stale link is removed");
        assert!(!dir.join("stale").is_symlink(), "stale link removed");
        assert!(dir.join("mine").is_symlink(), "external user link kept");

        let _ = std::fs::remove_dir_all(&base);
    }
}
