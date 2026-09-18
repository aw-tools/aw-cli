//! Agent definition discovery and linking.
//!
//! A Claude Code agent definition is a single Markdown file that must
//! physically live under `.claude/agents/` — unlike a skill, there is no
//! generic Agent Skills standard here and no second harness path to satisfy,
//! so linking targets exactly one discovery directory. The mechanics mirror
//! `skills.rs`: manifest opt-in, containment rules, first-claim-wins
//! precedence, symlink-and-prune.

use crate::manifest::Manifest;
use anyhow::{Context, Result};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

/// The one discovery directory Claude Code reads agent definitions from.
pub const AGENTS_DIR: &str = ".claude/agents";

/// Source directories scanned inside an opted-in member repository when its
/// manifest entry names none of its own.
pub const DEFAULT_AGENT_DIRS: &[&str] = &[".claude/agents"];

#[derive(Debug)]
pub struct AgentDefinition {
    pub name: String,
    pub target: PathBuf,
    pub repo: String,
}

/// An explicitly configured source directory that does not exist on disk. A
/// typo in `dirs` must surface; an absent default directory does not, so only
/// configured directories are recorded here.
#[derive(Debug)]
pub struct MissingDir {
    pub repo: String,
    pub dir: String,
}

/// An `only` allowlist entry that matched no discovered agent definition.
#[derive(Debug)]
pub struct UnmatchedOnly {
    pub repo: String,
    pub entry: String,
}

#[derive(Debug)]
pub struct Resolution {
    pub linked: Vec<AgentDefinition>,
    /// Definitions that lost a name collision, paired with the repo that won.
    pub shadowed: Vec<(AgentDefinition, String)>,
    /// Explicitly configured member directories that do not exist.
    pub missing_dirs: Vec<MissingDir>,
    /// Configured `only` allowlist entries that matched no discovered
    /// definition.
    pub unmatched_only: Vec<UnmatchedOnly>,
}

/// Resolve every agent definition the manifest makes available, applying
/// precedence: opted-in member repositories are considered in manifest order;
/// within a repository, configured directories in listed order. First claim
/// wins; every loser is reported.
pub fn resolve(root: &Path, manifest: &Manifest) -> Result<Resolution> {
    let mut candidates = Vec::new();
    let mut missing_dirs = Vec::new();
    let mut unmatched_only = Vec::new();

    for repo in &manifest.repos {
        let Some(config) = repo.agent_config() else {
            continue;
        };
        let checkout = root.join(&repo.path);
        let canonical_checkout = checkout
            .is_dir()
            .then(|| std::fs::canonicalize(&checkout).ok())
            .flatten();
        let configured = config.dirs.is_some();
        let dirs: Vec<&str> = config.dirs.map_or_else(
            || DEFAULT_AGENT_DIRS.to_vec(),
            |dirs| dirs.iter().map(String::as_str).collect(),
        );
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
            let source = if source.exists() {
                match contained(canonical_checkout.as_deref(), &source) {
                    Contained::Inside(canonical) => canonical,
                    Contained::Escape => anyhow::bail!(
                        "repo {} agents directory {dir:?} resolves outside the \
                         checkout; an agent definition source must stay inside \
                         its repository",
                        repo.path
                    ),
                    Contained::Unresolvable => source,
                }
            } else {
                source
            };
            for target in scan_dir(&source)? {
                let name = file_stem(&target)?;
                pending.remove(name.as_str());
                if !config.admits(&name) {
                    continue;
                }
                match contained(canonical_checkout.as_deref(), &target) {
                    Contained::Inside(_) => {}
                    Contained::Escape => anyhow::bail!(
                        "repo {} agent definition {name:?} resolves outside the \
                         checkout; an agent definition must stay inside its \
                         repository",
                        repo.path
                    ),
                    Contained::Unresolvable => continue,
                }
                candidates.push(AgentDefinition {
                    name,
                    target,
                    repo: repo.path.clone(),
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

    let mut claimed: BTreeMap<String, AgentDefinition> = BTreeMap::new();
    let mut shadowed = Vec::new();
    for definition in candidates {
        match claimed.get(&definition.name) {
            Some(winner) => {
                let winner_repo = winner.repo.clone();
                shadowed.push((definition, winner_repo));
            }
            None => {
                claimed.insert(definition.name.clone(), definition);
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

/// Create the symlinks for every resolved agent definition inside
/// `.claude/agents/`. Returns the number of links changed.
///
/// Managed symlinks that no longer correspond to a resolved definition are
/// removed; real files are never touched, only left in place.
pub fn link(root: &Path, resolution: &Resolution) -> Result<usize> {
    let dir = root.join(AGENTS_DIR);
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;

    let mut changed = 0;
    for definition in &resolution.linked {
        let link = dir.join(format!("{}.md", definition.name));
        let want = relative_from(&dir, &definition.target)?;
        if std::fs::read_link(&link).is_ok_and(|existing| existing == want) {
            continue;
        }
        if link.is_symlink() {
            std::fs::remove_file(&link)
                .with_context(|| format!("replacing stale link {}", link.display()))?;
        } else if link.exists() {
            // A real file here is the user's own definition. Leave it.
            continue;
        }
        std::os::unix::fs::symlink(&want, &link)
            .with_context(|| format!("linking {}", link.display()))?;
        changed += 1;
    }

    changed += prune(root, &dir, resolution)?;
    Ok(changed)
}

/// Remove symlinks in the discovery directory that no longer name a resolved
/// agent definition. Only symlinks are removed — never real files — and only
/// those pointing at a source inside the workspace.
fn prune(root: &Path, dir: &Path, resolution: &Resolution) -> Result<usize> {
    let mut removed = 0;
    for entry in read_dir(dir)? {
        let path = entry.path();
        if !path.is_symlink() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let stem = name.strip_suffix(".md").unwrap_or(&name);
        if resolution.linked.iter().any(|d| d.name == stem) {
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

/// The outcome of resolving an agent source or entry against its checkout
/// root.
enum Contained {
    Inside(PathBuf),
    Escape,
    Unresolvable,
}

/// Confirm an agent source directory or entry stays inside its member
/// checkout. This is the filesystem half of `manifest::check_contained`: that
/// guard is purely lexical and cannot see a symlink that leaves the checkout,
/// so the real-path check happens here, at the sweep, over the
/// already-canonical `root`.
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

/// Immediate Markdown files in a source directory, sorted for deterministic
/// output. Unlike a skill, an agent definition is the file itself, not a
/// subdirectory carrying a marker.
fn scan_dir(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut found: Vec<PathBuf> = read_dir(dir)?
        .into_iter()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "md"))
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

fn file_stem(path: &Path) -> Result<String> {
    path.file_stem()
        .map(|n| n.to_string_lossy().into_owned())
        .with_context(|| format!("agent definition has no name: {}", path.display()))
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

    /// Create an agent definition file `<root>/<relative>.md`.
    fn seed_agent(root: &Path, relative: &str) {
        let path = root.join(format!("{relative}.md"));
        std::fs::create_dir_all(path.parent().expect("parent")).expect("parent dir");
        std::fs::write(&path, "---\nname: x\n---\n").expect("definition file");
    }

    /// Parse a one-repo manifest at `member` carrying `agents_fragment`.
    fn manifest(agents_fragment: &str) -> Manifest {
        toml::from_str(&format!(
            "[workspace]\nname = \"w\"\n\
             [[repo]]\npath = \"member\"\nurl = \"u\"\n{agents_fragment}"
        ))
        .expect("fixture parses")
    }

    #[test]
    fn scans_configured_dirs_in_listed_order_and_dedupes_across_them() {
        let temp = tempfile::tempdir().expect("temp dir");
        // Canonicalised: resolution returns real paths, and on macOS the temp
        // dir is reached through a symlink.
        let root = &std::fs::canonicalize(temp.path()).expect("canonical temp dir");
        seed_agent(root, "member/first/worker");
        seed_agent(root, "member/first/only-first");
        seed_agent(root, "member/second/worker");
        seed_agent(root, "member/second/only-second");

        let resolution = resolve(
            root,
            &manifest("agents = { dirs = [\"first\", \"second\"] }\n"),
        )
        .expect("resolve");

        let worker = resolution
            .linked
            .iter()
            .find(|d| d.name == "worker")
            .expect("worker resolves");
        assert_eq!(
            worker.target,
            root.join("member/first/worker.md"),
            "the first listed dir wins the name"
        );
        assert!(resolution.linked.iter().any(|d| d.name == "only-first"));
        assert!(resolution.linked.iter().any(|d| d.name == "only-second"));
        assert_eq!(
            resolution.shadowed.len(),
            1,
            "the second dir's worker is shadowed, not linked twice"
        );
        assert_eq!(
            resolution.shadowed[0].0.target,
            root.join("member/second/worker.md")
        );
        assert_eq!(resolution.shadowed[0].1, "member");
    }

    #[test]
    fn warns_on_a_missing_configured_dir_but_still_scans_the_rest() {
        let temp = tempfile::tempdir().expect("temp dir");
        let root = temp.path();
        seed_agent(root, "member/present/live");

        let resolution = resolve(
            root,
            &manifest("agents = { dirs = [\"present\", \"absent\"] }\n"),
        )
        .expect("resolve");

        assert!(resolution.linked.iter().any(|d| d.name == "live"));
        assert_eq!(resolution.missing_dirs.len(), 1);
        assert_eq!(resolution.missing_dirs[0].repo, "member");
        assert_eq!(resolution.missing_dirs[0].dir, "absent");
    }

    #[test]
    fn default_dir_is_claude_agents() {
        let temp = tempfile::tempdir().expect("temp dir");
        let root = temp.path();
        seed_agent(root, "member/.claude/agents/worker");

        let resolution = resolve(root, &manifest("agents = true\n")).expect("resolve");

        assert!(resolution.linked.iter().any(|d| d.name == "worker"));
    }

    #[test]
    fn absent_default_dir_is_silent() {
        let temp = tempfile::tempdir().expect("temp dir");
        let root = temp.path();

        let resolution = resolve(root, &manifest("agents = true\n")).expect("resolve");

        assert!(resolution.linked.is_empty());
        assert!(
            resolution.missing_dirs.is_empty(),
            "an absent default dir is not a warning"
        );
    }

    #[test]
    fn only_narrows_the_discovered_definitions() {
        let temp = tempfile::tempdir().expect("temp dir");
        let root = temp.path();
        seed_agent(root, "member/src/kept");
        seed_agent(root, "member/src/dropped");

        let resolution = resolve(
            root,
            &manifest("agents = { dirs = [\"src\"], only = [\"kept\"] }\n"),
        )
        .expect("resolve");

        assert!(resolution.linked.iter().any(|d| d.name == "kept"));
        assert!(!resolution.linked.iter().any(|d| d.name == "dropped"));
        assert!(
            resolution.unmatched_only.is_empty(),
            "every `only` entry matched a discovered definition"
        );
    }

    #[test]
    fn warns_on_an_only_entry_matching_no_definition() {
        let temp = tempfile::tempdir().expect("temp dir");
        let root = temp.path();
        seed_agent(root, "member/src/kept");

        let resolution = resolve(
            root,
            &manifest("agents = { dirs = [\"src\"], only = [\"ghost\"] }\n"),
        )
        .expect("resolve");

        assert_eq!(resolution.unmatched_only.len(), 1);
        assert_eq!(resolution.unmatched_only[0].repo, "member");
        assert_eq!(resolution.unmatched_only[0].entry, "ghost");
    }

    #[test]
    fn rejects_a_configured_dir_symlinking_outside_the_checkout() {
        let base = tempfile::tempdir().expect("temp dir");
        let base = base.path();
        let root = base.join("ws");
        let checkout = root.join("member");
        std::fs::create_dir_all(&checkout).expect("checkout");
        let outside = base.join("outside");
        seed_agent(&outside, "leaked");
        std::os::unix::fs::symlink(&outside, checkout.join("escape")).expect("escape link");

        let err = resolve(&root, &manifest("agents = { dirs = [\"escape\"] }\n"))
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
    fn rejects_a_definition_entry_symlinking_outside_the_checkout() {
        let base = tempfile::tempdir().expect("temp dir");
        let base = base.path();
        let root = base.join("ws");
        let source = root.join("member/src");
        std::fs::create_dir_all(&source).expect("member/src");
        let outside = base.join("outside");
        seed_agent(&outside, "realdefinition");
        std::os::unix::fs::symlink(outside.join("realdefinition.md"), source.join("x.md"))
            .expect("escape entry link");

        let err = resolve(&root, &manifest("agents = { dirs = [\"src\"] }\n"))
            .expect_err("a definition entry that leaves the checkout is rejected");
        let msg = err.to_string();
        assert!(msg.contains("member"), "names the repo: {msg}");
        assert!(msg.contains("\"x\""), "names the entry: {msg}");
        assert!(
            msg.contains("outside the checkout"),
            "explains the escape: {msg}"
        );
    }

    #[test]
    fn links_resolved_definitions_and_prunes_stale_ones() {
        let temp = tempfile::tempdir().expect("temp dir");
        // Canonicalised: link targets are computed from real paths, and on
        // macOS the temp dir is reached through a symlink.
        let root = &std::fs::canonicalize(temp.path()).expect("canonical temp dir");
        seed_agent(root, "member/.claude/agents/worker");

        let resolution = resolve(root, &manifest("agents = true\n")).expect("resolve");
        let changed = link(root, &resolution).expect("link");

        assert_eq!(changed, 1);
        let link_path = root.join(".claude/agents/worker.md");
        assert!(link_path.is_symlink(), "worker.md is linked");
        assert_eq!(
            std::fs::read_link(&link_path).expect("read link"),
            Path::new("../../member/.claude/agents/worker.md")
        );

        // Re-linking with a stale entry present prunes it.
        std::os::unix::fs::symlink(
            "../../member/.claude/agents/gone.md",
            root.join(".claude/agents/stale.md"),
        )
        .expect("stale link");
        let resolution = resolve(root, &manifest("agents = true\n")).expect("resolve");
        let changed = link(root, &resolution).expect("re-link");
        assert_eq!(
            changed, 1,
            "only the stale link is pruned; worker.md is unchanged"
        );
        assert!(!root.join(".claude/agents/stale.md").exists());
    }

    #[test]
    fn a_real_file_at_the_link_path_is_left_untouched() {
        let temp = tempfile::tempdir().expect("temp dir");
        let root = temp.path();
        seed_agent(root, "member/.claude/agents/worker");
        std::fs::create_dir_all(root.join(".claude/agents")).expect("discovery dir");
        std::fs::write(root.join(".claude/agents/worker.md"), "mine").expect("user file");

        let resolution = resolve(root, &manifest("agents = true\n")).expect("resolve");
        let changed = link(root, &resolution).expect("link");

        assert_eq!(changed, 0, "the user's own file is never overwritten");
        assert_eq!(
            std::fs::read_to_string(root.join(".claude/agents/worker.md")).expect("read"),
            "mine"
        );
    }
}
