//! Detect whether a workspace leaves Claude Code's agent memory switched on.
//!
//! Only the workspace's own `.claude/settings.json` is read: not the
//! environment, the user's settings, `settings.local.json` or the memory
//! folder.

use serde::Serialize;
use serde_json::Value;
use std::path::Path;

/// The workspace settings file that carries Claude Code's memory switch.
pub const SETTINGS_FILE: &str = ".claude/settings.json";

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Finding {
    /// The file exists and does not set `autoMemoryEnabled` to `false`.
    Enabled,
    /// The file was read but is not valid JSON.
    InvalidJson { error: String },
    /// The file exists but cannot be read.
    Unreadable { error: String },
}

/// The memory finding for the workspace at `root`, or `None` when its settings
/// file is absent or sets `autoMemoryEnabled` to JSON `false`. Any other value,
/// the string `"false"` included, leaves memory on. A `.claude` that is not a
/// directory leaves the file absent.
pub fn check(root: &Path) -> Option<Finding> {
    let text = match std::fs::read_to_string(root.join(SETTINGS_FILE)) {
        Ok(text) => text,
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
            ) =>
        {
            return None;
        }
        Err(error) => {
            return Some(Finding::Unreadable {
                error: error.to_string(),
            });
        }
    };
    match serde_json::from_str::<Value>(&text) {
        Ok(settings) if settings.get("autoMemoryEnabled") == Some(&Value::Bool(false)) => None,
        Ok(_) => Some(Finding::Enabled),
        Err(error) => Some(Finding::InvalidJson {
            error: error.to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace_with(settings: Option<&str>) -> tempfile::TempDir {
        let root = tempfile::tempdir().expect("temp dir");
        if let Some(body) = settings {
            std::fs::create_dir(root.path().join(".claude")).expect(".claude dir");
            std::fs::write(root.path().join(SETTINGS_FILE), body).expect("write settings");
        }
        root
    }

    #[test]
    fn an_absent_settings_file_is_no_finding() {
        let root = workspace_with(None);
        assert_eq!(check(root.path()), None);
    }

    #[test]
    fn settings_without_the_key_leave_memory_on() {
        let root = workspace_with(Some(r#"{"permissions": {"allow": []}}"#));
        assert_eq!(check(root.path()), Some(Finding::Enabled));
    }

    #[test]
    fn the_key_set_to_true_leaves_memory_on() {
        let root = workspace_with(Some(r#"{"autoMemoryEnabled": true}"#));
        assert_eq!(check(root.path()), Some(Finding::Enabled));
    }

    #[test]
    fn the_key_set_to_false_is_no_finding() {
        let root = workspace_with(Some(r#"{"autoMemoryEnabled": false}"#));
        assert_eq!(check(root.path()), None);
    }

    #[test]
    fn the_key_set_to_the_string_false_leaves_memory_on() {
        let root = workspace_with(Some(r#"{"autoMemoryEnabled": "false"}"#));
        assert_eq!(check(root.path()), Some(Finding::Enabled));
    }

    #[test]
    fn malformed_json_is_an_invalid_json_finding() {
        let root = workspace_with(Some(r#"{"autoMemoryEnabled": false"#));
        assert!(matches!(
            check(root.path()),
            Some(Finding::InvalidJson { .. })
        ));
    }

    #[test]
    fn a_claude_file_in_place_of_the_directory_is_no_finding() {
        let root = workspace_with(None);
        std::fs::write(root.path().join(".claude"), "").expect("write .claude file");
        assert_eq!(check(root.path()), None);
    }

    #[test]
    fn a_settings_path_that_cannot_be_read_is_an_unreadable_finding() {
        let root = workspace_with(None);
        std::fs::create_dir_all(root.path().join(SETTINGS_FILE)).expect("settings dir");
        assert!(matches!(
            check(root.path()),
            Some(Finding::Unreadable { .. })
        ));
    }
}
