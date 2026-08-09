//! The Agency terminal daemon. Usage: `agency-termd <socket-path>`.
//! Self-daemonizes (detaches from the launching app) then serves the socket.
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

fn main() {
    let sock = match std::env::args().nth(1) {
        Some(s) => PathBuf::from(s),
        None => {
            eprintln!("usage: agency-termd <socket-path>");
            std::process::exit(2);
        }
    };

    // If a daemon already owns this socket, do nothing.
    if UnixStream::connect(&sock).is_ok() {
        return;
    }
    let _ = std::fs::remove_file(&sock);

    // Detach BEFORE spawning any threads (fork + threads do not mix).
    daemonize::Daemonize::new().working_directory(std::env::temp_dir()).start().expect("daemonize");

    if let Err(e) = agency_core::term::server::run(&sock) {
        eprintln!("agency-termd exited: {e}");
        std::process::exit(1);
    }
}
