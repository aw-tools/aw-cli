//! The contract's rules on a commit: what the staged tree may change from
//! `HEAD`, read from the model alone.
//!
//! As with the snapshot rules, no rule names a kind (clause 2.5). The SHOULD
//! clauses 3.4 and 3.5 never fail. Under the `supersession-marks` proposal, a
//! warning outside the contract joins them.

use super::entries::{self, Entry};
use super::model::{Artefact, Change, Head, Model};
use super::proposal::Proposal;
use super::rule::Rule;
use super::rules::{
    self, BINDING, CLOSED, EPHEMERAL, EPHEMERAL_TERMINAL, EPISODIC, Finding, OPEN, declared,
};
use crate::reporting;
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::ops::Range;

/// The frontmatter field whose addition or change declares a format change.
const REFORMATTED: &str = "reformatted";

/// What a commit breaks, and the report on the format change it declares, if
/// it declares one.
pub struct Checked {
    pub findings: Vec<Finding>,
    pub format_change: Option<FormatChange>,
}

/// The artefacts a commit declares a format change of, and every change of
/// words in their body content, grouped.
pub struct FormatChange {
    pub paths: Vec<String>,
    pub groups: Vec<Group>,
}

/// Every change of words in the commit that removes the same words and
/// inserts the same words.
#[derive(Debug, PartialEq, Eq)]
pub struct Group {
    pub removed: Vec<String>,
    pub inserted: Vec<String>,
    pub changes: usize,
    /// Each artefact a change falls in, in the order met, with the recorded
    /// entries the changes there touch.
    pub places: Vec<(String, Vec<u64>)>,
}

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
#[cfg(test)]
pub fn check(change: &Change, proposals: &[Proposal]) -> Vec<Finding> {
    checked(change, proposals).findings
}

/// Every finding in the staged tree, and in what it changes from `HEAD`,
/// under the proposals opted into, with the report on the format change the
/// commit declares, if it declares one.
pub fn checked(change: &Change, proposals: &[Proposal]) -> Checked {
    let mut findings = rules::check(&change.staged, proposals);
    let Some(head) = &change.head else {
        return Checked {
            findings,
            format_change: None,
        };
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
    let marks = proposals.contains(&Proposal::SupersessionMarks);
    let reformatting = if marks {
        reformatted(&change.staged, head)
    } else {
        Vec::new()
    };
    check_transitions(&change.staged, head, proposals, &reformatting, &mut found);
    let format_change = (!reformatting.is_empty())
        .then(|| check_format_change(&change.staged, head, &reformatting, &mut found));
    check_disposals(&change.staged, head, &mut found);
    check_closures(&change.staged, head, &mut found);
    if marks {
        check_same_day(&change.staged, head, &mut found);
    }
    Checked {
        findings,
        format_change,
    }
}

/// Clause 3.10: each binding artefact whose `reformatted` field the commit
/// adds or changes the text of, with what it continues at `HEAD`.
fn reformatted<'a>(staged: &'a Model, head: &'a Head) -> Vec<(&'a Artefact, &'a Artefact)> {
    staged
        .artefacts
        .iter()
        .filter(|artefact| binding(staged, artefact))
        .filter_map(|artefact| {
            let before =
                counterpart(head, &artefact.path).filter(|before| binding(&head.model, before))?;
            let value = artefact.field(REFORMATTED)?;
            (before.field(REFORMATTED) != Some(value)).then_some((artefact, before))
        })
        .collect()
}

/// Clauses 3.3 and 3.6: each artefact against what it was at `HEAD`.
fn check_transitions(
    staged: &Model,
    head: &Head,
    proposals: &[Proposal],
    reformatting: &[(&Artefact, &Artefact)],
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
            let reworded = reformatting
                .iter()
                .any(|(other, _)| other.path == artefact.path);
            if let Some(message) = rewritten(&before.body, &artefact.body, reworded) {
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
/// recorded, if it does. Each recorded entry must be read again under its
/// number, keeping its words and every entry its two sets name, new entries
/// must be numbered above the highest of them, and, where `before` holds no
/// entry, the words outside entries must begin with the recorded ones. In an
/// artefact whose format change the commit declares, no words are compared,
/// and a recorded entry not read again and every entry added are left to
/// clause 3.10.
fn rewritten(before: &str, after: &str, format_change: bool) -> Option<String> {
    let (was, now) = (entries::parse(before), entries::parse(after));
    let (pairs, added) = paired(&was.entries, &now.entries);
    for (recorded, entry) in pairs {
        let entry = match entry {
            Some(entry) if format_change || entry.words == recorded.words => entry,
            None if format_change => continue,
            _ => {
                return Some(format!(
                    "binding artefact changes or removes entry {}, already recorded; a recorded \
                     entry keeps its words",
                    recorded.number
                ));
            }
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
    let highest = was
        .entries
        .iter()
        .map(|entry| entry.number)
        .max()
        .filter(|_| !format_change);
    let misplaced = highest.and_then(|highest| {
        added
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
    (!format_change && was.entries.is_empty() && !now.outside.starts_with(&was.outside)).then(
        || {
            "binding artefact changes body content already recorded outside its entries; new \
         content must follow it"
                .to_owned()
        },
    )
}

/// Each entry of `was` with the entry of `now` under its number, if there is
/// one, a repeated number pairing its entries in the order written; and the
/// entries of `now` left over, in the order written.
fn paired<'e, 'a>(
    was: &'e [Entry<'a>],
    now: &'e [Entry<'a>],
) -> (
    Vec<(&'e Entry<'a>, Option<&'e Entry<'a>>)>,
    Vec<&'e Entry<'a>>,
) {
    let mut unpaired: BTreeMap<u64, VecDeque<usize>> = BTreeMap::new();
    for (index, entry) in now.iter().enumerate() {
        unpaired.entry(entry.number).or_default().push_back(index);
    }
    let pairs = was
        .iter()
        .map(|recorded| {
            let index = unpaired
                .get_mut(&recorded.number)
                .and_then(VecDeque::pop_front);
            (recorded, index.map(|index| &now[index]))
        })
        .collect();
    let mut left: Vec<usize> = unpaired.into_values().flatten().collect();
    left.sort_unstable();
    (pairs, left.into_iter().map(|index| &now[index]).collect())
}

/// Clause 3.10 on the artefacts the commit declares a format change of, each
/// with what it continues at `HEAD`: every recorded entry stays under its
/// number, none is added anywhere where a declared artefact recorded one, and
/// the changes of words are grouped for the report.
fn check_format_change(
    staged: &Model,
    head: &Head,
    reformatting: &[(&Artefact, &Artefact)],
    found: &mut impl FnMut(&str, Rule, String),
) -> FormatChange {
    let mut changes: Vec<WordChange> = Vec::new();
    let mut entries_recorded = false;
    for (artefact, before) in reformatting {
        let path = artefact.path.as_str();
        let (was, now) = (entries::parse(&before.body), entries::parse(&artefact.body));
        if was.entries.is_empty() {
            found(
                path,
                Rule::FormatChangeNoEntries,
                "format change declared, but the recorded version holds no entry; every change \
                 of its words is reported without one"
                    .to_owned(),
            );
            let old: Vec<&str> = before.body.split_whitespace().collect();
            let new = now.content();
            for (removed, inserted) in diff(&old, &new) {
                changes.push((path, None, old[removed].to_vec(), new[inserted].to_vec()));
            }
            continue;
        }
        entries_recorded = true;
        for (recorded, entry) in paired(&was.entries, &now.entries).0 {
            let Some(entry) = entry else {
                found(
                    path,
                    Rule::EntryRemoved,
                    format!(
                        "format change loses entry {}, already recorded; every recorded entry \
                         must remain, under its number",
                        recorded.number
                    ),
                );
                continue;
            };
            for (removed, inserted) in diff(&recorded.words, &entry.words) {
                changes.push((
                    path,
                    Some(recorded.number),
                    recorded.words[removed].to_vec(),
                    entry.words[inserted].to_vec(),
                ));
            }
        }
    }

    if entries_recorded {
        check_added(staged, head, found);
    }
    let groups = grouped(changes);
    warn_narrow(&groups, found);
    FormatChange {
        paths: reformatting
            .iter()
            .map(|(artefact, _)| artefact.path.clone())
            .collect(),
        groups,
    }
}

/// One change of words: the artefact it falls in, the recorded entry it falls
/// in there, if any, and the words it removes and inserts.
type WordChange<'a> = (&'a str, Option<u64>, Vec<&'a str>, Vec<&'a str>);

/// Clause 3.10 where a declared artefact recorded an entry: each entry of a
/// staged binding artefact beyond as many under its number as the binding
/// artefacts at `HEAD` hold is added. Entries under a number their own
/// artefact held at `HEAD` are counted first, so the one left over is the one
/// written elsewhere.
fn check_added(staged: &Model, head: &Head, found: &mut impl FnMut(&str, Rule, String)) {
    let mut recorded: BTreeMap<u64, usize> = BTreeMap::new();
    for entry in head
        .model
        .artefacts
        .iter()
        .filter(|before| binding(&head.model, before))
        .flat_map(|before| entries::parse(&before.body).entries)
    {
        *recorded.entry(entry.number).or_default() += 1;
    }
    let homes: BTreeMap<&str, BTreeSet<u64>> = staged
        .artefacts
        .iter()
        .filter(|artefact| binding(staged, artefact))
        .filter_map(|artefact| {
            let before =
                counterpart(head, &artefact.path).filter(|before| binding(&head.model, before))?;
            let numbers = entries::parse(&before.body).entries.into_iter();
            Some((
                artefact.path.as_str(),
                numbers.map(|entry| entry.number).collect(),
            ))
        })
        .collect();
    let entries = binding_entries(staged);
    let mut counted = vec![false; entries.len()];
    for first in [true, false] {
        for (index, (path, entry)) in entries.iter().enumerate() {
            let home = homes
                .get(path)
                .is_some_and(|numbers| numbers.contains(&entry.number));
            if counted[index] || (first && !home) {
                continue;
            }
            if let Some(left) = recorded.get_mut(&entry.number).filter(|left| **left > 0) {
                *left -= 1;
                counted[index] = true;
            }
        }
    }
    for ((path, entry), counted) in entries.into_iter().zip(counted) {
        if !counted {
            found(
                path,
                Rule::EntryAdded,
                format!(
                    "entry {} is added in a commit declaring a format change of recorded \
                     entries; add it in a commit of its own",
                    entry.number
                ),
            );
        }
    }
}

/// Clause 3.10's warning of a group touching only one or two entries, on each
/// artefact holding one of them.
fn warn_narrow(groups: &[Group], found: &mut impl FnMut(&str, Rule, String)) {
    for group in groups {
        let touched: Vec<u64> = group
            .places
            .iter()
            .flat_map(|(_, entries)| entries.iter().copied())
            .collect();
        if !(1..=2).contains(&touched.len()) {
            continue;
        }
        let named: Vec<String> = touched.iter().map(u64::to_string).collect();
        let message = format!(
            "format change turns {} into {} in only {} {}; a change this narrow may change what \
             it says",
            reporting::lint_words(&group.removed),
            reporting::lint_words(&group.inserted),
            if touched.len() == 1 {
                "entry"
            } else {
                "entries"
            },
            named.join(" and ")
        );
        for (path, entries) in &group.places {
            if !entries.is_empty() {
                found(path, Rule::FormatChangeNarrow, message.clone());
            }
        }
    }
}

/// The changes of words gathered into groups, in the order each group is first
/// met.
fn grouped(changes: Vec<WordChange>) -> Vec<Group> {
    let mut groups: Vec<Group> = Vec::new();
    let mut indices: HashMap<(Vec<&str>, Vec<&str>), usize> = HashMap::new();
    for (path, entry, removed, inserted) in changes {
        let index =
            *indices
                .entry((removed, inserted))
                .or_insert_with_key(|(removed, inserted)| {
                    groups.push(Group {
                        removed: removed.iter().map(|word| (*word).to_owned()).collect(),
                        inserted: inserted.iter().map(|word| (*word).to_owned()).collect(),
                        changes: 0,
                        places: Vec::new(),
                    });
                    groups.len() - 1
                });
        let group = &mut groups[index];
        group.changes += 1;
        let place = group
            .places
            .iter()
            .position(|(at, _)| at == path)
            .unwrap_or_else(|| {
                group.places.push((path.to_owned(), Vec::new()));
                group.places.len() - 1
            });
        let touched = &mut group.places[place].1;
        if let Some(entry) = entry.filter(|entry| !touched.contains(entry)) {
            touched.push(entry);
        }
    }
    groups
}

/// The changes turning `old` into `new`, each a maximal run of differing
/// words, as the range of `old` it removes and the range of `new` it inserts.
fn diff(old: &[&str], new: &[&str]) -> Vec<(Range<usize>, Range<usize>)> {
    let mut changes = Vec::new();
    let (mut from_old, mut from_new) = (0, 0);
    for (at_old, at_new) in matches(old, new) {
        if at_old > from_old || at_new > from_new {
            changes.push((from_old..at_old, from_new..at_new));
        }
        (from_old, from_new) = (at_old + 1, at_new + 1);
    }
    if from_old < old.len() || from_new < new.len() {
        changes.push((from_old..old.len(), from_new..new.len()));
    }
    changes
}

/// The pairs of equal words a shortest edit from `old` to `new` keeps, in
/// order. A word found on one side only is never kept, so the search runs over
/// the words found on both, each turned into a number standing for it.
fn matches(old: &[&str], new: &[&str]) -> Vec<(usize, usize)> {
    let mut numbers: HashMap<&str, usize> = HashMap::new();
    let mut sides: Vec<[bool; 2]> = Vec::new();
    let mut sequences: [Vec<usize>; 2] = [Vec::new(), Vec::new()];
    for (side, words) in [old, new].into_iter().enumerate() {
        for word in words {
            let next = numbers.len();
            let number = *numbers.entry(*word).or_insert(next);
            if number == sides.len() {
                sides.push([false; 2]);
            }
            sides[number][side] = true;
            sequences[side].push(number);
        }
    }
    let shared = |sequence: &[usize]| -> (Vec<usize>, Vec<usize>) {
        sequence
            .iter()
            .enumerate()
            .filter(|(_, number)| sides[**number] == [true; 2])
            .map(|(at, number)| (at, *number))
            .unzip()
    };
    let (old_at, a) = shared(&sequences[0]);
    let (new_at, b) = shared(&sequences[1]);
    let mut pairs = Vec::new();
    common(&a, &b, (0, 0), &mut pairs);
    pairs
        .into_iter()
        .map(|(x, y)| (old_at[x], new_at[y]))
        .collect()
}

/// Adds to `pairs` the pairs of equal numbers a shortest edit from `a` to `b`
/// keeps, in order, placed from `at`, by Myers' linear-space refinement: the
/// edit is split at a point it passes through, and each side is solved alike.
fn common(a: &[usize], b: &[usize], at: (usize, usize), pairs: &mut Vec<(usize, usize)>) {
    let head = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    let tail = a[head..]
        .iter()
        .rev()
        .zip(b[head..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    pairs.extend((0..head).map(|step| (at.0 + step, at.1 + step)));
    let (middle_a, middle_b) = (&a[head..a.len() - tail], &b[head..b.len() - tail]);
    if !middle_a.is_empty() && !middle_b.is_empty() {
        let (x, y) = split(middle_a, middle_b);
        let from = (at.0 + head, at.1 + head);
        common(&middle_a[..x], &middle_b[..y], from, pairs);
        common(
            &middle_a[x..],
            &middle_b[y..],
            (from.0 + x, from.1 + y),
            pairs,
        );
    }
    let (x, y) = (at.0 + a.len() - tail, at.1 + b.len() - tail);
    pairs.extend((0..tail).map(|step| (x + step, y + step)));
}

/// A point, neither end, that a shortest edit from `old` to `new` passes
/// through: where a search forward from the start meets one backward from the
/// end. Both are non-empty and differ in their first and in their last number.
fn split(old: &[usize], new: &[usize]) -> (usize, usize) {
    let (ends_x, ends_y) = (old.len(), new.len());
    let offset = (ends_x + ends_y).div_ceil(2) + 1;
    // The furthest `x` each search reaches on each diagonal `x - y`, the
    // backward one counting from the end, stored at index `x - y + offset`.
    let mut forward: Vec<Option<usize>> = vec![None; 2 * offset + 1];
    let mut backward = forward.clone();
    forward[offset + 1] = Some(0);
    backward[offset + 1] = Some(0);
    // The diagonals each search has run off the grid on, at either side.
    let (mut forward_low, mut forward_high, mut backward_low, mut backward_high) = (0, 0, 0, 0);
    let odd = (ends_x + ends_y) % 2 == 1;
    // The other search's furthest point on the diagonal holding the points of
    // `diagonal` counted from the other end, if that point lies on the grid.
    let mirrored = |furthest: &[Option<usize>], diagonal: usize| {
        let mirror = (2 * offset + ends_x).checked_sub(ends_y + diagonal)?;
        let x = furthest.get(mirror).copied().flatten()?;
        let y = (x + offset).checked_sub(mirror)?;
        (x <= ends_x && y <= ends_y).then_some((x, y))
    };
    for d in 0..offset {
        for diagonal in
            (offset - d + forward_low..=(offset + d).saturating_sub(forward_high)).step_by(2)
        {
            let mut x = if diagonal == offset - d
                || (diagonal != offset + d && forward[diagonal - 1] < forward[diagonal + 1])
            {
                forward[diagonal + 1].unwrap_or(0)
            } else {
                forward[diagonal - 1].map_or(0, |x| x + 1)
            };
            let mut y = x + offset - diagonal;
            while x < ends_x && y < ends_y && old[x] == new[y] {
                x += 1;
                y += 1;
            }
            forward[diagonal] = Some(x);
            if x > ends_x {
                forward_high += 2;
            } else if y > ends_y {
                forward_low += 2;
            } else if odd
                && mirrored(&backward, diagonal).is_some_and(|(back, _)| x + back >= ends_x)
            {
                return (x, y);
            }
        }
        for diagonal in
            (offset - d + backward_low..=(offset + d).saturating_sub(backward_high)).step_by(2)
        {
            let mut x = if diagonal == offset - d
                || (diagonal != offset + d && backward[diagonal - 1] < backward[diagonal + 1])
            {
                backward[diagonal + 1].unwrap_or(0)
            } else {
                backward[diagonal - 1].map_or(0, |x| x + 1)
            };
            let mut y = x + offset - diagonal;
            while x < ends_x && y < ends_y && old[ends_x - 1 - x] == new[ends_y - 1 - y] {
                x += 1;
                y += 1;
            }
            backward[diagonal] = Some(x);
            if x > ends_x {
                backward_high += 2;
            } else if y > ends_y {
                backward_low += 2;
            } else if !odd {
                if let Some((ahead, ahead_y)) =
                    mirrored(&forward, diagonal).filter(|(ahead, _)| ahead + x >= ends_x)
                {
                    return (ahead, ahead_y);
                }
            }
        }
    }
    (ends_x, 0)
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
        assert_eq!(rewritten(before, before, false), None);
        assert_eq!(
            rewritten(
                before,
                "Intro.\n\n1. One.\n2. Two.\n3. Three.\n\nCoda.\n",
                false
            ),
            None
        );
    }

    #[test]
    fn under_the_proposal_an_entry_numbered_below_the_last_recorded_fails() {
        assert_eq!(
            rewritten("1. One.\n2. Two.\n", "1. One.\n2. Two.\n0. Zero.\n", false).as_deref(),
            Some(
                "binding artefact adds entry 0, not numbered above recorded entry 2; new entries \
                 must follow the recorded ones"
            )
        );
    }

    #[test]
    fn under_the_proposal_a_new_entry_numbered_below_the_highest_recorded_fails() {
        assert_eq!(
            rewritten(
                "1. A.\n5. E.\n2. B.\n",
                "1. A.\n5. E.\n2. B.\n3. C.\n",
                false
            )
            .as_deref(),
            Some(
                "binding artefact adds entry 3, not numbered above recorded entry 5; new entries \
                 must follow the recorded ones"
            )
        );
    }

    #[test]
    fn under_the_proposal_a_block_under_an_entry_line_is_recorded_in_it() {
        let before = "1. Use TOML for the registry:\n- one file per kind\n";
        assert_eq!(
            rewritten(
                before,
                "1. Use TOML for the registry:\n\n- one file per kind\n",
                false
            ),
            None
        );
        assert_eq!(
            rewritten(
                before,
                "1. Use TOML for the registry:\n- one file in all\n",
                false
            )
            .as_deref(),
            Some(
                "binding artefact changes or removes entry 1, already recorded; a recorded entry \
                 keeps its words"
            )
        );
    }

    #[test]
    fn under_the_proposal_recorded_entries_may_move_and_new_ones_sit_between_them() {
        assert_eq!(
            rewritten("1. One.\n2. Two.\n", "2. Two.\n3. Three.\n1. One.\n", false),
            None
        );
    }

    #[test]
    fn under_the_proposal_a_repeated_number_pairs_its_entries_in_the_order_written() {
        let before = "1. One.\n1. Again.\n";
        assert_eq!(rewritten(before, before, false), None);
        assert!(rewritten(before, "1. Again.\n1. One.\n", false).is_some());
        assert!(rewritten(before, "1. One.\n", false).is_some());
    }

    #[test]
    fn under_the_proposal_marks_may_join_entries_indented_up_to_three_spaces() {
        for indent in ["", " "] {
            let before = format!("{indent}1. Use TOML.\n 2. Use JSON.\n");
            let after = format!(
                "{indent}1. Use TOML. **Superseded by:** 2.\n 2. Use JSON. **Supersedes:** 1.\n"
            );
            assert_eq!(rewritten(&before, &after, false), None, "{indent:?}");
        }
    }

    #[test]
    fn under_the_proposal_a_mark_with_an_unspaced_list_may_join_a_recorded_entry() {
        assert_eq!(
            rewritten(
                "1. Use TOML.\n2. Keep one file.\n",
                "1. Use TOML.\n2. Keep one file.\n3. Use JSON. **Supersedes:** 1,2.\n",
                false
            ),
            None
        );
        assert_eq!(
            rewritten(
                "1. Use TOML.\n2. Keep one file.\n3. Use JSON.\n",
                "1. Use TOML.\n2. Keep one file.\n3. Use JSON. **Supersedes:** 1,2.\n",
                false
            ),
            None
        );
    }

    #[test]
    fn under_the_proposal_a_dropped_entry_fails_naming_it() {
        assert_eq!(
            rewritten("1. One.\n2. Two.\n", "1. One.\n", false).as_deref(),
            Some(
                "binding artefact changes or removes entry 2, already recorded; a recorded entry \
                 keeps its words"
            )
        );
    }

    #[test]
    fn under_the_proposal_words_must_extend_the_recorded_ones_where_no_entry_is_recorded() {
        let before = "Intro text.\n";
        assert_eq!(
            rewritten(before, "Intro, rewritten.\n\n1. One.\n", false).as_deref(),
            Some(
                "binding artefact changes body content already recorded outside its entries; \
                 new content must follow it"
            )
        );
        assert_eq!(rewritten(before, "Intro\ntext.\n\n1. One.\n", false), None);
    }

    #[test]
    fn under_the_proposal_words_outside_recorded_entries_may_change() {
        let before = "Intro text.\n\n1. One.\n\n---\n\nCoda.\n";
        assert_eq!(
            rewritten(before, "Intro, rewritten.\n\n1. One.\n", false),
            None
        );
    }

    #[test]
    fn under_the_proposal_a_number_dropped_from_a_mark_fails_naming_it() {
        let before = "1. One. **Superseded by:** 2, 3.\n2. Two. **Supersedes:** 1.\n\
                      3. Three. **Supersedes:** 1.\n";
        assert_eq!(
            rewritten(
                before,
                "1. One. **Superseded by:** 3.\n2. Two. **Supersedes:** 1.\n\
                 3. Three. **Supersedes:** 1.\n",
                false
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
                "1. One. **Superseded by:** 2, 3.\n2. Two.\n3. Three. **Supersedes:** 1.\n",
                false
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
                 3. Three. **Supersedes:** 1.\n",
                false
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
                 2. Two. **Supersedes:** 1.\n",
                false
            ),
            None
        );
        assert_eq!(
            rewritten(
                "1. One. SUPERSEDED by 2.\n2. Two.\n",
                "1. One.\n2. Two.\n",
                false
            )
            .as_deref(),
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

    /// The commit taking binding artefacts `notes/0.md` onwards from `before`
    /// to `after`, each text following the frontmatter's `kind` and `status`
    /// lines, under `proposals`.
    fn commit(before: &[&str], after: &[&str], proposals: &[Proposal]) -> Checked {
        let registry = RegistryFile::parse(REGISTRY, "registry").unwrap();
        let tree = |texts: &[&str]| {
            let files = texts
                .iter()
                .enumerate()
                .map(|(index, text)| {
                    (
                        format!("notes/{index}.md"),
                        format!("---\nkind: log\nstatus: active\n{text}"),
                    )
                })
                .collect();
            layout::read(&registry, files)
        };
        let change = Change {
            staged: tree(after),
            head: Some(Head {
                model: tree(before),
                renames: BTreeMap::new(),
                deleted: BTreeSet::new(),
            }),
        };
        checked(&change, proposals)
    }

    fn rules_of(checked: &Checked) -> Vec<(&str, Rule)> {
        checked
            .findings
            .iter()
            .map(|finding| (finding.path.as_str(), finding.rule))
            .collect()
    }

    const LABELLED: &str = "---\n1. **A.** Use TOML. **Rejected:** JSON.\n\
                            2. **B.** Use git. **Rejected:** Mercurial.\n\
                            3. **C.** Use YAML. **Rejected:** TOML.\n";

    /// `LABELLED` with every label renamed, under the frontmatter lines
    /// `header`.
    fn relabelled(header: &str) -> String {
        format!(
            "{header}---\n1. **A.** Use TOML. **Set aside:** JSON.\n\
             2. **B.** Use git. **Set aside:** Mercurial.\n\
             3. **C.** Use YAML. **Set aside:** TOML.\n"
        )
    }

    fn words(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| (*word).to_owned()).collect()
    }

    #[test]
    fn a_reformatted_field_added_declares_a_format_change_and_groups_its_changes() {
        let checked = commit(
            &[LABELLED],
            &[&relabelled("reformatted: 2026-10-29\n")],
            MARKS,
        );
        assert!(checked.findings.is_empty(), "{:?}", rules_of(&checked));
        let format_change = checked.format_change.unwrap();
        assert_eq!(format_change.paths, ["notes/0.md"]);
        assert_eq!(
            format_change.groups,
            [Group {
                removed: words(&["**Rejected:**"]),
                inserted: words(&["**Set", "aside:**"]),
                changes: 3,
                places: vec![("notes/0.md".to_owned(), vec![1, 2, 3])],
            }]
        );
    }

    #[test]
    fn an_empty_field_added_or_a_value_changed_declares_a_format_change() {
        for (before, header) in [
            ("", "reformatted:\n"),
            ("reformatted: labels\n", "reformatted: 'labels'\n"),
        ] {
            let checked = commit(
                &[&format!("{before}{LABELLED}")],
                &[&relabelled(header)],
                MARKS,
            );
            assert!(checked.findings.is_empty(), "{header}");
            assert!(checked.format_change.is_some(), "{header}");
        }
    }

    #[test]
    fn a_field_kept_respaced_or_removed_declares_nothing() {
        for header in ["reformatted: labels\n", "reformatted:   labels  \n", ""] {
            let checked = commit(
                &[&format!("reformatted: labels\n{LABELLED}")],
                &[&relabelled(header)],
                MARKS,
            );
            assert_eq!(
                rules_of(&checked),
                [("notes/0.md", Rule::BindingRewritten)],
                "{header:?}"
            );
            assert!(checked.format_change.is_none(), "{header:?}");
        }
    }

    #[test]
    fn a_field_on_an_artefact_the_commit_creates_declares_nothing() {
        let checked = commit(
            &[LABELLED],
            &[LABELLED, "reformatted: x\n---\n4. **D.** Use LF.\n"],
            MARKS,
        );
        assert!(checked.findings.is_empty());
        assert!(checked.format_change.is_none());
    }

    #[test]
    fn without_the_proposal_the_field_is_inert() {
        let checked = commit(
            &[LABELLED],
            &[&relabelled("reformatted: 2026-10-29\n")],
            &[],
        );
        assert_eq!(rules_of(&checked), [("notes/0.md", Rule::BindingRewritten)]);
        assert!(checked.format_change.is_none());
    }

    #[test]
    fn a_format_change_losing_an_entry_or_its_number_fails() {
        let checked = commit(
            &[LABELLED],
            &[
                "reformatted: x\n---\n1. **A.** Use TOML. **Rejected:** JSON.\n\
               2. **B.** Use git. **Rejected:** Mercurial.\n\
               4. **C.** Use YAML. **Rejected:** TOML.\n",
            ],
            MARKS,
        );
        assert_eq!(
            rules_of(&checked),
            [
                ("notes/0.md", Rule::EntryRemoved),
                ("notes/0.md", Rule::EntryAdded)
            ]
        );
        assert_eq!(
            checked.findings[0].message,
            "format change loses entry 3, already recorded; every recorded entry must remain, \
             under its number"
        );
        let checked = commit(
            &[LABELLED],
            &["reformatted: x\n---\n## 1. A\n\n## 2. B\n\n## 3. C\n"],
            MARKS,
        );
        assert_eq!(rules_of(&checked), [("notes/0.md", Rule::EntryRemoved); 3]);
    }

    #[test]
    fn a_format_change_of_recorded_entries_fails_an_entry_added_to_any_artefact() {
        let checked = commit(
            &["---\n1. One.\n", "---\n2. Two.\n"],
            &[
                "reformatted: x\n---\n1. One.\n",
                "---\n2. Two.\n",
                "---\n3. Three.\n",
            ],
            MARKS,
        );
        assert_eq!(rules_of(&checked), [("notes/2.md", Rule::EntryAdded)]);
    }

    #[test]
    fn a_format_change_where_no_entry_is_recorded_compares_the_whole_body_and_warns() {
        let checked = commit(
            &["---\n## 1. A\n\nUse TOML.\n"],
            &["reformatted: x\n---\n1. **A.** Use TOML.\n"],
            MARKS,
        );
        assert_eq!(
            rules_of(&checked),
            [("notes/0.md", Rule::FormatChangeNoEntries)]
        );
        assert_eq!(
            Rule::FormatChangeNoEntries.severity(),
            crate::lint::rule::Severity::Warning
        );
        let groups = checked.format_change.unwrap().groups;
        assert_eq!(
            groups,
            [
                Group {
                    removed: words(&["##"]),
                    inserted: Vec::new(),
                    changes: 1,
                    places: vec![("notes/0.md".to_owned(), Vec::new())],
                },
                Group {
                    removed: words(&["A"]),
                    inserted: words(&["**A.**"]),
                    changes: 1,
                    places: vec![("notes/0.md".to_owned(), Vec::new())],
                }
            ]
        );
    }

    #[test]
    fn a_group_touching_one_entry_warns_naming_it() {
        let after = "reformatted: x\n---\n1. **A.** Use TOML. **Rejected:** JSON.\n\
                     2. **B.** Use git. **Review by:** Mercurial.\n\
                     3. **C.** Use YAML. **Rejected:** TOML.\n";
        let checked = commit(&[LABELLED], &[after], MARKS);
        assert_eq!(
            rules_of(&checked),
            [("notes/0.md", Rule::FormatChangeNarrow)]
        );
        assert_eq!(
            checked.findings[0].message,
            "format change turns \"**Rejected:**\" into \"**Review by:**\" in only entry 2; a \
             change this narrow may change what it says"
        );
        assert_eq!(
            Rule::FormatChangeNarrow.severity(),
            crate::lint::rule::Severity::Warning
        );
    }

    #[test]
    fn a_group_touching_two_entries_warns_on_each_artefact() {
        let checked = commit(
            &[
                "---\n1. Use TOML. **Rejected:** JSON.\n",
                "---\n2. Use git. **Rejected:** Hg.\n",
            ],
            &[
                "reformatted: x\n---\n1. Use TOML. **Set aside:** JSON.\n",
                "reformatted: x\n---\n2. Use git. **Set aside:** Hg.\n",
            ],
            MARKS,
        );
        assert_eq!(
            rules_of(&checked),
            [
                ("notes/0.md", Rule::FormatChangeNarrow),
                ("notes/1.md", Rule::FormatChangeNarrow)
            ]
        );
        assert!(
            checked.findings[0]
                .message
                .contains("in only entries 1 and 2")
        );
        let groups = checked.format_change.unwrap().groups;
        assert_eq!(
            groups[0].places,
            [
                ("notes/0.md".to_owned(), vec![1]),
                ("notes/1.md".to_owned(), vec![2])
            ]
        );
    }

    #[test]
    fn a_format_change_keeps_the_marks_rule_and_adds_marks() {
        let before = "---\n1. One. **Superseded by:** 2, 3.\n2. Two. **Supersedes:** 1.\n\
                      3. Three. **Supersedes:** 1.\n4. Four.\n";
        let dropped = "reformatted: x\n---\n1. One. **Superseded by:** 2.\n\
                       2. Two. **Supersedes:** 1.\n3. Three.\n4. Four.\n";
        assert_eq!(
            rules_of(&commit(&[before], &[dropped], MARKS)),
            [("notes/0.md", Rule::BindingRewritten)]
        );
        let added = "reformatted: x\n---\n1. One. **Superseded by:** 2, 3, 4.\n\
                     2. Two. **Supersedes:** 1.\n3. Three. **Supersedes:** 1.\n\
                     4. Four. **Supersedes:** 1.\n";
        assert!(commit(&[before], &[added], MARKS).findings.is_empty());
    }

    #[test]
    fn an_undeclared_artefact_beside_a_format_change_keeps_the_word_comparison() {
        let checked = commit(
            &["---\n1. One.\n", "---\n2. Two.\n"],
            &["reformatted: x\n---\n1. One.\n", "---\n2. Two, reworded.\n"],
            MARKS,
        );
        assert_eq!(rules_of(&checked), [("notes/1.md", Rule::BindingRewritten)]);
    }

    #[test]
    fn a_format_change_where_no_entry_is_recorded_reports_no_mark() {
        let checked = commit(
            &["---\n## 1. A\n\nUse TOML.\n\n## 2. B\n\nUse JSON.\n"],
            &[
                "reformatted: x\n---\n1. **A.** Use TOML. **Superseded by:** 2.\n\
               2. **B.** Use JSON. **Supersedes:** 1.\n",
            ],
            MARKS,
        );
        assert_eq!(
            rules_of(&checked),
            [("notes/0.md", Rule::FormatChangeNoEntries)]
        );
        let place = || vec![("notes/0.md".to_owned(), Vec::new())];
        assert_eq!(
            checked.format_change.unwrap().groups,
            [
                Group {
                    removed: words(&["##"]),
                    inserted: Vec::new(),
                    changes: 2,
                    places: place(),
                },
                Group {
                    removed: words(&["A"]),
                    inserted: words(&["**A.**"]),
                    changes: 1,
                    places: place(),
                },
                Group {
                    removed: words(&["B"]),
                    inserted: words(&["**B.**"]),
                    changes: 1,
                    places: place(),
                }
            ]
        );
    }

    #[test]
    fn an_entry_added_below_the_recorded_ones_in_a_declared_artefact_is_only_added() {
        let checked = commit(
            &["---\n1. A.\n2. B.\n3. C.\n"],
            &["reformatted: x\n---\n1. A.\n2. B.\n3. C.\n0. Z.\n"],
            MARKS,
        );
        assert_eq!(rules_of(&checked), [("notes/0.md", Rule::EntryAdded)]);
    }

    #[test]
    fn a_format_change_fails_a_recorded_number_written_again() {
        let checked = commit(
            &["---\n1. One.\n"],
            &["reformatted: x\n---\n1. One.\n", "---\n1. Again.\n"],
            MARKS,
        );
        assert_eq!(rules_of(&checked), [("notes/1.md", Rule::EntryAdded)]);
        let checked = commit(
            &["---\n", "---\n1. One.\n"],
            &["---\n1. Again.\n", "reformatted: x\n---\n1. One.\n"],
            MARKS,
        );
        assert_eq!(rules_of(&checked), [("notes/0.md", Rule::EntryAdded)]);
    }

    #[test]
    fn a_format_change_follows_a_rename_in_the_same_commit() {
        let registry = RegistryFile::parse(REGISTRY, "registry").unwrap();
        let file = |path: &str, text: &str| {
            (
                path.to_owned(),
                format!("---\nkind: log\nstatus: active\n{text}"),
            )
        };
        let change = Change {
            staged: layout::read(
                &registry,
                vec![file("notes/new.md", &relabelled("reformatted: x\n"))],
            ),
            head: Some(Head {
                model: layout::read(&registry, vec![file("notes/old.md", LABELLED)]),
                renames: BTreeMap::from([("notes/old.md".to_owned(), "notes/new.md".to_owned())]),
                deleted: BTreeSet::new(),
            }),
        };
        let result = checked(&change, MARKS);
        assert!(result.findings.is_empty(), "{:?}", rules_of(&result));
        let format_change = result.format_change.unwrap();
        assert_eq!(format_change.paths, ["notes/new.md"]);
        assert_eq!(
            format_change.groups[0].places,
            [("notes/new.md".to_owned(), vec![1, 2, 3])]
        );
    }

    /// The length of a longest common subsequence of `old` and `new`.
    fn longest(old: &[&str], new: &[&str]) -> usize {
        let mut row = vec![0; new.len() + 1];
        for word in old {
            let mut diagonal = 0;
            for (index, other) in new.iter().enumerate() {
                let above = row[index + 1];
                row[index + 1] = if word == other {
                    diagonal + 1
                } else {
                    above.max(row[index])
                };
                diagonal = above;
            }
        }
        row[new.len()]
    }

    #[test]
    fn matches_keep_a_longest_common_subsequence() {
        let mut seed: u64 = 1;
        let mut next = |bound: u64| {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            usize::try_from((seed >> 33) % bound).unwrap()
        };
        let vocabulary = ["a", "b", "c", "d", "e"];
        for _ in 0..2000 {
            let (old_length, kinds) = (next(14), next(5) + 1);
            let old: Vec<&str> = (0..old_length)
                .map(|_| vocabulary[next(5) % kinds])
                .collect();
            let new_length = next(14);
            let new: Vec<&str> = (0..new_length)
                .map(|_| vocabulary[next(5) % kinds])
                .collect();
            let pairs = matches(&old, &new);
            assert_eq!(pairs.len(), longest(&old, &new), "{old:?} {new:?}");
            assert!(
                pairs.iter().all(|&(x, y)| old[x] == new[y]),
                "{old:?} {new:?}"
            );
            assert!(
                pairs
                    .windows(2)
                    .all(|pair| pair[0].0 < pair[1].0 && pair[0].1 < pair[1].1),
                "{old:?} {new:?}"
            );
        }
    }

    #[test]
    fn a_diff_finds_each_maximal_run_of_differing_words() {
        assert!(diff(&["a", "b"], &["a", "b"]).is_empty());
        assert_eq!(
            diff(&["a", "b", "c"], &["a", "x", "y", "c"]),
            [(1..2, 1..3)]
        );
        assert_eq!(
            diff(&["a", "b", "c", "d"], &["x", "b", "c", "y"]),
            [(0..1, 0..1), (3..4, 3..4)]
        );
        assert_eq!(diff(&[], &["a", "b"]), [(0..0, 0..2)]);
        assert_eq!(diff(&["a", "b"], &[]), [(0..2, 0..0)]);
        assert_eq!(
            diff(&["a", "b", "c"], &["c", "a", "b"]),
            [(0..0, 0..1), (2..3, 3..3)]
        );
    }
}
