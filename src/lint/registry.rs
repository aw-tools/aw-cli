//! The registry file: classes and kinds, and in the template's layout the
//! declared subdirectories and the engagement register.

use super::Refusal;
use super::model::{Caps, Registry};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

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
    /// The house style's numbers; the table's presence opens its gate.
    pub state: Option<Caps>,
}

#[derive(Deserialize)]
struct ClassEntry {
    statuses: Vec<String>,
}

#[derive(Deserialize)]
struct KindEntry {
    class: String,
    home: Option<String>,
}

#[derive(Deserialize)]
pub struct EngagementEntry {
    pub status: String,
    #[serde(default, rename = "depends-on")]
    pub depends_on: Vec<String>,
    /// `None` when the key is absent, which differs from an empty list.
    pub remits: Option<Vec<String>>,
    /// `active` unless the registry says otherwise.
    pub activity: Option<String>,
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

    /// Kind name to the directory its artefacts sit in, for each kind that
    /// names one.
    pub fn homes(&self) -> BTreeMap<String, String> {
        self.kind
            .iter()
            .filter_map(|(name, entry)| Some((name.clone(), entry.home.clone()?)))
            .collect()
    }

    /// The declared subdirectories marked `tracked = true`: the ones a topic
    /// may hold in a tracked path.
    pub fn tracked_subdirectories(&self) -> Vec<String> {
        self.subdirectory
            .iter()
            .filter(|(_, entry)| entry.get("tracked").and_then(toml::Value::as_bool) == Some(true))
            .map(|(name, _)| name.clone())
            .collect()
    }

    /// Every remit any engagement lists.
    pub fn remit_values(&self) -> BTreeSet<String> {
        self.engagements
            .values()
            .flat_map(|entry| entry.remits.iter().flatten().cloned())
            .collect()
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
        assert_eq!(file.homes()["ticket"], "notes");
    }

    #[test]
    fn only_subdirectories_marked_tracked_are_tracked() {
        let file = RegistryFile::parse(
            "[subdirectory.attachments]\ntracked = true\n\n\
             [subdirectory.tmp]\ntracked = false\n\n[subdirectory.loose]\n",
            "registry.toml",
        )
        .unwrap();
        assert_eq!(file.tracked_subdirectories(), ["attachments"]);
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
