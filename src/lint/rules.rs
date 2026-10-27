//! The contract's rules on a snapshot: what one tree must hold, read from the
//! model alone.
//!
//! No rule names a kind; the registry gives each its class (clause 2.5). The
//! class and state names below are the contract's own, fixed by its section 3.

use super::entries::{self, Entry};
use super::model::{Artefact, Header, Model, Registry, Unit};
use super::proposal::Proposal;
use super::rule::Rule;

pub(super) const BINDING: &str = "binding";
pub(super) const EPHEMERAL: &str = "ephemeral";
pub(super) const EPISODIC: &str = "episodic";
pub(super) const STANDING: &str = "standing";
const SUPERSEDED: &str = "superseded";
pub(super) const EPHEMERAL_TERMINAL: [&str; 2] = ["graduated", "expired"];
pub(super) const OPEN: &str = "open";
pub(super) const CLOSED: &str = "closed";
const COMPACTED: &str = "compacted";

/// The field a superseded binding artefact names its successor in.
const SUCCESSOR: &str = "superseded_by";

/// One rule broken by one file.
pub struct Finding {
    pub path: String,
    pub message: String,
    pub rule: Rule,
}

/// Every finding in the model, in artefact order, then the entries' marks
/// under the proposals opted into.
pub fn check(model: &Model, proposals: &[Proposal]) -> Vec<Finding> {
    let mut findings = Vec::new();
    for artefact in &model.artefacts {
        let mut found = |rule: Rule, message: String| {
            findings.push(Finding {
                path: artefact.path.clone(),
                message,
                rule,
            });
        };
        check_artefact(artefact, &model.registry, &model.units, &mut found);
    }
    if proposals.contains(&Proposal::SupersessionMarks) {
        check_marks(model, &mut findings);
    }
    findings
}

fn check_artefact(
    artefact: &Artefact,
    registry: &Registry,
    units: &[Unit],
    found: &mut impl FnMut(Rule, String),
) {
    match &artefact.header {
        Header::Absent => {
            return found(
                Rule::FrontmatterMissing,
                "missing frontmatter: the file must open with a `---` line".to_owned(),
            );
        }
        Header::Unclosed => {
            return found(
                Rule::FrontmatterUnclosed,
                "frontmatter has no closing `---` line".to_owned(),
            );
        }
        Header::Fields(_) => {}
    }

    if artefact.field("class").is_some() {
        found(
            Rule::ClassDeclared,
            "frontmatter declares 'class', which only the registry gives, by kind".to_owned(),
        );
    }
    let kind = declared(artefact, "kind");
    let status = declared(artefact, "status");
    if kind.is_none() {
        found(Rule::KindMissing, "frontmatter has no 'kind'".to_owned());
    }
    if status.is_none() {
        found(
            Rule::StatusMissing,
            "frontmatter has no 'status'".to_owned(),
        );
    }

    let Some(kind) = kind else { return };
    let Some(class) = registry.kinds.get(kind) else {
        return found(
            Rule::KindUnregistered,
            format!("kind '{kind}' not in registry"),
        );
    };
    let Some(legal) = registry.classes.get(class) else {
        return found(
            Rule::ClassUndeclared,
            format!("kind '{kind}' maps to class '{class}', which the registry does not declare"),
        );
    };
    let Some(status) = status else { return };
    if !legal.iter().any(|state| state == status) {
        return match registry
            .classes
            .iter()
            .find(|(_, states)| states.iter().any(|state| state == status))
        {
            Some((other, _)) => found(
                Rule::StatusForeign,
                format!(
                    "status '{status}' belongs to class '{other}', not to '{class}' (kind '{kind}')"
                ),
            ),
            None => found(
                Rule::StatusUnknown,
                format!(
                    "status '{status}' illegal for class '{class}' (kind '{kind}'; legal: {})",
                    legal.join(" ")
                ),
            ),
        };
    }

    match class.as_str() {
        BINDING if status == SUPERSEDED && declared(artefact, SUCCESSOR).is_none() => found(
            Rule::SuccessorMissing,
            format!("superseded without naming its successor in '{SUCCESSOR}'"),
        ),
        EPHEMERAL if EPHEMERAL_TERMINAL.contains(&status) => found(
            Rule::EphemeralTerminal,
            format!("ephemeral artefact is '{status}' and must leave the working tree"),
        ),
        EPISODIC => check_episodic(artefact, status, units, found),
        _ => {}
    }
}

/// Clauses 4.2.1, 4.2.3 and 4.2.4: an episodic artefact, its unit of work and
/// its compaction.
fn check_episodic(
    artefact: &Artefact,
    status: &str,
    units: &[Unit],
    found: &mut impl FnMut(Rule, String),
) {
    if status == COMPACTED && artefact.body.trim().is_empty() {
        found(
            Rule::CompactedEmpty,
            "compacted artefact keeps nothing beyond its frontmatter".to_owned(),
        );
    }
    let owners: Vec<&Unit> = units
        .iter()
        .filter(|unit| unit.members.contains(&artefact.path))
        .collect();
    if owners.is_empty() {
        found(
            Rule::UnitUndeclared,
            "episodic artefact belongs to no declared unit of work".to_owned(),
        );
    }
    for unit in owners {
        if unit.state == CLOSED && status == OPEN {
            found(
                Rule::OpenInClosedUnit,
                format!(
                    "unit of work '{}' is closed, but this artefact is still 'open'",
                    unit.name
                ),
            );
        }
    }
}

/// Clauses 3.8 and 3.9 of the `supersession-marks` proposal, over the entries
/// of every binding artefact: marks agree, each named entry on its own, and a
/// supersedes mark names only earlier entries. A finding falls on the artefact
/// holding the mark.
fn check_marks(model: &Model, findings: &mut Vec<Finding>) {
    let entries: Vec<(&str, Entry)> = model
        .artefacts
        .iter()
        .filter(|artefact| {
            declared(artefact, "kind")
                .and_then(|kind| model.registry.kinds.get(kind))
                .is_some_and(|class| class == BINDING)
        })
        .flat_map(|artefact| {
            entries::parse(&artefact.body)
                .entries
                .into_iter()
                .map(|entry| (artefact.path.as_str(), entry))
        })
        .collect();
    // A number no binding artefact holds never agrees, and a repeated number
    // agrees only if every entry carrying it does.
    let agree = |number: u64, marked: &dyn Fn(&Entry) -> bool| {
        let mut named = entries
            .iter()
            .filter(|(_, entry)| entry.number == number)
            .peekable();
        named.peek().is_some() && named.all(|(_, entry)| marked(entry))
    };
    for (path, entry) in &entries {
        let mut found = |rule: Rule, message: String| {
            findings.push(Finding {
                path: (*path).to_owned(),
                message,
                rule,
            });
        };
        let number = entry.number;
        for &later in &entry.superseded_by {
            if !agree(later, &|other| other.supersedes.contains(&number)) {
                found(
                    Rule::EntryMarkUnmatched,
                    format!(
                        "entry {number} is marked superseded by entry {later}, which does not \
                         mark that it supersedes entry {number}"
                    ),
                );
            }
        }
        for &earlier in &entry.supersedes {
            if !agree(earlier, &|other| other.superseded_by.contains(&number)) {
                found(
                    Rule::EntryMarkUnmatched,
                    format!(
                        "entry {number} marks that it supersedes entry {earlier}, which is not \
                         marked superseded by entry {number}"
                    ),
                );
            }
        }
        for &named in entry.supersedes.iter().filter(|&&named| named >= number) {
            found(
                Rule::EntrySupersedesLater,
                format!(
                    "entry {number} marks that it supersedes entry {named}, which does not come \
                     before it"
                ),
            );
        }
    }
}

/// A field's value, unless it is absent or empty.
pub(super) fn declared<'a>(artefact: &'a Artefact, key: &str) -> Option<&'a str> {
    artefact.field(key).filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lint::frontmatter;

    const REGISTRY: &str = "\
[class.episodic]
statuses = [\"open\", \"closed\", \"compacted\"]

[kind.minute]
class = \"episodic\"

[engagements.alpha]
status = \"open\"
";

    /// The rules each finding in `text`, read as the one artefact of a
    /// workspace whose engagement `alpha` owns it.
    fn rules_for(text: &str) -> Vec<Rule> {
        let registry = crate::lint::registry::RegistryFile::parse(REGISTRY, "registry").unwrap();
        let model = crate::lint::layout::read(
            &registry,
            vec![(
                "context/engagements/alpha/minute-a.md".to_owned(),
                text.to_owned(),
            )],
        );
        check(&model, &[])
            .into_iter()
            .map(|finding| finding.rule)
            .collect()
    }

    #[test]
    fn a_clean_artefact_has_no_findings() {
        assert!(rules_for("---\nkind: minute\nstatus: open\n---\nBody.\n").is_empty());
    }

    #[test]
    fn a_class_field_fails_by_name() {
        assert_eq!(
            rules_for("---\nkind: minute\nstatus: open\nclass: episodic\n---\nBody.\n"),
            [Rule::ClassDeclared]
        );
    }

    #[test]
    fn an_unclosed_frontmatter_block_fails_by_name() {
        assert_eq!(
            rules_for("---\nkind: minute\nstatus: open\nBody.\n"),
            [Rule::FrontmatterUnclosed]
        );
    }

    #[test]
    fn an_empty_value_counts_as_missing() {
        assert_eq!(
            rules_for("---\nkind: minute\nstatus:\n---\n"),
            [Rule::StatusMissing]
        );
    }

    #[test]
    fn frontmatter_below_the_first_line_is_missing() {
        let text = "# Minute\n\n---\nkind: minute\nstatus: open\n---\n";
        assert_eq!(frontmatter::parse(text).0, Header::Absent);
        assert_eq!(rules_for(text), [Rule::FrontmatterMissing]);
    }

    /// The entry findings in binding artefacts holding `bodies`, a superseded
    /// one among them, under the supersession-marks proposal.
    fn marks_for(bodies: &[&str]) -> Vec<(String, String, Rule)> {
        let registry = crate::lint::registry::RegistryFile::parse(
            "[class.binding]\nstatuses = [\"active\", \"superseded\"]\n\n\
             [kind.verdict]\nclass = \"binding\"\n",
            "registry",
        )
        .unwrap();
        let files = bodies
            .iter()
            .enumerate()
            .map(|(index, body)| {
                let status = if index == 0 {
                    "superseded\nsuperseded_by: v/1.md"
                } else {
                    "active"
                };
                (
                    format!("v/{index}.md"),
                    format!("---\nkind: verdict\nstatus: {status}\n---\n{body}"),
                )
            })
            .collect();
        let model = crate::lint::layout::read(&registry, files);
        check(&model, &[Proposal::SupersessionMarks])
            .into_iter()
            .map(|finding| (finding.path, finding.message, finding.rule))
            .collect()
    }

    #[test]
    fn marks_agree_across_artefacts_whatever_their_status() {
        assert!(
            marks_for(&[
                "1. One. **Superseded by:** 2.\n",
                "2. Two. **Supersedes:** 1.\n"
            ])
            .is_empty()
        );
    }

    #[test]
    fn each_named_entry_is_checked_on_its_own_a_missing_one_included() {
        let found = marks_for(&["1. One. **Superseded by:** 2, 9.\n2. Two. **Supersedes:** 1.\n"]);
        assert_eq!(
            found,
            [(
                "v/0.md".to_owned(),
                "entry 1 is marked superseded by entry 9, which does not mark that it \
                 supersedes entry 1"
                    .to_owned(),
                Rule::EntryMarkUnmatched
            )]
        );
    }

    #[test]
    fn an_unmatched_supersedes_mark_falls_on_the_artefact_holding_it() {
        let found = marks_for(&["1. One.\n", "2. Two. **Supersedes:** 1.\n"]);
        assert_eq!(
            found,
            [(
                "v/1.md".to_owned(),
                "entry 2 marks that it supersedes entry 1, which is not marked superseded by \
                 entry 2"
                    .to_owned(),
                Rule::EntryMarkUnmatched
            )]
        );
    }

    #[test]
    fn a_duplicated_number_agrees_only_if_every_entry_carries_the_mark() {
        let rules = |body: &str| -> Vec<Rule> {
            marks_for(&[body])
                .into_iter()
                .map(|(_, _, rule)| rule)
                .collect()
        };
        assert_eq!(
            rules("1. One. **Superseded by:** 2.\n2. Two. **Supersedes:** 1.\n2. Again.\n"),
            [Rule::EntryMarkUnmatched]
        );
        assert!(
            rules(
                "1. One. **Superseded by:** 2.\n2. Two. **Supersedes:** 1.\n\
                 2. Again. **Supersedes:** 1.\n"
            )
            .is_empty()
        );
    }

    #[test]
    fn a_supersedes_mark_naming_its_own_or_a_later_entry_fails() {
        let found = marks_for(&["1. One. **Supersedes:** 1, 2. **Superseded by:** 1.\n\
                                 2. Two. **Superseded by:** 1.\n"]);
        let rules: Vec<(String, Rule)> = found
            .into_iter()
            .map(|(_, message, rule)| (message, rule))
            .collect();
        assert_eq!(
            rules,
            [
                (
                    "entry 1 marks that it supersedes entry 1, which does not come before it"
                        .to_owned(),
                    Rule::EntrySupersedesLater
                ),
                (
                    "entry 1 marks that it supersedes entry 2, which does not come before it"
                        .to_owned(),
                    Rule::EntrySupersedesLater
                ),
            ]
        );
    }

    #[test]
    fn marks_are_not_read_without_the_proposal() {
        let registry = crate::lint::registry::RegistryFile::parse(
            "[class.binding]\nstatuses = [\"active\"]\n\n[kind.verdict]\nclass = \"binding\"\n",
            "registry",
        )
        .unwrap();
        let model = crate::lint::layout::read(
            &registry,
            vec![(
                "v.md".to_owned(),
                "---\nkind: verdict\nstatus: active\n---\n2. Two. **Supersedes:** 9.\n".to_owned(),
            )],
        );
        assert!(check(&model, &[]).is_empty());
    }
}
