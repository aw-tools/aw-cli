//! Fetch many repositories at once, behind a live progress indicator.
//!
//! The pool knows nothing about what a caller reports. A caller hands it a task
//! list and a step to run after each fetch; the step decides the line printed
//! and what the task counts as.

use crate::git::{self, FetchOutcome};
use crate::reporting;
use anyhow::Result;
use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Repositories fetched at once when neither the command line nor the manifest
/// says otherwise. Four is a starting point, not a measurement: enough to hide
/// the round-trip latency that dominates a fetch of an unchanged repository,
/// low enough that a workspace reaching one host does not arrive as a burst.
/// `--concurrency 1` restores the sequential behaviour if a remote objects.
const DEFAULT_CONCURRENCY: usize = 4;

/// Assumed terminal width when stderr cannot answer for its own size.
const DEFAULT_WIDTH: usize = 80;

/// Settle how many repositories to fetch at once: the command line wins over
/// the manifest, which wins over the built-in default. Zero and one both mean
/// sequential, so the pool always has at least one worker.
pub fn resolve_concurrency(flag: Option<usize>, configured: Option<usize>) -> usize {
    flag.or(configured).unwrap_or(DEFAULT_CONCURRENCY).max(1)
}

/// One task's contribution to the run: the line it prints and what the caller
/// counts it as.
pub struct Report<C> {
    pub line: String,
    pub count: C,
}

/// A unit of work for the pool.
///
/// Everything decidable without touching the network is meant to be resolved
/// while planning, into `Settled`, so the pool holds nothing but real fetches
/// and the progress indicator's total counts only work that can actually block.
/// `job` is whatever the caller's step needs to know about a fetched task.
pub enum Task<J, C> {
    Settled(Report<C>),
    Fetch {
        path: PathBuf,
        label: String,
        job: J,
    },
}

/// Run every task, printing each task's line the moment it is known, and
/// return the counts in the order their lines were printed.
///
/// `step` runs after each fetch, outside every lock, so it may do slow work of
/// its own. An `Err` from `git::fetch` or from `step` is a hard error, unlike a
/// fetch that runs and reports failure.
///
/// `git::fetch` blocks a thread on a subprocess wait, so the parallelism here
/// is a bounded pool of threads, not an async runtime. Workers pull from a
/// shared cursor into `tasks`, so the task order is the order work starts in
/// and — at concurrency 1 — the order it finishes in too. Above that, lines
/// land in completion order: a slow repository no longer delays the report of
/// a fast one behind it.
pub fn run<J, C, S>(tasks: &[Task<J, C>], concurrency: usize, step: S) -> Result<Vec<C>>
where
    J: Sync,
    C: Copy + Send + Sync,
    S: Fn(&str, &J, FetchOutcome) -> Result<Report<C>> + Sync,
{
    let total = tasks
        .iter()
        .filter(|task| matches!(task, Task::Fetch { .. }))
        .count();
    let cursor = AtomicUsize::new(0);
    let progress = Mutex::new(Progress::new(total));
    let counts = Mutex::new(Vec::with_capacity(tasks.len()));
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
                            lock(&counts).push(report.count);
                            progress.emit(&report.line);
                        }
                        Task::Fetch { path, label, job } => {
                            lock(&progress).begin(label);
                            // No lock is held across the fetch or the step:
                            // they are the slow part, and holding one would
                            // serialise the very work being parallelised.
                            let report =
                                git::fetch(path).and_then(|outcome| step(label, job, outcome));
                            let mut progress = lock(&progress);
                            progress.finish(label);
                            match report {
                                Ok(report) => {
                                    lock(&counts).push(report.count);
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
    Ok(std::mem::take(&mut *lock(&counts)))
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

    /// Settled tasks never reach the step, and at concurrency 1 their counts
    /// come back in task order.
    #[test]
    fn settled_tasks_return_their_counts_in_order() {
        let tasks: Vec<Task<(), usize>> = (0..3)
            .map(|count| {
                Task::Settled(Report {
                    line: format!("line {count}"),
                    count,
                })
            })
            .collect();
        let counts = run(&tasks, 1, |_, (), _| unreachable!("no fetch task")).unwrap();
        assert_eq!(counts, vec![0, 1, 2]);
    }

    /// The step sees each fetch's real outcome and its own job, and with more
    /// workers than tasks every task still runs exactly once.
    #[test]
    fn the_step_sees_each_outcome_once_under_concurrency() {
        let dir = tempfile::tempdir().unwrap();
        let origin = dir.path().join("origin");
        git_in(dir.path(), &["init", "-q", "--bare", "origin"]);
        let good = clone_with_origin(dir.path(), "good", &origin);
        let bad = clone_with_origin(dir.path(), "bad", &dir.path().join("missing"));
        let tasks = vec![
            fetch_task(&good, "good", true),
            Task::Settled(Report {
                line: "settled".to_owned(),
                count: ("settled", false),
            }),
            fetch_task(&bad, "bad", false),
        ];

        let mut counts = run(&tasks, 8, |label, expect_fetched, outcome| {
            let fetched = matches!(outcome, FetchOutcome::Fetched);
            assert_eq!(fetched, *expect_fetched, "{label}");
            let label = if label == "good" { "good" } else { "bad" };
            Ok(Report {
                line: label.to_owned(),
                count: (label, fetched),
            })
        })
        .unwrap();
        counts.sort_unstable();
        assert_eq!(
            counts,
            vec![("bad", false), ("good", true), ("settled", false)]
        );
    }

    /// An error from the step is a hard error for the whole run, not a line.
    #[test]
    fn a_step_error_fails_the_run() {
        let dir = tempfile::tempdir().unwrap();
        let origin = dir.path().join("origin");
        git_in(dir.path(), &["init", "-q", "--bare", "origin"]);
        let repo = clone_with_origin(dir.path(), "repo", &origin);
        let tasks = vec![fetch_task(&repo, "repo", ())];

        let result = run(&tasks, 1, |_, (), _| -> Result<Report<()>> {
            anyhow::bail!("step broke")
        });
        assert_eq!(result.unwrap_err().to_string(), "step broke");
    }

    fn fetch_task<J, C>(path: &std::path::Path, label: &str, job: J) -> Task<J, C> {
        Task::Fetch {
            path: path.to_path_buf(),
            label: label.to_owned(),
            job,
        }
    }

    /// An empty repository whose `origin` points at `url`, which need not exist.
    fn clone_with_origin(parent: &std::path::Path, name: &str, url: &std::path::Path) -> PathBuf {
        git_in(parent, &["init", "-q", name]);
        let path = parent.join(name);
        git_in(
            &path,
            &["remote", "add", "origin", &url.display().to_string()],
        );
        path
    }

    fn git_in(dir: &std::path::Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?} failed");
    }
}
