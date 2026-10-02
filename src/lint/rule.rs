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

/// Which part of the checking a rule belongs to: the contract, the template's
/// layout, or the house style, which runs only when the registry carries a
/// `[state]` table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    Contract,
    Layout,
    HouseStyle,
}

/// Declares the enum, each rule's printed name, severity and tier, and the
/// list of every rule from one table, so a rule cannot be left out of the
/// list.
macro_rules! rules {
    ($($variant:ident => ($name:literal, $severity:ident, $tier:ident),)*) => {
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

            pub fn tier(self) -> Tier {
                match self {
                    $(Rule::$variant => Tier::$tier,)*
                }
            }
        }
    };
}

rules! {
    // The contract on one tree, in the order `rules.rs` checks them.
    FrontmatterMissing => ("frontmatter-missing", Failure, Contract),
    FrontmatterUnclosed => ("frontmatter-unclosed", Failure, Contract),
    ClassDeclared => ("class-declared", Failure, Contract),
    KindMissing => ("kind-missing", Failure, Contract),
    StatusMissing => ("status-missing", Failure, Contract),
    KindUnregistered => ("kind-unregistered", Failure, Contract),
    ClassUndeclared => ("class-undeclared", Failure, Contract),
    StatusForeign => ("status-foreign", Failure, Contract),
    StatusUnknown => ("status-unknown", Failure, Contract),
    SuccessorMissing => ("successor-missing", Failure, Contract),
    EphemeralTerminal => ("ephemeral-terminal", Failure, Contract),
    CompactedEmpty => ("compacted-empty", Failure, Contract),
    UnitUndeclared => ("unit-undeclared", Failure, Contract),
    OpenInClosedUnit => ("open-in-closed-unit", Failure, Contract),
    // The contract on a commit, in the order `commit.rs` checks them.
    StatusSkipped => ("status-skipped", Failure, Contract),
    StatusBackwards => ("status-backwards", Failure, Contract),
    BindingRewritten => ("binding-rewritten", Failure, Contract),
    ResidueMissing => ("residue-missing", Failure, Contract),
    EphemeralArchived => ("ephemeral-archived", Failure, Contract),
    CloseWithUnit => ("close-with-unit", Failure, Contract),
    // The template's layout, in the order `placement.rs` checks them.
    HomeMisplaced => ("home-misplaced", Failure, Layout),
    KindPrefix => ("kind-prefix", Failure, Layout),
    TopicMisplaced => ("topic-misplaced", Failure, Layout),
    EphemeralOpen => ("ephemeral-open", Warning, Layout),
    EngagementUnregistered => ("engagement-unregistered", Failure, Layout),
    EngagementMisplaced => ("engagement-misplaced", Failure, Layout),
    SubdirectoryUndeclared => ("subdirectory-undeclared", Failure, Layout),
    EngagementAbsent => ("engagement-absent", Failure, Layout),
    LedgerMissing => ("ledger-missing", Failure, Layout),
    DependencyUnregistered => ("dependency-unregistered", Failure, Layout),
    ArtefactUntracked => ("artefact-untracked", Warning, Layout),
    // The house style, in the order `style.rs` checks them.
    RemitsMissing => ("remits-missing", Failure, HouseStyle),
    RemitMalformed => ("remit-malformed", Failure, HouseStyle),
    RemitRepeated => ("remit-repeated", Failure, HouseStyle),
    RemitNew => ("remit-new", Warning, HouseStyle),
    StateFenced => ("state-fenced", Failure, HouseStyle),
    StateSectionMissing => ("state-section-missing", Failure, HouseStyle),
    NextFreeText => ("next-free-text", Failure, HouseStyle),
    NextOverCap => ("next-over-cap", Failure, HouseStyle),
    NextDangling => ("next-dangling", Failure, HouseStyle),
    RollupMalformed => ("rollup-malformed", Failure, HouseStyle),
    RollupLong => ("rollup-long", Failure, HouseStyle),
    RollupEngagement => ("rollup-engagement", Failure, HouseStyle),
    ItemFreeText => ("item-free-text", Failure, HouseStyle),
    ItemFieldMissing => ("item-field-missing", Failure, HouseStyle),
    ItemFieldLong => ("item-field-long", Failure, HouseStyle),
    ItemUndated => ("item-undated", Failure, HouseStyle),
    ItemStale => ("item-stale", Warning, HouseStyle),
    ItemPrefix => ("item-prefix", Failure, HouseStyle),
    ItemEngagement => ("item-engagement", Failure, HouseStyle),
    ItemsOverCap => ("items-over-cap", Failure, HouseStyle),
    EngagementUnlisted => ("engagement-unlisted", Failure, HouseStyle),
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
            | Rule::ArtefactUntracked
            | Rule::RemitsMissing
            | Rule::RemitMalformed
            | Rule::RemitRepeated
            | Rule::RemitNew
            | Rule::StateFenced
            | Rule::StateSectionMissing
            | Rule::NextFreeText
            | Rule::NextOverCap
            | Rule::NextDangling
            | Rule::RollupMalformed
            | Rule::RollupLong
            | Rule::RollupEngagement
            | Rule::ItemFreeText
            | Rule::ItemFieldMissing
            | Rule::ItemFieldLong
            | Rule::ItemUndated
            | Rule::ItemStale
            | Rule::ItemPrefix
            | Rule::ItemEngagement
            | Rule::ItemsOverCap
            | Rule::EngagementUnlisted => &[],
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

    #[test]
    fn exactly_the_contract_rules_cite_clauses() {
        for rule in Rule::ALL {
            assert_eq!(
                rule.clauses().is_empty(),
                rule.tier() != Tier::Contract,
                "{}",
                rule.name()
            );
        }
    }
}
