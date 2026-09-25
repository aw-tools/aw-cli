//! Refresh remote-tracking refs and workspace skill links.

use crate::fetch::{self, Report, Task};
use crate::git::FetchOutcome;
use crate::{
    garden, git,
    manifest::{self, Manifest},
    reporting, skills,
};
use anyhow::Result;
use std::path::Path;

/// Name the workspace root reports under, matching `aw status`'s own label for
/// it.
const WORKSPACE_LABEL: &str = "workspace";

/// Fetch the workspace layer and every present member, and re-run skill
/// linking, without changing any working tree or checked-out commit.
///
/// Fetches run concurrently, through the shared pool in `fetch`.
pub fn run(root: &Path, concurrency: Option<usize>) -> Result<bool> {
    let manifest = Manifest::load(root)?;
    println!("{}", reporting::sync_workspace(&manifest.workspace.name));

    let concurrency = fetch::resolve_concurrency(
        concurrency,
        manifest.sync.as_ref().and_then(|sync| sync.concurrency),
    );
    let tasks = plan(root, &manifest)?;
    let mut tally = Tally::default();
    for count in fetch::run(&tasks, concurrency, |label, layer, outcome| {
        Ok(report_fetch(label, *layer, &outcome))
    })? {
        tally.add(count);
    }

    let resolution = skills::resolve(root, &manifest)?;
    for (harness, changed) in skills::link(root, &resolution)? {
        println!("{}", reporting::sync_skills(harness, changed));
    }
    for skill in &resolution.linked {
        println!(
            "{}",
            reporting::sync_skill(&skill.name, skill.origin.label())
        );
    }
    for (loser, winner) in &resolution.shadowed {
        let path = loser.target.strip_prefix(root).unwrap_or(&loser.target);
        println!(
            "{}",
            reporting::sync_shadowed(&loser.name, &path.display().to_string(), winner.label())
        );
    }
    for missing in &resolution.missing_dirs {
        println!(
            "{}",
            reporting::sync_missing_skill_dir(&missing.repo, &missing.dir)
        );
    }
    for unmatched in &resolution.unmatched_only {
        println!(
            "{}",
            reporting::sync_unmatched_only(&unmatched.repo, &unmatched.entry)
        );
    }

    println!(
        "{}",
        reporting::sync_verify(
            tally.present,
            tally.skipped,
            tally.failed,
            resolution.linked.len(),
            resolution.shadowed.len()
        )
    );
    Ok(tally.failed == 0 && !tally.layer_failed)
}

/// What a finished task contributes to the run's tallies. The layer is counted
/// apart from the members: `aw status` reports it alongside them but it is not
/// one of them, and the verify line has always counted members only.
#[derive(Clone, Copy)]
enum Count {
    MemberFetched,
    MemberFailed,
    MemberSkipped,
    Layer,
    LayerFailed,
}

/// Running totals for the fetch phase.
#[derive(Default)]
struct Tally {
    present: usize,
    skipped: usize,
    failed: usize,
    layer_failed: bool,
}

impl Tally {
    fn add(&mut self, count: Count) {
        match count {
            Count::MemberFetched => self.present += 1,
            Count::MemberFailed => {
                self.present += 1;
                self.failed += 1;
            }
            Count::MemberSkipped => self.skipped += 1,
            Count::Layer => {}
            Count::LayerFailed => self.layer_failed = true,
        }
    }
}

/// Classify the layer and every member into tasks, in the order `aw sync` has
/// always reported them: the layer first, then the manifest's members. A fetch
/// task's job says whether it is the layer.
fn plan(root: &Path, manifest: &Manifest) -> Result<Vec<Task<bool, Count>>> {
    let mut tasks = Vec::with_capacity(manifest.repos.len() + 1);

    // The workspace root is a repository too, and `aw status` reports its
    // ahead/behind alongside the members'. Fetch it here so that report reads a
    // ref something refreshes. Guard on the checkout the same way the member
    // arm does: the root is a workspace because it carries the manifest, not
    // because it is a repository, and `git config --local` in a directory that
    // is not one either fails outright or answers for an enclosing repository.
    tasks.push(if !git::is_repo_checked(root)? {
        Task::Settled(Report {
            line: reporting::sync_layer_absent(WORKSPACE_LABEL),
            count: Count::Layer,
        })
    } else if git::has_origin(root)? {
        Task::Fetch {
            path: root.to_path_buf(),
            label: WORKSPACE_LABEL.to_owned(),
            job: true,
        }
    } else {
        Task::Settled(Report {
            line: reporting::sync_layer_no_origin(WORKSPACE_LABEL),
            count: Count::Layer,
        })
    });

    for repo in &manifest.repos {
        let path = garden::checkout_path(root, &repo.path);
        if !git::is_repo_checked(&path)? {
            tasks.push(Task::Settled(Report {
                line: reporting::sync_repo_skipped(&repo.path),
                count: Count::MemberSkipped,
            }));
            continue;
        }
        match manifest::canonicalise_member(root, &path) {
            Ok((path, _)) => tasks.push(Task::Fetch {
                path,
                label: repo.path.clone(),
                job: false,
            }),
            Err(error) => tasks.push(Task::Settled(Report {
                line: reporting::sync_repo_failed(&repo.path, &format!("unsafe checkout: {error}")),
                count: Count::MemberFailed,
            })),
        }
    }
    Ok(tasks)
}

/// Turn a fetch outcome into the line and tally movement it reports.
fn report_fetch(label: &str, layer: bool, outcome: &FetchOutcome) -> Report<Count> {
    match (layer, outcome) {
        (true, FetchOutcome::Fetched) => Report {
            line: reporting::sync_layer_fetched(label),
            count: Count::Layer,
        },
        (true, FetchOutcome::Failed(why)) => Report {
            line: reporting::sync_layer_failed(label, why),
            count: Count::LayerFailed,
        },
        (false, FetchOutcome::Fetched) => Report {
            line: reporting::sync_repo_fetched(label),
            count: Count::MemberFetched,
        },
        (false, FetchOutcome::Failed(why)) => Report {
            line: reporting::sync_repo_failed(label, why),
            count: Count::MemberFailed,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tally_counts_a_failed_member_as_present_and_failed() {
        let mut tally = Tally::default();
        tally.add(Count::MemberFetched);
        tally.add(Count::MemberFailed);
        tally.add(Count::MemberSkipped);
        assert_eq!((tally.present, tally.skipped, tally.failed), (2, 1, 1));
    }

    /// The layer stays out of the member tallies and carries its own flag,
    /// which is what makes `aw sync` exit non-zero when only the layer fails.
    #[test]
    fn tally_keeps_the_layer_out_of_the_member_counts() {
        let mut tally = Tally::default();
        tally.add(Count::Layer);
        tally.add(Count::LayerFailed);
        assert_eq!((tally.present, tally.skipped, tally.failed), (0, 0, 0));
        assert!(tally.layer_failed);
    }

    #[test]
    fn fetch_outcomes_report_under_the_right_prefix() {
        let failure = FetchOutcome::Failed("remote hung up".to_owned());
        assert!(
            report_fetch("workspace", true, &FetchOutcome::Fetched)
                .line
                .starts_with("layer")
        );
        assert!(
            report_fetch("skills", false, &failure)
                .line
                .starts_with("repo")
        );
        assert!(
            report_fetch("skills", false, &failure)
                .line
                .contains("remote hung up")
        );
    }
}
