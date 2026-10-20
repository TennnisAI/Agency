//! Which local process is on the other end of a loopback TCP connection, and
//! whose descendant it is (AGE-209).
//!
//! The run tools have to know the caller is a run's own agent. A secret cannot
//! say so on its own: it has to be handed to the agent somewhere, and anywhere
//! the agent can read it, the user's other processes can too, a dispatched
//! child among them. What a process cannot choose is its parent. Every
//! process a run's agent starts descends from the process the terminal daemon
//! spawned for that run's session, and nothing else does, so "descends from
//! one of this run's session processes" is an identity no file can be copied
//! to forge.
//!
//! The parsers are pure and tested; the readers that feed them are thin.
//! Every reader answers `None` when it cannot tell, and every caller treats
//! `None` as "not the run's agent". On macOS the readers run the system's own
//! `lsof` and `ps` by absolute path, so a `PATH` cannot substitute them; on
//! Linux they read `/proc`. Anywhere else nothing is read and nobody passes.

use std::collections::HashMap;

/// The pid of the process holding the client end of the loopback connection
/// from `peer_port` to `server_port`, both on 127.0.0.1. `None` if no single
/// process can be named.
pub fn tcp_peer_pid(server_port: u16, peer_port: u16) -> Option<u32> {
    platform::tcp_peer_pid(server_port, peer_port)
}

/// Every process's parent, as `pid -> ppid`. `None` if the table cannot be
/// read.
pub fn process_parents() -> Option<HashMap<u32, u32>> {
    platform::process_parents()
}

/// Whether `pid` is one of `roots` or descends from one of them, by `parents`.
/// A process whose parent is not in the table, or a chain that loops, ends
/// the walk with no.
pub fn descends_from(pid: u32, roots: &[u32], parents: &HashMap<u32, u32>) -> bool {
    let mut at = pid;
    // A real chain is a few dozen deep at most; the bound only guards a table
    // read mid-change that happens to describe a cycle.
    for _ in 0..256 {
        if roots.contains(&at) {
            return true;
        }
        match parents.get(&at) {
            Some(&up) if up != at && up > 1 => at = up,
            _ => return false,
        }
    }
    false
}

/// `ps -axo pid=,ppid=` output, as `pid -> ppid`. Lines that are not two
/// numbers are skipped.
pub fn parse_ps_parents(text: &str) -> HashMap<u32, u32> {
    text.lines()
        .filter_map(|line| {
            let mut cols = line.split_whitespace();
            let pid = cols.next()?.parse().ok()?;
            let ppid = cols.next()?.parse().ok()?;
            cols.next().is_none().then_some((pid, ppid))
        })
        .collect()
}

/// The pids in `lsof -F pn` output holding a socket named exactly
/// `127.0.0.1:<peer_port>->127.0.0.1:<server_port>`: the client's end of the
/// connection, and not the server's own, whose name runs the other way.
pub fn parse_lsof_client_pids(text: &str, server_port: u16, peer_port: u16) -> Vec<u32> {
    let want = format!("n127.0.0.1:{peer_port}->127.0.0.1:{server_port}");
    let mut pid = None;
    let mut out = Vec::new();
    for line in text.lines() {
        if let Some(p) = line.strip_prefix('p') {
            pid = p.parse::<u32>().ok();
        } else if line == want {
            if let Some(p) = pid.filter(|p| !out.contains(p)) {
                out.push(p);
            }
        }
    }
    out
}

/// The socket inode of the connection from 127.0.0.1:`peer_port` to
/// 127.0.0.1:`server_port` in `/proc/net/tcp` text.
pub fn parse_proc_net_tcp_inode(text: &str, server_port: u16, peer_port: u16) -> Option<u64> {
    // /proc/net/tcp writes an IPv4 address as the host-order u32 in hex, which
    // for 127.0.0.1 on every little-endian machine is 0100007F, and the port
    // as plain big hex.
    let local = format!("0100007F:{peer_port:04X}");
    let remote = format!("0100007F:{server_port:04X}");
    text.lines().skip(1).find_map(|line| {
        let cols: Vec<&str> = line.split_whitespace().collect();
        (cols.len() > 9 && cols[1] == local && cols[2] == remote)
            .then(|| cols[9].parse().ok())
            .flatten()
    })
}

/// The ppid in a `/proc/<pid>/stat` line. The command name is in parentheses
/// and may itself hold spaces and parentheses, so fields count from the last
/// `)`.
pub fn parse_proc_stat_ppid(stat: &str) -> Option<u32> {
    let after = &stat[stat.rfind(')')? + 1..];
    after.split_whitespace().nth(1)?.parse().ok()
}

#[cfg(target_os = "macos")]
mod platform {
    use std::collections::HashMap;
    use std::process::Command;

    fn run(program: &str, args: &[&str]) -> Option<String> {
        let out = Command::new(program).args(args).output().ok()?;
        out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    }

    pub fn tcp_peer_pid(server_port: u16, peer_port: u16) -> Option<u32> {
        let spec = format!("-iTCP@127.0.0.1:{peer_port}");
        let text = run("/usr/sbin/lsof", &["-nP", "-a", &spec, "-sTCP:ESTABLISHED", "-Fpn"])?;
        match super::parse_lsof_client_pids(&text, server_port, peer_port)[..] {
            [pid] => Some(pid),
            _ => None,
        }
    }

    pub fn process_parents() -> Option<HashMap<u32, u32>> {
        let parents = super::parse_ps_parents(&run("/bin/ps", &["-axo", "pid=,ppid="])?);
        (!parents.is_empty()).then_some(parents)
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use std::collections::HashMap;

    pub fn tcp_peer_pid(server_port: u16, peer_port: u16) -> Option<u32> {
        let table = std::fs::read_to_string("/proc/net/tcp").ok()?;
        let inode = super::parse_proc_net_tcp_inode(&table, server_port, peer_port)?;
        let link = format!("socket:[{inode}]");
        let mut found = Vec::new();
        for entry in std::fs::read_dir("/proc").ok()?.flatten() {
            let Some(pid) = entry.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else {
                continue;
            };
            let Ok(fds) = std::fs::read_dir(entry.path().join("fd")) else { continue };
            let holds = fds.flatten().any(|fd| {
                std::fs::read_link(fd.path()).is_ok_and(|t| t.to_str() == Some(link.as_str()))
            });
            if holds {
                found.push(pid);
            }
        }
        match found[..] {
            [pid] => Some(pid),
            _ => None,
        }
    }

    pub fn process_parents() -> Option<HashMap<u32, u32>> {
        let mut parents = HashMap::new();
        for entry in std::fs::read_dir("/proc").ok()?.flatten() {
            let Some(pid) = entry.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else {
                continue;
            };
            let stat = std::fs::read_to_string(entry.path().join("stat")).unwrap_or_default();
            if let Some(ppid) = super::parse_proc_stat_ppid(&stat) {
                parents.insert(pid, ppid);
            }
        }
        (!parents.is_empty()).then_some(parents)
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
mod platform {
    use std::collections::HashMap;

    pub fn tcp_peer_pid(_: u16, _: u16) -> Option<u32> {
        None
    }

    pub fn process_parents() -> Option<HashMap<u32, u32>> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(pairs: &[(u32, u32)]) -> HashMap<u32, u32> {
        pairs.iter().copied().collect()
    }

    #[test]
    fn a_descendant_of_a_root_passes_and_a_sibling_tree_does_not() {
        // termd 100 spawned the parent's session 200 and the child's 300.
        let t = table(&[(100, 1), (200, 100), (201, 200), (202, 201), (300, 100), (301, 300)]);
        assert!(descends_from(202, &[200], &t), "the parent's agent's own process");
        assert!(descends_from(200, &[200], &t), "the root itself");
        assert!(!descends_from(301, &[200], &t), "the child's process, under the same daemon");
        assert!(!descends_from(100, &[200], &t), "the daemon is above the root, not under it");
        assert!(!descends_from(999, &[200], &t), "a process the table does not know");
    }

    #[test]
    fn a_loop_or_an_orphan_ends_the_walk() {
        assert!(!descends_from(5, &[200], &table(&[(5, 6), (6, 5)])));
        // A process that double-forked out from under the session is launchd's.
        assert!(!descends_from(5, &[200], &table(&[(5, 1)])));
        assert!(!descends_from(5, &[200], &table(&[(5, 5)])));
    }

    #[test]
    fn ps_output_parses_and_skips_noise() {
        let t = parse_ps_parents("    1     0\n  412     1\nPID PPID\n  9  412 extra\n");
        assert_eq!(t, table(&[(1, 0), (412, 1)]));
    }

    #[test]
    fn lsof_names_only_the_client_end() {
        // The app's own server socket names the pair the other way round.
        let text = "p700\nf12\nn127.0.0.1:5249->127.0.0.1:53001\n\
                    p900\nf33\nn127.0.0.1:53001->127.0.0.1:5249\nf34\nn*:8080\n";
        assert_eq!(parse_lsof_client_pids(text, 5249, 53001), vec![900]);
        assert!(parse_lsof_client_pids(text, 5249, 53002).is_empty());
        // Two processes on one socket (a fork that kept it) name nobody.
        let shared =
            "p900\nn127.0.0.1:53001->127.0.0.1:5249\np901\nn127.0.0.1:53001->127.0.0.1:5249\n";
        assert_eq!(parse_lsof_client_pids(shared, 5249, 53001), vec![900, 901]);
    }

    #[test]
    fn proc_net_tcp_finds_the_client_sockets_inode() {
        let text = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n\
           0: 0100007F:1481 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 11111 1\n\
           1: 0100007F:CF09 0100007F:1481 01 00000000:00000000 00:00000000 00000000  1000        0 22222 1\n\
           2: 0100007F:1481 0100007F:CF09 01 00000000:00000000 00:00000000 00000000  1000        0 33333 1\n";
        // 0x1481 = 5249, 0xCF09 = 53001.
        assert_eq!(parse_proc_net_tcp_inode(text, 5249, 53001), Some(22222));
        assert_eq!(parse_proc_net_tcp_inode(text, 5249, 53002), None);
    }

    #[test]
    fn proc_stat_ppid_survives_a_command_name_with_parentheses() {
        assert_eq!(parse_proc_stat_ppid("42 (node) S 41 42 42 0"), Some(41));
        assert_eq!(parse_proc_stat_ppid("43 (a) b) (c) R 7 43"), Some(7));
        assert_eq!(parse_proc_stat_ppid("garbage"), None);
    }

    /// The readers against this very process: the test binary is connected
    /// to a listener it owns, so the peer of that connection is itself, and it
    /// descends from itself.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn the_readers_name_this_process_as_its_own_peer() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let server_port = listener.local_addr().unwrap().port();
        let client = std::net::TcpStream::connect(("127.0.0.1", server_port)).unwrap();
        let (_server_end, _) = listener.accept().unwrap();
        let peer_port = client.local_addr().unwrap().port();
        let me = std::process::id();
        // Both ends are this process, which the client-end rule still names
        // once: the server end's socket is the other way round.
        assert_eq!(tcp_peer_pid(server_port, peer_port), Some(me));
        let parents = process_parents().unwrap();
        assert!(descends_from(me, &[me], &parents));
        let up = parents[&me];
        assert!(descends_from(me, &[up], &parents), "and from the process that started it");
    }
}
