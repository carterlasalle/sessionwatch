//! Reconstruction of previous connections from the wtmp database
//! (`/var/log/wtmp`) — the append-only log of every login and logout, the same
//! file `last(1)` reads.
//!
//! The record layout is identical to utmp (400 bytes, glibc and musl alike);
//! see `utmp.rs` for the field map. Records are matched by terminal line:
//! a `USER_PROCESS` opens a connection, the following `DEAD_PROCESS` on the
//! same line closes it. A `USER_PROCESS` with no matching logout (or still
//! open when the file ends) is a live connection.
#![allow(dead_code)] // used by the Linux collector; kept compiled on all hosts so tests run

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use crate::model::{Connection, SessionKind};

use super::incremental::IncrementalFile;

pub const WTMP_PATH: &str = "/var/log/wtmp";
const RECORD_LEN: usize = 400;
const BOOT_TIME: i16 = 2;
const USER_PROCESS: i16 = 7;
const DEAD_PROCESS: i16 = 8;
/// keep at most this many reconstructed connections.
pub const MAX_CONNECTIONS: usize = 500;

/// Parse a byte slice of wtmp records into connections (oldest first).
/// A connection whose `USER_PROCESS` record has no matching logout yet gets
/// `logout_unix: None`.
pub fn parse_wtmp(bytes: &[u8]) -> Vec<Connection> {
    let mut open: HashMap<String, Connection> = HashMap::new();
    let mut out: Vec<Connection> = Vec::new();

    for rec in bytes.chunks_exact(RECORD_LEN) {
        let ut_type = i16::from_le_bytes([rec[0], rec[1]]);
        let tv_sec = i64::from_le_bytes(rec[344..352].try_into().unwrap());
        let line = cstr(rec, 8, 32);
        match ut_type {
            USER_PROCESS => {
                let user = cstr(rec, 44, 32);
                if user.is_empty() || line.is_empty() {
                    continue;
                }
                let host = cstr(rec, 76, 256);
                let host = if host.is_empty() {
                    "localhost".to_string()
                } else {
                    host
                };
                let pid = u32::from_le_bytes(rec[4..8].try_into().unwrap());
                let kind = if host == "localhost" || host.starts_with(':') {
                    SessionKind::Local
                } else {
                    SessionKind::Ssh
                };
                // A new login on a line that was still open (missed logout,
                // e.g. wtmp was truncated): close the previous connection.
                if let Some(mut prev) = open.insert(
                    line.clone(),
                    Connection {
                        user,
                        line: line.clone(),
                        host,
                        identity: None,
                        name: None,
                        commands: Vec::new(),
                        shell_history: Vec::new(),
                        kind,
                        pid,
                        login_unix: tv_sec,
                        logout_unix: None,
                    },
                ) {
                    prev.logout_unix = Some(tv_sec);
                    out.push(prev);
                }
            }
            DEAD_PROCESS => {
                if let Some(mut c) = open.remove(&line) {
                    c.logout_unix = Some(tv_sec);
                    out.push(c);
                }
            }
            _ => {} // BOOT_TIME, RUN_LVL, OLD/NEW_TIME, ...
        }
    }

    out.extend(open.into_values());
    out
}

fn cstr(rec: &[u8], off: usize, len: usize) -> String {
    let field = &rec[off..off + len];
    let end = field.iter().position(|&b| b == 0).unwrap_or(field.len());
    String::from_utf8_lossy(&field[..end]).into_owned()
}

/// Incremental wtmp reader: re-parses only the appended tail plus a context
/// window each refresh, so a 1s refresh cycle costs a few KB of IO, not the
/// whole (potentially tens of MB) file.
pub struct WtmpReader {
    file: IncrementalFile,
    seen: HashSet<(u32, i64, String)>, // (pid, login, line) dedupe
    pub conns: Vec<Connection>,
}

impl WtmpReader {
    pub fn new() -> Self {
        WtmpReader {
            file: IncrementalFile::new(PathBuf::from(WTMP_PATH)),
            seen: HashSet::new(),
            conns: Vec::new(),
        }
    }

    /// Re-read new records and return the reconstructed connections.
    pub fn refresh(&mut self) -> Vec<Connection> {
        let bytes = self.file.tail();
        if bytes.is_empty() && !self.conns.is_empty() {
            // file vanished entirely
            if std::fs::metadata(WTMP_PATH).is_err() {
                self.conns.clear();
                self.seen.clear();
                self.file.reset();
                return Vec::new();
            }
        }
        for c in parse_wtmp(&bytes) {
            if self.seen.insert((c.pid, c.login_unix, c.line.clone())) {
                self.conns.push(c);
            }
        }
        // trim to the cap, dropping the oldest
        if self.conns.len() > MAX_CONNECTIONS {
            let drop = self.conns.len() - MAX_CONNECTIONS;
            self.conns.drain(0..drop);
            // stale dedupe keys beyond the cap can accumulate; prune once in a while
            if self.seen.len() > MAX_CONNECTIONS * 4 {
                self.seen.clear();
                self.seen.extend(
                    self.conns
                        .iter()
                        .map(|c| (c.pid, c.login_unix, c.line.clone())),
                );
            }
        }
        self.conns.clone()
    }
}

impl Default for WtmpReader {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(ut_type: i16, line: &str, user: &str, host: &str, secs: i64, pid: u32) -> Vec<u8> {
        let mut r = vec![0u8; RECORD_LEN];
        r[0..2].copy_from_slice(&ut_type.to_le_bytes());
        r[4..8].copy_from_slice(&pid.to_le_bytes());
        put(&mut r, 8, 32, line);
        put(&mut r, 44, 32, user);
        put(&mut r, 76, 256, host);
        r[344..352].copy_from_slice(&secs.to_le_bytes());
        r
    }

    fn put(r: &mut [u8], off: usize, len: usize, s: &str) {
        let n = s.len().min(len - 1);
        r[off..off + n].copy_from_slice(&s.as_bytes()[..n]);
    }

    #[test]
    fn reconstructs_ended_and_live_connections() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&rec(USER_PROCESS, "pts/1", "alice", "10.0.0.9", 1000, 101));
        bytes.extend_from_slice(&rec(DEAD_PROCESS, "pts/1", "", "", 2000, 101));
        bytes.extend_from_slice(&rec(USER_PROCESS, "pts/2", "bob", "", 1500, 202));
        bytes.extend_from_slice(&rec(BOOT_TIME, "~", "", "", 0, 0)); // ignored
        let conns = parse_wtmp(&bytes);
        assert_eq!(conns.len(), 2);
        let alice = conns.iter().find(|c| c.user == "alice").unwrap();
        assert_eq!(alice.line, "pts/1");
        assert_eq!(alice.host, "10.0.0.9");
        assert_eq!(alice.kind, SessionKind::Ssh);
        assert_eq!(alice.login_unix, 1000);
        assert_eq!(alice.logout_unix, Some(2000));
        assert!(!alice.is_live());
        let bob = conns.iter().find(|c| c.user == "bob").unwrap();
        assert_eq!(bob.host, "localhost"); // empty host => local
        assert_eq!(bob.logout_unix, None);
        assert!(bob.is_live());
    }

    #[test]
    fn closes_stale_open_line_on_reconnect() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&rec(USER_PROCESS, "pts/0", "alice", "h1", 100, 10));
        bytes.extend_from_slice(&rec(USER_PROCESS, "pts/0", "bob", "h2", 500, 11));
        bytes.extend_from_slice(&rec(DEAD_PROCESS, "pts/0", "", "", 900, 11));
        let conns = parse_wtmp(&bytes);
        assert_eq!(conns.len(), 2);
        let alice = conns.iter().find(|c| c.user == "alice").unwrap();
        assert_eq!(alice.logout_unix, Some(500)); // closed when bob took the line
        let bob = conns.iter().find(|c| c.user == "bob").unwrap();
        assert_eq!(bob.logout_unix, Some(900));
    }
}
