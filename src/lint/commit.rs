//! The contract's rules on a commit: what the staged tree may change from
//! `HEAD`, read from the model alone.
//!
//! As with the snapshot rules, no rule names a kind (clause 2.5). The SHOULD
//! clauses 3.4 and 3.5 never fail. Under the `supersession-marks` proposal, a
//! warning outside the contract joins them.

use super::entries;
use super::model::{Artefact, Change, Head, Model};
use super::proposal::Proposal;
use super::rule::Rule;
use super::rules::{
    self, BINDING, CLOSED, EPHEMERAL, EPHEMERAL_TERMINAL, EPISODIC, Finding, OPEN, declared,
};
use std::collections::BTreeSet;

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
    if proposals.contains(&Proposal::SupersessionMarks) {
        check_same_day(&change.staged, head, &mut found);
    }
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

/// Clause 3.6 as the `supersession-marks` proposal words it, marks, legacy
/// notes and layout aside: how a binding artefact's body changes what `before`
/// recorded, if it does. The recorded entries must open the body, each keeping
/// its number, its words and every entry its two sets name, new entries must be
/// numbered above the highest of them, and the words outside entries must begin
/// with the recorded ones.
fn rewritten(before: &str, after: &str) -> Option<String> {
    let (was, now) = (entries::parse(before), entries::parse(after));
    for (index, recorded) in was.entries.iter().enumerate() {
        let Some(entry) = now
            .entries
            .get(index)
            .filter(|entry| entry.number == recorded.number && entry.words == recorded.words)
        else {
            return Some(format!(
                "binding artefact changes or moves entry {}, already recorded; new entries must \
                 follow the recorded ones",
                recorded.number
            ));
        };
        let number = entry.number;
        let supersedes = |entry: &entries::Entry| -> BTreeSet<u64> {
            entry.supersedes.iter().copied().collect()
        };
        if let Some(named) = supersedes(recorded).difference(&supersedes(entry)).next() {
            return Some(format!(
                "binding artefact drops entry {named} from what entry {number} supersedes, \
                 already recorded; an entry once named must stay named"
            ));
        }
        let superseded_by = |entry: &entries::Entry| -> BTreeSet<u64> {
            entry
                .superseded_by
                .iter()
                .chain(&entry.noted)
                .copied()
                .collect()
        };
        if let Some(named) = superseded_by(recorded)
            .difference(&superseded_by(entry))
            .next()
        {
            return Some(format!(
                "binding artefact drops entry {named} from what supersedes entry {number}, \
                 already recorded; an entry once named must stay named"
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

/// An entry the commit adds superseding one that carries the same date, once
/// for each entry so named. An entry is added when no artefact at `HEAD` holds
/// its number at a path binding at `HEAD` or in the staged tree, so a mark
/// joining a recorded entry never warns, nor does a commit that first makes a
/// register binding.
fn check_same_day(staged: &Model, head: &Head, found: &mut impl FnMut(&str, Rule, String)) {
    let recorded: BTreeSet<u64> = head
        .model
        .artefacts
        .iter()
        .filter(|before| {
            binding(&head.model, before)
                || staged
                    .artefacts
                    .iter()
                    .any(|artefact| artefact.path == before.path && binding(staged, artefact))
        })
        .flat_map(|before| entries::parse(&before.body).entries)
        .map(|entry| entry.number)
        .collect();
    let entries = binding_entries(staged);
    for (path, entry) in &entries {
        let Some(date) = &entry.date else {
            continue;
        };
        if recorded.contains(&entry.number) {
            continue;
        }
        let named: BTreeSet<u64> = entry.supersedes.iter().copied().collect();
        for earlier in named {
            let same_day = entries
                .iter()
                .any(|(_, other)| other.number == earlier && other.date.as_ref() == Some(date));
            if same_day {
                found(
                    path,
                    Rule::EntrySupersedesSameDay,
                    format!(
                        "entry {} supersedes entry {earlier}, which carries the same date, {date}",
                        entry.number
                    ),
                );
            }
        }
    }
}

/// Every entry of the model's binding artefacts, binding by class whatever
/// their status, with the path of the artefact holding it.
fn binding_entries(model: &Model) -> Vec<(&str, entries::Entry<'_>)> {
    model
        .artefacts
        .iter()
        .filter(|artefact| binding(model, artefact))
        .flat_map(|artefact| {
            entries::parse(&artefact.body)
                .entries
                .into_iter()
                .map(|entry| (artefact.path.as_str(), entry))
        })
        .collect()
}

/// Whether the model's registry gives an artefact's kind the binding class.
fn binding(model: &Model, artefact: &Artefact) -> bool {
    declared(artefact, "kind")
        .and_then(|kind| model.registry.kinds.get(kind))
        .is_some_and(|class| class == BINDING)
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

    #[test]
    fn under_the_proposal_a_number_dropped_from_a_mark_fails_naming_it() {
        let before = "1. One. **Superseded by:** 2, 3.\n2. Two. **Supersedes:** 1.\n\
                      3. Three. **Supersedes:** 1.\n";
        assert_eq!(
            rewritten(
                before,
                "1. One. **Superseded by:** 3.\n2. Two. **Supersedes:** 1.\n\
                 3. Three. **Supersedes:** 1.\n"
            )
            .as_deref(),
            Some(
                "binding artefact drops entry 2 from what supersedes entry 1, already recorded; \
                 an entry once named must stay named"
            )
        );
        assert_eq!(
            rewritten(
                before,
                "1. One. **Superseded by:** 2, 3.\n2. Two.\n3. Three. **Supersedes:** 1.\n"
            )
            .as_deref(),
            Some(
                "binding artefact drops entry 1 from what entry 2 supersedes, already recorded; \
                 an entry once named must stay named"
            )
        );
    }

    #[test]
    fn under_the_proposal_marks_may_move_reorder_and_gain_numbers() {
        let before = "1. One. **Superseded by:** 2.\n2. Two. **Supersedes:** 1.\n3. Three.\n";
        assert_eq!(
            rewritten(
                before,
                "1. **Superseded by:** 3, 2. One.\n2. Two. **Supersedes:** 1.\n\
                 3. Three. **Supersedes:** 1.\n"
            ),
            None
        );
    }

    #[test]
    fn under_the_proposal_a_legacy_note_may_become_a_mark_but_not_vanish() {
        let before = "1. One.\n   - _SUPERSEDED by 2 (this bullet only)._\n2. Two.\n";
        assert_eq!(
            rewritten(
                before,
                "1. One.\n   - _**Superseded by:** 2. (this bullet only)._\n\
                 2. Two. **Supersedes:** 1.\n"
            ),
            None
        );
        assert_eq!(
            rewritten("1. One. SUPERSEDED by 2.\n2. Two.\n", "1. One.\n2. Two.\n").as_deref(),
            Some(
                "binding artefact drops entry 2 from what supersedes entry 1, already recorded; \
                 an entry once named must stay named"
            )
        );
    }

    /// The model of binding artefacts `notes/0.md` onwards, holding `bodies`.
    fn logs(bodies: &[&str]) -> Model {
        let registry = RegistryFile::parse(REGISTRY, "registry").unwrap();
        let files = bodies
            .iter()
            .enumerate()
            .map(|(index, body)| {
                (
                    format!("notes/{index}.md"),
                    format!("---\nkind: log\nstatus: active\n---\n{body}"),
                )
            })
            .collect();
        layout::read(&registry, files)
    }

    /// The same-day warnings, by path and message, of a commit taking binding
    /// artefacts from `before`, or from no `HEAD`, to `after`, under
    /// `proposals`.
    fn same_day(
        before: Option<&[&str]>,
        after: &[&str],
        proposals: &[Proposal],
    ) -> Vec<(String, String)> {
        let change = Change {
            staged: logs(after),
            head: before.map(|before| Head {
                model: logs(before),
                renames: BTreeMap::new(),
                deleted: BTreeSet::new(),
            }),
        };
        check(&change, proposals)
            .into_iter()
            .filter(|finding| finding.rule == Rule::EntrySupersedesSameDay)
            .map(|finding| (finding.path, finding.message))
            .collect()
    }

    const RECORDED: &str = "1. **Use TOML (2026-10-26).** One.\n";

    /// `RECORDED` stamped superseded by an added entry 2 titled `title`.
    fn superseded_by(title: &str) -> String {
        format!(
            "1. **Use TOML (2026-10-26).** One. **Superseded by:** 2.\n\
             2. {title} Two. **Supersedes:** 1.\n"
        )
    }

    const MARKS: &[Proposal] = &[Proposal::SupersessionMarks];

    #[test]
    fn an_added_entry_superseding_one_of_its_date_warns_once() {
        assert_eq!(
            same_day(
                Some(&[RECORDED]),
                &[&superseded_by("**Use JSON (2026-10-26).**")],
                MARKS
            ),
            [(
                "notes/0.md".to_owned(),
                "entry 2 supersedes entry 1, which carries the same date, 2026-10-26".to_owned()
            )]
        );
    }

    #[test]
    fn the_same_day_rule_is_a_warning() {
        assert_eq!(
            Rule::EntrySupersedesSameDay.severity(),
            crate::lint::rule::Severity::Warning
        );
    }

    #[test]
    fn the_same_day_warning_falls_on_the_artefact_holding_the_mark() {
        let found = same_day(
            Some(&[RECORDED]),
            &[
                "1. **Use TOML (2026-10-26).** One. **Superseded by:** 2.\n",
                "2. **Use JSON (2026-10-26).** Two. **Supersedes:** 1.\n",
            ],
            MARKS,
        );
        let paths: Vec<&str> = found.iter().map(|(path, _)| path.as_str()).collect();
        assert_eq!(paths, ["notes/1.md"]);
    }

    #[test]
    fn an_added_entry_superseding_one_of_another_date_does_not_warn() {
        let after = superseded_by("**Use JSON (2026-10-27).**");
        assert!(same_day(Some(&[RECORDED]), &[&after], MARKS).is_empty());
    }

    #[test]
    fn an_undated_side_does_not_warn() {
        let after = superseded_by("**Use JSON.**");
        assert!(same_day(Some(&[RECORDED]), &[&after], MARKS).is_empty());
        let before = "1. **Use TOML.** One.\n";
        let after = "1. **Use TOML.** One. **Superseded by:** 2.\n\
                     2. **Use JSON (2026-10-26).** Two. **Supersedes:** 1.\n";
        assert!(same_day(Some(&[before]), &[after], MARKS).is_empty());
    }

    #[test]
    fn a_mark_added_to_a_recorded_entry_does_not_warn() {
        let before = "1. **Use TOML (2026-10-26).** One.\n2. **Use JSON (2026-10-26).** Two.\n";
        let after = superseded_by("**Use JSON (2026-10-26).**");
        assert!(same_day(Some(&[before]), &[&after], MARKS).is_empty());
    }

    #[test]
    fn a_commit_with_no_head_does_not_warn() {
        let after = superseded_by("**Use JSON (2026-10-26).**");
        assert!(same_day(None, &[&after], MARKS).is_empty());
    }

    #[test]
    fn the_snapshot_rules_lint_all_runs_do_not_warn_of_a_same_day_entry() {
        let after = superseded_by("**Use JSON (2026-10-26).**");
        assert!(
            rules::check(&logs(&[&after]), MARKS)
                .iter()
                .all(|finding| finding.rule != Rule::EntrySupersedesSameDay)
        );
    }

    #[test]
    fn without_the_proposal_a_same_day_entry_does_not_warn() {
        let after = superseded_by("**Use JSON (2026-10-26).**");
        assert!(same_day(Some(&[RECORDED]), &[&after], &[]).is_empty());
    }

    #[test]
    fn an_added_entry_whose_title_nests_a_bold_run_does_not_warn() {
        let after = superseded_by("**Use **only** JSON (2026-10-26).**");
        assert!(same_day(Some(&[RECORDED]), &[&after], MARKS).is_empty());
    }

    #[test]
    fn an_added_entry_whose_title_wraps_across_lines_warns() {
        let after = superseded_by("**Use JSON, one file\n   per kind (2026-10-26).**");
        assert_eq!(same_day(Some(&[RECORDED]), &[&after], MARKS).len(), 1);
    }

    #[test]
    fn a_supersedes_mark_naming_no_entry_does_not_warn() {
        let after = "1. **Use TOML (2026-10-26).** One.\n\
                     2. **Use JSON (2026-10-26).** Two. **Supersedes:** 9.\n";
        assert!(same_day(Some(&[RECORDED]), &[after], MARKS).is_empty());
    }

    #[test]
    fn an_added_entry_with_a_bold_run_only_in_its_body_does_not_warn() {
        let after = "1. **Use TOML (2026-10-26).** One. **Superseded by:** 2.\n\
                     2. Use JSON, as ruled **on 2026-10-26**. **Supersedes:** 1.\n";
        assert!(same_day(Some(&[RECORDED]), &[after], MARKS).is_empty());
    }

    #[test]
    fn an_added_entry_whose_title_nests_a_bold_run_after_its_date_does_not_warn() {
        let after = superseded_by("**Use JSON (2026-10-26) over **TOML**.**");
        assert!(same_day(Some(&[RECORDED]), &[&after], MARKS).is_empty());
    }

    #[test]
    fn titles_holding_bold_marks_in_code_spans_warn_once() {
        let after = superseded_by("**Lint reads `**Supersedes:**` marks (2026-10-26).**");
        assert_eq!(same_day(Some(&[RECORDED]), &[&after], MARKS).len(), 1);
        let before = "1. **Use `**TOML**` (2026-10-26).** One.\n";
        let after = "1. **Use `**TOML**` (2026-10-26).** One. **Superseded by:** 2.\n\
                     2. **Use JSON (2026-10-26).** Two. **Supersedes:** 1.\n";
        assert_eq!(same_day(Some(&[before]), &[after], MARKS).len(), 1);
    }

    #[test]
    fn an_added_entry_whose_opening_bold_run_is_unclosed_does_not_warn() {
        let after = "1. **Use TOML (2026-10-26).** One. **Superseded by:** 2.\n\
                     2. **Use JSON (2026-10-26). Two, as **ruled**. **Supersedes:** 1.\n";
        assert!(same_day(Some(&[RECORDED]), &[after], MARKS).is_empty());
    }

    #[test]
    fn a_commit_first_making_a_register_binding_does_not_warn_of_its_settled_entries() {
        let registry = RegistryFile::parse(REGISTRY, "registry").unwrap();
        let file = |header: &str| {
            let text = format!(
                "---\n{header}---\n\
                 1. **Use TOML (2026-10-26).** One. **Superseded by:** 2.\n\
                 2. **Use JSON (2026-10-26).** Two. **Supersedes:** 1.\n"
            );
            vec![("notes/0.md".to_owned(), text)]
        };
        let no_registry = RegistryFile::default();
        for (head_registry, head_header) in [
            (&registry, "status: active\n"),
            (&no_registry, "kind: log\nstatus: active\n"),
        ] {
            let change = Change {
                staged: layout::read(&registry, file("kind: log\nstatus: active\n")),
                head: Some(Head {
                    model: layout::read(head_registry, file(head_header)),
                    renames: BTreeMap::new(),
                    deleted: BTreeSet::new(),
                }),
            };
            assert!(
                check(&change, MARKS)
                    .iter()
                    .all(|finding| finding.rule != Rule::EntrySupersedesSameDay),
                "{head_header}"
            );
        }
    }
}
