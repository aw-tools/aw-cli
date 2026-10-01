//! The layout reader: builds the model from a workspace laid out as the
//! template lays it out.
//!
//! The registry is `context/artefacts.toml`; units of work are its engagements,
//! each owning the topic directory of its name under `context/engagements/` or
//! `context/archive/`. The scope of the artefact declaration is every tracked
//! `*.md` file, less what sits in a topic's declared subdirectory and what
//! `.awlintignore` matches. No path is skipped in code.

use super::Refusal;
use super::frontmatter;
use super::model::{Artefact, Model, Unit};
use super::registry::RegistryFile;
use crate::git;
use anyhow::{Context, Result};
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use std::io::ErrorKind;
use std::path::Path;

pub const REGISTRY: &str = "context/artefacts.toml";
pub const IGNORE_FILE: &str = ".awlintignore";
const TOPIC_TREES: [&str; 2] = ["context/engagements", "context/archive"];

/// Which tracked paths are artefacts.
pub struct Scope {
    ignore: Gitignore,
    subdirectories: Vec<String>,
}

impl Scope {
    /// `ignore` is the text of `.awlintignore`; without one, nothing is
    /// skipped.
    pub fn new(ignore: Option<&str>, registry: &RegistryFile) -> Result<Self, Refusal> {
        let mut builder = GitignoreBuilder::new(".");
        for (index, line) in ignore.unwrap_or_default().lines().enumerate() {
            builder
                .add_line(None, line)
                .map_err(|err| Refusal::new(format!("{IGNORE_FILE}:{}: {err}", index + 1)))?;
        }
        let ignore = builder
            .build()
            .map_err(|err| Refusal::new(format!("{IGNORE_FILE}: {err}")))?;
        Ok(Self {
            ignore,
            subdirectories: registry.subdirectory.keys().cloned().collect(),
        })
    }

    #[expect(
        clippy::case_sensitive_file_extension_comparisons,
        reason = "the declaration's scope is `*.md`, case-sensitive as git's pathspec is"
    )]
    pub fn admits(&self, path: &str) -> bool {
        path.ends_with(".md")
            && !topic_place(path)
                .and_then(|(_, subdirectory)| subdirectory)
                .is_some_and(|name| self.subdirectories.iter().any(|s| s == name))
            && !self
                .ignore
                .matched_path_or_any_parents(path, false)
                .is_ignore()
    }
}

/// Build the model from the tracked `*.md` files in the working tree, as
/// `aw lint --all` reads them.
pub fn read_worktree(root: &Path) -> Result<Model> {
    let registry_text = std::fs::read_to_string(root.join(REGISTRY))
        .map_err(|err| Refusal::new(format!("{REGISTRY}: {err}")))?;
    let registry = RegistryFile::parse(&registry_text, REGISTRY)?;
    let ignore = match std::fs::read_to_string(root.join(IGNORE_FILE)) {
        Ok(text) => Some(text),
        Err(err) if err.kind() == ErrorKind::NotFound => None,
        Err(err) => return Err(Refusal::new(format!("{IGNORE_FILE}: {err}")).into()),
    };
    let scope = Scope::new(ignore.as_deref(), &registry)?;

    let listing = git::run(root, &["ls-files", "-z", "--", "*.md"])?;
    let mut files = Vec::new();
    for path in listing
        .split_terminator('\0')
        .filter(|path| scope.admits(path))
    {
        match std::fs::read(root.join(path)) {
            Ok(bytes) => files.push((
                path.to_owned(),
                String::from_utf8_lossy(&bytes).into_owned(),
            )),
            // Tracked but deleted from the working tree: nothing to read.
            Err(err) if err.kind() == ErrorKind::NotFound => {}
            Err(err) => return Err(err).with_context(|| format!("reading {path}")),
        }
    }
    Ok(read(&registry, files))
}

/// Build the model from in-scope files, each a path and its contents.
pub fn read(registry: &RegistryFile, files: Vec<(String, String)>) -> Model {
    let artefacts: Vec<Artefact> = files
        .into_iter()
        .map(|(path, text)| {
            let (header, body) = frontmatter::parse(&text);
            let body = body.to_owned();
            Artefact { path, header, body }
        })
        .collect();
    let units = registry
        .engagements
        .iter()
        .map(|(name, entry)| Unit {
            name: name.clone(),
            state: entry.status.clone(),
            members: artefacts
                .iter()
                .filter(|artefact| topic_place(&artefact.path) == Some((name.as_str(), None)))
                .map(|artefact| artefact.path.clone())
                .collect(),
            depends_on: entry.depends_on.clone(),
        })
        .collect();
    Model {
        registry: registry.registry(),
        artefacts,
        units,
    }
}

/// The topic a path sits under in a topic tree, with the topic's subdirectory
/// holding it, if any. `None` for a path outside a topic.
fn topic_place(path: &str) -> Option<(&str, Option<&str>)> {
    let rest = TOPIC_TREES
        .iter()
        .find_map(|tree| path.strip_prefix(tree)?.strip_prefix('/'))?;
    let mut parts = rest.split('/');
    let topic = parts.next()?;
    let second = parts.next()?;
    Some((topic, parts.next().map(|_| second)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    /// The file the template ships, as this instance carries it.
    const SHIPPED_IGNORE: &str = "\
# Paths `aw lint` skips when checking artefact declarations, in gitignore
# syntax. A basename matches at any depth; a leading slash anchors at the root.

# Repository documents, not context artefacts.
README.md
AGENTS.md
CLAUDE.md
CONTRACT.md
CHANGELOG.md
CONTRIBUTING.md

# Agent surfaces, whose markdown carries a foreign frontmatter schema.
/.skills/
/.claude/
/.agents/
";

    const REGISTRY_TEXT: &str = "\
[subdirectory.attachments]
tracked = true

[subdirectory.tmp]
tracked = false

[class.episodic]
statuses = [\"open\", \"closed\", \"compacted\"]

[kind.plan]
class = \"episodic\"

[engagements.alpha]
status = \"open\"
depends-on = [
  \"beta\",
]

[engagements.beta]
status = \"closed\"
";

    fn registry() -> RegistryFile {
        RegistryFile::parse(REGISTRY_TEXT, REGISTRY).unwrap()
    }

    const PATHS: [&str; 13] = [
        "README.md",
        "context/README.md",
        "AGENTS.md",
        "CLAUDE.md",
        "context/STATE.md",
        ".claude/skills/x/SKILL.md",
        "context/.claude/notes.md",
        "context/engagements/alpha/plan-a.md",
        "context/engagements/alpha/attachments/transcript.md",
        "context/engagements/alpha/notes/stray.md",
        "context/archive/beta/plan-b.md",
        "context/archive/beta/tmp/scratch.md",
        "bin/lint",
    ];

    fn admitted(scope: &Scope) -> Vec<&'static str> {
        PATHS
            .into_iter()
            .filter(|path| scope.admits(path))
            .collect()
    }

    #[test]
    fn the_shipped_ignore_file_skips_documents_and_agent_surfaces() {
        let scope = Scope::new(Some(SHIPPED_IGNORE), &registry()).unwrap();
        assert_eq!(
            admitted(&scope),
            [
                "context/STATE.md",
                "context/.claude/notes.md",
                "context/engagements/alpha/plan-a.md",
                "context/engagements/alpha/notes/stray.md",
                "context/archive/beta/plan-b.md",
            ]
        );
    }

    #[test]
    fn without_an_ignore_file_nothing_but_declared_subdirectories_is_skipped() {
        let scope = Scope::new(None, &registry()).unwrap();
        assert_eq!(
            admitted(&scope),
            [
                "README.md",
                "context/README.md",
                "AGENTS.md",
                "CLAUDE.md",
                "context/STATE.md",
                ".claude/skills/x/SKILL.md",
                "context/.claude/notes.md",
                "context/engagements/alpha/plan-a.md",
                "context/engagements/alpha/notes/stray.md",
                "context/archive/beta/plan-b.md",
            ]
        );
    }

    #[test]
    fn a_negated_pattern_readmits_a_path() {
        let scope = Scope::new(Some("*.md\n!context/STATE.md\n"), &registry()).unwrap();
        assert_eq!(admitted(&scope), ["context/STATE.md"]);
    }

    #[test]
    fn an_unparseable_ignore_line_is_refused_naming_it() {
        let refusal = Scope::new(Some("README.md\n{a,b\n"), &registry())
            .err()
            .expect("an unclosed alternation does not parse");
        assert!(
            refusal.to_string().starts_with(".awlintignore:2: "),
            "{refusal}"
        );
    }

    #[test]
    fn units_own_the_topic_level_artefacts_of_their_directory() {
        let files = [
            "context/STATE.md",
            "context/engagements/alpha/plan-a.md",
            "context/engagements/alpha/notes/stray.md",
            "context/archive/beta/plan-b.md",
            "context/engagements/gamma/plan-c.md",
        ]
        .map(|path| {
            (
                path.to_owned(),
                "---\nkind: plan\n---\n\nBody.\n".to_owned(),
            )
        });
        let model = read(&registry(), files.to_vec());

        assert_eq!(model.artefacts.len(), 5);
        assert_eq!(model.artefacts[1].field("kind"), Some("plan"));
        assert_eq!(model.artefacts[1].body, "\nBody.\n");
        let units: Vec<_> = model
            .units
            .iter()
            .map(|unit| {
                (
                    unit.name.as_str(),
                    unit.state.as_str(),
                    unit.members.clone(),
                    unit.depends_on.clone(),
                )
            })
            .collect();
        assert_eq!(
            units,
            [
                (
                    "alpha",
                    "open",
                    vec!["context/engagements/alpha/plan-a.md".to_owned()],
                    vec!["beta".to_owned()]
                ),
                (
                    "beta",
                    "closed",
                    vec!["context/archive/beta/plan-b.md".to_owned()],
                    vec![]
                ),
            ]
        );
        assert_eq!(model.registry.kinds["plan"], "episodic");
    }

    fn git(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    fn write(root: &Path, path: &str, text: &str) {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn worktree_paths(root: &Path) -> Vec<String> {
        read_worktree(root)
            .unwrap()
            .artefacts
            .into_iter()
            .map(|artefact| artefact.path)
            .collect()
    }

    #[test]
    fn the_worktree_reader_reads_tracked_files_in_scope() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        git(root, &["init", "--quiet"]);
        write(root, REGISTRY, REGISTRY_TEXT);
        for path in PATHS {
            write(root, path, "---\nkind: plan\nstatus: open\n---\n");
        }
        write(root, "context/untracked.md", "");
        write(root, "context/Optionen – Notizen.md", "");
        git(root, &["add", "--", ".", ":!context/untracked.md"]);
        std::fs::remove_file(root.join("context/archive/beta/plan-b.md")).unwrap();

        let mut expected = vec![
            ".claude/skills/x/SKILL.md",
            "AGENTS.md",
            "CLAUDE.md",
            "README.md",
            "context/.claude/notes.md",
            "context/Optionen – Notizen.md",
            "context/README.md",
            "context/STATE.md",
            "context/engagements/alpha/notes/stray.md",
            "context/engagements/alpha/plan-a.md",
        ];
        assert_eq!(worktree_paths(root), expected);

        write(root, IGNORE_FILE, SHIPPED_IGNORE);
        expected.retain(|path| {
            !path.starts_with(".claude/")
                && !["AGENTS.md", "CLAUDE.md"].contains(path)
                && !path.ends_with("README.md")
        });
        assert_eq!(worktree_paths(root), expected);
    }

    #[test]
    fn a_missing_registry_is_refused() {
        let temp = tempfile::tempdir().unwrap();
        git(temp.path(), &["init", "--quiet"]);
        let error = read_worktree(temp.path()).err().expect("no registry");
        let refusal = error.downcast_ref::<Refusal>().expect("a refusal");
        assert!(refusal.to_string().starts_with(REGISTRY), "{refusal}");
    }
}
