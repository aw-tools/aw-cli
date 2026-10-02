//! The one model every rule reads.

use std::collections::{BTreeMap, BTreeSet};

pub struct Model {
    pub registry: Registry,
    pub artefacts: Vec<Artefact>,
    pub units: Vec<Unit>,
    /// What the template's layout adds, when the model was read from one.
    pub layout: Option<Layout>,
}

/// The classes with their states, and the kind each class is reached through.
pub struct Registry {
    /// Class name to its states, in declared order.
    pub classes: BTreeMap<String, Vec<String>>,
    /// Kind name to the class it maps to, which the registry may not declare.
    pub kinds: BTreeMap<String, String>,
}

/// A file in the scope of the artefact declaration.
pub struct Artefact {
    /// Relative to the workspace root, `/`-separated.
    pub path: String,
    pub header: Header,
    pub body: String,
}

/// What an artefact's frontmatter block holds.
#[derive(Debug, PartialEq, Eq)]
pub enum Header {
    /// The file does not open with a `---` line.
    Absent,
    /// The opening `---` has no closing one.
    Unclosed,
    /// Each `key: value` line, the first occurrence of a key winning.
    Fields(BTreeMap<String, String>),
}

impl Artefact {
    /// A frontmatter field, if the block is closed and declares it.
    pub fn field(&self, key: &str) -> Option<&str> {
        match &self.header {
            Header::Fields(fields) => fields.get(key).map(String::as_str),
            Header::Absent | Header::Unclosed => None,
        }
    }
}

/// A unit of work: an engagement, in the template's layout.
pub struct Unit {
    pub name: String,
    pub state: String,
    /// Paths of the artefacts belonging to it.
    pub members: Vec<String>,
    /// Units this one names as prerequisites.
    pub depends_on: Vec<String>,
}

/// Where the template lays each kind out, and the paths the artefact
/// declaration does not reach.
pub struct Layout {
    /// Kind name to the directory its artefacts sit in, for each kind that
    /// names one.
    pub homes: BTreeMap<String, String>,
    /// Subdirectories a topic may hold in a tracked path.
    pub subdirectories: Vec<String>,
    /// Every tracked path under a topic tree, artefact or not.
    pub topic_paths: Vec<String>,
    /// `*.md` files at the root or under `context/` that git does not track.
    pub untracked: Vec<String>,
}

/// A commit about to be made: the staged tree, and `HEAD` when there is one.
pub struct Change {
    pub staged: Model,
    pub head: Option<Head>,
}

/// The tree at `HEAD`, and where the staged tree moves its files.
pub struct Head {
    pub model: Model,
    /// Every file renamed in the commit, artefact or not, old path to new.
    pub renames: BTreeMap<String, String>,
    /// Every file the commit deletes, artefact or not.
    pub deleted: BTreeSet<String>,
}
