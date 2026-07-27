//! Typed, sectioned reporting for `aw status`.
//!
//! Each section owns its data projection, human rendering, JSON value, and
//! finding classification. The top-level report only iterates sections, so a
//! later status capability adds one section without changing existing ones.

use crate::garden;
use crate::git;
use crate::manifest::Manifest;
use anyhow::{Context, Result};
use serde::ser::SerializeStruct;
use serde::{Serialize, Serializer};
use serde_json::Value;
use std::collections::BTreeSet;
use std::fmt::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const ORIGIN_URL_CONFIG_KEY: &str = "remote.origin.url";

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
    path: PathBuf,
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

#[derive(Clone, Copy, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum MeasurementAge {
    Fetched { timestamp: u64 },
    Cloned { timestamp: u64 },
    Unknown,
}

#[derive(Clone, Copy, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
enum AheadBehind {
    Measured {
        ahead: u64,
        behind: u64,
        as_of: MeasurementAge,
    },
    NoUpstream,
}

struct AheadBehindEntry {
    repository: String,
    kind: RepositoryKind,
    measurement: AheadBehind,
}

struct AheadBehindSection {
    entries: Vec<AheadBehindEntry>,
}

impl AheadBehindSection {
    fn inspect(repositories: &[RepositoryState]) -> Result<Self> {
        let mut entries = Vec::new();
        for repository in repositories {
            if matches!(repository.state, CheckoutState::Absent) {
                continue;
            }
            let measurement = match git::upstream_comparison(&repository.path)? {
                Some(comparison) => {
                    let as_of = if let Some(timestamp) =
                        git::newest_remote_reflog_timestamp(&repository.path, &comparison.upstream)?
                    {
                        MeasurementAge::Fetched { timestamp }
                    } else if let Some(timestamp) = git::clone_reflog_timestamp(&repository.path)? {
                        MeasurementAge::Cloned { timestamp }
                    } else {
                        MeasurementAge::Unknown
                    };
                    AheadBehind::Measured {
                        ahead: comparison.ahead,
                        behind: comparison.behind,
                        as_of,
                    }
                }
                None => AheadBehind::NoUpstream,
            };
            entries.push(AheadBehindEntry {
                repository: repository.name.clone(),
                kind: repository.kind,
                measurement,
            });
        }
        Ok(Self { entries })
    }
}

impl ReportSection for AheadBehindSection {
    fn name(&self) -> &'static str {
        "ahead_behind"
    }

    fn has_finding(&self) -> bool {
        false
    }

    fn render_human(&self, output: &mut String) {
        output.push_str("ahead/behind\n");
        for entry in &self.entries {
            let measurement = match entry.measurement {
                AheadBehind::Measured {
                    ahead,
                    behind,
                    as_of,
                } => format!(
                    "ahead {ahead}, behind {behind} ({})",
                    render_measurement_age(as_of)
                ),
                AheadBehind::NoUpstream => "no upstream".to_owned(),
            };
            writeln!(output, "{:<24} {measurement}", entry.repository)
                .expect("writing to a string cannot fail");
        }
    }

    fn json(&self) -> Value {
        #[derive(Serialize)]
        struct Entry<'a> {
            repository: &'a str,
            kind: RepositoryKind,
            measurement: AheadBehind,
        }

        serde_json::to_value(
            self.entries
                .iter()
                .map(|entry| Entry {
                    repository: &entry.repository,
                    kind: entry.kind,
                    measurement: entry.measurement,
                })
                .collect::<Vec<_>>(),
        )
        .expect("ahead-behind entries always serialise")
    }
}

struct ConfigurationDriftEntry {
    repository: String,
    kind: RepositoryKind,
    keys: Vec<&'static str>,
}

struct ConfigurationDriftSection {
    entries: Vec<ConfigurationDriftEntry>,
}

impl ConfigurationDriftSection {
    fn inspect(repositories: &[RepositoryState], manifest: &Manifest) -> Result<Self> {
        let identity = garden::identity_pairs(manifest.identity.as_ref());
        let mut entries = Vec::new();

        for repository in repositories {
            if matches!(repository.state, CheckoutState::Absent) {
                continue;
            }
            let mut keys = Vec::new();
            for (key, expected) in &identity {
                if git::config_value(&repository.path, key)?.as_deref() != Some(expected) {
                    keys.push(*key);
                }
            }

            match repository.kind {
                RepositoryKind::Workspace => {
                    if repository.path.join(crate::HOOKS_DIR).is_dir()
                        && git::config_value(&repository.path, crate::HOOKS_CONFIG_KEY)?.is_none()
                    {
                        keys.push(crate::HOOKS_CONFIG_KEY);
                    }
                }
                RepositoryKind::Member => {
                    let expected = manifest
                        .repos
                        .iter()
                        .find(|member| member.path == repository.name)
                        .expect("member repository state comes from the manifest")
                        .url
                        .as_str();
                    if git::config_value(&repository.path, ORIGIN_URL_CONFIG_KEY)?.as_deref()
                        != Some(expected)
                    {
                        keys.push(ORIGIN_URL_CONFIG_KEY);
                    }
                }
            }

            if !keys.is_empty() {
                entries.push(ConfigurationDriftEntry {
                    repository: repository.name.clone(),
                    kind: repository.kind,
                    keys,
                });
            }
        }

        Ok(Self { entries })
    }
}

impl ReportSection for ConfigurationDriftSection {
    fn name(&self) -> &'static str {
        "configuration_drift"
    }

    fn has_finding(&self) -> bool {
        !self.entries.is_empty()
    }

    fn render_human(&self, output: &mut String) {
        output.push_str("configuration drift\n");
        for entry in &self.entries {
            for key in &entry.keys {
                writeln!(output, "{:<24} {key}", entry.repository)
                    .expect("writing to a string cannot fail");
            }
        }
    }

    fn json(&self) -> Value {
        #[derive(Serialize)]
        struct Entry<'a> {
            repository: &'a str,
            kind: RepositoryKind,
            keys: &'a [&'static str],
        }

        serde_json::to_value(
            self.entries
                .iter()
                .map(|entry| Entry {
                    repository: &entry.repository,
                    kind: entry.kind,
                    keys: &entry.keys,
                })
                .collect::<Vec<_>>(),
        )
        .expect("configuration-drift entries always serialise")
    }
}

struct UnlistedCheckoutsSection {
    paths: Vec<String>,
}

impl UnlistedCheckoutsSection {
    fn inspect(root: &Path, manifest: &Manifest) -> Result<Self> {
        let members = manifest
            .repos
            .iter()
            .map(|repository| garden::checkout_path(root, &repository.path))
            .collect::<BTreeSet<_>>();
        let mut paths = Vec::new();

        for entry in std::fs::read_dir(root)
            .with_context(|| format!("reading workspace root {}", root.display()))?
        {
            let Ok(entry) = entry else {
                continue;
            };
            let path = entry.path();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_dir()
                && !members.contains(&path)
                && git::is_repo_checked(&path).unwrap_or(false)
            {
                paths.push(entry.file_name().to_string_lossy().into_owned());
            }
        }
        paths.sort();

        Ok(Self { paths })
    }
}

impl ReportSection for UnlistedCheckoutsSection {
    fn name(&self) -> &'static str {
        "unlisted_checkouts"
    }

    fn has_finding(&self) -> bool {
        false
    }

    fn render_human(&self, output: &mut String) {
        output.push_str("unlisted checkouts\n");
        for path in &self.paths {
            writeln!(output, "{path:<24} present, not declared")
                .expect("writing to a string cannot fail");
        }
    }

    fn json(&self) -> Value {
        #[derive(Serialize)]
        struct Entry<'a> {
            path: &'a str,
        }

        serde_json::to_value(
            self.paths
                .iter()
                .map(|path| Entry { path })
                .collect::<Vec<_>>(),
        )
        .expect("unlisted-checkout entries always serialise")
    }
}

fn render_measurement_age(age: MeasurementAge) -> String {
    match age {
        MeasurementAge::Fetched { timestamp } => {
            format!("fetched {} ago", elapsed_age(timestamp))
        }
        MeasurementAge::Cloned { timestamp } => {
            format!("cloned {} ago", elapsed_age(timestamp))
        }
        MeasurementAge::Unknown => "age unknown; run `aw sync` to refresh".to_owned(),
    }
}

fn elapsed_age(timestamp: u64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs());
    format_elapsed(now.saturating_sub(timestamp))
}

fn format_elapsed(seconds: u64) -> String {
    match seconds {
        0..=59 => format!("{seconds}s"),
        60..=3_599 => format!("{}m", seconds / 60),
        3_600..=86_399 => format!("{}h", seconds / 3_600),
        _ => format!("{}d", seconds / 86_400),
    }
}

pub fn build(root: &Path) -> Result<StatusReport> {
    let manifest = Manifest::load(root)?;
    let repositories = inspect_repositories(root, &manifest)?;
    let ahead_behind = AheadBehindSection::inspect(&repositories)?;
    let configuration_drift = ConfigurationDriftSection::inspect(&repositories, &manifest)?;
    let unlisted_checkouts = UnlistedCheckoutsSection::inspect(root, &manifest)?;
    let mut report = StatusReport::new(manifest.workspace.name);
    report.add_section(PresenceSection {
        repositories: repositories.clone(),
    });
    report.add_section(WorkingTreeSection {
        repositories: repositories.clone(),
    });
    report.add_section(ahead_behind);
    report.add_section(configuration_drift);
    report.add_section(unlisted_checkouts);
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
    Ok(RepositoryState {
        name,
        kind,
        path: path.to_owned(),
        state,
    })
}

#[cfg(test)]
mod tests {
    use super::format_elapsed;

    #[test]
    fn elapsed_age_uses_compact_unit_boundaries() {
        assert_eq!(format_elapsed(59), "59s");
        assert_eq!(format_elapsed(60), "1m");
        assert_eq!(format_elapsed(3_599), "59m");
        assert_eq!(format_elapsed(3_600), "1h");
        assert_eq!(format_elapsed(86_399), "23h");
        assert_eq!(format_elapsed(86_400), "1d");
    }
}
