//! The conformance-suite reader and runner, test-only.
//!
//! A fixture is a directory holding `fixture.toml` (the verdict, the globs
//! declaring which files are artefacts, an optional revision) and a tree:
//! `registry.toml`, an optional `units.toml`, and the files themselves. A
//! fixture whose clause concerns commits holds its trees under `steps/01` and
//! `steps/02`; the runner commits the first, stages the second and runs staged
//! mode. The copy under `tests/suite/` names the commit it was taken from.
//!
//! A proposal's fixtures sit under `tests/suite/proposals/<name>/` and run
//! over revision 1's, a fixture of the same name taking the place of
//! revision 1's, with the workspace opted into the proposal.

use super::model::{Change, Head, Model, Unit};
use super::proposal::Proposal;
use super::registry::RegistryFile;
use super::rule::Rule;
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

/// The verdict `aw lint` reaches on a fixture under `proposals`: the
/// refusal, or the findings.
fn verdict(fixture: &Fixture, proposals: &[Proposal]) -> Result<Vec<rules::Finding>, Refusal> {
    if let Some(revision) = &fixture.file.revision {
        super::recognise(revision)?;
    }
    Ok(commit::check(&fixture.change, proposals))
}

fn suite() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/suite/rev1")
}

fn proposal_suite(proposal: Proposal) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/suite/proposals")
        .join(proposal.name())
}

/// The fixture directories under `dir`, by name.
fn fixtures(dir: &Path) -> BTreeMap<String, PathBuf> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            (
                path.file_name().unwrap().to_string_lossy().into_owned(),
                path,
            )
        })
        .collect()
}

/// Each fixture of `dirs` that `aw lint` under `proposals` does not reach
/// the verdict of, with the verdict it reaches.
fn wrong_verdicts(dirs: &BTreeMap<String, PathBuf>, proposals: &[Proposal]) -> Vec<String> {
    let mut wrong = Vec::new();
    for (name, dir) in dirs {
        let fixture = read(dir);
        let clause = fixture.file.clause.as_str();
        let verdict = verdict(&fixture, proposals);
        let correct = match (fixture.file.outcome.as_str(), &verdict) {
            ("pass", Ok(findings)) => findings.is_empty(),
            ("fail", Err(_)) => clause == "1.1.1",
            // Clause 2.5 forbids reading meaning into a kind's name, so nothing
            // in a tree breaks it; its failing fixture holds a status the
            // registry's class for that kind does not allow.
            ("fail", Ok(findings)) => findings
                .iter()
                .any(|f| clause == "2.5" || f.rule.clauses().contains(&clause)),
            _ => false,
        };
        if !correct {
            let reached = match verdict {
                Err(refusal) => format!("refused: {refusal}"),
                Ok(findings) if findings.is_empty() => "no findings".to_owned(),
                Ok(findings) => findings
                    .iter()
                    .map(|f| format!("{}: {} [{}]", f.path, f.message, f.rule.name()))
                    .collect::<Vec<_>>()
                    .join("; "),
            };
            wrong.push(format!(
                "{name}: expected {}, reached {reached}",
                fixture.file.outcome
            ));
        }
    }
    wrong
}

fn with_steps(dirs: &BTreeMap<String, PathBuf>) -> usize {
    dirs.values()
        .filter(|dir| dir.join("steps").exists())
        .count()
}

#[test]
fn every_fixture_reaches_its_verdict() {
    let dirs = fixtures(&suite());
    assert_eq!(dirs.len(), 52, "revision 1 holds 52 fixtures");
    assert_eq!(with_steps(&dirs), 20, "20 of them hold commits");
    let wrong = wrong_verdicts(&dirs, &[]);
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[test]
fn every_fixture_reaches_its_verdict_with_supersession_marks_opted_into() {
    let mut dirs = fixtures(&suite());
    dirs.extend(fixtures(&proposal_suite(Proposal::SupersessionMarks)));
    assert_eq!(dirs.len(), 85, "the overlay holds 85 fixtures");
    assert_eq!(with_steps(&dirs), 42, "42 of them hold commits");
    let wrong = wrong_verdicts(&dirs, &[Proposal::SupersessionMarks]);
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// The rules `aw lint` without the opt-in finds in each of the
/// supersession-marks fixtures `chosen` picks, by fixture name.
fn rules_without_the_opt_in(chosen: impl Fn(&FixtureFile) -> bool) -> Vec<(String, Vec<Rule>)> {
    fixtures(&proposal_suite(Proposal::SupersessionMarks))
        .into_iter()
        .map(|(name, dir)| (name, read(&dir)))
        .filter(|(_, fixture)| chosen(&fixture.file))
        .map(|(name, fixture)| {
            let rules = verdict(&fixture, &[])
                .unwrap()
                .into_iter()
                .map(|finding| finding.rule)
                .collect();
            (name, rules)
        })
        .collect()
}

#[test]
fn the_proposals_passing_3_6_fixtures_fail_revision_1s_3_6() {
    let reached = rules_without_the_opt_in(|file| file.clause == "3.6" && file.outcome == "pass");
    assert_eq!(reached.len(), 12, "{reached:?}");
    for (name, rules) in reached {
        assert_eq!(rules, [Rule::BindingRewritten], "{name}");
    }
}

#[test]
fn the_proposals_3_8_and_3_9_fixtures_have_no_findings_without_the_opt_in() {
    let reached = rules_without_the_opt_in(|file| file.clause == "3.8" || file.clause == "3.9");
    assert_eq!(reached.len(), 11, "{reached:?}");
    for (name, rules) in reached {
        assert!(rules.is_empty(), "{name}: {rules:?}");
    }
}

#[test]
fn every_cited_clause_has_a_failing_fixture() {
    let failing = |dir: PathBuf| -> Vec<String> {
        fixtures(&dir)
            .into_iter()
            .filter(|(_, dir)| {
                let file: FixtureFile = toml::from_str(&text(&dir.join("fixture.toml"))).unwrap();
                file.outcome == "fail"
            })
            .map(|(name, _)| name)
            .collect()
    };
    for &rule in Rule::ALL {
        let mut names = failing(suite());
        if let Some(proposal) = rule.proposal() {
            names.extend(failing(proposal_suite(proposal)));
        }
        for clause in rule.clauses() {
            assert!(
                names
                    .iter()
                    .any(|name| name.starts_with(&format!("{clause}-"))),
                "{} cites {clause}, which has no failing fixture",
                rule.name()
            );
        }
    }
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
