//! The contract's rules on a commit: what the staged tree may change from
//! `HEAD`, read from the model alone.
//!
//! As with the snapshot rules, no rule names a kind (clause 2.5). The SHOULD
//! clauses 3.4 and 3.5 never fail.

use super::model::{Artefact, Change, Head, Model};
use super::rules::{
    self, BINDING, CLOSED, EPHEMERAL, EPHEMERAL_TERMINAL, EPISODIC, Finding, OPEN, declared,
};

/// Each class's machine from the contract's section 3, as the moves it allows.
const MOVES: [(&str, &str, &str); 6] = [
    ("standing", "live", "retired"),
    (BINDING, "active", "superseded"),
    (EPISODIC, OPEN, CLOSED),
    (EPISODIC, CLOSED, "compacted"),
    (EPHEMERAL, OPEN, "graduated"),
    (EPHEMERAL, OPEN, "expired"),
];

/// Every finding in the staged tree, and in what it changes from `HEAD`.
pub fn check(change: &Change) -> Vec<Finding> {
    let mut findings = rules::check(&change.staged);
    let Some(head) = &change.head else {
        return findings;
    };

    // Clause 4.1.1 lets one commit record an ephemeral artefact's terminal
    // state and a later one delete it, so only an artefact already terminal at
    // `HEAD` is held to it here.
    findings.retain(|finding| {
        finding.rule != "ephemeral-terminal"
            || counterpart(head, &finding.path)
                .and_then(|before| lifecycle(&head.model, before))
                .is_none_or(|(_, status)| EPHEMERAL_TERMINAL.contains(&status))
    });

    let mut found = |path: &str, rule: &'static str, message: String| {
        findings.push(Finding {
            path: path.to_owned(),
            message,
            rule,
        });
    };
    check_transitions(&change.staged, head, &mut found);
    check_disposals(&change.staged, head, &mut found);
    check_closures(&change.staged, head, &mut found);
    findings
}

/// Clauses 3.3 and 3.6: each artefact against what it was at `HEAD`.
fn check_transitions(
    staged: &Model,
    head: &Head,
    found: &mut impl FnMut(&str, &'static str, String),
) {
    for artefact in &staged.artefacts {
        let Some(before) = counterpart(head, &artefact.path) else {
            continue;
        };
        let (Some((class, status)), Some((was_class, was))) =
            (lifecycle(staged, artefact), lifecycle(&head.model, before))
        else {
            continue;
        };
        if class != was_class {
            continue;
        }
        if status != was
            && !MOVES.contains(&(class, was, status))
            && in_machine(class, was)
            && in_machine(class, status)
        {
            if reaches(class, was, status) {
                found(
                    &artefact.path,
                    "status-skipped",
                    format!("status moves from '{was}' to '{status}', skipping a state"),
                );
            } else {
                found(
                    &artefact.path,
                    "status-backwards",
                    format!(
                        "status moves from '{was}' to '{status}', not forward along class '{class}'"
                    ),
                );
            }
        }
        if class == BINDING && !artefact.body.starts_with(&before.body) {
            found(
                &artefact.path,
                "binding-rewritten",
                "binding artefact changes body content already recorded; new content must \
                 follow it"
                    .to_owned(),
            );
        }
    }
}

/// Clauses 4.1.2 and 4.1.4: an ephemeral artefact leaving the tree.
fn check_disposals(
    staged: &Model,
    head: &Head,
    found: &mut impl FnMut(&str, &'static str, String),
) {
    let records_residue = staged.artefacts.iter().any(|artefact| {
        lifecycle(staged, artefact)
            .is_some_and(|(class, _)| class != EPHEMERAL && class_in_contract(class))
            && counterpart(head, &artefact.path).is_none_or(|before| {
                before.header != artefact.header || before.body != artefact.body
            })
    });
    for before in &head.model.artefacts {
        let Some((EPHEMERAL, status)) = lifecycle(&head.model, before) else {
            continue;
        };
        if status == "graduated" && head.deleted.contains(&before.path) && !records_residue {
            found(
                &before.path,
                "residue-missing",
                "graduated ephemeral artefact deleted, but no standing, binding or episodic \
                 artefact changes in the commit to hold its residue"
                    .to_owned(),
            );
        }
        // Git pairs files by similarity, so a graduated artefact's content
        // carried into a new artefact of another class reads as a rename. That
        // is its residue, recorded as clause 4.1.2 asks, not an archive.
        let archived_to = head.renames.get(&before.path).filter(|to| {
            EPHEMERAL_TERMINAL.contains(&status)
                && staged
                    .artefacts
                    .iter()
                    .find(|artefact| artefact.path == **to)
                    .and_then(|artefact| lifecycle(staged, artefact))
                    .is_none_or(|(class, _)| class == EPHEMERAL || !class_in_contract(class))
        });
        if let Some(to) = archived_to {
            found(
                to,
                "ephemeral-archived",
                format!(
                    "'{status}' ephemeral artefact moved from '{}' instead of deleted; history \
                     keeps it",
                    before.path
                ),
            );
        }
    }
}

/// Clause 4.2.2: a unit of work closing closes its episodic artefacts.
fn check_closures(staged: &Model, head: &Head, found: &mut impl FnMut(&str, &'static str, String)) {
    for unit in staged.units.iter().filter(|unit| unit.state == CLOSED) {
        let closes_here = head
            .model
            .units
            .iter()
            .any(|was| was.name == unit.name && was.state == OPEN);
        if !closes_here {
            continue;
        }
        for artefact in &staged.artefacts {
            if unit.members.contains(&artefact.path)
                && lifecycle(staged, artefact) == Some((EPISODIC, OPEN))
            {
                found(
                    &artefact.path,
                    "close-with-unit",
                    format!(
                        "unit of work '{}' closes in this commit, but this artefact stays 'open'",
                        unit.name
                    ),
                );
            }
        }
    }
}

/// The artefact at `HEAD` a staged path continues: the file it was renamed
/// from, or the one at the same path.
fn counterpart<'a>(head: &'a Head, path: &str) -> Option<&'a Artefact> {
    let from = match head.renames.iter().find(|(_, to)| *to == path) {
        Some((from, _)) => from.as_str(),
        None if head.renames.contains_key(path) => return None,
        None => path,
    };
    head.model
        .artefacts
        .iter()
        .find(|artefact| artefact.path == from)
}

/// An artefact's class and status, when its registry knows both.
fn lifecycle<'a>(model: &'a Model, artefact: &'a Artefact) -> Option<(&'a str, &'a str)> {
    let class = model.registry.kinds.get(declared(artefact, "kind")?)?;
    let status = declared(artefact, "status")?;
    model
        .registry
        .classes
        .get(class)?
        .iter()
        .any(|state| state == status)
        .then_some((class.as_str(), status))
}

fn class_in_contract(class: &str) -> bool {
    MOVES.iter().any(|(machine, _, _)| *machine == class)
}

fn in_machine(class: &str, state: &str) -> bool {
    MOVES
        .iter()
        .any(|(machine, from, to)| *machine == class && (*from == state || *to == state))
}

/// Whether `to` lies further along `class`'s machine than `from`.
fn reaches(class: &str, from: &str, to: &str) -> bool {
    MOVES
        .iter()
        .filter(|(machine, start, _)| *machine == class && *start == from)
        .any(|(_, _, next)| *next == to || reaches(class, next, to))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lint::layout;
    use crate::lint::registry::RegistryFile;
    use std::collections::{BTreeMap, BTreeSet};

    const REGISTRY: &str = "\
[class.ephemeral]
statuses = [\"open\", \"graduated\", \"expired\"]

[class.binding]
statuses = [\"active\", \"superseded\"]

[kind.handover]
class = \"ephemeral\"

[kind.log]
class = \"binding\"
";

    /// The rules a commit breaks taking `notes/a.md` from `before`, or from
    /// nothing, to `after`.
    fn rules_for(before: Option<&str>, after: &str) -> Vec<&'static str> {
        let registry = RegistryFile::parse(REGISTRY, "registry").unwrap();
        let tree = |status: Option<&str>| {
            let files = status.map(|status| {
                (
                    "notes/a.md".to_owned(),
                    format!("---\nkind: handover\nstatus: {status}\n---\nBody.\n"),
                )
            });
            layout::read(&registry, files.into_iter().collect())
        };
        let change = Change {
            staged: tree(Some(after)),
            head: Some(Head {
                model: tree(before),
                renames: BTreeMap::new(),
                deleted: BTreeSet::new(),
            }),
        };
        check(&change)
            .into_iter()
            .map(|finding| finding.rule)
            .collect()
    }

    #[test]
    fn a_commit_may_record_the_terminal_state_for_a_later_one_to_delete() {
        assert!(rules_for(Some("open"), "graduated").is_empty());
    }

    #[test]
    fn a_terminal_ephemeral_left_in_place_fails() {
        assert_eq!(
            rules_for(Some("graduated"), "graduated"),
            ["ephemeral-terminal"]
        );
    }

    #[test]
    fn an_ephemeral_arriving_terminal_fails() {
        assert_eq!(rules_for(None, "expired"), ["ephemeral-terminal"]);
    }

    /// The rules a commit breaks when git pairs a graduated `notes/a.md` with
    /// a new `notes/b.md` of `kind`, as a rename.
    fn rules_for_rename(kind: &str, status: &str) -> Vec<&'static str> {
        let registry = RegistryFile::parse(REGISTRY, "registry").unwrap();
        let note = |path: &str, kind: &str, status: &str| {
            let text = format!("---\nkind: {kind}\nstatus: {status}\n---\nThe outcome.\n");
            layout::read(&registry, vec![(path.to_owned(), text)])
        };
        let change = Change {
            staged: note("notes/b.md", kind, status),
            head: Some(Head {
                model: note("notes/a.md", "handover", "graduated"),
                renames: BTreeMap::from([("notes/a.md".to_owned(), "notes/b.md".to_owned())]),
                deleted: BTreeSet::new(),
            }),
        };
        check(&change)
            .into_iter()
            .map(|finding| finding.rule)
            .collect()
    }

    #[test]
    fn residue_carried_into_another_class_is_not_an_archive() {
        assert!(rules_for_rename("log", "active").is_empty());
    }

    #[test]
    fn a_terminal_ephemeral_moved_as_an_ephemeral_is_archived() {
        assert_eq!(
            rules_for_rename("handover", "graduated"),
            ["ephemeral-terminal", "ephemeral-archived"]
        );
    }

    #[test]
    fn a_move_between_terminal_states_is_not_forward() {
        assert_eq!(
            rules_for(Some("graduated"), "expired"),
            ["ephemeral-terminal", "status-backwards"]
        );
    }
}
