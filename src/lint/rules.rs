//! The contract's rules on a snapshot: what one tree must hold, read from the
//! model alone.
//!
//! No rule names a kind; the registry gives each its class (clause 2.5). The
//! class and state names below are the contract's own, fixed by its section 3.

use super::model::{Artefact, Header, Model, Registry, Unit};

const BINDING: &str = "binding";
const EPHEMERAL: &str = "ephemeral";
const EPISODIC: &str = "episodic";
const SUPERSEDED: &str = "superseded";
const EPHEMERAL_TERMINAL: [&str; 2] = ["graduated", "expired"];
const OPEN: &str = "open";
const CLOSED: &str = "closed";

/// The field a superseded binding artefact names its successor in.
const SUCCESSOR: &str = "superseded_by";

/// One rule broken by one file.
pub struct Finding {
    pub path: String,
    pub message: String,
    /// The rule's short name, printed in brackets after the message.
    pub rule: &'static str,
}

/// Every finding in the model, in artefact order.
pub fn check(model: &Model) -> Vec<Finding> {
    let mut findings = Vec::new();
    for artefact in &model.artefacts {
        let mut found = |rule: &'static str, message: String| {
            findings.push(Finding {
                path: artefact.path.clone(),
                message,
                rule,
            });
        };
        check_artefact(artefact, &model.registry, &model.units, &mut found);
    }
    findings
}

fn check_artefact(
    artefact: &Artefact,
    registry: &Registry,
    units: &[Unit],
    found: &mut impl FnMut(&'static str, String),
) {
    match &artefact.header {
        Header::Absent => {
            return found(
                "frontmatter-missing",
                "missing frontmatter: the file must open with a `---` line".to_owned(),
            );
        }
        Header::Unclosed => {
            return found(
                "frontmatter-unclosed",
                "frontmatter has no closing `---` line".to_owned(),
            );
        }
        Header::Fields(_) => {}
    }

    if artefact.field("class").is_some() {
        found(
            "class-declared",
            "frontmatter declares 'class', which only the registry gives, by kind".to_owned(),
        );
    }
    let kind = declared(artefact, "kind");
    let status = declared(artefact, "status");
    if kind.is_none() {
        found("kind-missing", "frontmatter has no 'kind'".to_owned());
    }
    if status.is_none() {
        found("status-missing", "frontmatter has no 'status'".to_owned());
    }

    let Some(kind) = kind else { return };
    let Some(class) = registry.kinds.get(kind) else {
        return found(
            "kind-unregistered",
            format!("kind '{kind}' not in registry"),
        );
    };
    let Some(legal) = registry.classes.get(class) else {
        return found(
            "class-undeclared",
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
                "status-foreign",
                format!(
                    "status '{status}' belongs to class '{other}', not to '{class}' (kind '{kind}')"
                ),
            ),
            None => found(
                "status-unknown",
                format!(
                    "status '{status}' illegal for class '{class}' (kind '{kind}'; legal: {})",
                    legal.join(" ")
                ),
            ),
        };
    }

    match class.as_str() {
        BINDING if status == SUPERSEDED && declared(artefact, SUCCESSOR).is_none() => found(
            "successor-missing",
            format!("superseded without naming its successor in '{SUCCESSOR}'"),
        ),
        EPHEMERAL if EPHEMERAL_TERMINAL.contains(&status) => found(
            "ephemeral-terminal",
            format!("ephemeral artefact is '{status}' and must leave the working tree"),
        ),
        EPISODIC => {
            let owners: Vec<&Unit> = units
                .iter()
                .filter(|unit| unit.members.contains(&artefact.path))
                .collect();
            if owners.is_empty() {
                found(
                    "unit-undeclared",
                    "episodic artefact belongs to no declared unit of work".to_owned(),
                );
            }
            for unit in owners {
                if unit.state == CLOSED && status == OPEN {
                    found(
                        "open-in-closed-unit",
                        format!(
                            "unit of work '{}' is closed, but this artefact is still 'open'",
                            unit.name
                        ),
                    );
                }
            }
        }
        _ => {}
    }
}

/// A field's value, unless it is absent or empty.
fn declared<'a>(artefact: &'a Artefact, key: &str) -> Option<&'a str> {
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
    fn rules_for(text: &str) -> Vec<&'static str> {
        let registry = crate::lint::registry::RegistryFile::parse(REGISTRY, "registry").unwrap();
        let model = crate::lint::layout::read(
            &registry,
            vec![(
                "context/engagements/alpha/minute-a.md".to_owned(),
                text.to_owned(),
            )],
        );
        check(&model)
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
            ["class-declared"]
        );
    }

    #[test]
    fn an_unclosed_frontmatter_block_fails_by_name() {
        assert_eq!(
            rules_for("---\nkind: minute\nstatus: open\nBody.\n"),
            ["frontmatter-unclosed"]
        );
    }

    #[test]
    fn an_empty_value_counts_as_missing() {
        assert_eq!(
            rules_for("---\nkind: minute\nstatus:\n---\n"),
            ["status-missing"]
        );
    }

    #[test]
    fn frontmatter_below_the_first_line_is_missing() {
        let text = "# Minute\n\n---\nkind: minute\nstatus: open\n---\n";
        assert_eq!(frontmatter::parse(text).0, Header::Absent);
        assert_eq!(rules_for(text), ["frontmatter-missing"]);
    }
}
