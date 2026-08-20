//! Refresh remote-tracking refs and workspace skill links.

use crate::git::FetchOutcome;
use crate::{
    garden, git,
    manifest::{self, Manifest},
    reporting, skills,
};
use anyhow::Result;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Name the workspace root reports under, matching `aw status`'s own label for
/// it.
const WORKSPACE_LABEL: &str = "workspace";

/// Repositories fetched at once when neither the command line nor the manifest
/// says otherwise. Four is a starting point, not a measurement: enough to hide
/// the round-trip latency that dominates a fetch of an unchanged repository,
/// low enough that a workspace reaching one host does not arrive as a burst.
/// `--concurrency 1` restores the sequential behaviour if a remote objects.
const DEFAULT_CONCURRENCY: usize = 4;

/// Assumed terminal width when stderr cannot answer for its own size.
const DEFAULT_WIDTH: usize = 80;

/// Fetch the workspace layer and every present member, and re-run skill
/// linking, without changing any working tree or checked-out commit.
///
/// Fetches run concurrently. `git::fetch` blocks a thread on a subprocess wait,
/// so the parallelism here is a bounded pool of threads, not an async runtime.
pub fn run(root: &Path, concurrency: Option<usize>) -> Result<bool> {
    let manifest = Manifest::load(root)?;
    println!("{}", reporting::sync_workspace(&manifest.workspace.name));

    let concurrency = resolve_concurrency(
        concurrency,
        manifest.sync.as_ref().and_then(|sync| sync.concurrency),
    );
    let tasks = plan(root, &manifest)?;
    let tally = fetch_all(&tasks, concurrency)?;

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

/// Settle how many repositories to fetch at once: the command line wins over
/// the manifest, which wins over the built-in default. Zero and one both mean
/// sequential, so the pool always has at least one worker.
fn resolve_concurrency(flag: Option<usize>, configured: Option<usize>) -> usize {
    flag.or(configured).unwrap_or(DEFAULT_CONCURRENCY).max(1)
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

/// One repository's contribution to the fetch phase: the line it prints and
/// how it moves the tallies.
struct Report {
    line: String,
    count: Count,
}

/// A unit of the fetch phase.
///
/// Everything decidable without touching the network — a member that is not
/// checked out, a checkout that fails containment, a layer with no origin —
/// is resolved while planning, so the pool holds nothing but real fetches and
/// the progress indicator's total counts only work that can actually block.
enum Task {
    Settled(Report),
    Fetch {
        path: PathBuf,
        label: String,
        layer: bool,
    },
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
/// always reported them: the layer first, then the manifest's members.
fn plan(root: &Path, manifest: &Manifest) -> Result<Vec<Task>> {
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
            layer: true,
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
                layer: false,
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
fn report_fetch(label: &str, layer: bool, outcome: &FetchOutcome) -> Report {
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

/// Run the fetch phase, printing each task's line the moment it is known.
///
/// Workers pull from a shared cursor into `tasks`, so the manifest order is the
/// order work starts in and — at concurrency 1 — the order it finishes in too.
/// Above that, lines land in completion order: a slow repository no longer
/// delays the report of a fast one behind it.
fn fetch_all(tasks: &[Task], concurrency: usize) -> Result<Tally> {
    let total = tasks
        .iter()
        .filter(|task| matches!(task, Task::Fetch { .. }))
        .count();
    let cursor = AtomicUsize::new(0);
    let progress = Mutex::new(Progress::new(total));
    let tally = Mutex::new(Tally::default());
    // A fetch that fails to even run is a hard error, unlike a fetch that runs
    // and reports failure. Record the first and let the workers wind down:
    // tearing every in-flight child down mid-run is the interrupt path's job,
    // not an error path's.
    let failure = Mutex::new(None);

    std::thread::scope(|scope| {
        for _ in 0..concurrency.min(tasks.len().max(1)) {
            scope.spawn(|| {
                loop {
                    let index = cursor.fetch_add(1, Ordering::Relaxed);
                    let Some(task) = tasks.get(index) else { break };
                    if lock(&failure).is_some() {
                        break;
                    }
                    match task {
                        Task::Settled(report) => {
                            let mut progress = lock(&progress);
                            lock(&tally).add(report.count);
                            progress.emit(&report.line);
                        }
                        Task::Fetch { path, label, layer } => {
                            lock(&progress).begin(label);
                            // The lock is deliberately not held across the
                            // fetch: this is the only slow step, and holding it
                            // would serialise the very work being parallelised.
                            let outcome = git::fetch(path);
                            let mut progress = lock(&progress);
                            progress.finish(label);
                            match outcome {
                                Ok(outcome) => {
                                    let report = report_fetch(label, *layer, &outcome);
                                    lock(&tally).add(report.count);
                                    progress.emit(&report.line);
                                }
                                Err(error) => *lock(&failure) = Some(error),
                            }
                        }
                    }
                }
            });
        }
    });

    lock(&progress).erase();
    if let Some(error) = lock(&failure).take() {
        return Err(error);
    }
    Ok(std::mem::take(&mut *lock(&tally)))
}

/// Take a lock, recovering a poisoned one rather than panicking: a panic on one
/// fetch thread must not turn the whole run into a cascade of secondary panics
/// on every sibling that touches the same state.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The live fetch indicator and the redraw discipline that lets stdout result
/// lines land in the middle of it.
///
/// The indicator occupies a single stderr row, rewritten in place. Printing a
/// result line erases that row first and redraws it after, so the two streams
/// share a terminal without the indicator's remains being left mid-line.
///
/// Inactive unless stderr is a terminal: piped, redirected and CI runs carry no
/// progress output at all, in either stream.
struct Progress {
    total: usize,
    done: usize,
    in_flight: Vec<String>,
    /// Width sampled once, when the indicator is created. A sync is seconds
    /// long; re-sampling per redraw would buy correctness across a mid-run
    /// resize that nobody is there to perform.
    width: usize,
    active: bool,
}

impl Progress {
    fn new(total: usize) -> Self {
        let active = std::io::stderr().is_terminal();
        Self {
            total,
            done: 0,
            in_flight: Vec::new(),
            width: if active { terminal_width() } else { 0 },
            active,
        }
    }

    /// Note a fetch as started, so the indicator can name what it is waiting on.
    fn begin(&mut self, label: &str) {
        self.in_flight.push(label.to_owned());
        self.draw();
    }

    /// Note a fetch as finished. Paired with `emit`, which prints its line.
    fn finish(&mut self, label: &str) {
        if let Some(index) = self.in_flight.iter().position(|held| held == label) {
            self.in_flight.remove(index);
        }
        self.done += 1;
    }

    /// Print a result line to stdout without leaving the indicator broken.
    fn emit(&mut self, line: &str) {
        self.erase();
        println!("{line}");
        self.draw();
    }

    fn draw(&mut self) {
        if !self.active {
            return;
        }
        let in_flight: Vec<&str> = self.in_flight.iter().map(String::as_str).collect();
        let line = reporting::sync_progress(self.done, self.total, &in_flight, self.width);
        let mut stderr = std::io::stderr();
        // Carriage return plus erase-to-end-of-line, so a shorter line does not
        // leave the tail of a longer one behind it.
        let _ = write!(stderr, "\r\x1b[K{line}");
        let _ = stderr.flush();
    }

    fn erase(&mut self) {
        if !self.active {
            return;
        }
        let mut stderr = std::io::stderr();
        let _ = write!(stderr, "\r\x1b[K");
        let _ = stderr.flush();
    }
}

/// Columns available on stderr, or a conventional default when the terminal
/// cannot answer.
fn terminal_width() -> usize {
    rustix::termios::tcgetwinsize(std::io::stderr())
        .ok()
        .map(|size| usize::from(size.ws_col))
        .filter(|width| *width > 0)
        .unwrap_or(DEFAULT_WIDTH)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concurrency_prefers_the_flag_then_the_manifest_then_the_default() {
        assert_eq!(resolve_concurrency(Some(8), Some(2)), 8);
        assert_eq!(resolve_concurrency(None, Some(2)), 2);
        assert_eq!(resolve_concurrency(None, None), DEFAULT_CONCURRENCY);
    }

    /// Zero is a plausible thing to configure and must not leave the pool with
    /// no workers, which would silently fetch nothing at all.
    #[test]
    fn concurrency_floors_at_one_worker() {
        assert_eq!(resolve_concurrency(Some(0), None), 1);
        assert_eq!(resolve_concurrency(None, Some(0)), 1);
    }

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
