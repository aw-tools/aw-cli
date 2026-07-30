//! Skill discovery and linking.
//!
//! Agent Skills is an open standard: a skill is a directory containing
//! `SKILL.md`. Harnesses differ only in where they look for that directory, so
//! consumption is a symlink from each harness's discovery path to one canonical
//! skill directory. There is no package manager and no registry.

use crate::manifest::Manifest;
use anyhow::{Context, Result};
use std::collections::{BTreeMap, BTreeSet};
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

/// Source directories scanned inside an opted-in member repository when its
/// manifest entry names none of its own. These are the two harness discovery
/// paths a project already keeps, so an ordinary repository opts in with no
/// further configuration. A repository with a bespoke layout overrides this
/// with an explicit `dirs` list; there is deliberately no repository-root
/// fallback, which would sweep up arbitrary directories that merely happen to
/// hold a `SKILL.md`.
pub const DEFAULT_SKILL_DIRS: &[&str] = &[".agents/skills", ".claude/skills"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Origin {
    Workspace,
    Member,
}

impl Origin {
    pub const fn label(self) -> &'static str {
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

/// An explicitly configured source directory that does not exist on disk. A
/// typo in `dirs` must surface; an absent default directory does not, so only
/// configured directories are recorded here.
#[derive(Debug)]
pub struct MissingDir {
    pub repo: String,
    pub dir: String,
}

/// An `only` allowlist entry that matched no discovered skill. A typo in `only`
/// must surface, just as a typo in `dirs` does; it is independent of a missing
/// directory, so a repository can report both.
#[derive(Debug)]
pub struct UnmatchedOnly {
    pub repo: String,
    pub entry: String,
}

#[derive(Debug)]
pub struct Resolution {
    pub linked: Vec<Skill>,
    /// Skills that lost a name collision. Shadowing is always reported, never
    /// silent.
    pub shadowed: Vec<(Skill, Origin)>,
    /// Explicitly configured member directories that do not exist.
    pub missing_dirs: Vec<MissingDir>,
    /// Configured `only` allowlist entries that matched no discovered skill.
    pub unmatched_only: Vec<UnmatchedOnly>,
}

/// Resolve every skill the manifest makes available, applying precedence:
/// workspace-local beats opted-in member repositories, which are considered in
/// manifest order; within a repository, configured directories in listed order.
/// First claim wins; every loser is reported.
pub fn resolve(root: &Path, manifest: &Manifest) -> Result<Resolution> {
    let mut candidates = Vec::new();
    let mut missing_dirs = Vec::new();
    let mut unmatched_only = Vec::new();

    for target in scan_dir(&root.join(LOCAL_DIR))? {
        let name = dir_name(&target)?;
        candidates.push(Skill {
            name,
            target,
            origin: Origin::Workspace,
        });
    }

    for repo in &manifest.repos {
        let Some(config) = repo.skill_config() else {
            continue;
        };
        let checkout = root.join(&repo.path);
        // The escape boundary for every sweep below, resolved once per repo and
        // lazily: a member that is not yet on disk has no source to sweep, so
        // containment never arises and the checkout stays unresolved; a checkout
        // that cannot be canonicalised for a non-escape reason leaves each sweep
        // the soft skip `scan_dir` already performs for a missing source.
        let canonical_checkout = checkout
            .is_dir()
            .then(|| std::fs::canonicalize(&checkout).ok())
            .flatten();
        // A configured `dirs` list is honoured verbatim; its absence falls back
        // to the convention. Only the configured case reports a missing
        // directory, since most repositories legitimately lack the defaults.
        let configured = config.dirs.is_some();
        let dirs: Vec<&str> = config.dirs.map_or_else(
            || DEFAULT_SKILL_DIRS.to_vec(),
            |dirs| dirs.iter().map(String::as_str).collect(),
        );
        // Seed with every `only` entry and strike each as the scan encounters
        // it, before the `admits` filter so a matched-but-shadowed skill still
        // counts as matched. Whatever remains matched no discovered skill.
        let mut pending: BTreeSet<&str> = config
            .only
            .map(|names| names.iter().map(String::as_str).collect())
            .unwrap_or_default();
        for dir in dirs {
            let source = checkout.join(dir);
            if configured && !source.is_dir() {
                missing_dirs.push(MissingDir {
                    repo: repo.path.clone(),
                    dir: dir.to_owned(),
                });
            }
            // Resolve-and-reject: a source that exists must resolve inside its
            // checkout. The lexical guard in `manifest::check_contained` never
            // touches the filesystem, so a configured or defaulted directory
            // that is a symlink out of the checkout passes validation and would
            // otherwise be swept from outside the repository — breaking the
            // reproducibility the guard exists to protect. A missing or
            // otherwise unresolvable source stays the soft skip `scan_dir`
            // already applies.
            let source = if source.exists() {
                match contained(canonical_checkout.as_deref(), &source) {
                    Contained::Inside(canonical) => canonical,
                    Contained::Escape => anyhow::bail!(
                        "repo {} skills directory {dir:?} resolves outside the \
                         checkout; a skill source must stay inside its repository",
                        repo.path
                    ),
                    Contained::Unresolvable => source,
                }
            } else {
                source
            };
            for target in scan_dir(&source)? {
                let name = dir_name(&target)?;
                pending.remove(name.as_str());
                if !config.admits(&name) {
                    continue;
                }
                // Canonicalising the source parent does not resolve its
                // children, so a skill entry that is itself an escaping symlink
                // must be checked in its own right before it is admitted.
                match contained(canonical_checkout.as_deref(), &target) {
                    Contained::Inside(_) => {}
                    Contained::Escape => anyhow::bail!(
                        "repo {} skill {name:?} resolves outside the checkout; \
                         a skill must stay inside its repository",
                        repo.path
                    ),
                    Contained::Unresolvable => continue,
                }
                candidates.push(Skill {
                    name,
                    target,
                    origin: Origin::Member,
                });
            }
        }
        for entry in pending {
            unmatched_only.push(UnmatchedOnly {
                repo: repo.path.clone(),
                entry: entry.to_owned(),
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
        missing_dirs,
        unmatched_only,
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

#[derive(Debug, PartialEq, Eq)]
pub struct UserOwnedDirectory {
    pub harness: &'static str,
    pub path: PathBuf,
}

/// Real directories in harness discovery paths. These belong to the user;
/// linking and pruning leave them untouched.
pub fn user_owned_directories(root: &Path) -> Result<Vec<UserOwnedDirectory>> {
    let mut found = Vec::new();
    for harness in HARNESSES {
        let mut harness_directories = read_dir(&root.join(harness.dir))?
            .into_iter()
            .map(|entry| entry.path())
            .filter(|path| !path.is_symlink() && path.is_dir())
            .map(|path| UserOwnedDirectory {
                harness: harness.name,
                path,
            })
            .collect::<Vec<_>>();
        harness_directories.sort_by(|left, right| left.path.cmp(&right.path));
        found.extend(harness_directories);
    }
    Ok(found)
}

/// The outcome of resolving a skill source or entry against its checkout root.
enum Contained {
    /// Resolves inside the checkout; the canonical path is safe to sweep.
    Inside(PathBuf),
    /// Resolves outside the checkout — the escape the guard rejects.
    Escape,
    /// Cannot be canonicalised for a reason that is not an escape (the checkout
    /// root is unresolved, a permission error, or a path that vanished after the
    /// caller's existence check). Treated as the soft skip a missing source
    /// already receives, never as an escape.
    Unresolvable,
}

/// Confirm a skill source or entry stays inside its member checkout. This is the
/// filesystem half of `manifest::check_contained`: that guard is purely lexical
/// and cannot see a symlink that leaves the checkout, so the real-path check
/// happens here, at the sweep, over the already-canonical `root`.
fn contained(root: Option<&Path>, path: &Path) -> Contained {
    let (Some(root), Ok(resolved)) = (root, std::fs::canonicalize(path)) else {
        return Contained::Unresolvable;
    };
    if resolved.starts_with(root) {
        Contained::Inside(resolved)
    } else {
        Contained::Escape
    }
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
    use crate::manifest::Manifest;

    /// Create a skill directory `<root>/<relative>` holding a minimal `SKILL.md`.
    fn seed_skill(root: &Path, relative: &str) {
        let dir = root.join(relative);
        std::fs::create_dir_all(&dir).expect("skill dir");
        std::fs::write(dir.join("SKILL.md"), "---\nname: x\n---\n").expect("SKILL.md");
    }

    /// Parse a one-repo manifest at `member` carrying `skills_fragment`.
    fn manifest(skills_fragment: &str) -> Manifest {
        toml::from_str(&format!(
            "[workspace]\nname = \"w\"\n\
             [[repo]]\npath = \"member\"\nurl = \"u\"\n{skills_fragment}"
        ))
        .expect("fixture parses")
    }

    #[test]
    fn scans_configured_dirs_in_listed_order_and_dedupes_across_them() {
        let temp = tempfile::tempdir().expect("temp dir");
        let root = temp.path();
        seed_skill(root, "member/first/deploy");
        seed_skill(root, "member/first/only-first");
        seed_skill(root, "member/second/deploy");
        seed_skill(root, "member/second/only-second");

        let resolution = resolve(
            root,
            &manifest("skills = { dirs = [\"first\", \"second\"] }\n"),
        )
        .expect("resolve");

        let deploy = resolution
            .linked
            .iter()
            .find(|s| s.name == "deploy")
            .expect("deploy resolves");
        assert_eq!(
            deploy.target,
            root.join("member/first/deploy"),
            "the first listed dir wins the name"
        );
        assert!(resolution.linked.iter().any(|s| s.name == "only-first"));
        assert!(resolution.linked.iter().any(|s| s.name == "only-second"));
        assert_eq!(
            resolution.shadowed.len(),
            1,
            "the second dir's deploy is shadowed, not linked twice"
        );
        assert_eq!(
            resolution.shadowed[0].0.target,
            root.join("member/second/deploy")
        );
    }

    #[test]
    fn warns_on_a_missing_configured_dir_but_still_scans_the_rest() {
        let temp = tempfile::tempdir().expect("temp dir");
        let root = temp.path();
        seed_skill(root, "member/present/live");

        let resolution = resolve(
            root,
            &manifest("skills = { dirs = [\"present\", \"absent\"] }\n"),
        )
        .expect("resolve");

        assert!(resolution.linked.iter().any(|s| s.name == "live"));
        assert_eq!(resolution.missing_dirs.len(), 1);
        assert_eq!(resolution.missing_dirs[0].repo, "member");
        assert_eq!(resolution.missing_dirs[0].dir, "absent");
    }

    #[test]
    fn default_dirs_scan_agents_before_claude() {
        let temp = tempfile::tempdir().expect("temp dir");
        let root = temp.path();
        seed_skill(root, "member/.agents/skills/deploy");
        seed_skill(root, "member/.claude/skills/deploy");

        let resolution = resolve(root, &manifest("skills = true\n")).expect("resolve");

        let deploy = resolution
            .linked
            .iter()
            .find(|s| s.name == "deploy")
            .expect("deploy resolves");
        assert_eq!(
            deploy.target,
            root.join("member/.agents/skills/deploy"),
            "the first default dir wins the name"
        );
        assert_eq!(
            resolution.shadowed.len(),
            1,
            "the .claude/skills copy is shadowed, not linked twice"
        );
    }

    #[test]
    fn absent_default_dirs_are_silent() {
        let temp = tempfile::tempdir().expect("temp dir");
        let root = temp.path();

        let resolution = resolve(root, &manifest("skills = true\n")).expect("resolve");

        assert!(resolution.linked.is_empty());
        assert!(
            resolution.missing_dirs.is_empty(),
            "an absent default dir is not a warning"
        );
    }

    #[test]
    fn only_narrows_the_discovered_skills() {
        let temp = tempfile::tempdir().expect("temp dir");
        let root = temp.path();
        seed_skill(root, "member/src/kept");
        seed_skill(root, "member/src/dropped");

        let resolution = resolve(
            root,
            &manifest("skills = { dirs = [\"src\"], only = [\"kept\"] }\n"),
        )
        .expect("resolve");

        assert!(resolution.linked.iter().any(|s| s.name == "kept"));
        assert!(!resolution.linked.iter().any(|s| s.name == "dropped"));
        assert!(
            resolution.unmatched_only.is_empty(),
            "every `only` entry matched a discovered skill"
        );
    }

    #[test]
    fn warns_on_an_only_entry_matching_no_skill() {
        let temp = tempfile::tempdir().expect("temp dir");
        let root = temp.path();
        seed_skill(root, "member/src/kept");

        let resolution = resolve(
            root,
            &manifest("skills = { dirs = [\"src\"], only = [\"ghost\"] }\n"),
        )
        .expect("resolve");

        assert_eq!(resolution.unmatched_only.len(), 1);
        assert_eq!(resolution.unmatched_only[0].repo, "member");
        assert_eq!(resolution.unmatched_only[0].entry, "ghost");
    }

    #[test]
    fn reports_only_the_unmatched_entries() {
        let temp = tempfile::tempdir().expect("temp dir");
        let root = temp.path();
        seed_skill(root, "member/src/kept");

        let resolution = resolve(
            root,
            &manifest("skills = { dirs = [\"src\"], only = [\"kept\", \"ghost\"] }\n"),
        )
        .expect("resolve");

        assert!(resolution.linked.iter().any(|s| s.name == "kept"));
        assert_eq!(
            resolution
                .unmatched_only
                .iter()
                .map(|u| u.entry.as_str())
                .collect::<Vec<_>>(),
            ["ghost"],
            "only the entry with no matching skill is reported"
        );
    }

    #[test]
    fn a_missing_dir_and_an_unmatched_only_are_reported_together() {
        let temp = tempfile::tempdir().expect("temp dir");
        let root = temp.path();
        seed_skill(root, "member/present/kept");

        let resolution = resolve(
            root,
            &manifest("skills = { dirs = [\"present\", \"absent\"], only = [\"ghost\"] }\n"),
        )
        .expect("resolve");

        assert_eq!(
            resolution.missing_dirs.len(),
            1,
            "the absent dir is reported"
        );
        assert_eq!(resolution.missing_dirs[0].dir, "absent");
        assert_eq!(
            resolution.unmatched_only.len(),
            1,
            "the unmatched entry is reported independently of the missing dir"
        );
        assert_eq!(resolution.unmatched_only[0].entry, "ghost");
    }

    #[test]
    fn an_only_entry_matching_a_shadowed_skill_still_counts_as_matched() {
        let temp = tempfile::tempdir().expect("temp dir");
        let root = temp.path();
        seed_skill(root, "member/first/deploy");
        seed_skill(root, "member/second/deploy");

        let resolution = resolve(
            root,
            &manifest("skills = { dirs = [\"first\", \"second\"], only = [\"deploy\"] }\n"),
        )
        .expect("resolve");

        assert!(resolution.linked.iter().any(|s| s.name == "deploy"));
        assert_eq!(
            resolution.shadowed.len(),
            1,
            "the second deploy is shadowed"
        );
        assert!(
            resolution.unmatched_only.is_empty(),
            "a matched-but-shadowed skill still satisfies its `only` entry"
        );
    }

    #[test]
    fn rejects_a_configured_dir_symlinking_outside_the_checkout() {
        let base = tempfile::tempdir().expect("temp dir");
        let base = base.path();
        let root = base.join("ws");
        let checkout = root.join("member");
        std::fs::create_dir_all(&checkout).expect("checkout");
        let outside = base.join("outside");
        seed_skill(&outside, "leaked");
        std::os::unix::fs::symlink(&outside, checkout.join("escape")).expect("escape link");

        let err = resolve(&root, &manifest("skills = { dirs = [\"escape\"] }\n"))
            .expect_err("a symlinked source that leaves the checkout is rejected");
        let msg = err.to_string();
        assert!(msg.contains("member"), "names the repo: {msg}");
        assert!(msg.contains("escape"), "names the dir: {msg}");
        assert!(
            msg.contains("outside the checkout"),
            "explains the escape: {msg}"
        );
    }

    #[test]
    fn rejects_a_defaulted_dir_symlinking_outside_the_checkout() {
        let base = tempfile::tempdir().expect("temp dir");
        let base = base.path();
        let root = base.join("ws");
        // A defaulted `.claude/skills` bypasses the lexical `check_contained`
        // guard entirely, so the resolve-time check is its only containment.
        let claude = root.join("member/.claude");
        std::fs::create_dir_all(&claude).expect("member/.claude");
        let outside = base.join("outside");
        seed_skill(&outside, "leaked");
        std::os::unix::fs::symlink(&outside, claude.join("skills")).expect("escape link");

        let err = resolve(&root, &manifest("skills = true\n"))
            .expect_err("a defaulted source symlink that leaves the checkout is rejected");
        assert!(err.to_string().contains("outside the checkout"));
    }

    #[test]
    fn rejects_a_skill_entry_symlinking_outside_the_checkout() {
        let base = tempfile::tempdir().expect("temp dir");
        let base = base.path();
        let root = base.join("ws");
        // A contained source dir whose individual entry escapes: canonicalising
        // the source parent does not resolve its children, so the entry needs
        // its own check.
        let source = root.join("member/src");
        std::fs::create_dir_all(&source).expect("member/src");
        let outside = base.join("outside");
        seed_skill(&outside, "realskill");
        std::os::unix::fs::symlink(outside.join("realskill"), source.join("x"))
            .expect("escape entry link");

        let err = resolve(&root, &manifest("skills = { dirs = [\"src\"] }\n"))
            .expect_err("a skill entry that leaves the checkout is rejected");
        let msg = err.to_string();
        assert!(msg.contains("member"), "names the repo: {msg}");
        assert!(msg.contains("\"x\""), "names the entry: {msg}");
        assert!(
            msg.contains("outside the checkout"),
            "explains the escape: {msg}"
        );
    }

    #[test]
    fn admits_a_dir_symlink_that_stays_inside_the_checkout() {
        let base = tempfile::tempdir().expect("temp dir");
        let base = base.path();
        let root = base.join("ws");
        // Real skills live inside the checkout; a symlink pointing at them,
        // also inside the checkout, still resolves reproducibly from the tree.
        seed_skill(&root, "member/real/deploy");
        std::os::unix::fs::symlink(root.join("member/real"), root.join("member/link"))
            .expect("inside link");

        let resolution = resolve(&root, &manifest("skills = { dirs = [\"link\"] }\n"))
            .expect("an inside-checkout symlink resolves");
        assert!(
            resolution.linked.iter().any(|s| s.name == "deploy"),
            "the symlinked-but-contained source sweeps normally"
        );
    }

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
            Path::new("/home/me/ws/skills/src/release"),
        )
        .expect("shares /home/me/ws");
        assert_eq!(rel, Path::new("../../skills/src/release"));
    }

    #[test]
    fn normalises_parent_traversal() {
        assert_eq!(normalise(Path::new("/a/b/../c/./d")), Path::new("/a/c/d"));
    }

    #[test]
    fn prune_removes_stale_workspace_links_but_keeps_external_ones() {
        let base = tempfile::tempdir().expect("temp dir");
        let base = base.path();
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
            missing_dirs: Vec::new(),
            unmatched_only: Vec::new(),
        };
        let removed = prune(&root, &dir, &resolution).expect("prune");

        assert_eq!(removed, 1, "only the in-workspace stale link is removed");
        assert!(!dir.join("stale").is_symlink(), "stale link removed");
        assert!(dir.join("mine").is_symlink(), "external user link kept");
    }

    #[test]
    fn reports_real_discovery_directories_as_user_owned() {
        let base = tempfile::tempdir().expect("temp dir");
        let base = base.path();
        let root = base.join("ws");
        let dir = root.join(".claude/skills");
        let managed = root.join(".skills/managed");
        std::fs::create_dir_all(dir.join("mine")).expect("user-owned skill");
        std::fs::create_dir_all(&managed).expect("managed skill");
        std::os::unix::fs::symlink("../../.skills/managed", dir.join("managed"))
            .expect("managed link");

        let found = user_owned_directories(&root).expect("inspect discovery dirs");

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].harness, "Claude Code");
        assert_eq!(found[0].path, dir.join("mine"));
    }
}
