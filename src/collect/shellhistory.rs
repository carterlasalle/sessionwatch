//! Retroactive command history from users' shell history files.
//!
//! For connections that predate sessionwatch (or that it never observed
//! live — e.g. Tailscale sessions that came and went), the only record of
//! what the person actually ran is their shell history. This module parses
//! the common formats:
//!   - bash:     plain lines, or `#<unix>` timestamp lines when HISTTIMEFORMAT
//!   - zsh:      `: <unix>:0;<command>` (extended history)
//!   - fish:     `- cmd: <command>` / `  when: <unix>` pairs
//!
//! This is a box-owner's tool: histories are read only for users that show
//! up in the connection history, and only while running as root (or a user
//! with read access to those homes).
#![allow(dead_code)] // used by the Linux collector; kept compiled on all hosts so tests run

use std::fs;
use std::path::{Path, PathBuf};

/// Cap on untimed entries attached per connection.
const MAX_UNTIMED: usize = 150;
/// Cap on total entries attached per connection.
const MAX_TOTAL: usize = 500;

/// Parse a bash history file. Timestamped (`#<unix>`) lines and plain lines
/// can be mixed; a `#` line that isn't a timestamp is kept as a command.
pub fn parse_bash_history(text: &str) -> Vec<(Option<i64>, String)> {
    let mut out = Vec::new();
    let mut pending_ts: Option<i64> = None;
    for line in text.lines() {
        if let Some(ts) = line.trim().strip_prefix('#') {
            if let Ok(t) = ts.trim().parse::<i64>() {
                pending_ts = Some(t);
                continue;
            }
        }
        let cmd = line.trim();
        if cmd.is_empty() {
            continue;
        }
        out.push((pending_ts.take(), cmd.to_string()));
    }
    out
}

/// Parse a zsh extended-history file: `: <unix>:<dur>;<command>`.
pub fn parse_zsh_history(text: &str) -> Vec<(Option<i64>, String)> {
    let mut out = Vec::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix(": ") {
            if let Some(semi) = rest.find(":0;") {
                if let Ok(ts) = rest[..semi].trim().parse::<i64>() {
                    out.push((Some(ts), rest[semi + 3..].to_string()));
                    continue;
                }
            }
        }
        let cmd = line.trim();
        if !cmd.is_empty() {
            out.push((None, cmd.to_string()));
        }
    }
    out
}

/// Parse a fish history file: `- cmd: <command>` followed by `  when: <unix>`.
pub fn parse_fish_history(text: &str) -> Vec<(Option<i64>, String)> {
    let mut out = Vec::new();
    let mut pending_cmd: Option<String> = None;
    for line in text.lines() {
        let t = line.trim();
        if let Some(c) = t.strip_prefix("- cmd:") {
            if let Some(cmd) = pending_cmd.take() {
                out.push((None, cmd));
            }
            pending_cmd = Some(c.trim().to_string());
        } else if let Some(w) = t.strip_prefix("when:") {
            let ts = w.trim().parse::<i64>().ok();
            if let Some(cmd) = pending_cmd.take() {
                out.push((ts, cmd));
            }
        }
    }
    if let Some(cmd) = pending_cmd {
        out.push((None, cmd));
    }
    out
}

/// Read and parse every history file we know about for `home`. Dedupes exact
/// duplicate commands across files (keeps the first occurrence).
pub fn read_shell_histories(home: &Path) -> Vec<(Option<i64>, String)> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    let candidates = [
        (
            home.join(".bash_history"),
            parse_bash_history as fn(&str) -> Vec<(Option<i64>, String)>,
        ),
        (home.join(".zsh_history"), parse_zsh_history),
        (home.join(".fish_history"), parse_fish_history),
    ];
    for (path, parser) in candidates {
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        if text.len() > 2 * 1024 * 1024 {
            continue; // don't slurp absurd files
        }
        for (ts, cmd) in parser(&text) {
            if seen.insert(cmd.clone()) {
                out.push((ts, cmd));
            }
        }
    }
    out
}

/// Attach history entries to one connection: timed entries are windowed to
/// the connection's lifetime (with a little slack); untimed entries are
/// attached in order but capped.
pub fn filter_for_connection(
    history: &[(Option<i64>, String)],
    login_unix: i64,
    logout_unix: Option<i64>,
    now_unix: i64,
) -> Vec<(Option<i64>, String)> {
    let end = logout_unix.unwrap_or(now_unix);
    let mut out: Vec<(Option<i64>, String)> = Vec::new();
    let mut untimed: Vec<(Option<i64>, String)> = Vec::new();
    for e in history {
        match e.0 {
            Some(ts) => {
                if ts >= login_unix - 60 && ts <= end + 60 {
                    out.push(e.clone());
                }
            }
            None => untimed.push(e.clone()),
        }
    }
    let drop = untimed.len().saturating_sub(MAX_UNTIMED);
    out.extend(untimed.into_iter().skip(drop));
    if out.len() > MAX_TOTAL {
        let drop = out.len() - MAX_TOTAL;
        out.drain(0..drop);
    }
    out
}

/// `Some(path)` for the given user's home dir via getpwnam_r (Linux only).
#[cfg(target_os = "linux")]
pub fn user_home(user: &str) -> Option<PathBuf> {
    let mut pwd = unsafe { std::mem::zeroed::<libc::passwd>() };
    let mut buf = vec![0u8; 2048];
    let mut result: *mut libc::passwd = std::ptr::null_mut();
    let c_user = std::ffi::CString::new(user).ok()?;
    let r = unsafe {
        libc::getpwnam_r(
            c_user.as_ptr(),
            &mut pwd,
            buf.as_mut_ptr() as *mut _,
            buf.len(),
            &mut result,
        )
    };
    if r != 0 || result.is_null() || pwd.pw_dir.is_null() {
        return None;
    }
    let mut chars = Vec::new();
    unsafe {
        let mut i = 0;
        loop {
            let c = *pwd.pw_dir.add(i);
            if c == 0 {
                break;
            }
            chars.push(c as u8);
            i += 1;
        }
    }
    let dir = String::from_utf8_lossy(&chars).into_owned();
    if dir.is_empty() {
        None
    } else {
        Some(PathBuf::from(dir))
    }
}

#[cfg(not(target_os = "linux"))]
pub fn user_home(_user: &str) -> Option<PathBuf> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bash_plain_and_timestamped() {
        let timed = "#1700000000\nls -la\n#1700000010\ngit status\n";
        let parsed = parse_bash_history(timed);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0], (Some(1700000000), "ls -la".to_string()));
        assert_eq!(parsed[1], (Some(1700000010), "git status".to_string()));

        let plain = "ls -la\ngit status\n";
        let parsed = parse_bash_history(plain);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0], (None, "ls -la".to_string()));

        // a # comment line is a command, not a timestamp
        let mixed = "# not a timestamp\nls\n";
        let parsed = parse_bash_history(mixed);
        assert_eq!(parsed[0], (None, "# not a timestamp".to_string()));
        assert_eq!(parsed[1], (None, "ls".to_string()));
    }

    #[test]
    fn zsh_extended_format() {
        let text = ": 1700000000:0;echo hi\n: 1700000010:0;vim x\n";
        let parsed = parse_zsh_history(text);
        assert_eq!(parsed[0], (Some(1700000000), "echo hi".to_string()));
        assert_eq!(parsed[1], (Some(1700000010), "vim x".to_string()));
    }

    #[test]
    fn fish_pairs() {
        let text = "- cmd: ls\n  when: 1700000000\n- cmd: echo hi\n  when: 1700000010\n";
        let parsed = parse_fish_history(text);
        assert_eq!(parsed[0], (Some(1700000000), "ls".to_string()));
        assert_eq!(parsed[1], (Some(1700000010), "echo hi".to_string()));
    }

    #[test]
    fn filters_by_connection_window() {
        let history = vec![
            (Some(1000), "before".to_string()),
            (Some(1500), "inside".to_string()),
            (Some(3000), "after".to_string()),
            (None, "untimed-1".to_string()),
            (None, "untimed-2".to_string()),
        ];
        let filtered = filter_for_connection(&history, 1400, Some(1600), 9999);
        let cmds: Vec<&str> = filtered.iter().map(|(_, c)| c.as_str()).collect();
        assert!(cmds.contains(&"inside"));
        assert!(!cmds.contains(&"before"));
        assert!(!cmds.contains(&"after"));
        assert!(cmds.contains(&"untimed-2"));
        assert!(cmds.contains(&"untimed-1"));
    }
}
