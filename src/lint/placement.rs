//! The template's layout rules: where each artefact sits and what it is named,
//! the engagement register against the topic trees, and the two warnings.
//!
//! Unlike the contract's rules these may name the template's classes and
//! kinds. They run only on a model read from the template's layout.

use super::layout::{ARCHIVE, ENGAGEMENTS, REGISTRY, topic_place};
use super::model::{Artefact, Layout, Model};
use super::rule::Rule;
use super::rules::{BINDING, CLOSED, EPHEMERAL, Finding, OPEN, STANDING, declared};
use std::collections::BTreeSet;

/// The registry's prose companion, a standing artefact lowercase by design.
const COMPANION: &str = "context/artefacts.md";

/// Every layout finding in the model, or none when it was not read from the
/// template's layout.
pub fn check(model: &Model) -> Vec<Finding> {
    let Some(layout) = &model.layout else {
        return Vec::new();
    };
    let mut findings = Vec::new();
    let mut found = |path: &str, rule: Rule, message: String| {
        findings.push(Finding {
            path: path.to_owned(),
            message,
            rule,
        });
    };
    for artefact in &model.artefacts {
        check_artefact(artefact, model, layout, &mut found);
    }
    check_topics(model, layout, &mut found);
    check_register(model, layout, &mut found);
    for path in &layout.untracked {
        found(
            path,
            Rule::ArtefactUntracked,
            "untracked, so invisible to every other check until `git add`".to_owned(),
        );
    }
    findings
}

/// Where one artefact sits and what it is named, by its kind's class.
fn check_artefact(
    artefact: &Artefact,
    model: &Model,
    layout: &Layout,
    found: &mut impl FnMut(&str, Rule, String),
) {
    let Some(kind) = declared(artefact, "kind") else {
        return;
    };
    let Some(class) = model.registry.kinds.get(kind) else {
        return;
    };
    let path = artefact.path.as_str();
    match class.as_str() {
        STANDING | BINDING => {
            // A kind naming no home has nowhere to be held to.
            let Some(home) = layout.homes.get(kind) else {
                return;
            };
            let name = path
                .strip_prefix(home.as_str())
                .and_then(|rest| rest.strip_prefix('/'))
                .and_then(|rest| rest.strip_suffix(".md"));
            let uppercase = name.is_some_and(|name| {
                !name.is_empty()
                    && name
                        .bytes()
                        .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
            });
            if !uppercase && path != COMPANION {
                found(
                    path,
                    Rule::HomeMisplaced,
                    format!(
                        "kind '{kind}' belongs directly in {home}/ under an uppercase name, \
                         words separated by underscore"
                    ),
                );
            }
        }
        _ => match topic_place(path) {
            Some((_, _, None)) => {
                let base = path.rsplit('/').next().unwrap_or(path);
                if !base.starts_with(&format!("{kind}-")) {
                    found(
                        path,
                        Rule::KindPrefix,
                        format!("filename must carry its kind ('{kind}-<slug>.md', got '{base}')"),
                    );
                }
            }
            _ => found(
                path,
                Rule::TopicMisplaced,
                format!(
                    "kind '{kind}' belongs in {ENGAGEMENTS}/<engagement>/{kind}-<slug>.md (or \
                     {ARCHIVE}/ once the engagement is closed)"
                ),
            ),
        },
    }
    // A commit hears only of the open ephemerals it stages, as the script's
    // staged file list gives it; `--all` lists every one.
    if class == EPHEMERAL
        && declared(artefact, "status") == Some(OPEN)
        && layout
            .staged
            .as_ref()
            .is_none_or(|staged| staged.contains(path))
    {
        found(
            path,
            Rule::EphemeralOpen,
            "ephemeral artefact still open; graduate or expire it when its unit lands".to_owned(),
        );
    }
}

/// Each topic directory against the register, and the subdirectories it holds.
/// Reported once per directory, never again per file.
fn check_topics(model: &Model, layout: &Layout, found: &mut impl FnMut(&str, Rule, String)) {
    let mut topics = BTreeSet::new();
    let mut subdirectories = BTreeSet::new();
    for path in &layout.topic_paths {
        let Some((tree, topic, sub)) = topic_place(path) else {
            continue;
        };
        topics.insert((tree, topic));
        if let Some(sub) = sub {
            subdirectories.insert((tree, topic, sub));
        }
    }
    for (tree, topic) in topics {
        let directory = format!("{tree}/{topic}/");
        match model.units.iter().find(|unit| unit.name == topic) {
            None => found(
                &directory,
                Rule::EngagementUnregistered,
                "engagement not in registry".to_owned(),
            ),
            Some(unit) if tree == ENGAGEMENTS && unit.state == CLOSED => found(
                &directory,
                Rule::EngagementMisplaced,
                format!("engagement is closed, belongs under {ARCHIVE}/{topic}/"),
            ),
            Some(unit) if tree == ARCHIVE && unit.state != CLOSED => found(
                &directory,
                Rule::EngagementMisplaced,
                format!("engagement is not closed, belongs under {ENGAGEMENTS}/{topic}/"),
            ),
            Some(_) => {}
        }
    }
    for (tree, topic, sub) in subdirectories {
        if !layout.subdirectories.iter().any(|name| name == sub) {
            found(
                &format!("{tree}/{topic}/{sub}/"),
                Rule::SubdirectoryUndeclared,
                format!(
                    "undeclared subdirectory (registry permits: {})",
                    layout.subdirectories.join(" ")
                ),
            );
        }
    }
}

/// Each engagement in the register: present on disk while open, with a ledger
/// once started, and depending only on registered engagements.
#[expect(
    clippy::case_sensitive_file_extension_comparisons,
    reason = "the script's `ledger-*.md` glob is case-sensitive"
)]
fn check_register(model: &Model, layout: &Layout, found: &mut impl FnMut(&str, Rule, String)) {
    for unit in &model.units {
        if unit.state == OPEN {
            let mut files = layout
                .topic_paths
                .iter()
                .filter_map(|path| match topic_place(path) {
                    Some((ENGAGEMENTS, topic, sub)) if topic == unit.name => Some((path, sub)),
                    _ => None,
                })
                .peekable();
            if files.peek().is_none() {
                found(
                    REGISTRY,
                    Rule::EngagementAbsent,
                    format!(
                        "open engagement '{}' has no directory under {ENGAGEMENTS}/",
                        unit.name
                    ),
                );
            }
            // Started: a topic-level file beyond its own plan or spec.
            let (mut started, mut ledger) = (false, false);
            for (path, _) in files.filter(|(_, sub)| sub.is_none()) {
                let base = path.rsplit('/').next().unwrap_or(path);
                let prefixed = |kind: &str| base.starts_with(kind) && base.ends_with(".md");
                if prefixed("ledger-") {
                    ledger = true;
                } else if !prefixed("plan-") && !prefixed("spec-") {
                    started = true;
                }
            }
            if started && !ledger {
                found(
                    &format!("{ENGAGEMENTS}/{}/", unit.name),
                    Rule::LedgerMissing,
                    "engagement has started (holds a file beyond plan/spec) but has no \
                     ledger-*.md"
                        .to_owned(),
                );
            }
        }
        for dependency in &unit.depends_on {
            if !model.units.iter().any(|other| other.name == *dependency) {
                found(
                    REGISTRY,
                    Rule::DependencyUnregistered,
                    format!(
                        "engagement '{}' depends on unregistered '{dependency}'",
                        unit.name
                    ),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lint::layout;
    use crate::lint::registry::RegistryFile;

    const REGISTRY_TEXT: &str = "\
[subdirectory.attachments]
tracked = true

[subdirectory.tmp]
tracked = false

[class.standing]
statuses = [\"live\", \"retired\"]

[class.binding]
statuses = [\"active\", \"superseded\"]

[class.episodic]
statuses = [\"open\", \"closed\", \"compacted\"]

[class.ephemeral]
statuses = [\"open\", \"graduated\", \"expired\"]

[kind.state]
class = \"standing\"
home = \"context\"

[kind.doctrine]
class = \"standing\"
home = \"context\"

[kind.decision]
class = \"binding\"
home = \"context\"

[kind.charter]
class = \"standing\"

[kind.plan]
class = \"episodic\"
home = \"context/engagements\"

[kind.ledger]
class = \"episodic\"
home = \"context/engagements\"

[kind.report]
class = \"ephemeral\"
home = \"context/engagements\"

[engagements.alpha]
status = \"open\"
depends-on = [\"beta\"]

[engagements.beta]
status = \"closed\"
";

    /// A workspace in the template's layout, clean under every layout rule.
    const CLEAN: [(&str, &str); 7] = [
        ("context/STATE.md", "state live"),
        ("context/DECISIONS.md", "decision active"),
        ("context/PRODUCT_ROADMAP2.md", "doctrine live"),
        ("context/artefacts.md", "doctrine live"),
        ("context/engagements/alpha/plan-a.md", "plan open"),
        ("context/engagements/alpha/ledger-a.md", "ledger open"),
        ("context/archive/beta/plan-b.md", "plan closed"),
    ];

    /// The layout findings for `CLEAN` with `files` added, each a path and its
    /// kind and status; `others` are tracked topic paths holding no artefact.
    fn findings(
        files: &[(&str, &str)],
        others: &[&str],
        untracked: &[&str],
    ) -> Vec<(String, Rule)> {
        findings_in(REGISTRY_TEXT, files, others, untracked)
    }

    fn findings_in(
        registry: &str,
        files: &[(&str, &str)],
        others: &[&str],
        untracked: &[&str],
    ) -> Vec<(String, Rule)> {
        let registry = RegistryFile::parse(registry, "registry").unwrap();
        let files: Vec<(String, String)> = CLEAN
            .iter()
            .chain(files)
            .map(|(path, declaration)| {
                let (kind, status) = declaration.split_once(' ').unwrap();
                (
                    (*path).to_owned(),
                    format!("---\nkind: {kind}\nstatus: {status}\n---\nBody.\n"),
                )
            })
            .collect();
        let topic_paths = files
            .iter()
            .map(|(path, _)| path.as_str())
            .chain(others.iter().copied())
            .filter(|path| topic_place(path).is_some())
            .map(str::to_owned)
            .collect();
        let mut model = layout::read(&registry, files);
        model.layout = Some(Layout {
            topic_paths,
            untracked: untracked.iter().map(|path| (*path).to_owned()).collect(),
            ..layout::declared(&registry)
        });
        check(&model)
            .into_iter()
            .map(|finding| (finding.path, finding.rule))
            .collect()
    }

    fn found(path: &str, rule: Rule) -> (String, Rule) {
        (path.to_owned(), rule)
    }

    #[test]
    fn the_template_layout_is_clean() {
        assert_eq!(
            findings(
                &[],
                &["context/engagements/alpha/attachments/notes.txt"],
                &[]
            ),
            []
        );
    }

    #[test]
    fn a_model_read_from_no_layout_gets_no_layout_rules() {
        let registry = RegistryFile::parse(REGISTRY_TEXT, "registry").unwrap();
        let model = layout::read(
            &registry,
            vec![(
                "notes/plan-a.md".to_owned(),
                "---\nkind: plan\nstatus: open\n---\n".to_owned(),
            )],
        );
        assert!(check(&model).is_empty());
    }

    #[test]
    fn a_standing_or_binding_artefact_sits_in_its_home_uppercase() {
        assert_eq!(
            findings(
                &[
                    ("context/Notes.md", "doctrine live"),
                    ("context/notes/NOTES.md", "doctrine live"),
                    ("DECISIONS.md", "decision active"),
                    ("context/.md", "doctrine live"),
                ],
                &[],
                &[]
            ),
            [
                found("context/Notes.md", Rule::HomeMisplaced),
                found("context/notes/NOTES.md", Rule::HomeMisplaced),
                found("DECISIONS.md", Rule::HomeMisplaced),
                found("context/.md", Rule::HomeMisplaced),
            ]
        );
    }

    #[test]
    fn a_kind_naming_no_home_is_not_held_to_one() {
        assert_eq!(
            findings(&[("notes/charter.md", "charter live")], &[], &[]),
            []
        );
    }

    #[test]
    fn an_episodic_or_ephemeral_artefact_sits_one_topic_down() {
        assert_eq!(
            findings(
                &[
                    ("context/plan-x.md", "plan open"),
                    ("context/engagements/plan-x.md", "plan open"),
                ],
                &[],
                &[]
            ),
            [
                found("context/plan-x.md", Rule::TopicMisplaced),
                found("context/engagements/plan-x.md", Rule::TopicMisplaced),
            ]
        );
    }

    #[test]
    fn a_file_in_an_undeclared_subdirectory_fails_placement_too() {
        // The script skips every subdirectory's contents per file, so it
        // reports only the directory.
        assert_eq!(
            findings(
                &[("context/engagements/alpha/notes/plan-x.md", "plan open")],
                &[],
                &[]
            ),
            [
                found(
                    "context/engagements/alpha/notes/plan-x.md",
                    Rule::TopicMisplaced
                ),
                found(
                    "context/engagements/alpha/notes/",
                    Rule::SubdirectoryUndeclared
                ),
            ]
        );
    }

    #[test]
    fn a_topic_level_filename_carries_its_kind() {
        assert_eq!(
            findings(
                &[
                    ("context/engagements/alpha/a-plan.md", "plan open"),
                    ("context/engagements/alpha/planning.md", "plan open"),
                ],
                &[],
                &[]
            ),
            [
                found("context/engagements/alpha/a-plan.md", Rule::KindPrefix),
                found("context/engagements/alpha/planning.md", Rule::KindPrefix),
            ]
        );
    }

    #[test]
    fn an_unregistered_topic_directory_is_reported_once() {
        assert_eq!(
            findings(
                &[
                    ("context/engagements/gamma/plan-a.md", "plan open"),
                    ("context/engagements/gamma/plan-b.md", "plan open"),
                ],
                &["context/archive/delta/attachments/notes.txt"],
                &[]
            ),
            [
                found("context/archive/delta/", Rule::EngagementUnregistered),
                found("context/engagements/gamma/", Rule::EngagementUnregistered),
            ]
        );
    }

    #[test]
    fn a_topic_directory_sits_in_the_tree_its_status_names() {
        assert_eq!(
            findings(
                &[
                    ("context/engagements/beta/plan-c.md", "plan closed"),
                    ("context/archive/alpha/plan-d.md", "plan open"),
                ],
                &[],
                &[]
            ),
            [
                found("context/archive/alpha/", Rule::EngagementMisplaced),
                found("context/engagements/beta/", Rule::EngagementMisplaced),
            ]
        );
    }

    #[test]
    fn an_open_engagement_has_a_directory() {
        let registry = format!("{REGISTRY_TEXT}\n[engagements.gamma]\nstatus = \"open\"\n");
        assert_eq!(
            findings_in(
                &registry,
                &[],
                &["context/archive/gamma/attachments/notes.txt"],
                &[]
            ),
            [
                found("context/archive/gamma/", Rule::EngagementMisplaced),
                found(layout::REGISTRY, Rule::EngagementAbsent),
            ]
        );
    }

    #[test]
    fn a_dependency_is_registered() {
        let registry = format!(
            "{REGISTRY_TEXT}\n[engagements.gamma]\nstatus = \"closed\"\n\
             depends-on = [\"alpha\", \"omega\"]\n"
        );
        assert_eq!(
            findings_in(&registry, &[], &[], &[]),
            [found(layout::REGISTRY, Rule::DependencyUnregistered)]
        );
    }

    #[test]
    fn a_topic_holds_only_declared_tracked_subdirectories() {
        assert_eq!(
            findings(
                &[],
                &[
                    "context/engagements/alpha/data/a.json",
                    "context/engagements/alpha/data/b.json",
                    "context/archive/beta/tmp/scratch.txt",
                ],
                &[]
            ),
            [
                found("context/archive/beta/tmp/", Rule::SubdirectoryUndeclared),
                found(
                    "context/engagements/alpha/data/",
                    Rule::SubdirectoryUndeclared
                ),
            ]
        );
    }

    #[test]
    fn a_started_engagement_holds_a_ledger() {
        let registry = REGISTRY_TEXT.replace(
            "[engagements.beta]",
            "[engagements.gamma]\nstatus = \"open\"\n\n[engagements.beta]",
        );
        let gamma = |extra: &[&str]| {
            let mut others = vec![
                "context/engagements/gamma/plan-g.md",
                "context/engagements/gamma/spec-g.md",
                "context/engagements/gamma/attachments/source.pdf",
            ];
            others.extend(extra);
            findings_in(&registry, &[], &others, &[])
        };
        assert_eq!(gamma(&[]), []);
        assert_eq!(
            gamma(&["context/engagements/gamma/report-01-g.md"]),
            [found("context/engagements/gamma/", Rule::LedgerMissing)]
        );
        assert_eq!(
            gamma(&["context/engagements/gamma/diagram.svg"]),
            [found("context/engagements/gamma/", Rule::LedgerMissing)]
        );
        assert_eq!(
            gamma(&[
                "context/engagements/gamma/report-01-g.md",
                "context/engagements/gamma/ledger-g.md",
            ]),
            []
        );
    }

    #[test]
    fn an_open_ephemeral_warns() {
        let findings = findings(
            &[
                ("context/engagements/alpha/report-01-a.md", "report open"),
                ("context/engagements/alpha/report-02-a.md", "report expired"),
            ],
            &[],
            &[],
        );
        assert_eq!(
            findings,
            [found(
                "context/engagements/alpha/report-01-a.md",
                Rule::EphemeralOpen
            )]
        );
        assert_eq!(
            Rule::EphemeralOpen.severity(),
            crate::lint::rule::Severity::Warning
        );
    }

    #[test]
    fn an_untracked_markdown_file_warns() {
        assert_eq!(
            findings(&[], &[], &["context/NOTES.md"]),
            [found("context/NOTES.md", Rule::ArtefactUntracked)]
        );
        assert_eq!(
            Rule::ArtefactUntracked.severity(),
            crate::lint::rule::Severity::Warning
        );
    }
}
