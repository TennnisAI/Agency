//! Background installs: the tools of `tools.rs` and the agent CLIs of the
//! catalog, run without a project and without a terminal.
//!
//! Every earlier install flow opened an in-app terminal, and an in-app
//! terminal is a run, and a run belongs to a project. Onboarding has no
//! project yet, so its "Install…" could only hand over a line to paste into
//! some other application, and the user then had to come back and re-check
//! the tile by hand. This runs the line here, keeps the output, and reports
//! the exit so the caller can re-probe PATH and tick the tile itself.
//!
//! One job per key at a time. Output is capped: an installer that streams a
//! progress bar for ten minutes would otherwise grow without bound, and the
//! only reader is the failure dialog, which shows the last lines.

use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

/// Bytes of output kept per job. Enough for the tail of any install log that
/// matters (npm's EACCES trace is under 2 KB, apt's is a screen).
pub const OUTPUT_CAP: usize = 64 * 1024;
/// Lines the status carries back to the UI.
pub const TAIL_LINES: usize = 40;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum JobState {
    /// Accepted, waiting for another job in its lane to finish (see `start`).
    Queued,
    Running,
    Succeeded,
    Failed,
}

/// What the UI sees of a job: its state and the tail of what it printed.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobStatus {
    pub key: String,
    pub state: JobState,
    /// `None` while running or queued, or when the process died to a signal.
    pub exit_code: Option<i32>,
    pub output: String,
}

struct Job {
    state: JobState,
    exit_code: Option<i32>,
    output: Arc<Mutex<String>>,
}

/// A job accepted into a lane but not yet spawned.
struct Pending {
    key: String,
    /// The PATH the script runs with: the process PATH plus whatever
    /// `pathenv` has adopted since startup, captured when the job was accepted.
    path: String,
    script: String,
    output: Arc<Mutex<String>>,
    on_exit: Box<dyn FnOnce(JobStatus) + Send>,
}

/// One serialization lane: at most one job runs, the rest wait in order.
#[derive(Default)]
struct Lane {
    running: bool,
    queue: VecDeque<Pending>,
}

type Jobs = Arc<Mutex<HashMap<String, Job>>>;
type Lanes = Arc<Mutex<HashMap<String, Lane>>>;

/// Background installs, managed by Tauri for the process's lifetime.
///
/// Jobs in the same lane run one at a time; different lanes run in parallel.
/// The lane exists because system package managers hold a single machine-wide
/// lock: two `apt-get install` at once fail with "Could not get lock ... held
/// by process N" (observed 2026-09-10, git and gh installed together), and so
/// do parallel `npm install -g` to one prefix. The caller names the lane
/// (`commands::start_install`); a job with no shared resource gets its own key
/// as its lane and so never waits.
pub struct Installs {
    jobs: Jobs,
    lanes: Lanes,
    /// The interpreter every script runs under. Always `/bin/sh` outside the
    /// tests, which point it at a path that does not exist to exercise the
    /// could-not-start branch.
    sh: String,
}

impl Default for Installs {
    fn default() -> Self {
        Self { jobs: Jobs::default(), lanes: Lanes::default(), sh: SH.to_string() }
    }
}

/// The scripts are POSIX `sh`, so that is what runs them. The first version
/// ran them under the user's `$SHELL -lc`, which broke on the platform this
/// exists for: fish accepts `-lc` and then rejects `p="$(npm prefix -g)"`,
/// `then` and `fi` from `tools::install_script`, so every npm agent install
/// failed with a syntax error, and tcsh rejects `-lc` itself ("Unknown
/// option"), so nothing installed at all. The login shell was only ever there
/// for its PATH; `pathenv::repair` has already folded that into the process
/// environment, and the job carries the effective PATH explicitly.
const SH: &str = "/bin/sh";

impl Installs {
    /// Accept `script` (run under `/bin/sh -c` with `path` as its PATH) as
    /// job `key` in serialization lane `lane`. Runs at once if the lane is
    /// idle, else waits its turn.
    /// Refused while a job with the same `key` is still queued or running: two
    /// installs of one thing racing each other is how a half-written
    /// `node_modules` happens. `on_exit` runs on the job's own thread once the
    /// process is gone, with the final status.
    pub fn start(
        &self,
        key: &str,
        lane: &str,
        path: &str,
        script: &str,
        on_exit: impl FnOnce(JobStatus) + Send + 'static,
    ) -> Result<(), String> {
        let output = Arc::new(Mutex::new(String::new()));
        {
            let mut jobs = self.jobs.lock().unwrap();
            if jobs
                .get(key)
                .is_some_and(|j| matches!(j.state, JobState::Running | JobState::Queued))
            {
                return Err(format!("{key} is already installing"));
            }
            jobs.insert(
                key.to_string(),
                Job { state: JobState::Queued, exit_code: None, output: output.clone() },
            );
        }
        let pending = Pending {
            key: key.to_string(),
            path: path.to_string(),
            script: script.to_string(),
            output,
            on_exit: Box::new(on_exit),
        };
        let mut lanes = self.lanes.lock().unwrap();
        let l = lanes.entry(lane.to_string()).or_default();
        if l.running {
            l.queue.push_back(pending);
        } else {
            l.running = true;
            drop(lanes);
            spawn_pending(
                self.jobs.clone(),
                self.lanes.clone(),
                &self.sh,
                lane.to_string(),
                pending,
            );
        }
        Ok(())
    }

    /// Every job started this session, running, queued or finished.
    pub fn list(&self) -> Vec<JobStatus> {
        let jobs = self.jobs.lock().unwrap();
        let mut out: Vec<JobStatus> = jobs
            .iter()
            .map(|(key, job)| JobStatus {
                key: key.clone(),
                state: job.state,
                exit_code: job.exit_code,
                output: tail(&job.output.lock().unwrap(), TAIL_LINES),
            })
            .collect();
        out.sort_by(|a, b| a.key.cmp(&b.key));
        out
    }
}

/// Spawn `p`, and when it finishes run the next job waiting in `lane` (or free
/// the lane). A free function over the shared maps, not a method, so the job's
/// own thread can advance the lane without a handle to `Installs`.
///
/// The process is started from inside the waiter thread, not before it. The
/// first version spawned the process first and then `let _ =` the thread; if
/// the thread could not be created the process ran with nobody to reap it,
/// the job stayed Running forever and the lane stayed busy, so every later
/// install in it queued behind a job that would never finish.
fn spawn_pending(jobs: Jobs, lanes: Lanes, sh: &str, lane: String, p: Pending) {
    let Pending { key, path, script, output, on_exit } = p;
    if let Some(job) = jobs.lock().unwrap().get_mut(&key) {
        job.state = JobState::Running;
    }
    // Shared with the thread so that whichever side fails can still report.
    let on_exit: OnExit = Arc::new(Mutex::new(Some(on_exit)));
    let run = {
        let (jobs, lanes, sh, lane, key, output, on_exit) = (
            jobs.clone(),
            lanes.clone(),
            sh.to_string(),
            lane.clone(),
            key.clone(),
            output.clone(),
            on_exit.clone(),
        );
        move || {
            let spawned = Command::new(&sh)
                .arg("-c")
                .arg(&script)
                .env("PATH", &path)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn();
            let mut child = match spawned {
                Ok(c) => c,
                Err(e) => {
                    // Could not even start the shell. The reason goes into the
                    // job's own output too, so a remounted onboarding step,
                    // which reads `list()`, sees why and not an empty failure.
                    let msg = format!("could not start {sh}: {e}");
                    append_capped(&mut output.lock().unwrap(), &msg, OUTPUT_CAP);
                    finish(jobs, lanes, &sh, lane, key, output, on_exit, JobState::Failed, None);
                    return;
                }
            };
            let readers = [
                child.stdout.take().map(|s| pump(s, output.clone())),
                child.stderr.take().map(|s| pump(s, output.clone())),
            ];
            let status = child.wait().ok();
            for r in readers.into_iter().flatten() {
                let _ = r.join();
            }
            let exit_code = status.and_then(|s| s.code());
            let state = if status.is_some_and(|s| s.success()) {
                JobState::Succeeded
            } else {
                JobState::Failed
            };
            finish(jobs, lanes, &sh, lane, key, output, on_exit, state, exit_code);
        }
    };
    if let Err(e) = std::thread::Builder::new().name(format!("install {key}")).spawn(run) {
        let msg = format!("could not start a thread for {key}: {e}");
        append_capped(&mut output.lock().unwrap(), &msg, OUTPUT_CAP);
        finish(jobs, lanes, sh, lane, key, output, on_exit, JobState::Failed, None);
    }
}

type OnExit = Arc<Mutex<Option<Box<dyn FnOnce(JobStatus) + Send>>>>;

/// Record a job's final state, report it once, and move the lane on.
#[allow(clippy::too_many_arguments)]
fn finish(
    jobs: Jobs,
    lanes: Lanes,
    sh: &str,
    lane: String,
    key: String,
    output: Arc<Mutex<String>>,
    on_exit: OnExit,
    state: JobState,
    exit_code: Option<i32>,
) {
    if let Some(job) = jobs.lock().unwrap().get_mut(&key) {
        job.state = state;
        job.exit_code = exit_code;
    }
    let tailed = tail(&output.lock().unwrap(), TAIL_LINES);
    if let Some(f) = on_exit.lock().unwrap().take() {
        f(JobStatus { key, state, exit_code, output: tailed });
    }
    advance_lane(jobs, lanes, sh, lane);
}

/// Run the next job in `lane`, or mark the lane idle when its queue is empty.
fn advance_lane(jobs: Jobs, lanes: Lanes, sh: &str, lane: String) {
    let next = {
        let mut map = lanes.lock().unwrap();
        match map.get_mut(&lane).and_then(|l| l.queue.pop_front()) {
            Some(p) => Some(p),
            None => {
                if let Some(l) = map.get_mut(&lane) {
                    l.running = false;
                }
                None
            }
        }
    };
    if let Some(p) = next {
        spawn_pending(jobs, lanes, sh, lane, p);
    }
}

/// Copy everything a pipe produces into `buf`, dropping the oldest bytes past
/// [`OUTPUT_CAP`].
fn pump(
    mut src: impl Read + Send + 'static,
    buf: Arc<Mutex<String>>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut chunk = [0u8; 4096];
        loop {
            match src.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => append_capped(
                    &mut buf.lock().unwrap(),
                    &String::from_utf8_lossy(&chunk[..n]),
                    OUTPUT_CAP,
                ),
            }
        }
    })
}

/// Append `chunk`, then trim the front so the buffer stays under `cap` bytes,
/// cutting on a char boundary so the string stays valid.
pub fn append_capped(buf: &mut String, chunk: &str, cap: usize) {
    buf.push_str(chunk);
    if buf.len() > cap {
        let mut cut = buf.len() - cap;
        while !buf.is_char_boundary(cut) {
            cut += 1;
        }
        buf.drain(..cut);
    }
}

/// The last `lines` lines of `output`, with carriage-return progress
/// redraws collapsed to what was finally shown on each line: npm and curl
/// both animate with `\r`, and the raw bytes would read as one enormous line.
pub fn tail(output: &str, lines: usize) -> String {
    let normalized: Vec<&str> = output
        .lines()
        .map(|l| l.rsplit('\r').next().unwrap_or(l))
        .filter(|l| !l.trim().is_empty())
        .collect();
    let start = normalized.len().saturating_sub(lines);
    normalized[start..].join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path() -> String {
        std::env::var("PATH").unwrap_or_default()
    }

    impl Installs {
        fn with_shell(sh: &str) -> Self {
            Self { sh: sh.to_string(), ..Self::default() }
        }
    }

    #[test]
    fn tail_keeps_the_last_lines_and_the_final_frame_of_a_progress_line() {
        let out = "a\nb\n[####      ] 40%\r[########  ] 80%\r[##########] 100%\nc\n";
        assert_eq!(tail(out, 2), "[##########] 100%\nc");
        assert_eq!(tail(out, 10), "a\nb\n[##########] 100%\nc");
        assert_eq!(tail("", 5), "");
    }

    #[test]
    fn tail_drops_blank_lines() {
        assert_eq!(tail("x\n\n\n\ny\n", 5), "x\ny");
    }

    #[test]
    fn append_capped_trims_the_front_on_a_char_boundary() {
        let mut buf = String::new();
        append_capped(&mut buf, "héllo", 4);
        assert_eq!(buf, "llo");
        append_capped(&mut buf, "wörld", 4);
        assert_eq!(buf, "rld");
    }

    #[test]
    fn a_job_runs_to_completion_and_reports_its_exit() {
        let installs = Installs::default();
        let (tx, rx) = std::sync::mpsc::channel();
        installs
            .start("test:ok", "lane:a", &path(), "echo one; echo two >&2; exit 0", move |s| {
                tx.send(s).unwrap();
            })
            .unwrap();
        let status = rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap();
        assert_eq!(status.state, JobState::Succeeded);
        assert_eq!(status.exit_code, Some(0));
        assert!(status.output.contains("one") && status.output.contains("two"));
        let listed = installs.list();
        assert!(listed.iter().any(|j| j.key == "test:ok" && j.state == JobState::Succeeded));
    }

    #[test]
    fn a_failing_job_reports_failure_and_its_output() {
        let installs = Installs::default();
        let (tx, rx) = std::sync::mpsc::channel();
        installs
            .start("test:fail", "lane:a", &path(), "echo nope >&2; exit 3", move |s| {
                tx.send(s).unwrap();
            })
            .unwrap();
        let status = rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap();
        assert_eq!(status.state, JobState::Failed);
        assert_eq!(status.exit_code, Some(3));
        assert_eq!(status.output, "nope");
    }

    #[test]
    fn a_running_key_refuses_a_second_start() {
        let installs = Installs::default();
        let (tx, rx) = std::sync::mpsc::channel();
        installs
            .start("test:busy", "lane:a", &path(), "sleep 2", move |s| {
                tx.send(s).unwrap();
            })
            .unwrap();
        let err = installs.start("test:busy", "lane:a", &path(), "true", |_| {}).unwrap_err();
        assert!(err.contains("already installing"), "{err}");
        let _ = rx.recv_timeout(std::time::Duration::from_secs(10));
    }

    #[test]
    fn jobs_in_one_lane_run_one_at_a_time_and_keep_their_order() {
        // Two jobs, same lane, each appends to a shared file. The lane must run
        // the first to completion before the second starts, so the file ends
        // "a\nb" and never interleaves — the apt-lock collision this prevents.
        let installs = Installs::default();
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("order");
        let f = f.to_string_lossy().to_string();
        let (tx, rx) = std::sync::mpsc::channel();
        let script = |tag: &str| {
            format!("printf 'start-{tag}\\n' >> {f}; sleep 0.3; printf 'end-{tag}\\n' >> {f}")
        };
        for tag in ["a", "b"] {
            let tx = tx.clone();
            installs
                .start(&format!("t:{tag}"), "shared", &path(), &script(tag), move |s| {
                    tx.send(s.key).unwrap();
                })
                .unwrap();
        }
        // The second was queued behind the first, not run in parallel.
        assert!(installs.list().iter().any(|j| j.key == "t:b" && j.state == JobState::Queued));
        let first = rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap();
        let second = rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap();
        assert_eq!((first.as_str(), second.as_str()), ("t:a", "t:b"));
        let body = std::fs::read_to_string(dir.path().join("order")).unwrap();
        assert_eq!(body, "start-a\nend-a\nstart-b\nend-b\n", "the lane interleaved");
    }

    #[test]
    fn a_shell_that_cannot_start_fails_the_job_with_a_reason_and_frees_the_lane() {
        // A job whose interpreter is missing: the job is Failed, the reason is
        // in its own output (not only in the callback), and the lane runs the
        // next job instead of wedging.
        let installs = Installs::with_shell("/nonexistent/agency-test-sh");
        let (tx, rx) = std::sync::mpsc::channel();
        for key in ["x:1", "x:2"] {
            let tx = tx.clone();
            installs.start(key, "lane", &path(), "true", move |s| tx.send(s).unwrap()).unwrap();
        }
        let first = rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap();
        let second = rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap();
        assert_eq!((first.key.as_str(), second.key.as_str()), ("x:1", "x:2"));
        assert_eq!(first.state, JobState::Failed);
        assert!(first.output.contains("could not start"), "{}", first.output);
        let listed = installs.list();
        let job = listed.iter().find(|j| j.key == "x:1").unwrap();
        assert_eq!(job.state, JobState::Failed);
        assert!(job.output.contains("could not start"), "list() lost the reason: {:?}", job.output);
        // The lane is free again: a third job in it runs at once.
        installs.start("x:3", "lane", &path(), "true", |_| {}).unwrap();
        assert!(installs.list().iter().all(|j| j.state != JobState::Queued));
    }

    #[test]
    fn scripts_run_under_sh_with_the_given_path() {
        // The wrapper `tools::install_script` emits is sh syntax; a fish or
        // tcsh login shell must never see it, and the PATH the job carries is
        // the one the script resolves commands against.
        let installs = Installs::default();
        let dir = tempfile::tempdir().unwrap();
        let tool = dir.path().join("agency-test-tool");
        std::fs::write(&tool, "#!/bin/sh\necho from-tool\n").unwrap();
        std::fs::set_permissions(&tool, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        installs
            .start(
                "sh:1",
                "lane",
                &format!("{}:{}", dir.path().display(), path()),
                "p=1; if [ \"$p\" = 1 ]; then agency-test-tool; fi",
                move |s| tx.send(s).unwrap(),
            )
            .unwrap();
        let status = rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap();
        assert_eq!(status.state, JobState::Succeeded, "{}", status.output);
        assert_eq!(status.output, "from-tool");
    }

    #[test]
    fn separate_lanes_run_in_parallel() {
        // Two long jobs in different lanes are both Running at once.
        let installs = Installs::default();
        installs.start("p:1", "lane:1", &path(), "sleep 1", |_| {}).unwrap();
        installs.start("p:2", "lane:2", &path(), "sleep 1", |_| {}).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(200));
        let running = installs.list().into_iter().filter(|j| j.state == JobState::Running).count();
        assert_eq!(running, 2, "different lanes should not serialize");
    }
}
