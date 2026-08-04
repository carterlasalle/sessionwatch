//! Real Linux collector: enumerates login sessions from utmpx and watches every
//! process attached to a terminal by scanning `/proc`.

#![cfg(target_os = "linux")]

use std::collections::HashMap;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::model::{FailedLogin, Proc, Session, SessionKind, Snapshot};

use super::btmp::BtmpReader;
use super::journal::{merge_connections, Journal};
use super::tailscale::TailscaleMap;
use super::utmp;
use super::wtmp::WtmpReader;
use super::Collector;

pub struct LinuxCollector {
    proc_root: PathBuf,
    clk_tck: f64,
    page_size: u64,
    /// pid -> (utime, stime) in clock ticks, from the previous pass.
    prev_ticks: HashMap<u32, (u64, u64)>,
    prev_at: Option<Instant>,
    users: HashMap<u32, String>,
    wtmp: WtmpReader,
    btmp: BtmpReader,
    tailscale: TailscaleMap,
    journal: Journal,
    prev_sessions: Vec<Session>,
}

impl LinuxCollector {
    pub fn new() -> Self {
        LinuxCollector {
            proc_root: PathBuf::from("/proc"),
            clk_tck: unsafe { libc::sysconf(libc::_SC_CLK_TCK) as f64 }.max(1.0),
            page_size: unsafe { libc::sysconf(libc::_SC_PAGESIZE) as u64 }.max(4096),
            prev_ticks: HashMap::new(),
            prev_at: None,
            users: HashMap::new(),
            wtmp: WtmpReader::new(),
            btmp: BtmpReader::new(),
            tailscale: TailscaleMap::new(),
            journal: Journal::new(),
            prev_sessions: Vec::new(),
        }
    }
}

impl Collector for LinuxCollector {
    fn source(&self) -> &'static str {
        "live:/proc"
    }

    fn collect(&mut self) -> Snapshot {
        let now = Instant::now();
        let now_unix = unix_now();
        let dt = self
            .prev_at
            .map(|p| now.duration_since(p).as_secs_f64())
            .unwrap_or(0.25);
        let (uptime, load) = read_proc_meta(&self.proc_root);
        let boot_unix = (now_unix as f64 - uptime).round() as i64;

        let mut sessions = read_sessions();
        // Resolve each session's device to a (major, minor) for tty matching.
        let sess_dev: Vec<Option<(u32, u32)>> = sessions
            .iter()
            .map(|s| devnum(Path::new(&s.device)))
            .collect();
        let mut idx_by_dev: HashMap<(u32, u32), usize> = HashMap::new();
        for (i, md) in sess_dev.iter().enumerate() {
            if let Some(d) = md {
                idx_by_dev.insert(*d, i);
            }
        }

        let mut procs: Vec<Proc> = Vec::new();
        let mut orphans: Vec<Proc> = Vec::new();
        let mut cur_ticks: HashMap<u32, (u64, u64)> = HashMap::new();

        for pid in read_dir_numeric(&self.proc_root) {
            let base = self.proc_root.join(pid.to_string());
            let Some((comm, post)) = read_stat(&base) else {
                continue;
            };
            // post[0]=state post[1]=ppid post[4]=tty_nr post[11]=utime
            // post[12]=stime post[19]=starttime post[21]=rss(pages)
            let state = post.first().copied().map(|v| v as u8 as char).unwrap_or('?');
            let _ppid = get(&post, 1).unwrap_or(0) as u32;
            let tnr = get(&post, 4).unwrap_or(0);
            let utime = get(&post, 11).unwrap_or(0) as u64;
            let stime = get(&post, 12).unwrap_or(0) as u64;
            let start_ticks = get(&post, 19).unwrap_or(0) as f64;
            let start_secs = start_ticks / self.clk_tck;

            let (major, minor) = decode_tty(tnr);
            if major == 0 {
                continue; // no controlling terminal: not session activity.
            }

            let uid = read_uid(&base).unwrap_or(u32::MAX);
            let user = self.uid_name(uid);
            let command = read_cmdline(&base);
            let command = if command.is_empty() { comm.clone() } else { command };

            let rss_kb = get(&post, 21).unwrap_or(0) as u64 * self.page_size / 1024;
            let elapsed = (uptime - start_secs).max(0.0);
            let start_unix = now_unix as f64 - uptime + start_secs;

            let (pu, ps) = self.prev_ticks.get(&pid).copied().unwrap_or((0, 0));
            let dticks = (utime.saturating_sub(pu) + stime.saturating_sub(ps)) as f64;
            let cpu_pct = dticks / self.clk_tck / dt.max(1e-6) * 100.0;
            cur_ticks.insert(pid, (utime, stime));

            let session_idx = idx_by_dev.get(&(major, minor)).copied();

            let mut p = Proc {
                pid,
                user,
                session_idx,
                cpu_pct,
                rss_kb,
                start_unix,
                elapsed,
                state,
                cmdline: command,
            };

            if session_idx.is_some() {
                procs.push(p);
            } else {
                p.session_idx = None;
                orphans.push(p);
            }
        }

        assign_kinds(&mut sessions, &procs);

        // Human session names from tmux/screen/zellij argv on each tty.
        for (i, s) in sessions.iter_mut().enumerate() {
            if s.name.is_none() {
                let cmdlines = procs
                    .iter()
                    .filter(|p| p.session_idx == Some(i))
                    .map(|p| p.cmdline.as_str());
                s.name = super::session_name_from_cmdlines(cmdlines);
            }
            // Tailscale: turn CGNAT addresses into real tailnet machine names.
            s.host = self.tailscale.resolve(&s.host);
        }

        // Journal: persist what we observe live so history is richer later.
        self.journal.observe(&self.prev_sessions, &sessions);
        self.prev_sessions = sessions.clone();

        self.prev_ticks = cur_ticks;
        self.prev_at = Some(now);

        let mut connections = merge_connections(
            self.wtmp.refresh(),
            [self.journal.loaded(), self.journal.observed()].concat(),
        );
        for c in connections.iter_mut() {
            c.host = self.tailscale.resolve(&c.host);
        }
        let mut failed: Vec<FailedLogin> = self.btmp.refresh();
        for f in failed.iter_mut() {
            f.host = self.tailscale.resolve(&f.host);
        }

        Snapshot {
            sessions,
            procs,
            orphans,
            connections,
            failed,
            load,
            boot_unix,
            taken_at: now_unix,
        }
    }
}

fn get(v: &[i64], i: usize) -> Option<i64> {
    v.get(i).copied()
}

/// Refine each session's kind using the processes we can observe on its tty.
fn assign_kinds(sessions: &mut [Session], procs: &[Proc]) {
    for (i, s) in sessions.iter_mut().enumerate() {
        let mut procs_here = procs.iter().filter(|p| p.session_idx == Some(i));
        let has_tmux = procs_here
            .clone()
            .any(|p| p.cmdline.starts_with("tmux:") || p.cmdline.starts_with("tmux "));
        let has_screen = procs_here.any(|p| p.cmdline.starts_with("screen"));
        if has_tmux {
            s.kind = SessionKind::Tmux;
        } else if has_screen {
            s.kind = SessionKind::Screen;
        } else if s.host == "localhost" || s.host.is_empty() {
            s.kind = SessionKind::Local;
        } else {
            s.kind = SessionKind::Ssh;
        }
    }
}

/// glibc utmpx encoding of the `/proc/pid/stat` tty_nr field (a kernel device).
fn decode_tty(tnr: i64) -> (u32, u32) {
    let t = tnr as u32;
    let major = (t >> 8) & 0xfff;
    let minor = (t & 0xff) | ((t >> 12) & 0xfff00);
    (major, minor)
}

fn devnum(path: &Path) -> Option<(u32, u32)> {
    let md = std::fs::metadata(path).ok()?;
    let r = md.rdev();
    Some((
        libc::major(r) as u32,
        libc::minor(r) as u32,
    ))
}

fn read_dir_numeric(root: &Path) -> Vec<u32> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(root) {
        for e in rd.flatten() {
            if let Some(s) = e.file_name().to_str() {
                if let Ok(n) = s.parse::<u32>() {
                    out.push(n);
                }
            }
        }
    }
    out
}

/// Parse `/proc/<pid>/stat`; returns (comm, numeric fields following the comm field).
fn read_stat(base: &Path) -> Option<(String, Vec<i64>)> {
    let data = std::fs::read_to_string(base.join("stat")).ok()?;
    let open = data.find('(')?;
    let close = data.rfind(')')?;
    let comm = &data[open + 1..close];
    let post: Vec<i64> = data[close + 1..]
        .split_whitespace()
        .filter_map(|t| t.parse().ok())
        .collect();
    Some((comm.to_string(), post))
}

fn read_uid(base: &Path) -> Option<u32> {
    let data = std::fs::read_to_string(base.join("status")).ok()?;
    for line in data.lines() {
        if let Some(rest) = line.strip_prefix("Uid:") {
            return rest.split_whitespace().nth(1).and_then(|t| t.parse().ok());
        }
    }
    None
}

fn read_cmdline(base: &Path) -> String {
    let raw = std::fs::read(base.join("cmdline")).unwrap_or_default();
    if raw.is_empty() || raw.iter().all(|&b| b == 0) {
        return String::new();
    }
    raw.iter()
        .map(|&b| if b == 0 { ' ' } else { b as char })
        .collect::<String>()
        .trim()
        .to_string()
}

fn read_proc_meta(root: &Path) -> (f64, [f64; 3]) {
    let uptime = std::fs::read_to_string(root.join("uptime"))
        .ok()
        .and_then(|s| s.split_whitespace().next()?.parse().ok())
        .unwrap_or(0.0);
    let mut load = [0.0; 3];
    if let Ok(ls) = std::fs::read_to_string(root.join("loadavg")) {
        for (i, tok) in ls.split_whitespace().take(3).enumerate() {
            load[i] = tok.parse().unwrap_or(0.0);
        }
    }
    (uptime, load)
}

fn unix_now() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

impl LinuxCollector {
    fn uid_name(&mut self, uid: u32) -> String {
        if let Some(n) = self.users.get(&uid) {
            return n.clone();
        }
        let mut pwd = unsafe { std::mem::zeroed::<libc::passwd>() };
        let mut buf = vec![0u8; 2048];
        let mut result: *mut libc::passwd = std::ptr::null_mut();
        let name = unsafe {
            let r = libc::getpwuid_r(
                uid as libc::uid_t,
                &mut pwd,
                buf.as_mut_ptr() as *mut _,
                buf.len(),
                &mut result,
            );
            if r == 0 && !result.is_null() {
                cstr_from_ptr(pwd.pw_name)
            } else {
                uid.to_string()
            }
        };
        self.users.insert(uid, name.clone());
        name
    }
}

fn cstr_from_ptr(p: *const libc::c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    let mut s = Vec::new();
    unsafe {
        let mut i = 0;
        loop {
            let c = *p.add(i);
            if c == 0 {
                break;
            }
            s.push(c as u8);
            i += 1;
        }
    }
    String::from_utf8_lossy(&s).into_owned()
}

fn read_sessions() -> Vec<Session> {
    utmp::read_utmp_file()
}
