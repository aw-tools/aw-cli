//! The conformance-suite reader and runner, test-only.
//!
//! A fixture is a directory holding `fixture.toml` (the verdict, the globs
//! declaring which files are artefacts, an optional revision) and a tree:
//! `registry.toml`, an optional `units.toml`, and the files themselves. A
//! fixture whose clause concerns commits holds its trees under `steps/01` and
//! `steps/02`; the runner commits the first, stages the second and runs staged
//! mode. The copy under `tests/suite/` names the commit it was taken from.

use super::model::{Change, Head, Model, Unit};
use super::registry::RegistryFile;
use super::tree::{self, Tree};
use super::{Refusal, commit, layout, rules};
use ignore::gitignore::GitignoreBuilder;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Deserialize)]
struct FixtureFile {
    clause: String,
    outcome: String,
    artefacts: Vec<String>,
    revision: Option<toml::Value>,
}

#[derive(Deserialize)]
struct UnitsFile {
    unit: Vec<UnitEntry>,
}

#[derive(Deserialize)]
struct UnitEntry {
    name: String,
    state: String,
    artefacts: Vec<String>,
}

struct Fixture {
    file: FixtureFile,
    change: Change,
}

fn read(dir: &Path) -> Fixture {
    let file: FixtureFile = toml::from_str(&text(&dir.join("fixture.toml"))).unwrap();
    let steps = dir.join("steps");
    let change = if steps.exists() {
        staged(&steps, &file.artefacts)
    } else {
        let mut files = files(dir);
        files.remove("fixture.toml");
        Change {
            staged: model(&files, &file.artefacts),
            head: None,
        }
    };
    Fixture { file, change }
}

/// Commit `steps/01` in a fresh repository, stage `steps/02` over it, and
/// read both trees from git as staged mode does.
fn staged(steps: &Path, globs: &[String]) -> Change {
    let repository = tempfile::tempdir().unwrap();
    let root = repository.path();
    git(root, &["init", "--quiet"]);
    copy(&steps.join("01"), root);
    git(root, &["add", "--all"]);
    git(root, &["commit", "--quiet", "--message", "01"]);
    for entry in std::fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.file_name() == Some(".git".as_ref()) {
            continue;
        }
        if path.is_dir() {
            std::fs::remove_dir_all(path).unwrap();
        } else {
            std::fs::remove_file(path).unwrap();
        }
    }
    copy(&steps.join("02"), root);
    git(root, &["add", "--all"]);

    let index = Tree::index(root).unwrap();
    let head = Tree::head(root).unwrap();
    let (renames, deleted) = tree::moves(root, &head, &index).unwrap();
    Change {
        staged: model(&tree_files(&index), globs),
        head: Some(Head {
            model: model(&tree_files(&head), globs),
            renames,
            deleted,
        }),
    }
}

fn git(root: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=/dev/null",
        ])
        .args(args)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

fn copy(from: &Path, to: &Path) {
    for (path, text) in files(from) {
        let target = to.join(path);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, text).unwrap();
    }
}

fn tree_files(tree: &Tree) -> BTreeMap<String, String> {
    tree.texts(tree.paths()).unwrap().into_iter().collect()
}

/// The model of one fixture tree, each file a path relative to the tree.
fn model(files: &BTreeMap<String, String>, globs: &[String]) -> Model {
    let registry = RegistryFile::parse(&files["registry.toml"], "registry.toml").unwrap();
    let mut declaration = GitignoreBuilder::new("");
    for glob in globs {
        declaration.add_line(None, glob).unwrap();
    }
    let declaration = declaration.build().unwrap();
    let artefacts = files
        .iter()
        .filter(|(path, _)| declaration.matched(path, false).is_ignore())
        .map(|(path, text)| (path.clone(), text.clone()))
        .collect();

    let mut model = layout::read(&registry, artefacts);
    model.units = files.get("units.toml").map_or_else(Vec::new, |units| {
        let units: UnitsFile = toml::from_str(units).unwrap();
        units
            .unit
            .into_iter()
            .map(|entry| Unit {
                name: entry.name,
                state: entry.state,
                members: entry.artefacts,
                depends_on: Vec::new(),
            })
            .collect()
    });
    model
}

/// Every file under `dir`, by path relative to it.
fn files(dir: &Path) -> BTreeMap<String, String> {
    walk(dir)
        .into_iter()
        .map(|path| {
            let relative = path.strip_prefix(dir).unwrap().to_str().unwrap().to_owned();
            (relative, text(&path))
        })
        .collect()
}

fn text(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|err| panic!("{}: {err}", path.display()))
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            paths.extend(walk(&path));
        } else {
            paths.push(path);
        }
    }
    paths
}

/// The verdict `aw lint` reaches on a fixture: the refusal, or the findings.
fn verdict(fixture: &Fixture) -> Result<Vec<rules::Finding>, Refusal> {
    if let Some(revision) = &fixture.file.revision {
        super::recognise(revision)?;
    }
    Ok(commit::check(&fixture.change))
}

/// The clauses a rule enforces, so a failing fixture is known to fail for its
/// own clause and not by accident.
fn clauses(rule: &str) -> &'static [&'static str] {
    match rule {
        "frontmatter-missing" => &["5.1", "5.2"],
        "frontmatter-unclosed" => &["5.2"],
        "class-declared" => &["2.2", "5.3"],
        "kind-missing" | "status-missing" => &["2.1"],
        "kind-unregistered" => &["2.4"],
        "class-undeclared" => &["2.3"],
        "status-unknown" => &["3.1"],
        "status-foreign" => &["3.1", "3.2"],
        "successor-missing" => &["3.7"],
        "ephemeral-terminal" => &["4.1.1"],
        "unit-undeclared" => &["4.2.1"],
        "open-in-closed-unit" => &["4.2.3"],
        "compacted-empty" => &["4.2.4"],
        "status-skipped" | "status-backwards" => &["3.3"],
        "binding-rewritten" => &["3.6"],
        "residue-missing" => &["4.1.2"],
        "ephemeral-archived" => &["4.1.4"],
        "close-with-unit" => &["4.2.2"],
        other => panic!("rule {other} cites no clause"),
    }
}

fn suite() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/suite/rev1")
}

#[test]
fn every_fixture_reaches_its_verdict() {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(suite())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    dirs.sort();
    assert_eq!(dirs.len(), 52, "revision 1 holds 52 fixtures");
    assert_eq!(
        dirs.iter().filter(|dir| dir.join("steps").exists()).count(),
        20,
        "20 of them hold commits"
    );

    let mut wrong = Vec::new();
    for dir in &dirs {
        let name = dir.file_name().unwrap().to_string_lossy();
        let fixture = read(dir);
        let clause = fixture.file.clause.as_str();
        let verdict = verdict(&fixture);
        let correct = match (fixture.file.outcome.as_str(), &verdict) {
            ("pass", Ok(findings)) => findings.is_empty(),
            ("fail", Err(_)) => clause == "1.1.1",
            // Clause 2.5 forbids reading meaning into a kind's name, so nothing
            // in a tree breaks it; its failing fixture holds a status the
            // registry's class for that kind does not allow.
            ("fail", Ok(findings)) => findings
                .iter()
                .any(|f| clause == "2.5" || clauses(f.rule).contains(&clause)),
            _ => false,
        };
        if !correct {
            let reached = match verdict {
                Err(refusal) => format!("refused: {refusal}"),
                Ok(findings) if findings.is_empty() => "no findings".to_owned(),
                Ok(findings) => findings
                    .iter()
                    .map(|f| format!("{}: {} [{}]", f.path, f.message, f.rule))
                    .collect::<Vec<_>>()
                    .join("; "),
            };
            wrong.push(format!(
                "{name}: expected {}, reached {reached}",
                fixture.file.outcome
            ));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[test]
fn an_undeclared_file_is_not_read() {
    let fixture = read(&suite().join("2.6-undeclared-file-exempt"));
    let paths: Vec<&str> = fixture
        .change
        .staged
        .artefacts
        .iter()
        .map(|artefact| artefact.path.as_str())
        .collect();
    assert_eq!(paths, ["notes/verdict-alpha.md"]);
}
