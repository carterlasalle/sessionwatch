#![allow(dead_code)] // used by the Linux collector; tests also run on non-Linux hosts

//! Terminal attribution helpers shared by the Linux collector.
//!
//! Linux exposes a process's controlling terminal in `/proc/<pid>/stat` as
//! an encoded device number, but the fd symlinks are the most direct source
//! of truth for ptys. Resolve by fd path first, then fall back to tty_nr
//! decoding so this works across kernels, libc implementations, containers,
//! and musl/glibc device-number differences.

use std::collections::HashMap;
use std::path::Path;

/// Decode the kernel's `new_encode_dev()` representation in `/proc/*/stat`.
pub fn decode_tty(tty_nr: i64) -> (u32, u32) {
    let t = tty_nr as u32;
    let major = (t >> 8) & 0xfff;
    let minor = (t & 0xff) | ((t >> 12) & 0xfff00);
    (major, minor)
}

/// Normalize `/proc/<pid>/fd/{0,1,2}` targets and utmp lines to the same key.
pub fn normalize_tty_path(path: &str) -> Option<String> {
    let path = path.strip_suffix(" (deleted)").unwrap_or(path);
    let line = path.strip_prefix("/dev/").unwrap_or(path).trim();
    if line.starts_with("pts/") || line.starts_with("tty") || line == "console" {
        Some(line.to_string())
    } else {
        None
    }
}

/// Choose a session by a direct fd tty path first, then decoded device number.
pub fn resolve_session_index(
    fd_tty: Option<&str>,
    decoded_dev: (u32, u32),
    by_line: &HashMap<String, usize>,
    by_dev: &HashMap<(u32, u32), usize>,
) -> Option<usize> {
    fd_tty
        .and_then(|line| by_line.get(line).copied())
        .or_else(|| by_dev.get(&decoded_dev).copied())
}

/// Read fd 0/1/2 and return the first path that is one of the known session
/// ttys. Shells often have stdin redirected while stdout/stderr still point
/// to their controlling pty, so checking all three matters.
pub fn read_process_tty(base: &Path, by_line: &HashMap<String, usize>) -> Option<String> {
    for fd in ["0", "1", "2"] {
        let Ok(target) = std::fs::read_link(base.join("fd").join(fd)) else {
            continue;
        };
        let Some(line) = normalize_tty_path(&target.to_string_lossy()) else {
            continue;
        };
        if by_line.contains_key(&line) {
            return Some(line);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_pts_device_number() {
        // new_encode_dev(MKDEV(136, 7))
        assert_eq!(decode_tty((136_i64 << 8) | 7), (136, 7));
        assert_eq!(normalize_tty_path("/dev/pts/7"), Some("pts/7".into()));
    }

    #[test]
    fn fd_path_wins_when_device_numbers_disagree() {
        let by_line = HashMap::from([(String::from("pts/7"), 3usize)]);
        let by_dev = HashMap::from([((999u32, 999u32), 8usize)]);
        assert_eq!(
            resolve_session_index(Some("pts/7"), (999, 999), &by_line, &by_dev),
            Some(3)
        );
    }

    #[test]
    fn decoded_device_is_fallback() {
        let by_line = HashMap::new();
        let by_dev = HashMap::from([((136u32, 7u32), 3usize)]);
        assert_eq!(
            resolve_session_index(None, (136, 7), &by_line, &by_dev),
            Some(3)
        );
    }

    #[test]
    fn ignores_non_tty_fd_targets() {
        assert_eq!(normalize_tty_path("/dev/null"), None);
        assert_eq!(normalize_tty_path("pipe:[1234]"), None);
        assert_eq!(
            normalize_tty_path("/dev/pts/7 (deleted)"),
            Some("pts/7".into())
        );
    }
}
