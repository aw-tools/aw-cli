//! The remits every engagement lists, and the `--remits` listing.

use crate::lint::layout::{ARCHIVE, ENGAGEMENTS, REGISTRY};
use crate::lint::model::Layout;
use crate::lint::registry::RegistryFile;
use crate::lint::rule::Rule;
use crate::lint::rules::OPEN;
use std::collections::{BTreeMap, BTreeSet};

/// Each engagement lists its remits, each kebab-case and once; and, in a
/// commit, a remit the register has not held before is pointed out.
pub(super) fn check(layout: &Layout, found: &mut impl FnMut(&str, Rule, String)) {
    for (name, remits) in &layout.remits {
        let Some(remits) = remits else {
            found(
                REGISTRY,
                Rule::RemitsMissing,
                format!("engagement '{name}' has no remits key (required; [] means none)"),
            );
            continue;
        };
        let mut seen = BTreeMap::new();
        for remit in remits {
            if !kebab_case(remit) {
                found(
                    REGISTRY,
                    Rule::RemitMalformed,
                    format!("engagement '{name}' remit '{remit}' is not kebab-case"),
                );
            }
            let count = seen.entry(remit).or_insert(0);
            *count += 1;
            // Once per remit, however often it repeats, as the script does.
            if *count == 2 {
                found(
                    REGISTRY,
                    Rule::RemitRepeated,
                    format!("engagement '{name}' lists remit '{remit}' twice"),
                );
            }
        }
    }
    let Some(prior) = &layout.prior_remits else {
        return;
    };
    let existing = if prior.is_empty() {
        "none".to_owned()
    } else {
        prior
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join(", ")
    };
    let current: BTreeSet<&String> = layout.remits.values().flatten().flatten().collect();
    for remit in current.into_iter().filter(|remit| !prior.contains(*remit)) {
        found(
            REGISTRY,
            Rule::RemitNew,
            format!(
                "remit '{remit}' appears for the first time in the register (existing: {existing})"
            ),
        );
    }
}

fn kebab_case(value: &str) -> bool {
    value.split('-').all(|word| {
        !word.is_empty()
            && word
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
    })
}

/// `aw lint --remits`: one tab-separated line per remit an engagement lists,
/// giving the remit, the engagement, its status and activity, and its directory
/// if it sits where its status puts it, else `-`. An engagement listing no
/// remit appears under `-`; one with no `remits` key not at all. Sorted
/// bytewise; with `filter`, only that remit's lines.
pub fn remits(
    registry: &RegistryFile,
    is_dir: impl Fn(&str) -> bool,
    filter: Option<&str>,
) -> Vec<String> {
    let none = ["-".to_owned()];
    let mut lines = Vec::new();
    for (name, entry) in &registry.engagements {
        let Some(remits) = &entry.remits else {
            continue;
        };
        let status = entry.status.as_str();
        let activity = entry.activity.as_deref().unwrap_or("active");
        let tree = if status == OPEN { ENGAGEMENTS } else { ARCHIVE };
        let directory = format!("{tree}/{name}");
        let directory = if is_dir(&directory) {
            directory.as_str()
        } else {
            "-"
        };
        let remits = if remits.is_empty() { &none[..] } else { remits };
        for remit in remits {
            lines.push(format!(
                "{remit}\t{name}\t{status}\t{activity}\t{directory}"
            ));
        }
    }
    lines.sort();
    if let Some(filter) = filter.filter(|filter| !filter.is_empty()) {
        lines.retain(|line| line.split('\t').next() == Some(filter));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::super::tests::{CLEAN, REGISTRY_TEXT, findings_in};
    use super::*;
    use crate::lint::rule::Severity;

    #[test]
    fn every_engagement_lists_its_remits() {
        let registry = REGISTRY_TEXT.replace("remits = []\n", "");
        assert_eq!(
            findings_in(&registry, CLEAN, None),
            [(
                Rule::RemitsMissing,
                "engagement 'beta' has no remits key (required; [] means none)".to_owned()
            )]
        );
    }

    #[test]
    fn a_remit_is_kebab_case() {
        let registry = REGISTRY_TEXT.replace(
            "remits = [\"build\"]",
            "remits = [\"build-2\", \"Build\", \"a--b\", \"-a\", \"a_b\"]",
        );
        let found = findings_in(&registry, CLEAN, None);
        assert_eq!(found.len(), 4, "{found:?}");
        assert!(
            found
                .iter()
                .all(|(rule, message)| *rule == Rule::RemitMalformed
                    && !message.contains("'build-2'"))
        );
    }

    #[test]
    fn a_repeated_remit_is_reported_once() {
        let registry = REGISTRY_TEXT.replace(
            "remits = [\"build\"]",
            "remits = [\"build\", \"build\", \"build\"]",
        );
        assert_eq!(
            findings_in(&registry, CLEAN, None),
            [(
                Rule::RemitRepeated,
                "engagement 'alpha' lists remit 'build' twice".to_owned()
            )]
        );
    }

    #[test]
    fn a_commit_adding_a_remit_is_told_so() {
        assert_eq!(
            findings_in(REGISTRY_TEXT, CLEAN, Some(&["build", "publication"])),
            [(
                Rule::RemitNew,
                "remit 'operations' appears for the first time in the register (existing: \
                 build, publication)"
                    .to_owned()
            )]
        );
        let found = findings_in(REGISTRY_TEXT, CLEAN, Some(&[]));
        assert_eq!(found.len(), 2);
        assert!(found.iter().all(
            |(rule, message)| *rule == Rule::RemitNew && message.ends_with("(existing: none)")
        ));
        assert_eq!(Rule::RemitNew.severity(), Severity::Warning);
    }

    #[test]
    fn remits_lists_each_engagement_under_each_remit() {
        let registry = RegistryFile::parse(
            &format!("{REGISTRY_TEXT}\n[engagements.delta]\nstatus = \"open\"\n"),
            "registry",
        )
        .unwrap();
        let on_disk = [
            "context/engagements/alpha",
            "context/archive/beta",
            "context/archive/gamma",
        ];
        let is_dir = |dir: &str| on_disk.contains(&dir);
        assert_eq!(
            remits(&registry, is_dir, None),
            [
                "-\tbeta\tclosed\tactive\tcontext/archive/beta",
                "build\talpha\topen\tactive\tcontext/engagements/alpha",
                "operations\tgamma\topen\tdormant\t-",
            ]
        );
        assert_eq!(
            remits(&registry, is_dir, Some("build")),
            ["build\talpha\topen\tactive\tcontext/engagements/alpha"]
        );
        assert_eq!(remits(&registry, is_dir, Some("")).len(), 3);
    }
}
