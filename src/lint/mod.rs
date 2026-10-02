//! `aw lint` — checks a workspace's artefacts against the contract.
//!
//! Every rule reads one internal model, [`model::Model`]. Readers build it: the
//! layout reader from a workspace laid out as the template lays it out, and a
//! test-only reader from a conformance-suite fixture. No rule reads a file.
//! Staged mode builds two, the staged tree and `HEAD`, and the commit rules
//! compare them.

mod commit;
mod frontmatter;
mod layout;
mod model;
mod placement;
mod registry;
mod rule;
mod rules;
#[cfg(test)]
mod suite;
mod tree;

use crate::manifest::{Manifest, TemplateBlock};
use crate::reporting;
use anyhow::Result;
use rule::Severity;
use std::path::Path;

/// The conformance-suite revision this `aw` implements. `aw init` records it as
/// `contract` in the manifest's `[template]` block.
pub const REVISION: i64 = 1;

/// An input `aw lint` cannot run against: a missing or unknown revision, or an
/// unreadable registry or ignore file. It exits with [`Refusal::CODE`], apart
/// from findings, which exit 1.
#[derive(Debug)]
pub struct Refusal(String);

impl Refusal {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }

    pub const CODE: u8 = 2;
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Refusal {}

/// Refuse a workspace that names no suite revision, or one this `aw` does not
/// implement.
pub fn check_revision(template: Option<&TemplateBlock>) -> Result<(), Refusal> {
    let Some(contract) = template.and_then(|block| block.contract.as_ref()) else {
        return Err(Refusal::new(format!(
            "workspace.toml has no `contract` in its [template] block; replaying the \
             template's changelog adds `contract = {REVISION}` together with `.awlintignore`"
        )));
    };
    recognise(contract)
}

/// Refuse a declared revision this `aw` does not implement.
fn recognise(contract: &toml::Value) -> Result<(), Refusal> {
    match contract {
        toml::Value::Integer(REVISION) => Ok(()),
        other => Err(Refusal::new(format!(
            "workspace.toml declares `contract = {other}`, a suite revision this aw does not \
             implement; it implements {REVISION}"
        ))),
    }
}

/// `aw lint --all`: check every tracked artefact as it stands in the working
/// tree. Failures go to stdout, the count to stderr. Returns whether there
/// were none.
pub fn run_all(root: &Path) -> Result<bool> {
    check_revision(Manifest::load(root)?.template.as_ref())?;
    let model = layout::read_worktree(root)?;
    let mut findings = rules::check(&model);
    findings.extend(placement::check(&model));
    Ok(report(&findings))
}

/// `aw lint`: check the staged tree, and what it changes from `HEAD`. A
/// repository with no `HEAD` gets the snapshot rules alone.
pub fn run_staged(root: &Path) -> Result<bool> {
    check_revision(Manifest::load(root)?.template.as_ref())?;
    let change = layout::read_staged(root)?;
    let mut findings = commit::check(&change);
    findings.extend(placement::check(&change.staged));
    Ok(report(&findings))
}

/// Print each failure to stdout, each warning and the count to stderr; whether
/// there were no failures.
fn report(findings: &[rules::Finding]) -> bool {
    let (mut failures, mut warnings) = (0, 0);
    for finding in findings {
        let line = reporting::lint_finding(&finding.path, &finding.message, finding.rule.name());
        match finding.rule.severity() {
            Severity::Failure => {
                failures += 1;
                println!("{line}");
            }
            Severity::Warning => {
                warnings += 1;
                eprintln!("{}", reporting::lint_warning(&line));
            }
        }
    }
    eprintln!("{}", reporting::lint_count(failures, warnings));
    failures == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn template(fragment: &str) -> Option<TemplateBlock> {
        let manifest: crate::manifest::Manifest =
            toml::from_str(&format!("{fragment}\n[workspace]\nname = \"fixture\"\n")).unwrap();
        manifest.template
    }

    #[test]
    fn warnings_never_fail_the_run() {
        let finding = |rule: rule::Rule| rules::Finding {
            path: "context/a.md".to_owned(),
            message: "m".to_owned(),
            rule,
        };
        assert!(report(&[finding(rule::Rule::EphemeralOpen)]));
        assert!(!report(&[
            finding(rule::Rule::EphemeralOpen),
            finding(rule::Rule::KindPrefix)
        ]));
    }

    #[test]
    fn a_refusal_exits_2_as_the_shell_linter_does() {
        assert_eq!(Refusal::CODE, 2);
    }

    #[test]
    fn the_implemented_revision_is_accepted() {
        assert!(check_revision(template("[template]\ncontract = 1").as_ref()).is_ok());
    }

    #[test]
    fn a_missing_contract_is_refused_naming_the_key() {
        for fragment in ["", "[template]\nurl = \"u\"\nref = \"r\"\nsha = \"s\""] {
            let refusal = check_revision(template(fragment).as_ref()).unwrap_err();
            assert!(refusal.to_string().contains("no `contract`"), "{refusal}");
            assert!(refusal.to_string().contains(".awlintignore"), "{refusal}");
        }
    }

    #[test]
    fn an_unknown_contract_is_refused_naming_the_key() {
        for value in ["999", "0", "\"1\"", "1.0"] {
            let block = template(&format!("[template]\ncontract = {value}"));
            let refusal = check_revision(block.as_ref()).unwrap_err();
            assert!(
                refusal
                    .to_string()
                    .contains(&format!("`contract = {value}`")),
                "{refusal}"
            );
        }
    }
}
