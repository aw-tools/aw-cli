//! The contract's rules on a commit: what the staged tree may change from
//! `HEAD`, read from the model alone.
//!
//! As with the snapshot rules, no rule names a kind (clause 2.5). The SHOULD
//! clauses 3.4 and 3.5 never fail.

use super::entries;
use super::model::{Artefact, Change, Head, Model};
use super::proposal::Proposal;
use super::rule::Rule;
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

/// Every finding in the staged tree, and in what it changes from `HEAD`,
/// under the proposals opted into.
pub fn check(change: &Change, proposals: &[Proposal]) -> Vec<Finding> {
    let mut findings = rules::check(&change.staged, proposals);
    let Some(head) = &change.head else {
        return findings;
    };

    // Clause 4.1.1 lets one commit record an ephemeral artefact's terminal
    // state and a later one delete it, so only an artefact already terminal at
    // `HEAD` is held to it here.
    findings.retain(|finding| {
        finding.rule != Rule::EphemeralTerminal
            || counterpart(head, &finding.path)
                .and_then(|before| lifecycle(&head.model, before))
                .is_none_or(|(_, status)| EPHEMERAL_TERMINAL.contains(&status))
    });

    let mut found = |path: &str, rule: Rule, message: String| {
        findings.push(Finding {
            path: path.to_owned(),
            message,
            rule,
        });
    };
    check_transitions(&change.staged, head, proposals, &mut found);
    check_disposals(&change.staged, head, &mut found);
    check_closures(&change.staged, head, &mut found);
    findings
}

/// Clauses 3.3 and 3.6: each artefact against what it was at `HEAD`.
fn check_transitions(
    staged: &Model,
    head: &Head,
    proposals: &[Proposal],
    found: &mut impl FnMut(&str, Rule, String),
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
                    Rule::StatusSkipped,
                    format!("status moves from '{was}' to '{status}', skipping a state"),
                );
            } else {
                found(
                    &artefact.path,
                    Rule::StatusBackwards,
                    format!(
                        "status moves from '{was}' to '{status}', not forward along class '{class}'"
                    ),
                );
            }
        }
        if class == BINDING && proposals.contains(&Proposal::SupersessionMarks) {
            if let Some(message) = rewritten(&before.body, &artefact.body) {
                found(&artefact.path, Rule::BindingRewritten, message);
            }
        } else if class == BINDING && !artefact.body.starts_with(&before.body) {
            found(
                &artefact.path,
                Rule::BindingRewritten,
                "binding artefact changes body content already recorded; new content must \
                 follow it"
                    .to_owned(),
            );
        }
    }
}

/// Clause 3.6 as the `supersession-marks` proposal words it, marks and layout
/// aside: how a binding artefact's body changes what `before` recorded, if it
/// does. The recorded entries must open the body, each keeping its number and
/// words, new entries must be numbered above the highest of them, and the words
/// outside entries must begin with the recorded ones.
fn rewritten(before: &str, after: &str) -> Option<String> {
    let (was, now) = (entries::parse(before), entries::parse(after));
    for (index, recorded) in was.entries.iter().enumerate() {
        let kept = now
            .entries
            .get(index)
            .is_some_and(|entry| entry.number == recorded.number && entry.words == recorded.words);
        if !kept {
            return Some(format!(
                "binding artefact changes or moves entry {}, already recorded; new entries must \
                 follow the recorded ones",
                recorded.number
            ));
        }
    }
    let highest = was.entries.iter().map(|entry| entry.number).max();
    let misplaced = highest.and_then(|highest| {
        now.entries[was.entries.len()..]
            .iter()
            .find(|entry| entry.number <= highest)
            .map(|entry| (entry.number, highest))
    });
    if let Some((number, highest)) = misplaced {
        return Some(format!(
            "binding artefact adds entry {number}, not numbered above recorded entry {highest}; \
             new entries must follow the recorded ones"
        ));
    }
    (!now.outside.starts_with(&was.outside)).then(|| {
        "binding artefact changes body content already recorded outside its entries; new \
         content must follow it"
            .to_owned()
    })
}

/// Clauses 4.1.2 and 4.1.4: an ephemeral artefact leaving the tree.
fn check_disposals(staged: &Model, head: &Head, found: &mut impl FnMut(&str, Rule, String)) {
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
                Rule::ResidueMissing,
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
                Rule::EphemeralArchived,
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
fn check_closures(staged: &Model, head: &Head, found: &mut impl FnMut(&str, Rule, String)) {
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
                    Rule::CloseWithUnit,
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
    fn rules_for(before: Option<&str>, after: &str) -> Vec<Rule> {
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
        check(&change, &[])
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
            [Rule::EphemeralTerminal]
        );
    }

    #[test]
    fn an_ephemeral_arriving_terminal_fails() {
        assert_eq!(rules_for(None, "expired"), [Rule::EphemeralTerminal]);
    }

    /// The rules a commit breaks when git pairs a graduated `notes/a.md` with
    /// a new `notes/b.md` of `kind`, as a rename.
    fn rules_for_rename(kind: &str, status: &str) -> Vec<Rule> {
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
        check(&change, &[])
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
            [Rule::EphemeralTerminal, Rule::EphemeralArchived]
        );
    }

    #[test]
    fn a_move_between_terminal_states_is_not_forward() {
        assert_eq!(
            rules_for(Some("graduated"), "expired"),
            [Rule::EphemeralTerminal, Rule::StatusBackwards]
        );
    }

    #[test]
    fn under_the_proposal_new_entries_and_words_may_follow_the_recorded_ones() {
        let before = "Intro.\n\n1. One.\n2. Two.\n";
        assert_eq!(rewritten(before, before), None);
        assert_eq!(
            rewritten(before, "Intro.\n\n1. One.\n2. Two.\n3. Three.\n\nCoda.\n"),
            None
        );
    }

    #[test]
    fn under_the_proposal_an_entry_numbered_below_the_last_recorded_fails() {
        assert_eq!(
            rewritten("1. One.\n2. Two.\n", "1. One.\n2. Two.\n0. Zero.\n").as_deref(),
            Some(
                "binding artefact adds entry 0, not numbered above recorded entry 2; new entries \
                 must follow the recorded ones"
            )
        );
    }

    #[test]
    fn under_the_proposal_a_new_entry_numbered_below_the_highest_recorded_fails() {
        assert_eq!(
            rewritten("1. A.\n5. E.\n2. B.\n", "1. A.\n5. E.\n2. B.\n3. C.\n").as_deref(),
            Some(
                "binding artefact adds entry 3, not numbered above recorded entry 5; new entries \
                 must follow the recorded ones"
            )
        );
    }

    #[test]
    fn under_the_proposal_a_block_under_an_entry_line_keeps_its_words_outside_it() {
        assert_eq!(
            rewritten(
                "1. Use TOML for the registry:\n- one file per kind\n",
                "1. Use TOML for the registry:\n\n- one file per kind\n"
            ),
            None
        );
    }

    #[test]
    fn under_the_proposal_marks_may_join_entries_indented_up_to_three_spaces() {
        for indent in ["", " "] {
            let before = format!("{indent}1. Use TOML.\n 2. Use JSON.\n");
            let after = format!(
                "{indent}1. Use TOML. **Superseded by:** 2.\n 2. Use JSON. **Supersedes:** 1.\n"
            );
            assert_eq!(rewritten(&before, &after), None, "{indent:?}");
        }
    }

    #[test]
    fn under_the_proposal_a_mark_with_an_unspaced_list_may_join_a_recorded_entry() {
        assert_eq!(
            rewritten(
                "1. Use TOML.\n2. Keep one file.\n",
                "1. Use TOML.\n2. Keep one file.\n3. Use JSON. **Supersedes:** 1,2.\n"
            ),
            None
        );
        assert_eq!(
            rewritten(
                "1. Use TOML.\n2. Keep one file.\n3. Use JSON.\n",
                "1. Use TOML.\n2. Keep one file.\n3. Use JSON. **Supersedes:** 1,2.\n"
            ),
            None
        );
    }

    #[test]
    fn under_the_proposal_a_dropped_entry_fails_naming_it() {
        assert_eq!(
            rewritten("1. One.\n2. Two.\n", "1. One.\n").as_deref(),
            Some(
                "binding artefact changes or moves entry 2, already recorded; new entries must \
                 follow the recorded ones"
            )
        );
    }

    #[test]
    fn under_the_proposal_words_outside_entries_must_extend_the_recorded_ones() {
        let before = "Intro text.\n\n1. One.\n";
        assert_eq!(
            rewritten(before, "Intro, rewritten.\n\n1. One.\n").as_deref(),
            Some(
                "binding artefact changes body content already recorded outside its entries; \
                 new content must follow it"
            )
        );
        assert_eq!(rewritten(before, "Intro\ntext.\n\n1. One.\n"), None);
    }
}
