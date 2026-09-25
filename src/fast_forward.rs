//! Fast-forward members that sit cleanly on their default branch.
//!
//! The one verb that writes a member's working tree. It fetches every present
//! member through the same pool `aw sync` uses, then moves only a checkout that
//! passes every eligibility check, and only with `git merge --ff-only`. The
//! workspace layer is never fetched or moved here.

use crate::fetch::{self, Report, Task};
use crate::garden;
use crate::git::{self, FetchOutcome};
use crate::manifest::{self, Manifest};
use crate::reporting::{self, FastForwardSkip};
use anyhow::Result;
use std::path::{Path, PathBuf};

/// Fetch every present member, then fast-forward each eligible one. With
/// `dry_run`, report what would move and move nothing. Returns whether no fetch
/// and no move failed.
pub fn run(root: &Path, concurrency: Option<usize>, dry_run: bool) -> Result<bool> {
    let manifest = Manifest::load(root)?;
    println!(
        "{}",
        reporting::fast_forward_workspace(&manifest.workspace.name)
    );

    let concurrency = fetch::resolve_concurrency(
        concurrency,
        manifest.sync.as_ref().and_then(|sync| sync.concurrency),
    );
    let tasks = plan(root, &manifest)?;
    let mut tally = Tally::default();
    // A member that cannot be assessed or moved is a failure line, never an
    // error: an error from the step stops the whole run, and every later
    // member must still be processed.
    for count in fetch::run(&tasks, concurrency, |label, path, outcome| {
        Ok(match outcome {
            FetchOutcome::Fetched => advance(path, dry_run)
                .unwrap_or_else(|error| Outcome::Failed(format!("{error:#}")))
                .report(label),
            FetchOutcome::Failed(why) => Outcome::Failed(why).report(label),
        })
    })? {
        tally.add(count);
    }

    println!(
        "{}",
        reporting::fast_forward_verify(
            tally.advanced,
            tally.up_to_date,
            tally.skipped,
            tally.failed,
            dry_run
        )
    );
    Ok(tally.failed == 0)
}

/// Classify every member into tasks in manifest order. An absent member or an
/// unsafe checkout is settled without a fetch; a fetch task's job is the
/// checkout to assess once its fetch lands.
fn plan(root: &Path, manifest: &Manifest) -> Result<Vec<Task<PathBuf, Count>>> {
    let mut tasks = Vec::with_capacity(manifest.repos.len());
    for repo in &manifest.repos {
        let path = garden::checkout_path(root, &repo.path);
        if !git::is_repo_checked(&path)? {
            tasks.push(Task::Settled(Report {
                line: reporting::fast_forward_absent(&repo.path),
                count: Count::Skipped,
            }));
            continue;
        }
        match manifest::canonicalise_member(root, &path) {
            Ok((path, _)) => tasks.push(Task::Fetch {
                path: path.clone(),
                label: repo.path.clone(),
                job: path,
            }),
            Err(error) => tasks.push(Task::Settled(
                Outcome::Failed(format!("unsafe checkout: {error}")).report(&repo.path),
            )),
        }
    }
    Ok(tasks)
}

/// What happened to one fetched member.
enum Outcome {
    Advanced {
        old: String,
        new: String,
        commits: u64,
    },
    WouldAdvance {
        old: String,
        new: String,
        commits: u64,
    },
    UpToDate,
    Skipped(FastForwardSkip),
    Failed(String),
}

impl Outcome {
    fn report(self, label: &str) -> Report<Count> {
        match self {
            Self::Advanced { old, new, commits } => Report {
                line: reporting::fast_forward_advanced(label, &old, &new, commits),
                count: Count::Advanced,
            },
            Self::WouldAdvance { old, new, commits } => Report {
                line: reporting::fast_forward_would_advance(label, &old, &new, commits),
                count: Count::Advanced,
            },
            Self::UpToDate => Report {
                line: reporting::fast_forward_up_to_date(label),
                count: Count::UpToDate,
            },
            Self::Skipped(reason) => Report {
                line: reporting::fast_forward_skipped(label, &reason),
                count: Count::Skipped,
            },
            Self::Failed(why) => Report {
                line: reporting::fast_forward_failed(label, &why),
                count: Count::Failed,
            },
        }
    }
}

/// Check a freshly fetched checkout against every eligibility rule, in order,
/// and move it only when all of them hold.
fn advance(path: &Path, dry_run: bool) -> Result<Outcome> {
    let Some(branch) = git::current_branch(path)? else {
        return Ok(Outcome::Skipped(FastForwardSkip::Detached));
    };
    // `aw`'s fetch passes an explicit refspec, which bypasses git's own upkeep
    // of `origin/HEAD`, so a member pushed rather than cloned never gets one.
    // Creating it writes a remote-tracking ref, the same class of change a
    // fetch makes, so a dry run does it too.
    let default = match git::origin_default_branch(path)? {
        Some(default) => default,
        None => match git::set_origin_head(path)? {
            Err(why) => return Ok(Outcome::Skipped(FastForwardSkip::NoDefault(why))),
            Ok(()) => match git::origin_default_branch(path)? {
                Some(default) => default,
                None => {
                    return Ok(Outcome::Skipped(FastForwardSkip::NoDefault(
                        "origin/HEAD is still unset".to_owned(),
                    )));
                }
            },
        },
    };
    if branch != default {
        return Ok(Outcome::Skipped(FastForwardSkip::NotDefault {
            branch,
            default,
        }));
    }
    // The move is `merge --ff-only @{u}`, so the upstream must be the default
    // branch on origin: tracking anything else would pull another branch in.
    let Some(upstream) = git::upstream_ref(path, &branch)? else {
        return Ok(Outcome::Skipped(FastForwardSkip::NoUpstream));
    };
    if upstream != format!("refs/remotes/origin/{default}") {
        return Ok(Outcome::Skipped(FastForwardSkip::OtherUpstream {
            upstream: upstream
                .strip_prefix("refs/remotes/")
                .unwrap_or(&upstream)
                .to_owned(),
            default,
        }));
    }
    let Some(comparison) = git::upstream_comparison(path)? else {
        return Ok(Outcome::Skipped(FastForwardSkip::NoUpstream));
    };
    if git::has_tracked_changes(path)? {
        return Ok(Outcome::Skipped(FastForwardSkip::TrackedChanges));
    }
    if let Some(operation) = git::operation_in_progress(path)? {
        return Ok(Outcome::Skipped(FastForwardSkip::InProgress(operation)));
    }
    let (ahead, behind) = (comparison.ahead, comparison.behind);
    if ahead > 0 {
        return Ok(Outcome::Skipped(FastForwardSkip::Ahead { ahead, behind }));
    }
    if behind == 0 {
        return Ok(Outcome::UpToDate);
    }

    let old = git::short_commit(path, "HEAD")?;
    if dry_run {
        let new = git::short_commit(path, "@{u}")?;
        return Ok(Outcome::WouldAdvance {
            old,
            new,
            commits: behind,
        });
    }
    if let Err(why) = git::merge_ff_only(path)? {
        return Ok(Outcome::Failed(why));
    }
    let new = git::short_commit(path, "HEAD")?;
    Ok(Outcome::Advanced {
        old,
        new,
        commits: behind,
    })
}

/// What a finished task contributes to the verify line. A dry run's "would
/// advance" counts as advanced; the line's wording carries the difference.
#[derive(Clone, Copy)]
enum Count {
    Advanced,
    UpToDate,
    Skipped,
    Failed,
}

#[derive(Default)]
struct Tally {
    advanced: usize,
    up_to_date: usize,
    skipped: usize,
    failed: usize,
}

impl Tally {
    fn add(&mut self, count: Count) {
        match count {
            Count::Advanced => self.advanced += 1,
            Count::UpToDate => self.up_to_date += 1,
            Count::Skipped => self.skipped += 1,
            Count::Failed => self.failed += 1,
        }
    }
}
