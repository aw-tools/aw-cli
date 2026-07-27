//! Typed, sectioned reporting for `aw status`.
//!
//! Each section owns its data projection, human rendering, JSON value, and
//! finding classification. The top-level report only iterates sections, so a
//! later status capability adds one section without changing existing ones.

use crate::garden;
use crate::git;
use crate::manifest::Manifest;
use anyhow::Result;
use serde::ser::SerializeStruct;
use serde::{Serialize, Serializer};
use serde_json::Value;
use std::fmt::Write;
use std::path::Path;

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum RepositoryKind {
    Workspace,
    Member,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum Presence {
    Present,
    Absent,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum WorkingTreeState {
    Clean,
    Dirty,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum ReportedWorkingTree {
    Clean,
    Dirty,
    NotPresent,
}

#[derive(Clone)]
struct RepositoryState {
    name: String,
    kind: RepositoryKind,
    state: CheckoutState,
}

#[derive(Clone, Copy)]
enum CheckoutState {
    Absent,
    Present { working_tree: WorkingTreeState },
}

impl RepositoryState {
    fn presence(&self) -> Presence {
        match self.state {
            CheckoutState::Absent => Presence::Absent,
            CheckoutState::Present { .. } => Presence::Present,
        }
    }

    fn working_tree(&self) -> ReportedWorkingTree {
        match self.state {
            CheckoutState::Absent => ReportedWorkingTree::NotPresent,
            CheckoutState::Present {
                working_tree: WorkingTreeState::Clean,
            } => ReportedWorkingTree::Clean,
            CheckoutState::Present {
                working_tree: WorkingTreeState::Dirty,
            } => ReportedWorkingTree::Dirty,
        }
    }
}

trait ReportSection {
    fn name(&self) -> &'static str;
    fn has_finding(&self) -> bool;
    fn render_human(&self, output: &mut String);
    fn json(&self) -> Value;
}

pub struct StatusReport {
    workspace: String,
    sections: Vec<Box<dyn ReportSection>>,
}

impl StatusReport {
    fn new(workspace: String) -> Self {
        Self {
            workspace,
            sections: Vec::new(),
        }
    }

    fn add_section(&mut self, section: impl ReportSection + 'static) {
        self.sections.push(Box::new(section));
    }

    pub fn has_findings(&self) -> bool {
        self.sections.iter().any(|section| section.has_finding())
    }

    pub fn render_human(&self) -> String {
        let mut output = format!("workspace {}\n", self.workspace);
        for section in &self.sections {
            section.render_human(&mut output);
        }
        output
    }
}

impl Serialize for StatusReport {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let sections = self
            .sections
            .iter()
            .map(|section| SerializableSection(section.as_ref()))
            .collect::<Vec<_>>();
        let mut report = serializer.serialize_struct("StatusReport", 2)?;
        report.serialize_field("workspace", &self.workspace)?;
        report.serialize_field("sections", &sections)?;
        report.end()
    }
}

struct SerializableSection<'a>(&'a dyn ReportSection);

impl Serialize for SerializableSection<'_> {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut section = serializer.serialize_struct("StatusSection", 3)?;
        section.serialize_field("name", self.0.name())?;
        section.serialize_field("finding", &self.0.has_finding())?;
        section.serialize_field("data", &self.0.json())?;
        section.end()
    }
}

struct PresenceSection {
    repositories: Vec<RepositoryState>,
}

impl ReportSection for PresenceSection {
    fn name(&self) -> &'static str {
        "presence"
    }

    fn has_finding(&self) -> bool {
        self.repositories.iter().any(|repository| {
            matches!(repository.kind, RepositoryKind::Member)
                && matches!(repository.state, CheckoutState::Absent)
        })
    }

    fn render_human(&self, output: &mut String) {
        output.push_str("presence\n");
        for repository in &self.repositories {
            let state = match repository.presence() {
                Presence::Present => "present",
                Presence::Absent => "declared, not present",
            };
            writeln!(output, "{:<24} {state}", repository.name)
                .expect("writing to a string cannot fail");
        }
    }

    fn json(&self) -> Value {
        #[derive(Serialize)]
        struct Entry<'a> {
            repository: &'a str,
            kind: RepositoryKind,
            state: Presence,
        }

        serde_json::to_value(
            self.repositories
                .iter()
                .map(|repository| Entry {
                    repository: &repository.name,
                    kind: repository.kind,
                    state: repository.presence(),
                })
                .collect::<Vec<_>>(),
        )
        .expect("presence entries always serialise")
    }
}

struct WorkingTreeSection {
    repositories: Vec<RepositoryState>,
}

impl ReportSection for WorkingTreeSection {
    fn name(&self) -> &'static str {
        "working_tree"
    }

    fn has_finding(&self) -> bool {
        false
    }

    fn render_human(&self, output: &mut String) {
        output.push_str("working tree\n");
        for repository in &self.repositories {
            let state = match repository.working_tree() {
                ReportedWorkingTree::Clean => "clean",
                ReportedWorkingTree::Dirty => "dirty",
                ReportedWorkingTree::NotPresent => "not present",
            };
            writeln!(output, "{:<24} {state}", repository.name)
                .expect("writing to a string cannot fail");
        }
    }

    fn json(&self) -> Value {
        #[derive(Serialize)]
        struct Entry<'a> {
            repository: &'a str,
            kind: RepositoryKind,
            state: ReportedWorkingTree,
        }

        serde_json::to_value(
            self.repositories
                .iter()
                .map(|repository| Entry {
                    repository: &repository.name,
                    kind: repository.kind,
                    state: repository.working_tree(),
                })
                .collect::<Vec<_>>(),
        )
        .expect("working-tree entries always serialise")
    }
}

pub fn build(root: &Path) -> Result<StatusReport> {
    let manifest = Manifest::load(root)?;
    let repositories = inspect_repositories(root, &manifest)?;
    let mut report = StatusReport::new(manifest.workspace.name);
    report.add_section(PresenceSection {
        repositories: repositories.clone(),
    });
    report.add_section(WorkingTreeSection { repositories });
    Ok(report)
}

fn inspect_repositories(root: &Path, manifest: &Manifest) -> Result<Vec<RepositoryState>> {
    let mut repositories = Vec::with_capacity(manifest.repos.len() + 1);
    repositories.push(inspect_repository(
        "workspace".to_owned(),
        RepositoryKind::Workspace,
        root,
    )?);
    for repository in &manifest.repos {
        repositories.push(inspect_repository(
            repository.path.clone(),
            RepositoryKind::Member,
            &garden::checkout_path(root, &repository.path),
        )?);
    }
    Ok(repositories)
}

fn inspect_repository(name: String, kind: RepositoryKind, path: &Path) -> Result<RepositoryState> {
    let present = git::is_repo_checked(path)?;
    let state = if present {
        let working_tree = if git::is_dirty(path)? {
            WorkingTreeState::Dirty
        } else {
            WorkingTreeState::Clean
        };
        CheckoutState::Present { working_tree }
    } else {
        CheckoutState::Absent
    };
    Ok(RepositoryState { name, kind, state })
}
