//! Shared data model for sessions, processes, and activity events.

/// The kind of terminal session.
#[cfg_attr(
    not(any(test, target_os = "linux")),
    allow(dead_code) // constructed by the Linux collector / test fixture only
)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SessionKind {
    Ssh,
    Local,
    Tmux,
    Screen,
}

impl SessionKind {
    pub fn label(self) -> &'static str {
        match self {
            SessionKind::Ssh => "SSH",
            SessionKind::Local => "LOCAL",
            SessionKind::Tmux => "TMUX",
            SessionKind::Screen => "SCREEN",
        }
    }
}

/// A logged-in terminal session (from utmpx).
#[derive(Clone, Debug)]
pub struct Session {
    pub user: String,
    /// terminal line, e.g. `pts/3` or `tty1`; empty if unknown.
    pub line: String,
    /// device path, e.g. `/dev/pts/3`.
    pub device: String,
    /// remote host, or "console"/"localhost" for local sessions.
    pub host: String,
    /// human session name when meaningful: tmux/zellij/screen session name.
    pub name: Option<String>,
    pub kind: SessionKind,
    /// session start (unix seconds).
    pub login_unix: i64,
    /// owning pid (the getty/sshd/shell).
    pub pid: u32,
}

/// One process attached to a terminal (a live thing somebody is running).
#[derive(Clone, Debug)]
pub struct Proc {
    pub pid: u32,
    pub user: String,
    /// index into `Snapshot::sessions`, `None` when tty is unknown/unresolved.
    pub session_idx: Option<usize>,
    /// cpu % over the last interval (0..~100).
    pub cpu_pct: f64,
    /// resident set size in KiB.
    pub rss_kb: u64,
    /// approximate wall-clock start (unix seconds).
    pub start_unix: f64,
    /// elapsed seconds.
    pub elapsed: f64,
    /// schedule state char (R, S, Z, T, ...).
    pub state: char,
    /// full argv joined by spaces (falls back to the process name).
    pub cmdline: String,
}

/// A single activity event for the live ticker.
#[derive(Clone, Debug)]
pub struct Event {
    /// unix seconds when observed.
    pub at: i64,
    pub user: String,
    pub session_line: String,
    pub text: String,
    /// "spawn" | "exit" | "note"
    pub kind: &'static str,
}

/// A past (or still-open) terminal connection, reconstructed from wtmp
/// (and enriched by sessionwatch's own journal).
#[derive(Clone, Debug)]
pub struct Connection {
    pub user: String,
    /// terminal line, e.g. `pts/3` or `tty1`.
    pub line: String,
    /// remote host, Tailscale-resolved when possible; "localhost" for local.
    pub host: String,
    /// tmux/zellij/screen session name observed while it was live.
    pub name: Option<String>,
    pub kind: SessionKind,
    pub pid: u32,
    pub login_unix: i64,
    /// when the session ended; `None` = still connected.
    pub logout_unix: Option<i64>,
}

impl Connection {
    pub fn is_live(&self) -> bool {
        self.logout_unix.is_none()
    }
}

/// A failed login attempt (from btmp) — part of the real history of who
/// has been trying to reach the box.
#[derive(Clone, Debug)]
pub struct FailedLogin {
    pub user: String,
    pub host: String,
    pub line: String,
    pub at: i64,
}

/// One full observation of the system.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub sessions: Vec<Session>,
    /// processes attached to one of the known sessions.
    pub procs: Vec<Proc>,
    /// processes with a tty we could not resolve to a known session.
    pub orphans: Vec<Proc>,
    /// every previous connection reconstructed from wtmp (newest by login).
    pub connections: Vec<Connection>,
    /// failed login attempts from btmp.
    pub failed: Vec<FailedLogin>,
    pub load: [f64; 3],
    pub boot_unix: i64,
    pub taken_at: i64,
}
