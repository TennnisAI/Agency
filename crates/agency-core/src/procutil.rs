//! Small helpers around spawning child processes.

use std::io;

/// Retry `f` briefly when it fails with `ETXTBSY` ("text file busy").
///
/// On Linux, exec'ing a script this process wrote moments earlier can fail
/// with ETXTBSY when another thread forks inside the window where the writer's
/// file descriptor is still open: the forked child inherits that fd until its
/// own exec completes, and the kernel refuses to exec a file that anyone holds
/// open for writing. The only code that execs freshly written files is this
/// crate's own test suite (fabricated `gh`/`rg` stand-ins), which made
/// `cargo test` flaky on Linux. A real binary on PATH never trips this, and
/// macOS does not raise the condition here at all, so outside the tests the
/// retry stays dormant.
pub(crate) fn retry_etxtbsy<T>(mut f: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    let mut delay_ms = 5u64;
    loop {
        match f() {
            Err(e) if e.kind() == io::ErrorKind::ExecutableFileBusy && delay_ms <= 160 => {
                std::thread::sleep(std::time::Duration::from_millis(delay_ms));
                delay_ms *= 2;
            }
            other => return other,
        }
    }
}
