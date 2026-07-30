//! Detect whether a member repository declares its own delivery model.
//!
//! The per-repo delivery model is authoritative in each member's own
//! instruction files, read live and never cached workspace-side. This is a
//! presence probe only: it reports which instruction file declares a model, or
//! that none does (the conservative default applies). It reads no content below
//! the heading and interprets nothing — surfacing the model's substance is a
//! member-repo concern, not the workspace's.

use std::path::Path;

/// Instruction files scanned, in precedence order. The first that declares a
/// delivery-model heading wins.
const INSTRUCTION_FILES: &[&str] = &["AGENTS.md", "CLAUDE.md"];

/// The instruction file at the checkout root that declares a `## Delivery model`
/// heading, or `None` when none does. A missing or unreadable file is simply
/// not-declared for that file, never an error, so this never fails a caller such
/// as `doctor` or `adopt`.
pub fn declared_in(checkout: &Path) -> Option<&'static str> {
    INSTRUCTION_FILES.iter().copied().find(|file| {
        std::fs::read_to_string(checkout.join(file))
            .is_ok_and(|text| text.lines().any(is_delivery_model_heading))
    })
}

/// Whether a line is a `## Delivery model` heading, tokenised on whitespace and
/// compared case-insensitively so `##   Delivery model` and `## delivery MODEL`
/// both match. Only a level-two heading of exactly those three tokens counts;
/// prose merely mentioning the phrase does not.
fn is_delivery_model_heading(line: &str) -> bool {
    let mut tokens = line.split_whitespace();
    tokens.next().is_some_and(|token| token == "##")
        && tokens
            .next()
            .is_some_and(|token| token.eq_ignore_ascii_case("delivery"))
        && tokens
            .next()
            .is_some_and(|token| token.eq_ignore_ascii_case("model"))
        && tokens.next().is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checkout_with(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("temp dir");
        for (name, body) in files {
            std::fs::write(dir.path().join(name), body).expect("write instruction file");
        }
        dir
    }

    #[test]
    fn detects_a_heading_in_agents_md() {
        let dir = checkout_with(&[("AGENTS.md", "# Title\n\n## Delivery model\n\ntext\n")]);
        assert_eq!(declared_in(dir.path()), Some("AGENTS.md"));
    }

    #[test]
    fn detects_a_heading_in_claude_md_when_agents_is_silent() {
        let dir = checkout_with(&[
            ("AGENTS.md", "# Title\n\n## Something else\n"),
            ("CLAUDE.md", "## Delivery model\n"),
        ]);
        assert_eq!(declared_in(dir.path()), Some("CLAUDE.md"));
    }

    #[test]
    fn agents_md_wins_when_both_declare() {
        let dir = checkout_with(&[
            ("AGENTS.md", "## Delivery model\n"),
            ("CLAUDE.md", "## Delivery model\n"),
        ]);
        assert_eq!(declared_in(dir.path()), Some("AGENTS.md"));
    }

    #[test]
    fn absent_when_no_file_declares() {
        let dir = checkout_with(&[("AGENTS.md", "# Title\n\nno heading here\n")]);
        assert_eq!(declared_in(dir.path()), None);
    }

    #[test]
    fn absent_when_no_instruction_files_exist() {
        let dir = tempfile::tempdir().expect("temp dir");
        assert_eq!(declared_in(dir.path()), None);
    }

    #[test]
    fn tolerates_surrounding_and_internal_whitespace() {
        let dir = checkout_with(&[("AGENTS.md", "##   Delivery   model  \n")]);
        assert_eq!(declared_in(dir.path()), Some("AGENTS.md"));
    }

    #[test]
    fn a_prose_mention_is_not_a_heading() {
        let dir = checkout_with(&[("AGENTS.md", "See the delivery model section below.\n")]);
        assert_eq!(declared_in(dir.path()), None);
    }

    #[test]
    fn a_case_variant_heading_still_matches() {
        let dir = checkout_with(&[("CLAUDE.md", "## delivery MODEL\n")]);
        assert_eq!(declared_in(dir.path()), Some("CLAUDE.md"));
    }

    #[test]
    fn an_unreadable_file_is_treated_as_absence_and_scanning_continues() {
        let dir = tempfile::tempdir().expect("temp dir");
        // A directory named like the instruction file cannot be read as text;
        // the read error is swallowed as not-declared, and the scan falls
        // through to the next file rather than failing.
        std::fs::create_dir(dir.path().join("AGENTS.md")).expect("dir shaped like AGENTS.md");
        std::fs::write(dir.path().join("CLAUDE.md"), "## Delivery model\n").expect("CLAUDE.md");
        assert_eq!(declared_in(dir.path()), Some("CLAUDE.md"));
    }
}
