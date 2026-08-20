//! One PTY-backed child process with reader + wait threads.
use anyhow::Result;
use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How long to wait for the rest of a write the kernel handed us in pieces.
/// It only has to outlive the gap between two reads of one write, which is the
/// time it takes the writer to be scheduled again: measured at well under
/// 100us, and 0.0-0.1ms as seen from the reader.
const STITCH_QUIET: Duration = Duration::from_micros(500);

/// Ceiling on how long one stitched chunk may keep accumulating. Without it an
/// agent writing steadily (or a `cat` of a large file) never leaves a gap, and
/// its output would be held back for as long as it kept going.
const STITCH_BURST: Duration = Duration::from_millis(4);

/// Ceiling on the size of one stitched chunk, for the same reason.
const STITCH_MAX: usize = 128 * 1024;

/// Reassemble the pieces the kernel tore one write into.
///
/// macOS returns at most 1024 bytes from a read on a pty master however large
/// the buffer you hand it: a single 200,000-byte write comes back as 195 reads
/// of 1024 and one of 320. So a terminal repaint bigger than a kilobyte — which
/// is most of them; Cursor Agent's measure 1.3-2 kB at a real pane size — never
/// reaches us whole, and each piece used to travel to the frontend as its own
/// frame and its own `term.write`.
///
/// That is AGE-151. xterm renders once per animation frame from whatever the
/// buffer holds at the time, so a frame boundary landing between two pieces of
/// one repaint paints half of it. Cursor Agent draws on the normal screen the
/// way an Ink app does — blank the old frame with `\x1b[2K\x1b[1A` per line,
/// then write the new one — so half a repaint is a band of blank lines where
/// the output was, and it flickered on every line it wrote. Claude Code shows
/// nothing of the sort: it repaints the alternate screen in place, overwriting
/// the cells it changes instead of blanking them first, so a torn repaint of
/// its own is invisible.
///
/// Pieces of one write arrive back to back, so a short quiet gap tells them
/// apart from the next write reliably, and the two ceilings keep a writer that
/// never goes quiet from being buffered indefinitely.
fn stitch_reads<F>(rx: Receiver<Vec<u8>>, quiet: Duration, burst: Duration, max: usize, mut emit: F)
where
    F: FnMut(Vec<u8>),
{
    while let Ok(mut acc) = rx.recv() {
        let deadline = Instant::now() + burst;
        while acc.len() < max {
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            // Gone quiet, out of burst, or the reader hung up: what we have is
            // a whole write. A hang-up ends the outer loop on the next recv.
            match rx.recv_timeout(quiet.min(deadline - now)) {
                Ok(more) => acc.extend_from_slice(&more),
                Err(_) => break,
            }
        }
        emit(acc);
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ProcStatus {
    Running,
    Exited(i32),
    Crashed,
}

pub struct PtyProcess {
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    status: Arc<Mutex<ProcStatus>>,
    killer: Box<dyn ChildKiller + Send + Sync>,
    // Wrapped in Mutex so PtyProcess is Sync (needed for Arc<Session>: Send).
    master: Mutex<Box<dyn MasterPty + Send>>,
}

impl Drop for PtyProcess {
    fn drop(&mut self) {
        let _ = self.killer.kill();
    }
}

impl PtyProcess {
    pub fn write_input(&self, data: &[u8]) -> Result<()> {
        let mut w = self.writer.lock().unwrap();
        w.write_all(data)?;
        w.flush()?;
        Ok(())
    }

    pub fn resize(&self, rows: u16, cols: u16) -> Result<()> {
        self.master.lock().unwrap().resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        Ok(())
    }

    pub fn status(&self) -> ProcStatus {
        self.status.lock().unwrap().clone()
    }
}

#[allow(clippy::too_many_arguments)]
pub fn spawn_pty<F, X>(
    command: &str,
    args: &[String],
    cwd: &Path,
    env: &[(String, String)],
    cols: u16,
    rows: u16,
    on_output: F,
    on_exit: X,
) -> Result<PtyProcess>
where
    F: Fn(Vec<u8>) + Send + 'static,
    X: Fn(i32) + Send + 'static,
{
    let pty_system = native_pty_system();
    let pair = pty_system.openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })?;

    let mut cmd = CommandBuilder::new(command);
    cmd.args(args);
    cmd.cwd(cwd);
    // Finder-launched bundles inherit no TERM/COLORTERM; default them unless the
    // caller overrides.
    cmd.env("TERM", "xterm-256color");
    if !env.iter().any(|(k, _)| k == "COLORTERM") {
        cmd.env("COLORTERM", "truecolor");
    }
    for (k, v) in env {
        cmd.env(k, v);
    }

    let mut child = pair.slave.spawn_command(cmd)?;
    drop(pair.slave);

    let killer = child.clone_killer();
    let mut reader = pair.master.try_clone_reader()?;
    let writer = pair.master.take_writer()?;
    let status = Arc::new(Mutex::new(ProcStatus::Running));

    // Read and stitch on separate threads: the reader must go straight back to
    // the pty so the pieces of a torn write are already waiting, and only the
    // stitcher is allowed to wait on the clock.
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
    });
    std::thread::spawn(move || {
        stitch_reads(rx, STITCH_QUIET, STITCH_BURST, STITCH_MAX, on_output);
    });

    let status_for_wait = status.clone();
    std::thread::spawn(move || {
        let code = match child.wait() {
            Ok(es) => es.exit_code() as i32,
            Err(_) => {
                *status_for_wait.lock().unwrap() = ProcStatus::Crashed;
                on_exit(-1);
                return;
            }
        };
        *status_for_wait.lock().unwrap() = ProcStatus::Exited(code);
        on_exit(code);
    });

    Ok(PtyProcess {
        writer: Arc::new(Mutex::new(writer)),
        status,
        killer,
        master: Mutex::new(pair.master),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::sync::mpsc::Sender;
    use std::time::Duration;

    #[test]
    fn spawns_echo_and_reports_exit() {
        let (tx, rx) = mpsc::channel();
        let (etx, erx) = mpsc::channel();
        let p = spawn_pty(
            "/bin/echo",
            &["hi-there".to_string()],
            std::env::temp_dir().as_path(),
            &[],
            80,
            24,
            move |bytes| {
                let _ = tx.send(bytes);
            },
            move |code| {
                let _ = etx.send(code);
            },
        )
        .unwrap();

        let mut seen = String::new();
        while let Ok(chunk) = rx.recv_timeout(Duration::from_secs(2)) {
            seen.push_str(&String::from_utf8_lossy(&chunk));
            if seen.contains("hi-there") {
                break;
            }
        }
        assert!(seen.contains("hi-there"), "got: {seen:?}");
        assert_eq!(erx.recv_timeout(Duration::from_secs(2)).unwrap(), 0);
        let _ = p;
    }

    #[test]
    fn a_write_the_kernel_tears_up_reaches_the_caller_whole() {
        // The end of AGE-151, against the real pty rather than a channel: `dd`
        // makes one 4096-byte write, which macOS hands back as four reads of
        // 1024. The caller has to see one chunk.
        let (tx, rx) = mpsc::channel();
        let p = spawn_pty(
            "/bin/dd",
            &["if=/dev/zero".to_string(), "bs=4096".to_string(), "count=1".to_string()],
            std::env::temp_dir().as_path(),
            &[],
            80,
            24,
            move |bytes| {
                let _ = tx.send(bytes);
            },
            |_| {},
        )
        .unwrap();

        let first = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let zeros = first.iter().take_while(|b| **b == 0).count();
        assert_eq!(zeros, 4096, "chunk of {} byte(s) held {zeros} of the 4096", first.len());
        let _ = p;
    }

    /// Run `feed` on a sender thread and collect what the stitcher emits.
    fn stitched(
        quiet: Duration,
        burst: Duration,
        max: usize,
        feed: impl FnOnce(Sender<Vec<u8>>) + Send + 'static,
    ) -> Vec<Vec<u8>> {
        let (tx, rx) = mpsc::channel();
        let writer = std::thread::spawn(move || feed(tx));
        let mut out = Vec::new();
        stitch_reads(rx, quiet, burst, max, |chunk| out.push(chunk));
        writer.join().unwrap();
        out
    }

    #[test]
    fn joins_the_pieces_of_one_torn_write() {
        // What macOS does to a 1.3 kB repaint: 1024 bytes, then the rest.
        let out = stitched(Duration::from_millis(50), Duration::from_secs(5), 1 << 20, |tx| {
            tx.send(b"\x1b[2K\x1b[1A\x1b[2K\x1b[G".to_vec()).unwrap();
            tx.send(b"the redraw".to_vec()).unwrap();
        });
        assert_eq!(out, vec![b"\x1b[2K\x1b[1A\x1b[2K\x1b[Gthe redraw".to_vec()]);
    }

    #[test]
    fn keeps_writes_separated_by_a_quiet_gap_apart() {
        let out = stitched(Duration::from_millis(20), Duration::from_secs(5), 1 << 20, |tx| {
            tx.send(b"first".to_vec()).unwrap();
            std::thread::sleep(Duration::from_millis(200));
            tx.send(b"second".to_vec()).unwrap();
        });
        assert_eq!(out, vec![b"first".to_vec(), b"second".to_vec()]);
    }

    #[test]
    fn stops_accumulating_at_the_size_ceiling() {
        let out = stitched(Duration::from_millis(50), Duration::from_secs(5), 4, |tx| {
            for _ in 0..3 {
                tx.send(b"abc".to_vec()).unwrap();
            }
        });
        // The ceiling is checked between pieces, so a piece is never split: the
        // first chunk runs past 4 bytes and the third piece starts a new one.
        assert_eq!(out, vec![b"abcabc".to_vec(), b"abc".to_vec()]);
    }

    #[test]
    fn stops_accumulating_when_the_writer_never_goes_quiet() {
        // A stream with no gap in it must not be held back for the whole stream.
        let out = stitched(Duration::from_millis(50), Duration::from_millis(50), 1 << 20, |tx| {
            let until = Instant::now() + Duration::from_millis(400);
            while Instant::now() < until {
                if tx.send(b"x".to_vec()).is_err() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        });
        assert!(out.len() > 1, "burst ceiling never fired: {} chunk(s)", out.len());
        assert!(out.iter().all(|c| !c.is_empty()));
    }

    #[test]
    fn emits_what_it_has_when_the_reader_hangs_up() {
        let out = stitched(Duration::from_secs(5), Duration::from_secs(5), 1 << 20, |tx| {
            tx.send(b"tail".to_vec()).unwrap();
        });
        assert_eq!(out, vec![b"tail".to_vec()]);
    }
}
