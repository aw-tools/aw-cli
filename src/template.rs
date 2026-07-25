//! The workspace template, vendored into this binary.
//!
//! This is the source of truth for what a new workspace contains. `aw init`
//! copies it rather than cloning, so provisioning needs no network and no
//! second repository. The `workspace.template` repository is a published
//! mirror of these files.

use anyhow::{Context, Result};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

/// `.gitignore` is stored without its leading dot: a real dotfile here would
/// apply to this repository's own `template/` subtree and untrack it.
const FILES: &[(&str, &str, u32)] = &[
    (".gitignore", include_str!("../template/gitignore"), 0o644),
    (
        "workspace.toml",
        include_str!("../template/workspace.toml"),
        0o644,
    ),
    (
        "garden.yaml",
        include_str!("../template/garden.yaml"),
        0o644,
    ),
    ("AGENTS.md", include_str!("../template/AGENTS.md"), 0o644),
    ("README.md", include_str!("../template/README.md"), 0o644),
    (
        "context/README.md",
        include_str!("../template/context/README.md"),
        0o644,
    ),
    (
        "bin/bootstrap",
        include_str!("../template/bin/bootstrap"),
        0o755,
    ),
    ("tmp/.keep", "", 0o644),
];

/// Harness-specific aliases for a file the template already provides, as
/// `(link, target)`. Never replaced if something is already at the link path.
const SYMLINKS: &[(&str, &str)] = &[("CLAUDE.md", "AGENTS.md")];

/// Materialise the template into `root`, skipping anything already present.
///
/// Returns the paths actually created, so `aw init` can report honestly when
/// re-run over an existing directory.
pub fn instantiate(root: &Path) -> Result<Vec<String>> {
    let mut created = Vec::new();

    for (relative, contents, mode) in FILES {
        let path = root.join(relative);
        if path.exists() {
            continue;
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        std::fs::write(&path, contents).with_context(|| format!("writing {}", path.display()))?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(*mode))
            .with_context(|| format!("setting mode on {}", path.display()))?;
        created.push((*relative).to_owned());
    }

    for (link, target) in SYMLINKS {
        let path = root.join(link);
        if path.exists() || path.is_symlink() {
            continue;
        }
        std::os::unix::fs::symlink(target, &path)
            .with_context(|| format!("linking {}", path.display()))?;
        created.push((*link).to_owned());
    }

    Ok(created)
}
