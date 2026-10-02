//! Every rule `aw lint` checks: its printed name, its severity, and the
//! contract clauses it enforces. A rule citing no clause is outside the
//! contract.

/// Whether a finding blocks: a failure does, a warning is printed and the run
/// still passes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Failure,
    Warning,
}

/// Declares the enum, each rule's printed name and severity, and the list of
/// every rule from one table, so a rule cannot be left out of the list.
macro_rules! rules {
    ($($variant:ident => ($name:literal, $severity:ident),)*) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum Rule {
            $($variant,)*
        }

        impl Rule {
            /// Every rule, in declaration order.
            #[cfg(test)]
            pub const ALL: &[Rule] = &[$(Rule::$variant,)*];

            /// The short name printed in brackets after a finding's message.
            pub fn name(self) -> &'static str {
                match self {
                    $(Rule::$variant => $name,)*
                }
            }

            pub fn severity(self) -> Severity {
                match self {
                    $(Rule::$variant => Severity::$severity,)*
                }
            }
        }
    };
}

rules! {
    // The contract on one tree, in the order `rules.rs` checks them.
    FrontmatterMissing => ("frontmatter-missing", Failure),
    FrontmatterUnclosed => ("frontmatter-unclosed", Failure),
    ClassDeclared => ("class-declared", Failure),
    KindMissing => ("kind-missing", Failure),
    StatusMissing => ("status-missing", Failure),
    KindUnregistered => ("kind-unregistered", Failure),
    ClassUndeclared => ("class-undeclared", Failure),
    StatusForeign => ("status-foreign", Failure),
    StatusUnknown => ("status-unknown", Failure),
    SuccessorMissing => ("successor-missing", Failure),
    EphemeralTerminal => ("ephemeral-terminal", Failure),
    CompactedEmpty => ("compacted-empty", Failure),
    UnitUndeclared => ("unit-undeclared", Failure),
    OpenInClosedUnit => ("open-in-closed-unit", Failure),
    // The contract on a commit, in the order `commit.rs` checks them.
    StatusSkipped => ("status-skipped", Failure),
    StatusBackwards => ("status-backwards", Failure),
    BindingRewritten => ("binding-rewritten", Failure),
    ResidueMissing => ("residue-missing", Failure),
    EphemeralArchived => ("ephemeral-archived", Failure),
    CloseWithUnit => ("close-with-unit", Failure),
    // The template's layout, in the order `placement.rs` checks them.
    HomeMisplaced => ("home-misplaced", Failure),
    KindPrefix => ("kind-prefix", Failure),
    TopicMisplaced => ("topic-misplaced", Failure),
    EphemeralOpen => ("ephemeral-open", Warning),
    EngagementUnregistered => ("engagement-unregistered", Failure),
    EngagementMisplaced => ("engagement-misplaced", Failure),
    SubdirectoryUndeclared => ("subdirectory-undeclared", Failure),
    EngagementAbsent => ("engagement-absent", Failure),
    LedgerMissing => ("ledger-missing", Failure),
    DependencyUnregistered => ("dependency-unregistered", Failure),
    ArtefactUntracked => ("artefact-untracked", Warning),
}

impl Rule {
    /// The contract clauses the rule enforces, so a failing fixture is known
    /// to fail for its own clause and not by accident. Only the suite reads
    /// them so far.
    #[cfg(test)]
    pub fn clauses(self) -> &'static [&'static str] {
        match self {
            Rule::FrontmatterMissing => &["5.1", "5.2"],
            Rule::FrontmatterUnclosed => &["5.2"],
            Rule::ClassDeclared => &["2.2", "5.3"],
            Rule::KindMissing | Rule::StatusMissing => &["2.1"],
            Rule::KindUnregistered => &["2.4"],
            Rule::ClassUndeclared => &["2.3"],
            Rule::StatusUnknown => &["3.1"],
            Rule::StatusForeign => &["3.1", "3.2"],
            Rule::SuccessorMissing => &["3.7"],
            Rule::EphemeralTerminal => &["4.1.1"],
            Rule::UnitUndeclared => &["4.2.1"],
            Rule::OpenInClosedUnit => &["4.2.3"],
            Rule::CompactedEmpty => &["4.2.4"],
            Rule::StatusSkipped | Rule::StatusBackwards => &["3.3"],
            Rule::BindingRewritten => &["3.6"],
            Rule::ResidueMissing => &["4.1.2"],
            Rule::EphemeralArchived => &["4.1.4"],
            Rule::CloseWithUnit => &["4.2.2"],
            Rule::HomeMisplaced
            | Rule::TopicMisplaced
            | Rule::KindPrefix
            | Rule::EngagementUnregistered
            | Rule::EngagementMisplaced
            | Rule::EngagementAbsent
            | Rule::DependencyUnregistered
            | Rule::SubdirectoryUndeclared
            | Rule::LedgerMissing
            | Rule::EphemeralOpen
            | Rule::ArtefactUntracked => &[],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn names_are_unique_and_kebab_case() {
        let names: BTreeSet<&str> = Rule::ALL.iter().map(|rule| rule.name()).collect();
        assert_eq!(names.len(), Rule::ALL.len(), "a name is used twice");
        for name in names {
            assert!(
                name.split('-')
                    .all(|word| !word.is_empty()
                        && word.bytes().all(|byte| byte.is_ascii_lowercase())),
                "{name} is not kebab-case"
            );
        }
    }
}
