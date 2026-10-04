//! Consent before `aw init` copies a template other than the default.
//!
//! Nothing in a template runs during `aw init`, but its hooks, executables,
//! garden configuration and agent instructions act later, through
//! `aw bootstrap` and every agent session in the workspace. A template the
//! user named is therefore shown, and accepted, before any of it lands in the
//! target.

use anyhow::{Context, Result};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use crate::garden;
use crate::manifest::Template;
use crate::reporting;
use crate::template;

/// Top-level directories whose every file can act on the machine or steer an
/// agent.
const ACTING_DIRS: &[&str] = &[".githooks", ".claude", ".agents", ".skills"];

/// Agent instruction files, listed wherever they sit.
const INSTRUCTION_FILES: &[&str] = &["AGENTS.md", "CLAUDE.md"];

/// A top-level directory listing more files than this prints as a count.
const FOLD_ABOVE: usize = 5;

/// How the answer to the consent prompt is obtained.
pub enum Answer<'a> {
    /// `--trust-template` was given: accept without asking.
    Trusted,
    /// Ask, reading one line of answer from this input.
    Ask(&'a mut dyn BufRead),
    /// Nothing to ask on: stdin is not a terminal.
    NoTerminal,
}

/// An `aw init` stopped because the template was not accepted. It carries its
/// own exit code: 1 when the user declined, 2 when there was no terminal to
/// ask on, so a script can tell a refusal it can fix with a flag from a no.
#[derive(Debug)]
pub struct Refused {
    code: u8,
    message: String,
}

impl Refused {
    pub fn code(&self) -> u8 {
        self.code
    }
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Refused {}

/// Whether seeding from `url` needs the user's consent: every template but the
/// default does, unless the workspace already records it and so was seeded from
/// it once.
pub fn needed(url: &str, recorded: Option<&Template>) -> bool {
    !template::same_source(url, template::DEFAULT_URL)
        && !recorded.is_some_and(|recorded| template::same_source(&recorded.url, url))
}

/// Show what the prepared template at `template_root` can act through, then
/// obtain consent to copy it. `target` names the directory for the decline
/// message, and `created` says whether this `aw init` created it.
pub fn obtain(
    answer: Answer<'_>,
    source_line: &str,
    template_root: &Path,
    target: &Path,
    created: bool,
) -> Result<()> {
    let input = match answer {
        Answer::Trusted => return Ok(()),
        Answer::Ask(input) => Some(input),
        Answer::NoTerminal => None,
    };

    eprintln!("{source_line}");
    let lines = listing(&acting_entries(template_root)?);
    if lines.is_empty() {
        eprintln!("{}", reporting::consent_nothing_acts());
    } else {
        eprintln!("{}", reporting::consent_intro());
        for line in &lines {
            eprintln!("{line}");
        }
    }
    eprintln!("{}", reporting::consent_verify());

    let Some(input) = input else {
        return Err(Refused {
            code: 2,
            message: reporting::consent_no_terminal(),
        }
        .into());
    };

    eprint!("{}", reporting::consent_question());
    std::io::stderr().flush().context("writing the prompt")?;
    let mut reply = String::new();
    input.read_line(&mut reply).context("reading the answer")?;
    if matches!(reply.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        return Ok(());
    }
    let message = if created {
        reporting::consent_cancelled_created(target)
    } else {
        reporting::consent_cancelled_existing(target)
    };
    Err(Refused { code: 1, message }.into())
}

/// A template entry that can act on the machine or steer an agent.
#[derive(Debug, Eq, PartialEq)]
enum Acting {
    File(PathBuf),
    Link { path: PathBuf, target: PathBuf },
}

/// Every acting entry of the template at `root`, in path order. A symlink is
/// listed rather than followed, whatever it points at.
fn acting_entries(root: &Path) -> Result<Vec<Acting>> {
    let mut found = Vec::new();
    walk(root, Path::new(""), &mut found)?;
    Ok(found)
}

fn walk(root: &Path, relative: &Path, found: &mut Vec<Acting>) -> Result<()> {
    let dir = root.join(relative);
    let mut entries = std::fs::read_dir(&dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .collect::<std::io::Result<Vec<_>>>()
        .with_context(|| format!("reading {}", dir.display()))?;
    entries.sort_by_key(std::fs::DirEntry::file_name);

    for entry in entries {
        if relative.as_os_str().is_empty() && entry.file_name() == ".git" {
            continue;
        }
        let path = relative.join(entry.file_name());
        let file_type = entry
            .file_type()
            .with_context(|| format!("reading type of {}", entry.path().display()))?;
        if file_type.is_symlink() {
            let target = std::fs::read_link(entry.path())
                .with_context(|| format!("reading link {}", entry.path().display()))?;
            found.push(Acting::Link { path, target });
        } else if file_type.is_dir() {
            walk(root, &path, found)?;
        } else {
            let mode = entry
                .metadata()
                .with_context(|| format!("reading metadata for {}", entry.path().display()))?
                .permissions()
                .mode();
            if acts(&path, mode) {
                found.push(Acting::File(path));
            }
        }
    }
    Ok(())
}

fn acts(path: &Path, mode: u32) -> bool {
    let in_acting_dir = top_dir(path).is_some_and(|top| ACTING_DIRS.contains(&top));
    let instructs = path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| INSTRUCTION_FILES.contains(&name));
    let configures = path == Path::new(garden::CONFIG_FILE);
    in_acting_dir || instructs || configures || mode & 0o111 != 0
}

/// The top-level directory holding `path`; none for a file at the root.
fn top_dir(path: &Path) -> Option<&str> {
    let mut components = path.components();
    let top = components.next()?;
    components.next()?;
    top.as_os_str().to_str()
}

/// The prompt's list lines. Files under a top-level directory holding more
/// than `FOLD_ABOVE` of them fold into one count line; symlinks always print
/// one per line with their target.
fn listing(entries: &[Acting]) -> Vec<String> {
    let mut per_dir: BTreeMap<&str, usize> = BTreeMap::new();
    for entry in entries {
        if let Acting::File(path) = entry {
            if let Some(top) = top_dir(path) {
                *per_dir.entry(top).or_default() += 1;
            }
        }
    }

    let mut folded = BTreeSet::new();
    let mut lines = Vec::new();
    for entry in entries {
        match entry {
            Acting::Link { path, target } => lines.push(reporting::consent_link(path, target)),
            Acting::File(path) => {
                let fold = top_dir(path)
                    .and_then(|top| per_dir.get_key_value(top))
                    .filter(|(_, count)| **count > FOLD_ABOVE);
                match fold {
                    Some((top, count)) => {
                        if folded.insert(*top) {
                            lines.push(reporting::consent_folded(top, *count));
                        }
                    }
                    None => lines.push(reporting::consent_file(path)),
                }
            }
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, relative: &str, mode: u32) {
        let path = root.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "x\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
    }

    fn recorded(url: &str) -> Template {
        Template {
            url: url.to_owned(),
            reference: "v1.0.0".to_owned(),
            sha: "0000000".to_owned(),
        }
    }

    #[test]
    fn the_default_template_needs_no_consent_under_either_address() {
        assert!(!needed(template::DEFAULT_URL, None));
        assert!(!needed(
            "git@github.com:aw-tools/workspace.template.git",
            None
        ));
    }

    #[test]
    fn any_other_template_needs_consent_until_the_workspace_records_it() {
        let url = "https://example.com/org/template.git";
        assert!(needed(url, None));
        assert!(needed(
            url,
            Some(&recorded("https://example.com/other.git"))
        ));
        assert!(!needed(url, Some(&recorded(url))));
    }

    #[test]
    fn lists_hooks_executables_agent_files_garden_config_and_links_only() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        write(root, ".git/hooks/pre-commit", 0o755);
        write(root, ".githooks/pre-commit", 0o644);
        write(root, ".claude/settings.json", 0o644);
        write(root, "bin/bootstrap", 0o755);
        write(root, "docs/AGENTS.md", 0o644);
        write(root, "CLAUDE.md", 0o644);
        write(root, "README.md", 0o644);
        write(root, "garden.yaml", 0o644);
        write(root, "docs/garden.yaml", 0o644);
        write(root, "workspace.toml", 0o644);
        std::os::unix::fs::symlink("/etc/passwd", root.join("notes")).unwrap();

        let paths: Vec<String> = acting_entries(root)
            .unwrap()
            .iter()
            .map(|entry| match entry {
                Acting::File(path) => path.display().to_string(),
                Acting::Link { path, target } => {
                    format!("{} -> {}", path.display(), target.display())
                }
            })
            .collect();
        assert_eq!(
            paths,
            [
                ".claude/settings.json",
                ".githooks/pre-commit",
                "CLAUDE.md",
                "bin/bootstrap",
                "docs/AGENTS.md",
                "garden.yaml",
                "notes -> /etc/passwd",
            ]
        );
    }

    #[test]
    fn folds_a_directory_above_five_files_and_keeps_its_links() {
        let mut entries: Vec<Acting> = (1..=6)
            .map(|n| Acting::File(PathBuf::from(format!(".claude/skills/s{n}/SKILL.md"))))
            .collect();
        entries.push(Acting::Link {
            path: PathBuf::from(".claude/skills/shared"),
            target: PathBuf::from("../../shared"),
        });
        entries.extend((1..=5).map(|n| Acting::File(PathBuf::from(format!(".githooks/h{n}")))));

        let lines = listing(&entries);
        assert_eq!(
            lines,
            [
                reporting::consent_folded(".claude", 6),
                reporting::consent_link(
                    Path::new(".claude/skills/shared"),
                    Path::new("../../shared")
                ),
                reporting::consent_file(Path::new(".githooks/h1")),
                reporting::consent_file(Path::new(".githooks/h2")),
                reporting::consent_file(Path::new(".githooks/h3")),
                reporting::consent_file(Path::new(".githooks/h4")),
                reporting::consent_file(Path::new(".githooks/h5")),
            ]
        );
    }

    #[test]
    fn only_yes_accepts() {
        let temp = tempfile::tempdir().unwrap();
        for (reply, accepted) in [
            ("y\n", true),
            ("YES\n", true),
            ("n\n", false),
            ("\n", false),
            ("", false),
            ("yep\n", false),
        ] {
            let mut input = reply.as_bytes();
            let result = obtain(
                Answer::Ask(&mut input),
                "source",
                temp.path(),
                Path::new("target"),
                true,
            );
            assert_eq!(result.is_ok(), accepted, "{reply:?}");
        }
    }

    #[test]
    fn a_refusal_carries_its_exit_code() {
        let temp = tempfile::tempdir().unwrap();
        let code = |answer| {
            obtain(answer, "source", temp.path(), Path::new("target"), true)
                .unwrap_err()
                .downcast::<Refused>()
                .unwrap()
                .code()
        };
        let mut no = "n\n".as_bytes();
        assert_eq!(code(Answer::Ask(&mut no)), 1);
        assert_eq!(code(Answer::NoTerminal), 2);
    }
}
