//! `workspace.toml` — the single source of truth for a workspace.
//!
//! Everything `aw` does is derived from this file. Nothing else on disk is
//! authoritative: `.aw/trees.yaml` is generated from it, and skill links are
//! computed from it.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::fs::{File, Metadata, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use toml_edit::{ArrayOfTables, DocumentMut, Item, Table, value};

pub const FILENAME: &str = "workspace.toml";
pub const WORKSPACE_NAME_PLACEHOLDER: &str = "CHANGEME";
static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Deserialize)]
pub struct Manifest {
    #[serde(default)]
    pub template: Option<Template>,
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

#[derive(Debug, Deserialize)]
pub struct Template {
    pub url: String,
    #[serde(rename = "ref")]
    pub reference: String,
    pub sha: String,
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
#[serde(deny_unknown_fields)]
pub struct Repo {
    pub path: String,
    pub url: String,
    pub branch: Option<String>,
    /// Opt-in to skill propagation. Absent means off: skills are never loaded
    /// from a repository that has not explicitly opted in.
    #[serde(default)]
    pub skills: Option<SkillOptIn>,
}

/// A repository's `skills` value: either a boolean toggle or a configuration
/// table. `true` opts in with the convention defaults; `false` is the same as
/// absent. The table narrows which source directories and skill names apply.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum SkillOptIn {
    Toggle(bool),
    Table(SkillTable),
}

/// Unknown keys are rejected: a typo such as `dir` for `dirs` would otherwise
/// parse as an empty table and silently opt in with the convention defaults.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillTable {
    /// Source directories relative to the checkout. Absent means the convention
    /// default (`DEFAULT_SKILL_DIRS` in `skills.rs`).
    pub dirs: Option<Vec<String>>,
    /// Allowlist of skill names. Absent admits every discovered skill.
    pub only: Option<Vec<String>>,
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

    /// The repository's effective skill configuration, if it opted in at all.
    /// `None` covers both an absent `skills` key and `skills = false`.
    pub fn skill_config(&self) -> Option<SkillConfig<'_>> {
        match self.skills.as_ref()? {
            SkillOptIn::Toggle(false) => None,
            SkillOptIn::Toggle(true) => Some(SkillConfig {
                dirs: None,
                only: None,
            }),
            SkillOptIn::Table(table) => Some(SkillConfig {
                dirs: table.dirs.as_deref(),
                only: table.only.as_deref(),
            }),
        }
    }
}

/// Resolved view of an opted-in repository's `skills` table, borrowing the
/// manifest. `dirs = None` means fall back to the convention default; `only =
/// None` means admit every discovered skill.
pub struct SkillConfig<'a> {
    pub dirs: Option<&'a [String]>,
    pub only: Option<&'a [String]>,
}

impl SkillConfig<'_> {
    pub fn admits(&self, name: &str) -> bool {
        self.only
            .is_none_or(|names| names.iter().any(|n| n == name))
    }
}

impl Manifest {
    pub fn load(root: &Path) -> Result<Self> {
        let path = root.join(FILENAME);
        regular_file_metadata(&path)?;
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
            validate_skills(repo)?;
        }
        Ok(())
    }
}

/// Validate an opted-in repository's `skills` table. Applies the containment
/// rules to each source directory and rejects opt-ins that admit nothing.
/// Errors name the offending repository so a typo is easy to locate.
fn validate_skills(repo: &Repo) -> Result<()> {
    let Some(SkillOptIn::Table(table)) = repo.skills.as_ref() else {
        return Ok(());
    };
    if let Some(dirs) = table.dirs.as_ref() {
        anyhow::ensure!(
            !dirs.is_empty(),
            "repo {} opts into skills with an empty `dirs`; drop the key for the defaults or name at least one directory",
            repo.path
        );
        let mut seen = std::collections::BTreeSet::new();
        for dir in dirs {
            check_contained(dir)
                .with_context(|| format!("repo {} skills directory {dir:?}", repo.path))?;
            // Dedupe on the normalised path so aliased spellings of one
            // directory ("src", "./src", "src/") cannot scan it twice and
            // report every skill in it as shadowed by its own copy.
            let normalised: PathBuf = Path::new(dir)
                .components()
                .filter(|c| !matches!(c, std::path::Component::CurDir))
                .collect();
            anyhow::ensure!(
                !normalised.as_os_str().is_empty(),
                "repo {} skills directory {dir:?} names the checkout root; skills are never swept from a repository root",
                repo.path
            );
            anyhow::ensure!(
                seen.insert(normalised),
                "repo {} lists skills directory {dir:?} more than once",
                repo.path
            );
        }
    }
    if let Some(only) = table.only.as_ref() {
        anyhow::ensure!(
            !only.is_empty(),
            "repo {} opts into skills with an empty `only`; drop the key to admit every skill or name at least one",
            repo.path
        );
    }
    Ok(())
}

/// Replace the template placeholder only in the `[workspace]` table.
///
/// Templates are hand-authored and retain comments and ordering, so this
/// transform deliberately edits the one contract line instead of
/// reserialising the whole manifest.
pub fn replace_workspace_name(text: &str, name: &str) -> Result<String> {
    let target = format!("name = \"{WORKSPACE_NAME_PLACEHOLDER}\"");
    let mut in_workspace = false;
    let mut offset = 0;

    for line in text.split_inclusive('\n') {
        let trimmed = line.trim();
        if trimmed == "[workspace]"
            || trimmed
                .strip_prefix("[workspace]")
                .is_some_and(|rest| rest.trim_start().starts_with('#'))
        {
            in_workspace = true;
        } else if trimmed.starts_with('[') {
            in_workspace = false;
        } else if in_workspace {
            let content = line.trim_start();
            if let Some(rest) = content.strip_prefix(&target) {
                if rest.trim().is_empty() || rest.trim_start().starts_with('#') {
                    let start = offset + line.len() - content.len();
                    let end = start + target.len();
                    let mut updated = text.to_owned();
                    updated.replace_range(start..end, &format!("name = \"{name}\""));
                    return Ok(updated);
                }
            }
        }
        offset += line.len();
    }

    anyhow::bail!(
        "template contract violation: [workspace] must contain name = \"{WORKSPACE_NAME_PLACEHOLDER}\""
    )
}

/// Reject a re-init whose resolved source conflicts with recorded provenance.
/// A target without a manifest or provenance is still eligible for its first
/// template overlay.
pub fn validate_template(root: &Path, url: &str, reference: &str, sha: &str) -> Result<()> {
    let path = root.join(FILENAME);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err).with_context(|| format!("reading {}", path.display())),
    };
    let parsed: Manifest =
        toml::from_str(&text).with_context(|| format!("parsing manifest {}", path.display()))?;
    parsed
        .validate()
        .with_context(|| format!("in manifest {}", path.display()))?;

    if let Some(existing) = parsed.template.as_ref() {
        anyhow::ensure!(
            existing.url == url && existing.reference == reference && existing.sha == sha,
            "workspace already records template {}@{} ({})",
            existing.url,
            existing.reference,
            existing.sha
        );
    }
    Ok(())
}

/// Add immutable template provenance without reserialising the human-edited
/// manifest. A repeat init from the same source is a no-op; a conflicting
/// source is rejected rather than rewriting history.
pub fn record_template(root: &Path, url: &str, reference: &str, sha: &str) -> Result<Manifest> {
    let path = root.join(FILENAME);
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let mut parsed: Manifest =
        toml::from_str(&text).with_context(|| format!("parsing manifest {}", path.display()))?;
    parsed
        .validate()
        .with_context(|| format!("in manifest {}", path.display()))?;

    if let Some(existing) = parsed.template.as_ref() {
        anyhow::ensure!(
            existing.url == url && existing.reference == reference && existing.sha == sha,
            "workspace already records template {}@{} ({})",
            existing.url,
            existing.reference,
            existing.sha
        );
        return Ok(parsed);
    }

    let quote = |value: &str| toml::Value::String(value.to_owned()).to_string();
    let provenance = format!(
        "[template]\nurl = {}\nref = {}\nsha = {}\n\n",
        quote(url),
        quote(reference),
        quote(sha)
    );
    std::fs::write(&path, format!("{provenance}{text}"))
        .with_context(|| format!("writing {}", path.display()))?;
    parsed.template = Some(Template {
        url: url.to_owned(),
        reference: reference.to_owned(),
        sha: sha.to_owned(),
    });
    Ok(parsed)
}

/// Resolve a checkout and express it as a portable workspace-relative path.
pub fn canonicalise_member(root: &Path, path: &Path) -> Result<(PathBuf, String)> {
    let root = std::fs::canonicalize(root)
        .with_context(|| format!("resolving workspace root {}", root.display()))?;
    let checkout = std::fs::canonicalize(path)
        .with_context(|| format!("resolving checkout {}", path.display()))?;
    let relative = checkout
        .strip_prefix(&root)
        .with_context(|| {
            format!(
                "checkout {} is outside the workspace {}",
                checkout.display(),
                root.display()
            )
        })?
        .to_str()
        .with_context(|| {
            format!(
                "checkout path {} cannot be represented in the manifest as UTF-8",
                checkout.display()
            )
        })?
        .to_owned();
    check_contained(&relative)?;
    Ok((checkout, relative))
}

/// Append one repository without reserialising the human-edited manifest.
pub fn append_repo(root: &Path, repo: &Repo) -> Result<()> {
    let path = root.join(FILENAME);
    let metadata = regular_file_metadata(&path)?;
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let mut document: DocumentMut = text
        .parse()
        .with_context(|| format!("parsing manifest {}", path.display()))?;

    let repos = document
        .entry("repo")
        .or_insert(Item::ArrayOfTables(ArrayOfTables::new()))
        .as_array_of_tables_mut()
        .with_context(|| format!("manifest {} has a non-array repo entry", path.display()))?;
    let mut table = Table::new();
    table.insert("path", value(&repo.path));
    table.insert("url", value(&repo.url));
    if let Some(branch) = &repo.branch {
        table.insert("branch", value(branch));
    }
    repos.push(table);

    replace_atomically(&path, document.to_string().as_bytes(), &metadata)
}

fn replace_atomically(path: &Path, contents: &[u8], metadata: &Metadata) -> Result<()> {
    replace_atomically_with(path, contents, metadata, |_| Ok(()))
}

fn replace_atomically_with(
    path: &Path,
    contents: &[u8],
    metadata: &Metadata,
    before_rename: impl FnOnce(&Path) -> Result<()>,
) -> Result<()> {
    let (temporary, mut file) = create_temporary_file(path)?;
    let result = (|| -> Result<()> {
        file.set_permissions(metadata.permissions())
            .with_context(|| format!("setting permissions on {}", temporary.display()))?;
        file.write_all(contents)
            .with_context(|| format!("writing temporary manifest {}", temporary.display()))?;
        file.sync_all()
            .with_context(|| format!("syncing temporary manifest {}", temporary.display()))?;
        drop(file);
        before_rename(&temporary)?;
        std::fs::rename(&temporary, path).with_context(|| {
            format!(
                "replacing manifest {} with {}",
                path.display(),
                temporary.display()
            )
        })
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

fn create_temporary_file(path: &Path) -> Result<(PathBuf, File)> {
    for _ in 0..100 {
        let sequence = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let temporary = path.with_file_name(format!(
            ".{FILENAME}.{}.{}.tmp",
            std::process::id(),
            sequence
        ));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
        {
            Ok(file) => return Ok((temporary, file)),
            Err(err) if err.kind() == ErrorKind::AlreadyExists => {}
            Err(err) => {
                return Err(err).with_context(|| {
                    format!("creating temporary manifest {}", temporary.display())
                });
            }
        }
    }
    anyhow::bail!(
        "could not create a unique temporary manifest beside {}",
        path.display()
    )
}

fn regular_file_metadata(path: &Path) -> Result<Metadata> {
    let metadata = std::fs::symlink_metadata(path)
        .with_context(|| format!("reading metadata for {}", path.display()))?;
    anyhow::ensure!(
        metadata.file_type().is_file(),
        "{} must be a regular file",
        path.display()
    );
    Ok(metadata)
}

fn manifest_marker(dir: &Path) -> Result<bool> {
    let path = dir.join(FILENAME);
    match std::fs::symlink_metadata(&path) {
        Ok(metadata) => {
            anyhow::ensure!(
                metadata.file_type().is_file(),
                "{} must be a regular file",
                path.display()
            );
            Ok(true)
        }
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(false),
        Err(err) => Err(err).with_context(|| format!("reading metadata for {}", path.display())),
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
            manifest_marker(&dir)?,
            "{} is not a workspace: no {FILENAME}",
            dir.display()
        );
        return Ok(dir);
    }

    let mut dir = std::env::current_dir().context("resolving current directory")?;
    loop {
        if manifest_marker(&dir)? {
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
    fn failed_atomic_replacement_preserves_the_manifest() {
        let temp = tempfile::tempdir().expect("temp dir");
        let root = temp.path();
        let path = root.join(FILENAME);
        std::fs::write(&path, "original\n").expect("write original manifest");
        let metadata = regular_file_metadata(&path).expect("read original metadata");

        let err = replace_atomically_with(&path, b"replacement\n", &metadata, |_| {
            anyhow::bail!("injected failure before rename")
        })
        .expect_err("replacement fails");

        assert!(err.to_string().contains("injected failure"));
        assert_eq!(
            std::fs::read_to_string(&path).expect("read preserved manifest"),
            "original\n"
        );
        assert_eq!(
            std::fs::read_dir(root)
                .expect("read test directory")
                .count(),
            1
        );
    }

    /// Parse a one-repo manifest whose repo carries `skills_fragment` verbatim,
    /// returning the manifest without validating it.
    fn manifest_with_skills(skills_fragment: &str) -> Manifest {
        toml::from_str(&format!(
            "[workspace]\nname = \"w\"\n\
             [[repo]]\npath = \"member\"\nurl = \"u\"\n{skills_fragment}"
        ))
        .expect("fixture parses")
    }

    #[test]
    fn absent_skills_key_is_off() {
        let manifest = manifest_with_skills("");
        assert!(manifest.repos[0].skill_config().is_none());
    }

    #[test]
    fn skills_false_is_off() {
        let manifest = manifest_with_skills("skills = false\n");
        assert!(manifest.repos[0].skill_config().is_none());
    }

    #[test]
    fn skills_true_opts_in_with_defaults() {
        let manifest = manifest_with_skills("skills = true\n");
        let config = manifest.repos[0].skill_config().expect("true opts in");
        assert!(config.dirs.is_none(), "defaults to the convention dirs");
        assert!(config.only.is_none(), "admits every skill");
        assert!(config.admits("anything"));
    }

    #[test]
    fn empty_skills_table_is_the_same_as_true() {
        let manifest = manifest_with_skills("skills = {}\n");
        let config = manifest.repos[0]
            .skill_config()
            .expect("an empty table opts in");
        assert!(config.dirs.is_none());
        assert!(config.only.is_none());
    }

    #[test]
    fn skills_table_carries_dirs_and_only() {
        let manifest =
            manifest_with_skills("skills = { dirs = [\"src\"], only = [\"release\"] }\n");
        let config = manifest.repos[0].skill_config().expect("a table opts in");
        assert_eq!(config.dirs, Some(["src".to_owned()].as_slice()));
        assert!(config.admits("release"));
        assert!(!config.admits("other"), "only narrows to the allowlist");
    }

    #[test]
    fn rejects_the_removed_list_form() {
        let err = toml::from_str::<Manifest>(
            "[workspace]\nname = \"w\"\n\
             [[repo]]\npath = \"member\"\nurl = \"u\"\nskills = [\"release\"]\n",
        )
        .expect_err("the bare list form is dropped");
        assert!(err.to_string().contains("skills"), "{err}");
    }

    #[test]
    fn rejects_empty_dirs() {
        let err = manifest_with_skills("skills = { dirs = [] }\n")
            .validate()
            .expect_err("an empty dirs list is a mistake");
        assert!(err.to_string().contains("empty `dirs`"), "{err}");
    }

    #[test]
    fn rejects_duplicate_dirs() {
        let err = manifest_with_skills("skills = { dirs = [\"src\", \"src\"] }\n")
            .validate()
            .expect_err("duplicate dirs are a mistake");
        assert!(err.to_string().contains("more than once"), "{err}");
    }

    #[test]
    fn rejects_empty_only() {
        let err = manifest_with_skills("skills = { only = [] }\n")
            .validate()
            .expect_err("an empty allowlist admits nothing");
        assert!(err.to_string().contains("empty `only`"), "{err}");
    }

    #[test]
    fn rejects_an_absolute_skills_dir() {
        let err = manifest_with_skills("skills = { dirs = [\"/etc\"] }\n")
            .validate()
            .expect_err("an absolute dir escapes the checkout");
        let chain = format!("{err:#}");
        assert!(chain.contains("absolute"), "{chain}");
        assert!(chain.contains("member"), "names the repo: {chain}");
    }

    #[test]
    fn rejects_a_skills_dir_escaping_the_checkout() {
        let err = manifest_with_skills("skills = { dirs = [\"../elsewhere\"] }\n")
            .validate()
            .expect_err("`..` escapes the checkout");
        assert!(format!("{err:#}").contains("escapes"), "{err:#}");
    }

    #[test]
    fn rejects_an_unknown_key_in_the_skills_table() {
        let err = toml::from_str::<Manifest>(
            "[workspace]\nname = \"w\"\n\
             [[repo]]\npath = \"member\"\nurl = \"u\"\nskills = { dir = [\"src\"] }\n",
        )
        .expect_err("a typo'd key must not silently opt in with defaults");
        assert!(err.to_string().contains("skills"), "{err}");
    }

    #[test]
    fn rejects_the_removed_skill_prefix_key() {
        let err = toml::from_str::<Manifest>(
            "[workspace]\nname = \"w\"\n\
             [[repo]]\npath = \"member\"\nurl = \"u\"\nskills = true\nskill-prefix = true\n",
        )
        .expect_err("the removed key is rejected, not silently ignored");
        assert!(err.to_string().contains("skill-prefix"), "{err}");
    }

    #[test]
    fn rejects_aliased_duplicate_dirs() {
        let err = manifest_with_skills("skills = { dirs = [\"src\", \"./src\"] }\n")
            .validate()
            .expect_err("aliased spellings of one directory are duplicates");
        assert!(err.to_string().contains("more than once"), "{err}");
    }

    #[test]
    fn rejects_a_skills_dir_naming_the_checkout_root() {
        let err = manifest_with_skills("skills = { dirs = [\".\"] }\n")
            .validate()
            .expect_err("the checkout root is never swept for skills");
        assert!(err.to_string().contains("checkout root"), "{err}");
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

    #[test]
    fn workspace_name_rewrite_preserves_inline_comments() {
        let manifest = "[identity]\nname = \"CHANGEME\"\n\
                        [workspace] # settings\n\
                        name = \"CHANGEME\" # replaced during init\n";

        let updated = replace_workspace_name(manifest, "demo").expect("placeholder exists");

        assert_eq!(
            updated,
            "[identity]\nname = \"CHANGEME\"\n\
             [workspace] # settings\n\
             name = \"demo\" # replaced during init\n"
        );
    }
}
