//! The registry file: classes and kinds, and in the template's layout the
//! declared subdirectories and the engagement register.

use super::Refusal;
use super::model::Registry;
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Deserialize, Default)]
pub struct RegistryFile {
    #[serde(default)]
    class: BTreeMap<String, ClassEntry>,
    #[serde(default)]
    kind: BTreeMap<String, KindEntry>,
    /// Directories a topic may hold beside its artefacts; nothing inside one is
    /// an artefact.
    #[serde(default)]
    pub subdirectory: BTreeMap<String, toml::Table>,
    #[serde(default)]
    pub engagements: BTreeMap<String, EngagementEntry>,
}

#[derive(Deserialize)]
struct ClassEntry {
    statuses: Vec<String>,
}

#[derive(Deserialize)]
struct KindEntry {
    class: String,
}

#[derive(Deserialize)]
pub struct EngagementEntry {
    pub status: String,
    #[serde(default, rename = "depends-on")]
    pub depends_on: Vec<String>,
}

impl RegistryFile {
    pub fn parse(text: &str, path: &str) -> Result<Self, Refusal> {
        toml::from_str(text).map_err(|err| Refusal::new(format!("{path}: {}", err.message())))
    }

    pub fn registry(&self) -> Registry {
        Registry {
            classes: self
                .class
                .iter()
                .map(|(name, entry)| (name.clone(), entry.statuses.clone()))
                .collect(),
            kinds: self
                .kind
                .iter()
                .map(|(name, entry)| (name.clone(), entry.class.clone()))
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_classes_and_kinds() {
        let file = RegistryFile::parse(
            "[class.episodic]\nstatuses = [\"open\", \"closed\"]\n\n\
             [kind.ticket]\nclass = \"episodic\"\nhome = \"notes\"\n",
            "registry.toml",
        )
        .unwrap();
        let registry = file.registry();
        assert_eq!(registry.classes["episodic"], ["open", "closed"]);
        assert_eq!(registry.kinds["ticket"], "episodic");
    }

    #[test]
    fn a_multi_line_depends_on_is_read_in_full() {
        let file = RegistryFile::parse(
            "[engagements.open-source]\nstatus = \"open\"\ndepends-on = [\n  \"alpha\",\n  \"beta\",\n  \"gamma\",\n]\nremits = [\"publication\"]\n",
            "registry.toml",
        )
        .unwrap();
        let entry = &file.engagements["open-source"];
        assert_eq!(entry.status, "open");
        assert_eq!(entry.depends_on, ["alpha", "beta", "gamma"]);
    }

    #[test]
    fn a_malformed_registry_is_refused_naming_the_file() {
        let refusal = RegistryFile::parse("[kind.ticket]\n", "context/artefacts.toml")
            .err()
            .expect("a kind without a class does not parse");
        assert!(
            refusal.to_string().starts_with("context/artefacts.toml: "),
            "{refusal}"
        );
    }
}
