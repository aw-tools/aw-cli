//! Contract proposals: changes to a revision that ship ahead of it, applied
//! only to a workspace that names them in `proposals` in its `[template]`
//! block.

use super::Refusal;

/// Every proposal this `aw` knows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Proposal {
    SupersessionMarks,
}

impl Proposal {
    pub const ALL: &[Proposal] = &[Proposal::SupersessionMarks];

    /// The name a workspace opts in by, and the suite directory of its
    /// fixtures.
    pub fn name(self) -> &'static str {
        match self {
            Proposal::SupersessionMarks => "supersession-marks",
        }
    }

    /// The contract revision the proposal amends.
    #[cfg(test)]
    pub fn amends(self) -> i64 {
        match self {
            Proposal::SupersessionMarks => 1,
        }
    }
}

/// The proposals a `proposals` value opts into; refuse anything but a list of
/// names this `aw` knows.
pub fn recognise(proposals: &toml::Value) -> Result<Vec<Proposal>, Refusal> {
    let toml::Value::Array(names) = proposals else {
        return Err(not_a_list(proposals));
    };
    names
        .iter()
        .map(|name| {
            let toml::Value::String(name) = name else {
                return Err(not_a_list(proposals));
            };
            Proposal::ALL
                .iter()
                .copied()
                .find(|proposal| proposal.name() == name)
                .ok_or_else(|| {
                    let known: Vec<&str> = Proposal::ALL.iter().map(|p| p.name()).collect();
                    Refusal::new(format!(
                        "workspace.toml names `{name}` in `proposals`, a proposal this aw does \
                         not know; it knows {}",
                        known.join(", ")
                    ))
                })
        })
        .collect()
}

fn not_a_list(proposals: &toml::Value) -> Refusal {
    Refusal::new(format!(
        "workspace.toml declares `proposals = {proposals}`; `proposals` takes a list of \
         proposal names"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lint::REVISION;

    #[test]
    fn every_proposal_amends_the_implemented_revision() {
        for proposal in Proposal::ALL {
            assert_eq!(proposal.amends(), REVISION, "{}", proposal.name());
        }
    }

    #[test]
    fn a_list_of_known_names_is_opted_into() {
        let value: toml::Value = toml::from_str("p = [\"supersession-marks\"]").unwrap();
        assert_eq!(
            recognise(&value["p"]).unwrap(),
            [Proposal::SupersessionMarks]
        );
    }

    #[test]
    fn an_empty_list_opts_into_nothing() {
        let value: toml::Value = toml::from_str("p = []").unwrap();
        assert!(recognise(&value["p"]).unwrap().is_empty());
    }

    #[test]
    fn an_unknown_name_is_refused_naming_it() {
        let value: toml::Value =
            toml::from_str("p = [\"supersession-marks\", \"no-such\"]").unwrap();
        let refusal = recognise(&value["p"]).unwrap_err().to_string();
        assert!(refusal.contains("`no-such` in `proposals`"), "{refusal}");
    }

    #[test]
    fn anything_but_a_list_of_names_is_refused_naming_the_key() {
        for fragment in ["\"supersession-marks\"", "[1]", "5"] {
            let value: toml::Value = toml::from_str(&format!("p = {fragment}")).unwrap();
            let refusal = recognise(&value["p"]).unwrap_err().to_string();
            assert!(
                refusal.contains(&format!("`proposals = {fragment}`")),
                "{refusal}"
            );
        }
    }
}
