//! Every rule `aw lint` checks: its printed name, and the contract clauses it
//! enforces. A rule citing no clause is outside the contract.

/// Declares the enum, each rule's printed name and the list of every rule from
/// one table, so a rule cannot be left out of the list.
macro_rules! rules {
    ($($variant:ident => $name:literal,)*) => {
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
        }
    };
}

rules! {
    FrontmatterMissing => "frontmatter-missing",
    FrontmatterUnclosed => "frontmatter-unclosed",
    ClassDeclared => "class-declared",
    KindMissing => "kind-missing",
    StatusMissing => "status-missing",
    KindUnregistered => "kind-unregistered",
    ClassUndeclared => "class-undeclared",
    StatusUnknown => "status-unknown",
    StatusForeign => "status-foreign",
    SuccessorMissing => "successor-missing",
    EphemeralTerminal => "ephemeral-terminal",
    UnitUndeclared => "unit-undeclared",
    OpenInClosedUnit => "open-in-closed-unit",
    CompactedEmpty => "compacted-empty",
    StatusSkipped => "status-skipped",
    StatusBackwards => "status-backwards",
    BindingRewritten => "binding-rewritten",
    ResidueMissing => "residue-missing",
    EphemeralArchived => "ephemeral-archived",
    CloseWithUnit => "close-with-unit",
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
