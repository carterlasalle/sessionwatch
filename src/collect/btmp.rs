//! Failed login attempts from the btmp database (`/var/log/btmp`), the file
//! `lastb(1)` reads — the other half of the real history: who has been trying
//! to reach the box (including brute-force scanners), not just who got in.
#![allow(dead_code)] // used by the Linux collector; kept compiled on all hosts so tests run

use std::collections::HashSet;
use std::path::PathBuf;

use crate::model::FailedLogin;

use super::incremental::IncrementalFile;

pub const BTMP_PATH: &str = "/var/log/btmp";
const RECORD_LEN: usize = 400;
const USER_PROCESS: i16 = 7;
/// keep at most this many failed attempts.
pub const MAX_FAILED: usize = 100;

/// Parse btmp records into failed logins (oldest first). Failed attempts are
/// written as `USER_PROCESS` records with a (possibly attempted) username.
pub fn parse_btmp(bytes: &[u8]) -> Vec<FailedLogin> {
    let mut out = Vec::new();
    for rec in bytes.chunks_exact(RECORD_LEN) {
        let ut_type = i16::from_le_bytes([rec[0], rec[1]]);
        if ut_type != USER_PROCESS {
            continue;
        }
        let user = cstr(rec, 44, 32);
        if user.is_empty() {
            continue;
        }
        let host = cstr(rec, 76, 256);
        let host = if host.is_empty() { "localhost".to_string() } else { host };
        let line = cstr(rec, 8, 32);
        let at = i64::from_le_bytes(rec[344..352].try_into().unwrap());
        out.push(FailedLogin {
            user,
            host,
            line,
            at,
        });
    }
    out
}

fn cstr(rec: &[u8], off: usize, len: usize) -> String {
    let field = &rec[off..off + len];
    let end = field.iter().position(|&b| b == 0).unwrap_or(field.len());
    String::from_utf8_lossy(&field[..end]).into_owned()
}

/// Incremental btmp reader (same tailing strategy as wtmp).
pub struct BtmpReader {
    file: IncrementalFile,
    seen: HashSet<(String, String, i64)>, // (user, host, at) dedupe
    pub failed: Vec<FailedLogin>,
}

impl BtmpReader {
    pub fn new() -> Self {
        BtmpReader {
            file: IncrementalFile::new(PathBuf::from(BTMP_PATH)),
            seen: HashSet::new(),
            failed: Vec::new(),
        }
    }

    pub fn refresh(&mut self) -> Vec<FailedLogin> {
        for f in parse_btmp(&self.file.tail()) {
            if self.seen.insert((f.user.clone(), f.host.clone(), f.at)) {
                self.failed.push(f);
            }
        }
        if self.failed.len() > MAX_FAILED {
            let drop = self.failed.len() - MAX_FAILED;
            self.failed.drain(0..drop);
        }
        self.failed.clone()
    }
}

impl Default for BtmpReader {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(line: &str, user: &str, host: &str, secs: i64) -> Vec<u8> {
        let mut r = vec![0u8; RECORD_LEN];
        r[0..2].copy_from_slice(&USER_PROCESS.to_le_bytes());
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
    fn parses_failed_attempts() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&rec("ssh:notty", "root", "203.0.113.7", 1000));
        bytes.extend_from_slice(&rec("ssh:notty", "admin", "100.64.0.9", 2000));
        bytes.extend_from_slice(&rec("tty1", "", "", 3000)); // no user: ignored
        let fails = parse_btmp(&bytes);
        assert_eq!(fails.len(), 2);
        assert_eq!(fails[0].user, "root");
        assert_eq!(fails[0].host, "203.0.113.7");
        assert_eq!(fails[0].at, 1000);
        assert_eq!(fails[1].host, "100.64.0.9");
    }
}
