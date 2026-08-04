//! Direct parser for the Linux utmp database (`/var/run/utmp`).
//!
//! We parse the file ourselves instead of calling `getutxent()` because musl
//! ships those functions as stubs — a static musl build would otherwise see
//! zero sessions. The record layout is identical for glibc and musl on all
//! word sizes (the `ut_session`/`ut_tv` fields are size-stable by design), so
//! one parser works everywhere.
#![allow(dead_code)] // used by the Linux collector; kept compiled on all hosts so tests run

use crate::model::{Session, SessionKind};

pub const RECORD_LEN: usize = 400;
pub const USER_PROCESS: i16 = 7;
pub const UTMP_PATH: &str = "/var/run/utmp";

/// Record offsets (Linux LP64 / compat layout):
/// | 0    | i16   | ut_type                    |
/// | 4    | i32   | ut_pid                     |
/// | 8    | 32    | ut_line (e.g. "pts/3")     |
/// | 40   | 4     | ut_id                      |
/// | 44   | 32    | ut_user                    |
/// | 76   | 256   | ut_host                    |
/// | 332  | 4     | ut_exit                    |
/// | 336  | i64   | ut_session                 |
/// | 344  | i64   | ut_tv.tv_sec               |
/// | 352  | i64   | ut_tv.tv_usec              |
/// | 360  | 16    | ut_addr_v6                 |
/// | 376  | 20    | reserved                   |
pub fn parse_utmp(bytes: &[u8]) -> Vec<Session> {
    let mut out = Vec::new();
    for rec in bytes.chunks_exact(RECORD_LEN) {
        let ut_type = i16::from_le_bytes([rec[0], rec[1]]);
        if ut_type != USER_PROCESS {
            continue;
        }
        let pid = u32::from_le_bytes(rec[4..8].try_into().unwrap());
        let user = cstr(rec, 44, 32);
        if user.is_empty() {
            continue;
        }
        let line = cstr(rec, 8, 32);
        let host = cstr(rec, 76, 256);
        let host = if host.is_empty() { "localhost".to_string() } else { host };
        let tv_sec = i64::from_le_bytes(rec[344..352].try_into().unwrap());
        let device = if line.starts_with('/') {
            line.clone()
        } else {
            format!("/dev/{}", line)
        };
        out.push(Session {
            user,
            line,
            device,
            host,
            identity: None,
            name: None,
            kind: SessionKind::Local, // refined later from observed processes
            login_unix: tv_sec,
            pid,
        });
    }
    out
}

/// Read and parse the system utmp file (empty vec when absent/unreadable).
pub fn read_utmp_file() -> Vec<Session> {
    std::fs::read(UTMP_PATH).map(|b| parse_utmp(&b)).unwrap_or_default()
}

/// Read a NUL-terminated string field out of a record.
fn cstr(rec: &[u8], off: usize, len: usize) -> String {
    let field = &rec[off..off + len];
    let end = field.iter().position(|&b| b == 0).unwrap_or(field.len());
    String::from_utf8_lossy(&field[..end]).into_owned()
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
    fn parses_user_process_records() {
        let alice = rec(USER_PROCESS, "pts/3", "alice", "10.0.0.5", 1_700_000_000, 1234);
        let bob = rec(USER_PROCESS, "tty1", "bob", "", 1_700_000_100, 5678);
        let sessions = parse_utmp(&[alice, bob].concat());
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0].user, "alice");
        assert_eq!(sessions[0].line, "pts/3");
        assert_eq!(sessions[0].device, "/dev/pts/3");
        assert_eq!(sessions[0].host, "10.0.0.5");
        assert_eq!(sessions[0].login_unix, 1_700_000_000);
        assert_eq!(sessions[0].pid, 1234);
        // local session: empty host becomes "localhost"
        assert_eq!(sessions[1].host, "localhost");
        assert_eq!(sessions[1].device, "/dev/tty1");
    }

    #[test]
    fn skips_non_user_records_and_garbage() {
        let boot = rec(2, "", "", "", 0, 0); // BOOT_TIME
        let dead = rec(8, "pts/3", "", "", 0, 0); // DEAD_PROCESS
        let user = rec(USER_PROCESS, "pts/0", "carol", "vpn-gw", 100, 99);
        // trailing garbage shorter than a record must be ignored
        let mut bytes = [boot, dead, user].concat();
        bytes.extend_from_slice(&[1, 2, 3]);
        let sessions = parse_utmp(&bytes);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].user, "carol");
    }

    #[test]
    fn truncates_at_nul_and_handles_empty_user() {
        // user "alice" then NUL then junk: must stop at the NUL.
        let mut r = rec(USER_PROCESS, "pts/9", "alice\0junk", "h", 5, 1);
        // also a record with empty user must be skipped
        let empty = rec(USER_PROCESS, "pts/8", "", "h", 5, 2);
        let sessions = parse_utmp(&[r.clone(), empty].concat());
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].user, "alice");
        let _ = &mut r;
    }
}
