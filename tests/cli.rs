//! CLI integration tests that need neither `garden` nor the network.
//!
//! These cover the argument surface, template-source validation, adopt
//! rejection paths, and `status` output against a fixture workspace. Flows that
//! shell out to `garden` and clone real repositories stay in `tests/e2e.sh`.

use assert_cmd::Command;
use predicates::prelude::*;
use std::path::Path;

/// A fresh `aw` invocation.
fn aw() -> Command {
    Command::cargo_bin("aw").expect("aw binary builds")
}

/// Write a minimal workspace manifest naming `name`, appending `members`
/// verbatim (empty for a member-less workspace).
fn write_manifest(root: &Path, name: &str, members: &str) {
    let manifest = format!("[workspace]\nname = \"{name}\"\n{members}");
    std::fs::write(root.join("workspace.toml"), manifest).expect("write manifest");
}

// --- argument and usage errors ----------------------------------------------

#[test]
fn no_subcommand_is_a_usage_error() {
    aw().assert().failure().code(2);
}

#[test]
fn an_unknown_subcommand_is_a_usage_error() {
    aw().arg("frobnicate").assert().failure().code(2);
}

#[test]
fn status_help_documents_json_as_unstable() {
    aw().args(["status", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("unstable during incubation"));
}

// --- init: template-source validation ---------------------------------------

#[test]
fn init_rejects_an_empty_template_source() {
    let dir = tempfile::tempdir().expect("temp dir");
    aw().args(["init", "--template", ""])
        .arg(dir.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("template source is empty"));
}

#[test]
fn init_rejects_credentials_in_an_http_template_url() {
    let dir = tempfile::tempdir().expect("temp dir");
    aw().args([
        "init",
        "--template",
        "https://user:secret@example.invalid/repo.git",
    ])
    .arg(dir.path())
    .assert()
    .failure()
    .stderr(predicate::str::contains("must not contain credentials"));
}

// --- adopt: rejection paths -------------------------------------------------

#[test]
fn adopt_rejects_a_non_repository() {
    let root = tempfile::tempdir().expect("temp dir");
    write_manifest(root.path(), "fixture", "");
    std::fs::create_dir(root.path().join("plain")).expect("plain dir");

    aw().current_dir(root.path())
        .args(["adopt", "plain"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("is not a git repository"));
}

#[test]
fn adopt_rejects_a_checkout_outside_the_workspace() {
    let root = tempfile::tempdir().expect("temp dir");
    write_manifest(root.path(), "fixture", "");
    let outside = tempfile::tempdir().expect("outside dir");

    aw().current_dir(root.path())
        .arg("adopt")
        .arg(outside.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("outside the workspace"));
}

// --- manifest: skills contract validation -----------------------------------

#[test]
fn a_command_rejects_an_empty_skills_dirs_list() {
    let root = tempfile::tempdir().expect("temp dir");
    write_manifest(
        root.path(),
        "fixture",
        "\n[[repo]]\npath = \"member\"\nurl = \"u\"\nskills = { dirs = [] }\n",
    );

    aw().arg("status")
        .arg(root.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("empty `dirs`"));
}

#[test]
fn a_command_accepts_a_configured_skills_table() {
    let root = tempfile::tempdir().expect("temp dir");
    write_manifest(
        root.path(),
        "fixture",
        "\n[[repo]]\npath = \"member\"\nurl = \"u\"\nskills = { dirs = [\"src\"], only = [\"release\"] }\n",
    );

    aw().arg("status")
        .arg("--json")
        .arg(root.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("\"repository\": \"member\""));
}

// --- status: JSON shape, exit-code semantics, stream split ------------------

const MEMBER: &str = "\n[[repo]]\npath = \"alpha\"\nurl = \"https://example.invalid/alpha.git\"\n";

#[test]
fn status_json_reports_the_fixture_shape_on_stdout() {
    let root = tempfile::tempdir().expect("temp dir");
    write_manifest(root.path(), "fixture", MEMBER);

    aw().arg("status")
        .arg("--json")
        .arg(root.path())
        .assert()
        .success()
        .stdout(
            predicate::str::contains("\"workspace\": \"fixture\"")
                .and(predicate::str::contains("\"name\": \"presence\""))
                .and(predicate::str::contains("\"repository\": \"alpha\""))
                .and(predicate::str::contains("\"state\": \"absent\"")),
        )
        .stderr(predicate::str::is_empty());
}

#[test]
fn status_json_replaces_the_human_table() {
    let root = tempfile::tempdir().expect("temp dir");
    write_manifest(root.path(), "fixture", MEMBER);

    aw().arg("status")
        .arg("--json")
        .arg(root.path())
        .assert()
        .success()
        .stdout(predicate::str::starts_with("{"));
}

#[test]
fn status_human_report_goes_to_stdout() {
    let root = tempfile::tempdir().expect("temp dir");
    write_manifest(root.path(), "fixture", MEMBER);

    aw().arg("status")
        .arg(root.path())
        .assert()
        .success()
        .stdout(
            predicate::str::contains("alpha")
                .and(predicate::str::contains("declared, not present")),
        )
        .stderr(predicate::str::is_empty());
}

#[test]
fn status_exit_code_fails_on_an_absent_member() {
    let root = tempfile::tempdir().expect("temp dir");
    write_manifest(root.path(), "fixture", MEMBER);

    aw().arg("status")
        .arg("--exit-code")
        .arg(root.path())
        .assert()
        .failure()
        .code(1);
}
