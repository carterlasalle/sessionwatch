//! Synthetic collector: generates a plausible, slowly-evolving set of sessions
//! and processes so the TUI can be run and screenshotted anywhere.
//!
//! Selected with `--demo`, or automatically on non-Linux hosts.

use crate::model::{Proc, Session, SessionKind, Snapshot};

use super::Collector;

struct DemoProc {
    pid: u32,
    user: String,
    session: usize,
    cmdline: String,
    /// born this many (sim) seconds ago
    age: f64,
    /// live for this many seconds before exiting
    life: f64,
    state: char,
    rss_kb: u64,
    cpu: f64,
}

pub struct DemoCollector {
    tick: u64,
    sim: f64, // sim seconds elapsed
    procs: Vec<DemoProc>,
    next_pid: u32,
}

/// (user, host, line) per session
const SESSIONS: &[(&str, &str, &str, SessionKind)] = &[
    ("alice", "10.20.30.5", "pts/0", SessionKind::Ssh),
    ("bob", "localhost", "pts/1", SessionKind::Local),
    ("carol", "172.16.8.12", "pts/2", SessionKind::Ssh),
    ("dev", "localhost", "pts/3", SessionKind::Tmux),
    ("ops", "localhost", "pts/4", SessionKind::Screen),
];

/// Plausible commands: (argv, min_life_s, max_life_s)
const COMMANDS: &[(&str, f64, f64)] = &[
    ("vim src/main.rs", 20.0, 400.0),
    ("bash", 600.0, 900.0),
    ("zsh", 600.0, 900.0),
    ("ls -la", 0.3, 1.2),
    ("grep -rn 'TODO' .", 2.0, 6.0),
    ("npm run dev", 120.0, 900.0),
    ("python3 train.py --epochs 10", 80.0, 600.0),
    ("apt-get install -y ripgrep", 8.0, 40.0),
    ("curl -s https://api.github.com/repos/rust-lang/rust", 1.0, 4.0),
    ("systemctl status sshd", 0.5, 2.0),
    ("git status", 0.4, 1.5),
    ("git log --oneline -20", 0.5, 2.0),
    ("tail -f /var/log/nginx/access.log", 200.0, 900.0),
    ("docker compose up -d", 6.0, 30.0),
    ("htop", 40.0, 300.0),
    ("top -b -n 1", 3.0, 6.0),
    ("cat /var/log/syslog", 1.0, 4.0),
    ("ssh deploy@staging 'docker ps'", 3.0, 8.0),
    ("cargo build --release", 60.0, 240.0),
    ("ps aux | grep postgres", 0.5, 2.0),
    ("find /var/log -name '*.gz' -mtime -7", 2.0, 5.0),
    ("kubectl get pods -A", 1.0, 3.0),
    ("tmux: server", 600.0, 900.0), // attached on the tmux pty
    ("screen -ls", 0.5, 1.5),
];

impl DemoCollector {
    pub fn new() -> Self {
        let mut c = DemoCollector {
            tick: 0,
            sim: 0.0,
            procs: Vec::new(),
            next_pid: 1000,
        };
        // Seed with a rich initial state: every session gets a long-lived
        // shell plus some activity so nothing ever looks empty.
        for i in 0..SESSIONS.len() {
            c.push_shell(i);
        }
        for _ in 0..6 {
            c.spawn(Some(0));
        }
        for _ in 0..4 {
            c.spawn(Some(1));
        }
        for _ in 0..5 {
            c.spawn(Some(2));
        }
        for _ in 0..6 {
            c.spawn(Some(3));
        }
        for _ in 0..3 {
            c.spawn(Some(4));
        }
        c
    }

    /// Push a login shell that lives for a while.
    fn push_shell(&mut self, session: usize) {
        let (user, _, _, _) = SESSIONS[session];
        let shell = ["bash", "zsh"][(self.rng() % 2) as usize];
        let age = 40.0 + (self.rng() % 300) as f64;
        let rss_kb = 3_000 + ((self.rng() % 100) as u64) * 32;
        let pid = self.next_pid;
        self.next_pid += 1;
        self.procs.push(DemoProc {
            pid,
            user: user.to_string(),
            session,
            cmdline: shell.to_string(),
            age,
            life: 3600.0,
            state: 'S',
            rss_kb,
            cpu: 0.4,
        });
    }

    fn rng(&mut self) -> u64 {
        // xorshift64*
        let mut x = self.tick.wrapping_mul(0x9E3779B97F4A7C15) ^ 0xDEADBEEF;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.tick = self.tick.wrapping_add(1);
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }

    fn spawn(&mut self, forced: Option<usize>) {
        let n = self.rng() % COMMANDS.len() as u64;
        let (cmdline, lo, hi) = COMMANDS[n as usize];
        let session = forced.unwrap_or((self.rng() % SESSIONS.len() as u64) as usize);
        let r = self.rng() as f64 / u64::MAX as f64;
        let life = lo + r * (hi - lo);
        let (user, _, _, _) = SESSIONS[session];
        let cpu = if cmdline.starts_with("vim")
            || cmdline.starts_with("htop")
            || cmdline.starts_with("python")
            || cmdline.starts_with("cargo")
        {
            8.0 + r * 60.0
        } else {
            r * 20.0
        };
        let rss_kb = 4_000 + ((self.rng() % 400) as u64) * 64;
        let pid = self.next_pid;
        self.next_pid += 1;
        self.procs.push(DemoProc {
            pid,
            user: user.to_string(),
            session,
            cmdline: cmdline.to_string(),
            age: 0.0,
            life,
            state: 'S',
            rss_kb,
            cpu,
        });
    }
}

impl Collector for DemoCollector {
    fn source(&self) -> &'static str {
        "demo"
    }

    fn collect(&mut self) -> Snapshot {
        self.sim += 1.0; // 1s per call
        self.tick += 1;

        // Advance ages; drop finished procs.
        let jitter: Vec<f64> = (0..self.procs.len()).map(|_| (self.rng() % 20) as f64).collect();
        for (p, d) in self.procs.iter_mut().zip(jitter) {
            p.age += 1.0;
            p.cpu = (p.cpu * 0.7 + d * 0.3).min(95.0);
        }
        self.procs.retain(|p| p.age < p.life);

        // Occasionally spawn new activity (0..=2 procs, biased to ssh sessions).
        let nspawn = (self.rng() % 3) as usize;
        for _ in 0..nspawn {
            let sess = if self.rng() % 100 < 60 {
                ((self.rng() % 2) * 2) as usize // alice(0) or carol(2): the ssh folks
            } else {
                (self.rng() % 4) as usize
            };
            self.spawn(Some(sess));
        }

        let now_unix = unix_now();
        let sessions: Vec<Session> = SESSIONS
            .iter()
            .enumerate()
            .map(|(i, (user, host, line, kind))| Session {
                user: user.to_string(),
                line: line.to_string(),
                device: format!("/dev/{}", line),
                host: host.to_string(),
                kind: *kind,
                login_unix: now_unix - 600 - (i as i64) * 900,
                pid: 700 + i as u32,
            })
            .collect();

        let mut procs: Vec<Proc> = Vec::new();
        for p in &self.procs {
            procs.push(Proc {
                pid: p.pid,
                user: p.user.clone(),
                session_idx: Some(p.session),
                cpu_pct: p.cpu,
                rss_kb: p.rss_kb,
                start_unix: now_unix as f64 - p.age,
                elapsed: p.age,
                state: p.state,
                cmdline: p.cmdline.clone(),
            });
        }

        let load = [(self.sim * 0.1) % 2.0, 1.0 + (self.sim * 0.05) % 1.0, 0.8];

        Snapshot {
            sessions,
            procs,
            orphans: Vec::new(),
            load,
            boot_unix: now_unix - 86_400,
            taken_at: now_unix,
        }
    }
}

fn unix_now() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
