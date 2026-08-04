//! Collector abstraction over "what is happening on the machine".
//!
//! On Linux we read `/proc` + utmpx directly. Non-Linux hosts cannot run the
//! live collector, so the binary refuses to start there (see `new()`).

pub mod linux;
pub mod utmp;
pub mod wtmp;

#[cfg(test)]
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(test)]
use crate::model::{Connection, Proc, Session, SessionKind};
use crate::model::Snapshot;

/// Produces a [`Snapshot`] on demand. Implementations keep whatever state they
/// need to compute deltas (e.g. per-pid CPU ticks).
pub trait Collector: Send {
    fn collect(&mut self) -> Snapshot;
    /// Human-readable source name, shown in the header (e.g. "live:/proc").
    fn source(&self) -> &'static str;
}

/// Build the live Linux collector. Fails on non-Linux hosts — there is no
/// `/proc` to watch, so sessionwatch refuses to pretend otherwise.
pub fn new() -> Result<Box<dyn Collector>, &'static str> {
    #[cfg(target_os = "linux")]
    {
        return Ok(Box::new(linux::LinuxCollector::new()));
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = ();
        Err("sessionwatch requires Linux: it watches /proc and utmpx, which do not exist on this platform")
    }
}

/// Extract a human session name from the argv of the processes on a tty.
///
/// Understands the common multiplexer invocations that carry a session name:
/// - `tmux (new|new-session|attach|attach-session|switch-client) [-s|-t] NAME`
/// - `screen [-S|-r|-x|-R] NAME`
/// - `zellij (attach|a|switch) NAME` / `zellij [-s|--session] NAME`
#[cfg(any(test, target_os = "linux"))]
pub(crate) fn session_name_from_cmdlines<'a>(
    cmdlines: impl Iterator<Item = &'a str>,
) -> Option<String> {
    for cmd in cmdlines {
        let mut parts = cmd.split_whitespace();
        let prog = parts.next()?;
        let prog = prog.trim_start_matches('-');
        let (flag_names, positional): (&[&str], bool) = match prog {
            "tmux" | "tmux:" => (&["-s", "-t", "--session"], true),
            "screen" => (&["-S", "-r", "-x", "-R"], false),
            "zellij" => (&["-s", "--session"], true),
            _ => continue,
        };
        let toks: Vec<&str> = parts.collect();
        // find a flag with a value
        let mut i = 0;
        while i < toks.len() {
            if flag_names.contains(&toks[i]) && i + 1 < toks.len() {
                let v = toks[i + 1];
                if !v.starts_with('-') {
                    return clean_name(v);
                }
                i += 2;
                continue;
            }
            i += 1;
        }
        // positional name: `zellij attach NAME`, `tmux attach NAME`
        if positional && !toks.is_empty() {
            let sub = toks
                .iter()
                .position(|t| {
                    matches!(*t, "new" | "new-session" | "attach" | "attach-session" | "a" | "switch" | "switch-client")
                })
                .and_then(|p| toks.get(p + 1).copied());
            if let Some(rest) = sub {
                if !rest.starts_with('-') {
                    return clean_name(rest);
                }
            }
        }
    }
    None
}

/// Strip a `session:window` target down to the session and reject junk.
#[cfg(any(test, target_os = "linux"))]
fn clean_name(v: &str) -> Option<String> {
    let v = v.split(':').next().unwrap_or(v);
    let v = v.trim().trim_matches(['\'', '"']);
    if v.is_empty() {
        None
    } else {
        Some(v.to_string())
    }
}

/// In-memory fixture collector for UI tests (the live collector is Linux-only,
/// so the render tests need a fixed snapshot to draw).
#[cfg(test)]
pub fn test_collector() -> Box<dyn Collector> {
    struct Fixed;
    impl Collector for Fixed {
        fn source(&self) -> &'static str {
            "test"
        }
        fn collect(&mut self) -> Snapshot {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            let mk = |user: &str, host: &str, line: &str, kind: SessionKind, name: Option<&str>, pid: u32, login_ago: i64| Session {
                user: user.into(),
                line: line.into(),
                device: format!("/dev/{line}"),
                host: host.into(),
                name: name.map(str::to_string),
                kind,
                login_unix: now - login_ago,
                pid,
            };
            let sessions = vec![
                mk("jackphelps", "10.20.30.5", "pts/0", SessionKind::Ssh, None, 900, 600),
                mk("alice", "localhost", "pts/1", SessionKind::Local, None, 901, 1500),
                mk("carol", "172.16.8.12", "pts/2", SessionKind::Ssh, None, 902, 2400),
                mk("dev", "localhost", "pts/3", SessionKind::Tmux, Some("cursor-env"), 903, 3300),
                mk("ops", "localhost", "pts/4", SessionKind::Screen, Some("deploy-prod"), 904, 4200),
            ];
            let mut procs = Vec::new();
            let mut n = 1000;
            let mut push = |user: &str, sess: usize, cmd: &str, age: f64, cpu: f64| {
                n += 1;
                procs.push(Proc {
                    pid: n,
                    user: user.into(),
                    session_idx: Some(sess),
                    cpu_pct: cpu,
                    rss_kb: 12_000,
                    start_unix: now as f64 - age,
                    elapsed: age,
                    state: 'S',
                    cmdline: cmd.into(),
                });
            };
            push("jackphelps", 0, "vim src/main.rs", 2.0, 22.5);
            push("jackphelps", 0, "bash", 500.0, 0.4);
            push("jackphelps", 0, "git status", 0.5, 3.0);
            push("alice", 1, "npm run dev", 300.0, 8.0);
            push("alice", 1, "zsh", 1200.0, 0.3);
            push("carol", 2, "docker compose up -d", 40.0, 12.0);
            push("carol", 2, "bash", 900.0, 0.5);
            push("dev", 3, "tmux attach -t cursor-env", 100.0, 1.0);
            push("dev", 3, "cargo build --release", 200.0, 41.0);
            push("dev", 3, "zsh", 800.0, 0.3);
            push("ops", 4, "tail -f /var/log/nginx/access.log", 60.0, 1.5);
            push("ops", 4, "screen", 400.0, 0.4);
            let connections = vec![
                Connection {
                    user: "jackphelps".into(),
                    line: "pts/0".into(),
                    host: "10.20.30.5".into(),
                    kind: SessionKind::Ssh,
                    pid: 900,
                    login_unix: now - 3600,
                    logout_unix: Some(now - 1800),
                },
                Connection {
                    user: "alice".into(),
                    line: "pts/1".into(),
                    host: "localhost".into(),
                    kind: SessionKind::Local,
                    pid: 901,
                    login_unix: now - 7200,
                    logout_unix: Some(now - 3600),
                },
                Connection {
                    user: "carol".into(),
                    line: "pts/2".into(),
                    host: "172.16.8.12".into(),
                    kind: SessionKind::Ssh,
                    pid: 902,
                    login_unix: now - 5400,
                    logout_unix: None,
                },
                Connection {
                    user: "jackphelps".into(),
                    line: "pts/0".into(),
                    host: "10.20.30.5".into(),
                    kind: SessionKind::Ssh,
                    pid: 903,
                    login_unix: now - 300,
                    logout_unix: None,
                },
            ];
            Snapshot {
                sessions,
                procs,
                orphans: Vec::new(),
                connections,
                load: [0.4, 0.7, 0.6],
                boot_unix: now - 86_400,
                taken_at: now,
            }
        }
    }
    Box::new(Fixed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tmux_new_and_attach() {
        let cmds = ["tmux new -s cursor-env", "tmux attach -t work:1.3", "tmux -2 attach-session -t jira", "tmux ls"];
        assert_eq!(
            session_name_from_cmdlines(cmds.iter().copied()),
            Some("cursor-env".to_string())
        );
        assert_eq!(
            session_name_from_cmdlines(["tmux attach -t work:1.3"].iter().copied()),
            Some("work".to_string())
        );
        assert_eq!(
            session_name_from_cmdlines(["tmux ls"].iter().copied()),
            None
        );
    }

    #[test]
    fn screen_and_zellij() {
        assert_eq!(
            session_name_from_cmdlines(["screen -S deploy"].iter().copied()),
            Some("deploy".to_string())
        );
        assert_eq!(
            session_name_from_cmdlines(["screen -r oldbox"].iter().copied()),
            Some("oldbox".to_string())
        );
        assert_eq!(
            session_name_from_cmdlines(["zellij attach work"].iter().copied()),
            Some("work".to_string())
        );
        assert_eq!(
            session_name_from_cmdlines(["zellij -s scratch"].iter().copied()),
            Some("scratch".to_string())
        );
    }

    #[test]
    fn ignores_plain_shells() {
        assert_eq!(
            session_name_from_cmdlines(["bash", "vim x"].iter().copied()),
            None
        );
    }
}
