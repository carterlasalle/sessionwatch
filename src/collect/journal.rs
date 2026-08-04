//! sessionwatch's own connection journal.
//!
//! wtmp gives login/logout pairs but no session context: tmux/zellij/screen
//! session names are only visible in live process lists, and once a session
//! is gone they're lost forever. So sessionwatch keeps an append-only journal
//! of every session it observes live (open + close records), which makes the
//! history view richer over time: real session names, resolved hosts, and
//! connections even on systems where wtmp is absent or rotated away.
//!
//! Format (one record per line, space-separated, `name` is the last field and
//! may contain spaces):
//!   `O <login_unix> <user> <host> <line> <pid> <name?>`
//!   `C <observed_unix> <line> <pid>`
#![allow(dead_code)] // used by the Linux collector; kept compiled on all hosts so tests run

use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

use crate::model::{Connection, Session};

/// Cap on connections loaded from the journal.
const MAX_LOAD: usize = 500;

fn default_path() -> PathBuf {
    // root -> system log; otherwise under the user's state dir
    let root = unsafe { libc::geteuid() == 0 };
    if root {
        PathBuf::from("/var/log/sessionwatch/history.log")
    } else {
        let base = std::env::var("XDG_STATE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                std::env::var("HOME")
                    .map(|h| PathBuf::from(h).join(".local/state"))
                    .unwrap_or_else(|_| PathBuf::from("."))
            });
        base.join("sessionwatch/history.log")
    }
}

pub struct Journal {
    path: PathBuf,
    /// (pid, line) -> open record seen in this process
    open: HashMap<(u32, String), Connection>,
    /// connections closed during this run (to merge with wtmp)
    closed: Vec<Connection>,
    loaded: Vec<Connection>,
}

impl Journal {
    pub fn new() -> Self {
        let mut j = Journal {
            path: default_path(),
            open: HashMap::new(),
            closed: Vec::new(),
            loaded: Vec::new(),
        };
        j.load();
        j
    }

    pub fn with_path(path: PathBuf) -> Self {
        let mut j = Journal {
            path,
            open: HashMap::new(),
            closed: Vec::new(),
            loaded: Vec::new(),
        };
        j.load();
        j
    }

    fn load(&mut self) {
        self.loaded = match fs::read_to_string(&self.path) {
            Ok(text) => {
                let mut conns = Vec::new();
                let mut open: HashMap<(u32, String), Connection> = HashMap::new();
                for line in text.lines() {
                    let mut toks = line.split_whitespace();
                    match toks.next() {
                        Some("O") => {
                            let Some(login) = toks.next().and_then(|t| t.parse::<i64>().ok()) else { continue };
                            let Some(user) = toks.next() else { continue };
                            let Some(host) = toks.next() else { continue };
                            let Some(line_s) = toks.next() else { continue };
                            let Some(pid) = toks.next().and_then(|t| t.parse::<u32>().ok()) else { continue };
                            let name: String = toks.collect::<Vec<_>>().join(" ");
                            let name = if name.is_empty() { None } else { Some(name) };
                            open.insert(
                                (pid, line_s.to_string()),
                                Connection {
                                    user: user.into(),
                                    line: line_s.into(),
                                    host: host.into(),
                                    name,
                                    kind: crate::model::SessionKind::Ssh, // refined at display/merge time
                                    pid,
                                    login_unix: login,
                                    logout_unix: None,
                                },
                            );
                        }
                        Some("C") => {
                            let Some(t) = toks.next().and_then(|t| t.parse::<i64>().ok()) else { continue };
                            let Some(line_s) = toks.next() else { continue };
                            let Some(pid) = toks.next().and_then(|t| t.parse::<u32>().ok()) else { continue };
                            if let Some(mut c) = open.remove(&(pid, line_s.to_string())) {
                                c.logout_unix = Some(t);
                                conns.push(c);
                            }
                        }
                        _ => {}
                    }
                }
                conns.extend(open.into_values());
                conns
            }
            Err(_) => Vec::new(),
        };
        if self.loaded.len() > MAX_LOAD {
            let drop = self.loaded.len() - MAX_LOAD;
            self.loaded.drain(0..drop);
        }
    }

    /// Diff the live session set and append open/close records for what
    /// changed. Call once per refresh with the previous and current sessions.
    pub fn observe(&mut self, prev: &[Session], now: &[Session]) {
        for s in now {
            if !prev.iter().any(|o| o.line == s.line && o.pid == s.pid) {
                // new session: record its open (name from the live session)
                let rec = format!(
                    "O {} {} {} {} {} {}\n",
                    s.login_unix,
                    sanitize(&s.user),
                    sanitize(&s.host),
                    sanitize(&s.line),
                    s.pid,
                    s.name.as_deref().map(sanitize).unwrap_or_default()
                );
                self.append(&rec);
                self.open.insert(
                    (s.pid, s.line.clone()),
                    Connection {
                        user: s.user.clone(),
                        line: s.line.clone(),
                        host: s.host.clone(),
                        name: s.name.clone(),
                        kind: s.kind,
                        pid: s.pid,
                        login_unix: s.login_unix,
                        logout_unix: None,
                    },
                );
            }
        }
        for s in prev {
            if !now.iter().any(|o| o.line == s.line && o.pid == s.pid) {
                let rec = format!("C {} {} {}\n", unix_now(), sanitize(&s.line), s.pid);
                self.append(&rec);
                if let Some(mut c) = self.open.remove(&(s.pid, s.line.clone())) {
                    c.logout_unix = Some(unix_now());
                    self.closed.push(c);
                }
            }
        }
    }

    fn append(&self, rec: &str) {
        if let Some(dir) = self.path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&self.path) {
            let _ = f.write_all(rec.as_bytes());
        }
    }

    /// Connections from this run (open + closed) for merging with wtmp.
    pub fn observed(&self) -> Vec<Connection> {
        let mut out = self.closed.clone();
        out.extend(self.open.values().cloned());
        out
    }

    pub fn loaded(&self) -> Vec<Connection> {
        self.loaded.clone()
    }
}

impl Default for Journal {
    fn default() -> Self {
        Self::new()
    }
}

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_whitespace() || c == '\n' || c == '\r' { '_' } else { c })
        .collect()
}

/// Merge wtmp history with the journal: journal entries win for `name` and
/// for hosts it observed live (which may be Tailscale-resolved); wtmp's exact
/// logout timestamps win. Journal-only connections (e.g. wtmp rotated/absent)
/// are appended.
pub fn merge_connections(wtmp: Vec<Connection>, journal: Vec<Connection>) -> Vec<Connection> {
    use super::tailscale::is_cgnat;
    let mut keyed: HashMap<(u32, String, i64), Connection> = HashMap::new();
    for c in wtmp {
        keyed.insert((c.pid, c.line.clone(), c.login_unix), c);
    }
    let mut out: Vec<Connection> = Vec::new();
    for j in journal {
        match keyed.get_mut(&(j.pid, j.line.clone(), j.login_unix)) {
            Some(w) => {
                if j.name.is_some() {
                    w.name = j.name;
                }
                // prefer the journal's observed host when wtmp only has a raw
                // CGNAT address (unresolved tailscale) or a bare localhost.
                if j.host != "localhost" && (w.host == "localhost" || is_cgnat(&w.host)) {
                    w.host = j.host.clone();
                }
            }
            None => {
                out.push(j);
            }
        }
    }
    out.extend(keyed.into_values());
    out
}

fn unix_now() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn now() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }

    fn sess(user: &str, host: &str, line: &str, pid: u32, login: i64, name: Option<&str>) -> Session {
        Session {
            user: user.into(),
            line: line.into(),
            device: format!("/dev/{line}"),
            host: host.into(),
            name: name.map(str::to_string),
            kind: crate::model::SessionKind::Ssh,
            login_unix: login,
            pid,
        }
    }

    #[test]
    fn roundtrip_observe_and_load() {
        let dir = std::env::temp_dir().join(format!("sw-journal-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join("history.log");
        let login = now() - 100;
        let mut j = Journal::with_path(path.clone());

        // open a session with a tmux name
        let s = sess("dev", "localhost", "pts/3", 777, login, Some("cursor-env"));
        j.observe(&[], &[s.clone()]);
        // close it
        j.observe(&[s.clone()], &[]);

        // a second Journal instance reading the same file reconstructs it
        let j2 = Journal::with_path(path);
        let conns = j2.loaded();
        assert_eq!(conns.len(), 1);
        assert_eq!(conns[0].user, "dev");
        assert_eq!(conns[0].line, "pts/3");
        assert_eq!(conns[0].pid, 777);
        assert_eq!(conns[0].login_unix, login);
        assert_eq!(conns[0].name.as_deref(), Some("cursor-env"));
        assert!(conns[0].logout_unix.is_some());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn merge_prefers_journal_enrichment() {
        let login = 1000;
        let wtmp = vec![Connection {
            user: "dev".into(),
            line: "pts/3".into(),
            host: "100.64.0.5".into(),
            name: None,
            kind: crate::model::SessionKind::Ssh,
            pid: 777,
            login_unix: login,
            logout_unix: Some(2000),
        }];
        let journal = vec![Connection {
            user: "dev".into(),
            line: "pts/3".into(),
            host: "jackphelps-mbp".into(),
            name: Some("cursor-env".into()),
            kind: crate::model::SessionKind::Ssh,
            pid: 777,
            login_unix: login,
            logout_unix: None, // wtmp's exact logout should win
        }];
        let merged = merge_connections(wtmp, journal);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].name.as_deref(), Some("cursor-env"));
        assert_eq!(merged[0].host, "jackphelps-mbp");
        assert_eq!(merged[0].logout_unix, Some(2000));
    }

    #[test]
    fn merge_keeps_journal_only_connections() {
        let login = 1000;
        let journal = vec![Connection {
            user: "ops".into(),
            line: "pts/4".into(),
            host: "localhost".into(),
            name: None,
            kind: crate::model::SessionKind::Local,
            pid: 42,
            login_unix: login,
            logout_unix: None,
        }];
        let merged = merge_connections(Vec::new(), journal);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].user, "ops");
    }
}
