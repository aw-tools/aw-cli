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

// --- init: consent for a non-default template -------------------------------

/// A contract-valid template repository with an executable hook, so the
/// consent listing has something to show.
fn local_template(root: &Path) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let seed = root.join("template");
    std::fs::create_dir_all(seed.join(".githooks")).expect("hooks dir");
    std::fs::write(seed.join(".gitignore"), "*\n").expect("gitignore");
    write_manifest(&seed, "CHANGEME", "");
    let hook = seed.join(".githooks/pre-commit");
    std::fs::write(&hook, "#!/bin/sh\n").expect("hook");
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).expect("chmod hook");
    for args in [
        &["init", "-q", "--initial-branch=main"][..],
        &["add", "-A", "--force"],
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-qm",
            "seed",
        ],
    ] {
        let ok = std::process::Command::new("git")
            .arg("-C")
            .arg(&seed)
            .args(args)
            .status()
            .expect("run git")
            .success();
        assert!(ok, "git {args:?} succeeds");
    }
    seed
}

#[test]
fn init_refuses_a_non_default_template_without_a_terminal() {
    let temp = tempfile::tempdir().expect("temp dir");
    let template = local_template(temp.path());
    let target = temp.path().join("demo.workspace");
    aw().arg("init")
        .arg("--template")
        .arg(&template)
        .arg(&target)
        .assert()
        .code(2)
        .stdout("")
        .stderr(predicate::str::contains(".githooks/pre-commit"))
        .stderr(predicate::str::contains("pass --trust-template"));
    assert!(!target.exists(), "a refused template leaves no directory");
}

#[test]
fn init_trusts_a_non_default_template_with_the_flag() {
    let temp = tempfile::tempdir().expect("temp dir");
    let template = local_template(temp.path());
    let target = temp.path().join("demo.workspace");
    aw().args(["init", "--trust-template", "--template"])
        .arg(&template)
        .arg(&target)
        .assert()
        .success()
        .stderr(predicate::str::contains("Use this template?").not());
    assert!(target.join(".githooks/pre-commit").is_file());
    let manifest = std::fs::read_to_string(target.join("workspace.toml")).expect("manifest");
    assert!(manifest.contains("\ncontract = 1\n"), "{manifest}");
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

// --- status: workspace hook health ------------------------------------------

/// Git-initialise `root` so the workspace repository reads as present. `status`
/// needs neither commits nor a remote, so an unborn HEAD is enough.
fn git_init(root: &Path) {
    let ok = std::process::Command::new("git")
        .args(["init", "-q"])
        .arg(root)
        .status()
        .expect("run git init")
        .success();
    assert!(ok, "git init succeeds");
}

fn set_hooks_path(root: &Path, value: &str) {
    let ok = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["config", "--local", "core.hooksPath", value])
        .status()
        .expect("run git config")
        .success();
    assert!(ok, "git config core.hooksPath succeeds");
}

fn write_executable_hook(hooks_dir: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let hook = hooks_dir.join("pre-commit");
    std::fs::write(&hook, "#!/bin/sh\n").expect("write hook");
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).expect("chmod hook");
}

/// A git-initialised, member-less workspace with a `.githooks/` directory but no
/// `[identity]`, so the only findings possible are hook-related.
fn hook_fixture(root: &Path) -> std::path::PathBuf {
    git_init(root);
    write_manifest(root, "fixture", "");
    let hooks_dir = root.join(".githooks");
    std::fs::create_dir(&hooks_dir).expect("githooks dir");
    hooks_dir
}

#[test]
fn status_reports_an_unset_workspace_hook_as_drift() {
    let root = tempfile::tempdir().expect("temp dir");
    hook_fixture(root.path());
    // core.hooksPath left unset.

    aw().args(["status", "--json"])
        .arg(root.path())
        .assert()
        .success()
        .stdout(
            predicate::str::contains("\"name\": \"configuration_drift\"")
                .and(predicate::str::contains("core.hooksPath")),
        );

    aw().args(["status", "--exit-code"])
        .arg(root.path())
        .assert()
        .failure()
        .code(1);
}

#[test]
fn status_is_clean_for_a_live_workspace_hook() {
    let root = tempfile::tempdir().expect("temp dir");
    let hooks_dir = hook_fixture(root.path());
    write_executable_hook(&hooks_dir);
    set_hooks_path(root.path(), ".githooks");

    aw().args(["status", "--exit-code"])
        .arg(root.path())
        .assert()
        .success();
}

#[test]
fn status_reports_a_dead_workspace_hook_without_double_counting() {
    let root = tempfile::tempdir().expect("temp dir");
    hook_fixture(root.path());
    set_hooks_path(root.path(), ".githooks");
    // `.githooks/pre-commit` is absent, so the configured redirect is correct
    // but the hook is dead.

    aw().args(["status", "--json"])
        .arg(root.path())
        .assert()
        .success()
        .stdout(
            predicate::str::contains("\"name\": \"hook_health\"")
                .and(predicate::str::contains(
                    "\"hook\": \".githooks/pre-commit\"",
                ))
                // Config redirect is correct, so it must not also surface as a
                // drifted configuration key.
                .and(predicate::str::contains("core.hooksPath").not()),
        );

    aw().args(["status", "--exit-code"])
        .arg(root.path())
        .assert()
        .failure()
        .code(1);
}

// --- lint -------------------------------------------------------------------

const LINT_REGISTRY: &str = "\
[class.standing]
statuses = [\"live\", \"retired\"]

[kind.doctrine]
class = \"standing\"
";

/// A git workspace whose manifest carries `template`, holding one tracked
/// artefact, `context/NOTES.md`, with the given text.
fn lint_fixture(template: &str, note: &str) -> tempfile::TempDir {
    let root = tempfile::tempdir().expect("temp dir");
    write_manifest(root.path(), "fixture", template);
    let context = root.path().join("context");
    std::fs::create_dir(&context).expect("create context");
    std::fs::write(context.join("artefacts.toml"), LINT_REGISTRY).expect("write registry");
    std::fs::write(context.join("NOTES.md"), note).expect("write artefact");
    git_init(root.path());
    let ok = std::process::Command::new("git")
        .arg("-C")
        .arg(root.path())
        .args(["add", "context"])
        .status()
        .expect("run git add")
        .success();
    assert!(ok, "git add succeeds");
    root
}

#[test]
fn lint_checks_the_staged_file_not_the_working_tree() {
    let root = lint_fixture(
        "\n[template]\ncontract = 1\n",
        "---\nkind: doctrine\nstatus: live\nclass: standing\n---\n",
    );
    std::fs::write(
        root.path().join("context/NOTES.md"),
        "---\nkind: doctrine\nstatus: live\n---\n",
    )
    .expect("fix the working tree only");
    aw().arg("lint")
        .arg(root.path())
        .assert()
        .code(1)
        .stdout(predicate::str::contains("[class-declared]"))
        .stderr("1 failure, 0 warnings\n");
    aw().args(["lint", "--all"])
        .arg(root.path())
        .assert()
        .success();
}

#[test]
fn lint_passes_a_clean_commit_on_top_of_head() {
    let root = lint_fixture(
        "\n[template]\ncontract = 1\n",
        "---\nkind: doctrine\nstatus: live\n---\n",
    );
    let git = |args: &[&str]| {
        let ok = std::process::Command::new("git")
            .arg("-C")
            .arg(root.path())
            .args(args)
            .status()
            .expect("run git")
            .success();
        assert!(ok, "git {args:?} succeeds");
    };
    git(&[
        "-c",
        "user.name=t",
        "-c",
        "user.email=t@t",
        "-c",
        "commit.gpgsign=false",
        "-c",
        "core.hooksPath=/dev/null",
        "commit",
        "-qm",
        "seed",
    ]);
    std::fs::write(
        root.path().join("context/NOTES.md"),
        "---\nkind: doctrine\nstatus: live\n---\nA note.\n",
    )
    .expect("edit the artefact");
    git(&["add", "context"]);
    aw().arg("lint")
        .arg(root.path())
        .assert()
        .success()
        .stdout(predicate::str::is_empty())
        .stderr("0 failures, 0 warnings\n");
}

#[test]
fn lint_all_passes_a_clean_workspace() {
    let root = lint_fixture(
        "\n[template]\ncontract = 1\n",
        "---\nkind: doctrine\nstatus: live\n---\n",
    );
    aw().args(["lint", "--all"])
        .arg(root.path())
        .assert()
        .success()
        .stdout(predicate::str::is_empty())
        .stderr("0 failures, 0 warnings\n");
}

#[test]
fn lint_all_prints_a_finding_and_exits_1() {
    let root = lint_fixture(
        "\n[template]\ncontract = 1\n",
        "---\nkind: doctrine\nstatus: live\nclass: standing\n---\n",
    );
    aw().args(["lint", "--all"])
        .arg(root.path())
        .assert()
        .code(1)
        .stdout(
            "context/NOTES.md: frontmatter declares 'class', which only the registry gives, \
             by kind [class-declared]\n",
        )
        .stderr("1 failure, 0 warnings\n");
}

#[test]
fn lint_warns_of_untracked_markdown_and_still_passes() {
    let root = lint_fixture(
        "\n[template]\ncontract = 1\n",
        "---\nkind: doctrine\nstatus: live\n---\n",
    );
    std::fs::write(root.path().join("context/DRAFT.md"), "").expect("write untracked file");
    for args in [&["lint"][..], &["lint", "--all"]] {
        aw().args(args)
            .arg(root.path())
            .assert()
            .success()
            .stdout(predicate::str::is_empty())
            .stderr(
                "warning: context/DRAFT.md: untracked, so invisible to every other check \
                 until `git add` [artefact-untracked]\n0 failures, 1 warning\n",
            );
    }
}

#[test]
fn lint_fails_on_an_unregistered_engagement_directory() {
    let root = lint_fixture(
        "\n[template]\ncontract = 1\n",
        "---\nkind: doctrine\nstatus: live\n---\n",
    );
    let topic = root.path().join("context/engagements/ghost");
    std::fs::create_dir_all(&topic).expect("create topic");
    std::fs::write(topic.join("notes.txt"), "").expect("write topic file");
    let ok = std::process::Command::new("git")
        .arg("-C")
        .arg(root.path())
        .args(["add", "context"])
        .status()
        .expect("run git add")
        .success();
    assert!(ok, "git add succeeds");
    for args in [&["lint"][..], &["lint", "--all"]] {
        aw().args(args)
            .arg(root.path())
            .assert()
            .code(1)
            .stdout(
                "context/engagements/ghost/: engagement not in registry \
                 [engagement-unregistered]\n",
            )
            .stderr("1 failure, 0 warnings\n");
    }
}

#[test]
fn lint_checks_the_house_style_only_behind_a_state_table() {
    let root = lint_fixture(
        "\n[template]\ncontract = 1\n",
        "---\nkind: doctrine\nstatus: live\n---\n",
    );
    let registry = root.path().join("context/artefacts.toml");
    let stage = |text: &str| {
        std::fs::write(&registry, text).expect("write registry");
        let ok = std::process::Command::new("git")
            .arg("-C")
            .arg(root.path())
            .args(["add", "context"])
            .status()
            .expect("run git add")
            .success();
        assert!(ok, "git add succeeds");
    };
    let unremitted = format!("{LINT_REGISTRY}\n[engagements.old]\nstatus = \"closed\"\n");
    stage(&unremitted);
    for args in [&["lint"][..], &["lint", "--all"]] {
        aw().args(args)
            .arg(root.path())
            .assert()
            .success()
            .stderr("0 failures, 0 warnings\n");
    }
    stage(&format!("{unremitted}\n[state]\n"));
    for args in [&["lint"][..], &["lint", "--all"]] {
        aw().args(args)
            .arg(root.path())
            .assert()
            .code(1)
            .stdout(
                "context/artefacts.toml: engagement 'old' has no remits key (required; [] \
                 means none) [remits-missing]\n",
            )
            .stderr("1 failure, 0 warnings\n");
    }
}

#[test]
fn lint_notices_a_remit_new_since_head_only_on_a_staged_commit() {
    let root = lint_fixture(
        "\n[template]\ncontract = 1\n",
        "---\nkind: doctrine\nstatus: live\n---\n",
    );
    let git = |args: &[&str]| {
        let ok = std::process::Command::new("git")
            .arg("-C")
            .arg(root.path())
            .args(args)
            .status()
            .expect("run git")
            .success();
        assert!(ok, "git {args:?} succeeds");
    };
    let commit = || {
        git(&[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--allow-empty",
            "-qm",
            "seed",
        ]);
    };
    let registry = root.path().join("context/artefacts.toml");
    let stage = |text: &str| {
        std::fs::write(&registry, text).expect("write registry");
        git(&["add", "context"]);
    };
    let remitted = |remits: &str| {
        format!(
            "{LINT_REGISTRY}\n[engagements.old]\nstatus = \"closed\"\nremits = {remits}\n\n[state]\n"
        )
    };
    let notice = |existing: &str| {
        format!(
            "warning: context/artefacts.toml: remit 'build' appears for the first time in the \
             register (existing: {existing}) [remit-new]\n0 failures, 1 warning\n"
        )
    };

    // A first commit has no `HEAD` to compare with, so nothing is new.
    stage(&remitted("[\"build\"]"));
    aw().arg("lint")
        .arg(root.path())
        .assert()
        .success()
        .stderr("0 failures, 0 warnings\n");

    // A `HEAD` without a registry has no remits, as the script reads it.
    git(&["rm", "-rq", "--cached", "context"]);
    commit();
    stage(&remitted("[\"build\"]"));
    aw().arg("lint")
        .arg(root.path())
        .assert()
        .success()
        .stderr(notice("none"));

    // A remit `HEAD` lacks is new to the staged commit, and to it alone.
    stage(&remitted("[\"operations\"]"));
    commit();
    stage(&remitted("[\"operations\", \"build\"]"));
    aw().arg("lint")
        .arg(root.path())
        .assert()
        .success()
        .stderr(notice("operations"));
    aw().args(["lint", "--all"])
        .arg(root.path())
        .assert()
        .success()
        .stderr("0 failures, 0 warnings\n");

    // A `HEAD` registry that does not parse gives no prior set to announce
    // against, so repairing it announces nothing.
    stage(&remitted("\"build\""));
    commit();
    stage(&remitted("[\"build\"]"));
    aw().arg("lint")
        .arg(root.path())
        .assert()
        .success()
        .stderr("0 failures, 0 warnings\n");
}

#[test]
fn lint_remits_lists_the_engagements_by_remit() {
    let root = lint_fixture(
        "\n[template]\ncontract = 1\n",
        "---\nkind: doctrine\nstatus: live\n---\n",
    );
    std::fs::write(
        root.path().join("context/artefacts.toml"),
        format!(
            "{LINT_REGISTRY}\n[engagements.old]\nstatus = \"closed\"\nremits = [\"build\"]\n\n\
             [engagements.new]\nstatus = \"open\"\nremits = [\"build\", \"operations\"]\n"
        ),
    )
    .expect("write registry");
    std::fs::create_dir_all(root.path().join("context/engagements/new")).expect("create topic");
    aw().arg("lint")
        .arg(root.path())
        .arg("--remits")
        .assert()
        .success()
        .stdout(
            "build\tnew\topen\tactive\tcontext/engagements/new\n\
             build\told\tclosed\tactive\t-\n\
             operations\tnew\topen\tactive\tcontext/engagements/new\n",
        )
        .stderr(predicate::str::is_empty());
    aw().arg("lint")
        .arg(root.path())
        .args(["--remits", "operations"])
        .assert()
        .success()
        .stdout("operations\tnew\topen\tactive\tcontext/engagements/new\n");
    aw().arg("lint")
        .arg(root.path())
        .args(["--remits", "--all"])
        .assert()
        .code(2);
}

#[test]
fn lint_all_refuses_a_workspace_declaring_no_revision() {
    let root = lint_fixture("", "---\nkind: doctrine\nstatus: live\n---\n");
    aw().args(["lint", "--all"])
        .arg(root.path())
        .assert()
        .code(2)
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("no `contract`"));
}

#[test]
fn lint_refuses_a_manifest_that_does_not_parse() {
    let root = lint_fixture("\n[template\n", "---\nkind: doctrine\nstatus: live\n---\n");
    for args in [&["lint"][..], &["lint", "--all"]] {
        aw().args(args)
            .arg(root.path())
            .assert()
            .code(2)
            .stdout(predicate::str::is_empty());
    }
}

#[test]
fn lint_refuses_a_root_outside_git() {
    let root = tempfile::tempdir().expect("temp dir");
    write_manifest(root.path(), "fixture", "\n[template]\ncontract = 1\n");
    let context = root.path().join("context");
    std::fs::create_dir(&context).expect("create context");
    std::fs::write(context.join("artefacts.toml"), LINT_REGISTRY).expect("write registry");
    for args in [&["lint"][..], &["lint", "--all"]] {
        aw().args(args)
            .arg(root.path())
            .assert()
            .code(2)
            .stdout(predicate::str::is_empty());
    }
}

#[test]
fn lint_warns_of_an_open_ephemeral_only_in_a_commit_that_stages_it() {
    let root = lint_fixture(
        "\n[template]\ncontract = 1\n",
        "---\nkind: doctrine\nstatus: live\n---\n",
    );
    let git = |args: &[&str]| {
        let ok = std::process::Command::new("git")
            .arg("-C")
            .arg(root.path())
            .args(args)
            .status()
            .expect("run git")
            .success();
        assert!(ok, "git {args:?} succeeds");
    };
    std::fs::write(
        root.path().join("context/artefacts.toml"),
        format!(
            "{LINT_REGISTRY}\n[class.episodic]\nstatuses = [\"open\", \"closed\"]\n\n\
             [class.ephemeral]\nstatuses = [\"open\", \"graduated\", \"expired\"]\n\n\
             [kind.ledger]\nclass = \"episodic\"\n\n[kind.report]\nclass = \"ephemeral\"\n\n\
             [engagements.alpha]\nstatus = \"open\"\nremits = []\n"
        ),
    )
    .expect("write registry");
    let alpha = root.path().join("context/engagements/alpha");
    std::fs::create_dir_all(&alpha).expect("create engagement");
    std::fs::write(
        alpha.join("ledger-alpha.md"),
        "---\nkind: ledger\nstatus: open\n---\n",
    )
    .expect("write ledger");
    let report = alpha.join("report-01-alpha.md");
    std::fs::write(&report, "---\nkind: report\nstatus: open\n---\n").expect("write report");
    git(&["add", "context"]);
    let warning = "warning: context/engagements/alpha/report-01-alpha.md: ephemeral artefact \
                   still open; graduate or expire it when its unit lands [ephemeral-open]\n\
                   0 failures, 1 warning\n";

    // A first commit stages every file, the open report among them.
    aw().arg("lint")
        .arg(root.path())
        .assert()
        .success()
        .stderr(warning);
    git(&[
        "-c",
        "user.name=t",
        "-c",
        "user.email=t@t",
        "-c",
        "commit.gpgsign=false",
        "-c",
        "core.hooksPath=/dev/null",
        "commit",
        "-qm",
        "seed",
    ]);

    // A commit that leaves the open report alone hears nothing of it.
    std::fs::write(
        root.path().join("context/NOTES.md"),
        "---\nkind: doctrine\nstatus: live\n---\nA note.\n",
    )
    .expect("edit the note");
    git(&["add", "context"]);
    aw().arg("lint")
        .arg(root.path())
        .assert()
        .success()
        .stderr("0 failures, 0 warnings\n");

    // A mode change alone stages it, as `git diff` lists it.
    git(&[
        "update-index",
        "--chmod=+x",
        "context/engagements/alpha/report-01-alpha.md",
    ]);
    aw().arg("lint")
        .arg(root.path())
        .assert()
        .success()
        .stderr(warning);

    // So does a change to its text, and `--all` lists it either way.
    std::fs::write(&report, "---\nkind: report\nstatus: open\n---\nMore.\n").expect("edit report");
    git(&["add", "context"]);
    aw().arg("lint")
        .arg(root.path())
        .assert()
        .success()
        .stderr(warning);
    aw().args(["lint", "--all"])
        .arg(root.path())
        .assert()
        .success()
        .stderr(warning);
}

#[cfg(unix)]
#[test]
fn lint_skips_a_symlinked_artefact_in_both_modes() {
    let root = lint_fixture(
        "\n[template]\ncontract = 1\n",
        "---\nkind: doctrine\nstatus: live\n---\n",
    );
    // The link's target carries no frontmatter, so reading through the link
    // would fail it.
    std::fs::write(root.path().join("plain.txt"), "No frontmatter.\n").expect("write target");
    std::os::unix::fs::symlink("../plain.txt", root.path().join("context/LINK.md"))
        .expect("create link");
    let ok = std::process::Command::new("git")
        .arg("-C")
        .arg(root.path())
        .args(["add", "context"])
        .status()
        .expect("run git add")
        .success();
    assert!(ok, "git add succeeds");
    for args in [&["lint"][..], &["lint", "--all"]] {
        aw().args(args)
            .arg(root.path())
            .assert()
            .success()
            .stdout(predicate::str::is_empty())
            .stderr("0 failures, 0 warnings\n");
    }
}
