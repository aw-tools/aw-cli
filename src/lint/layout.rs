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
use super::model::{Artefact, Change, Head, Layout, Model, Unit};
use super::registry::RegistryFile;
use super::tree::{self, Tree};
use crate::git;
use anyhow::{Context, Result};
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use std::collections::BTreeSet;
use std::io::ErrorKind;
use std::path::Path;

pub const REGISTRY: &str = "context/artefacts.toml";
pub const IGNORE_FILE: &str = ".awlintignore";
/// Where an open engagement's topic directory sits.
pub const ENGAGEMENTS: &str = "context/engagements";
/// Where a closed engagement's topic directory sits.
pub const ARCHIVE: &str = "context/archive";
const TOPIC_TREES: [&str; 2] = [ENGAGEMENTS, ARCHIVE];
/// The engagement activity that takes a roll-up line in `STATE.md` instead of
/// an item.
pub const DORMANT: &str = "dormant";

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
                .and_then(|(_, _, subdirectory)| subdirectory)
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
    let registry = read_registry(root)?;
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
        // A link is no artefact of its own, and staged mode reads regular
        // files only.
        if root.join(path).is_symlink() {
            continue;
        }
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
    let topic_paths = git::run(root, &["ls-files", "-z", "--", ENGAGEMENTS, ARCHIVE])?
        .split_terminator('\0')
        .filter(|path| topic_place(path).is_some())
        .map(str::to_owned)
        .collect();
    let mut model = read(&registry, files);
    model.layout = Some(layout(root, &registry, topic_paths)?);
    Ok(model)
}

/// The registry as it stands in the working tree.
pub fn read_registry(root: &Path) -> Result<RegistryFile, Refusal> {
    let text = std::fs::read_to_string(root.join(REGISTRY))
        .map_err(|err| Refusal::new(format!("{REGISTRY}: {err}")))?;
    RegistryFile::parse(&text, REGISTRY)
}

/// Build the model of the staged tree, and of `HEAD` with what the commit
/// moves, as plain `aw lint` reads them. No artefact is read from the working
/// tree; only the untracked `*.md` listing behind the `artefact-untracked`
/// warning looks at it, as the script's `find` does.
pub fn read_staged(root: &Path) -> Result<Change> {
    let index = Tree::index(root)?;
    let registry_text = index
        .text(REGISTRY)?
        .ok_or_else(|| Refusal::new(format!("{REGISTRY}: not in the index")))?;
    let registry = RegistryFile::parse(&registry_text, REGISTRY)?;
    let scope = Scope::new(index.text(IGNORE_FILE)?.as_deref(), &registry)?;
    let mut staged = read(
        &registry,
        index.texts(index.paths().filter(|path| scope.admits(path)))?,
    );
    let topic_paths = index
        .paths()
        .filter(|path| topic_place(path).is_some())
        .map(str::to_owned)
        .collect();
    staged.layout = Some(Layout {
        staged: Some(index.paths().map(str::to_owned).collect()),
        ..layout(root, &registry, topic_paths)?
    });
    if !git::has_head(root)? {
        return Ok(Change { staged, head: None });
    }

    let head = Tree::head(root)?;
    // A `HEAD` without a registry, or with one committed past the linter that
    // does not parse, gives the commit rules no class to compare by; the staged
    // tree is still checked in full. Which files are artefacts is the staged
    // tree's declaration on both sides.
    let head_registry = head
        .text(REGISTRY)?
        .map(|text| RegistryFile::parse(&text, REGISTRY).ok());
    // An unparsed registry gives no prior remits, so no remit is announced as
    // new rather than every one; a missing registry has none, as the script
    // reads it.
    let prior_remits = match &head_registry {
        Some(None) => None,
        Some(Some(registry)) => Some(registry.remit_values()),
        None => Some(BTreeSet::new()),
    };
    let head_registry = head_registry.flatten().unwrap_or_default();
    if let Some(layout) = &mut staged.layout {
        layout.prior_remits = prior_remits;
        layout.staged = Some(index.changed_from(&head));
    }
    let model = read(
        &head_registry,
        head.texts(head.paths().filter(|path| scope.admits(path)))?,
    );
    let (renames, deleted) = tree::moves(root, &head, &index)?;
    Ok(Change {
        staged,
        head: Some(Head {
            model,
            renames,
            deleted,
        }),
    })
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
                .filter(|artefact| {
                    topic_place(&artefact.path)
                        .is_some_and(|(_, topic, sub)| topic == name && sub.is_none())
                })
                .map(|artefact| artefact.path.clone())
                .collect(),
            depends_on: entry.depends_on.clone(),
        })
        .collect();
    Model {
        registry: registry.registry(),
        artefacts,
        units,
        layout: None,
    }
}

/// The layout facts beside the model: each kind's home, the permitted
/// subdirectories, `topic_paths` as the tree being checked holds them, and the
/// untracked `*.md` files in the working tree.
fn layout(root: &Path, registry: &RegistryFile, topic_paths: Vec<String>) -> Result<Layout> {
    // As the script's `find`, the listing ignores `.gitignore`; scratch under
    // any `tmp/` and the contents of a topic's subdirectories are not
    // artefacts.
    let listing = git::run(
        root,
        &[
            "ls-files",
            "-z",
            "--others",
            "--",
            ":(glob)*.md",
            ":(glob)context/**/*.md",
        ],
    )?;
    let untracked = listing
        .split_terminator('\0')
        .filter(|path| {
            !path.split('/').any(|part| part == "tmp")
                && topic_place(path).is_none_or(|(_, _, sub)| sub.is_none())
        })
        .map(str::to_owned)
        .collect();
    Ok(Layout {
        topic_paths,
        untracked,
        ..declared(registry)
    })
}

/// The layout facts the registry declares, with no tree read: no topic paths,
/// no untracked files and no `HEAD` to compare remits with.
pub fn declared(registry: &RegistryFile) -> Layout {
    Layout {
        homes: registry.homes(),
        subdirectories: registry.tracked_subdirectories(),
        topic_paths: Vec::new(),
        untracked: Vec::new(),
        state: registry.state,
        remits: registry
            .engagements
            .iter()
            .map(|(name, entry)| (name.clone(), entry.remits.clone()))
            .collect(),
        dormant: registry
            .engagements
            .iter()
            .filter(|(_, entry)| entry.activity.as_deref() == Some(DORMANT))
            .map(|(name, _)| name.clone())
            .collect(),
        prior_remits: None,
        staged: None,
    }
}

/// The topic tree a path sits in, the topic under it, and the topic's
/// subdirectory holding the path, if any. `None` for a path outside a topic.
pub fn topic_place(path: &str) -> Option<(&'static str, &str, Option<&str>)> {
    let (tree, rest) = TOPIC_TREES
        .iter()
        .find_map(|tree| Some((*tree, path.strip_prefix(tree)?.strip_prefix('/')?)))?;
    let mut parts = rest.split('/');
    let topic = parts.next()?;
    let second = parts.next()?;
    Some((tree, topic, parts.next().map(|_| second)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lint::rule::Rule;
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
    fn the_readers_gather_topic_paths_and_untracked_markdown() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        git(root, &["init", "--quiet"]);
        write(root, REGISTRY, REGISTRY_TEXT);
        write(root, ".gitignore", "context/ignored.md\n");
        for path in [
            "context/engagements/alpha/plan-a.md",
            "context/engagements/alpha/attachments/source.json",
            "context/engagements/stray.md",
            "context/archive/beta/plan-b.md",
            "context/STATE.md",
        ] {
            write(root, path, "---\nkind: plan\nstatus: open\n---\n");
        }
        git(root, &["add", "--all"]);
        for path in [
            "NOTES.md",
            "docs/guide.md",
            "context/ignored.md",
            "context/engagements/alpha/plan-new.md",
            "context/engagements/alpha/attachments/transcript.md",
            "context/engagements/alpha/tmp/scratch.md",
            "context/tmp/scratch.md",
            "context/notes.txt",
        ] {
            write(root, path, "");
        }

        let topic_paths = [
            "context/archive/beta/plan-b.md",
            "context/engagements/alpha/attachments/source.json",
            "context/engagements/alpha/plan-a.md",
        ];
        let untracked = [
            "NOTES.md",
            "context/engagements/alpha/plan-new.md",
            "context/ignored.md",
        ];
        let worktree = read_worktree(root).unwrap().layout.unwrap();
        assert_eq!(worktree.topic_paths, topic_paths);
        assert_eq!(worktree.untracked, untracked);
        assert_eq!(worktree.subdirectories, ["attachments"]);

        let staged = read_staged(root).unwrap().staged.layout.unwrap();
        assert_eq!(staged.topic_paths, topic_paths);
        assert_eq!(staged.untracked, untracked);
    }

    const STAGED_REGISTRY: &str = "\
[class.standing]
statuses = [\"live\", \"retired\"]

[class.ephemeral]
statuses = [\"open\", \"graduated\", \"expired\"]

[kind.doctrine]
class = \"standing\"

[kind.handover]
class = \"ephemeral\"
";
    const CLEAN: &str = "---\nkind: doctrine\nstatus: live\n---\n";
    const CLASSED: &str = "---\nkind: doctrine\nstatus: live\nclass: standing\n---\n";

    fn staged_repository() -> tempfile::TempDir {
        let temp = tempfile::tempdir().unwrap();
        git(temp.path(), &["init", "--quiet"]);
        write(temp.path(), REGISTRY, STAGED_REGISTRY);
        temp
    }

    fn commit_all(root: &Path) {
        git(root, &["add", "--all"]);
        git(
            root,
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "-c",
                "commit.gpgsign=false",
                "-c",
                "core.hooksPath=/dev/null",
                "commit",
                "--quiet",
                "--message",
                "commit",
            ],
        );
    }

    fn staged_findings(root: &Path) -> Vec<(String, Rule)> {
        super::super::commit::check(&read_staged(root).unwrap())
            .into_iter()
            .map(|finding| (finding.path, finding.rule))
            .collect()
    }

    #[test]
    fn a_partially_staged_file_is_checked_as_staged() {
        let temp = staged_repository();
        let root = temp.path();
        write(root, "context/NOTES.md", CLASSED);
        git(root, &["add", "--all"]);
        write(root, "context/NOTES.md", CLEAN);
        assert_eq!(
            staged_findings(root),
            [("context/NOTES.md".to_owned(), Rule::ClassDeclared)]
        );

        git(root, &["add", "--all"]);
        write(root, "context/NOTES.md", CLASSED);
        assert!(staged_findings(root).is_empty());
    }

    #[test]
    fn awkward_paths_are_read_whole() {
        let temp = staged_repository();
        let root = temp.path();
        git(root, &["config", "core.quotePath", "true"]);
        for path in [
            "context/a b.md",
            "context/-dash.md",
            "context/Optionen – Notizen.md",
        ] {
            write(root, path, CLASSED);
        }
        commit_all(root);
        git(root, &["mv", "--", "context/a b.md", "context/c d.md"]);

        let change = read_staged(root).unwrap();
        let renames: Vec<(&str, &str)> = change
            .head
            .as_ref()
            .unwrap()
            .renames
            .iter()
            .map(|(from, to)| (from.as_str(), to.as_str()))
            .collect();
        assert_eq!(renames, [("context/a b.md", "context/c d.md")]);
        assert_eq!(
            staged_findings(root),
            [
                "context/-dash.md",
                "context/Optionen – Notizen.md",
                "context/c d.md"
            ]
            .map(|path| (path.to_owned(), Rule::ClassDeclared))
        );
    }

    #[test]
    fn a_repository_with_no_head_gets_the_snapshot_rules_alone() {
        let temp = staged_repository();
        let root = temp.path();
        write(
            root,
            "context/handover-a.md",
            "---\nkind: handover\nstatus: graduated\n---\n",
        );
        git(root, &["add", "--all"]);
        assert!(read_staged(root).unwrap().head.is_none());
        assert_eq!(
            staged_findings(root),
            [("context/handover-a.md".to_owned(), Rule::EphemeralTerminal)]
        );
    }

    #[test]
    fn an_empty_register_and_an_empty_tree_pass() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        git(root, &["init", "--quiet"]);
        write(root, REGISTRY, "");
        git(root, &["add", "--all"]);
        assert!(staged_findings(root).is_empty());

        commit_all(root);
        write(root, REGISTRY, STAGED_REGISTRY);
        git(root, &["add", "--all"]);
        assert!(staged_findings(root).is_empty());
    }

    #[test]
    fn a_head_without_a_readable_registry_gets_no_commit_rules() {
        for head_registry in [None, Some("[class")] {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path();
            git(root, &["init", "--quiet"]);
            if let Some(text) = head_registry {
                write(root, REGISTRY, text);
            }
            write(
                root,
                "context/handover-a.md",
                "---\nkind: handover\nstatus: open\n---\n",
            );
            commit_all(root);
            write(root, REGISTRY, STAGED_REGISTRY);
            write(
                root,
                "context/handover-a.md",
                "---\nkind: handover\nstatus: graduated\n---\n",
            );
            git(root, &["add", "--all"]);
            // With no classes at `HEAD`, the commit cannot be shown to record
            // the terminal state, so the snapshot finding stands.
            assert_eq!(
                staged_findings(root),
                [("context/handover-a.md".to_owned(), Rule::EphemeralTerminal)],
                "{head_registry:?}"
            );
        }
    }

    #[test]
    fn a_registry_only_in_the_working_tree_is_refused() {
        let temp = staged_repository();
        let error = read_staged(temp.path()).err().expect("nothing staged");
        let refusal = error.downcast_ref::<Refusal>().expect("a refusal");
        assert_eq!(refusal.to_string(), format!("{REGISTRY}: not in the index"));
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
