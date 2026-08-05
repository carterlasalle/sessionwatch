//! Tailscale-aware host resolution.
//!
//! Sessions over Tailscale SSH show up in utmp/wtmp with the CGNAT address
//! (`100.x.y.z`) as the remote host, which says nothing about *who* connected.
//! `tailscale status` maps those addresses to tailnet machine names, so we
//! can show the real identity: `100.64.0.5` -> `jackphelps-mbp`.
//!
//! Resolution is best-effort: if the tailscale CLI is absent, not running,
//! or needs permissions we lack, hosts pass through unchanged.
#![allow(dead_code)] // used by the Linux collector; kept compiled on all hosts so tests run

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// CGNAT range used by Tailscale (and other overlay nets): 100.64.0.0/10.
pub(crate) fn is_cgnat(host: &str) -> bool {
    let mut parts = host.split('.');
    if parts.next() != Some("100") {
        return false;
    }
    let Some(second) = parts.next().and_then(|p| p.parse::<u8>().ok()) else {
        return false;
    };
    (64..=127).contains(&second)
}

/// Parse the plain-text `tailscale status` output into CGNAT IP -> short
/// tailnet machine name. Each node line looks like:
/// `100.64.0.5   jackphelps-mbp   jackphelps-mbp.tailnet.ts.net   linux   -`
/// (IP, short name, DNS name, OS, version, ...). We keep the short name.
pub fn parse_tailscale_status(text: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for line in text.lines() {
        let mut toks = line.split_whitespace();
        let Some(ip) = toks.next() else { continue };
        if !is_cgnat(ip) {
            continue;
        }
        let Some(name) = toks.next() else { continue };
        if name.contains(':') || name.starts_with('-') {
            continue;
        }
        map.insert(ip.to_string(), name.to_string());
    }
    map
}

/// Parse the Tailscale SSH server child's argv to recover the remote
/// identity. Tailscale tags each session's sshd-side process with
/// `--remote-user=<email> --remote-ip=<CGNAT ip>`, which is the only place
/// the *person's* identity (e.g. jackphelps20@gmail.com) is recorded —
/// utmp only holds the local username. Returns (email, ip).
pub fn parse_ts_identity(cmdline: &str) -> Option<(String, String)> {
    let mut email = None;
    let mut ip = None;
    for tok in cmdline.split_whitespace() {
        if let Some(v) = tok.strip_prefix("--remote-user=") {
            if !v.is_empty() {
                email = Some(v.to_string());
            }
        } else if let Some(v) = tok.strip_prefix("--remote-ip=") {
            if !v.is_empty() {
                ip = Some(v.to_string());
            }
        }
    }
    match (email, ip) {
        (Some(e), Some(i)) => Some((e, i)),
        _ => None,
    }
}

pub struct TailscaleMap {
    map: HashMap<String, String>,
    last_lookup: Option<Instant>,
    /// how often we re-run `tailscale status`
    ttl: Duration,
}

impl TailscaleMap {
    pub fn new() -> Self {
        TailscaleMap {
            map: HashMap::new(),
            last_lookup: None,
            ttl: Duration::from_secs(60),
        }
    }

    /// Resolve a host string to a human identity. Returns the resolved name
    /// for CGNAT addresses (tailscale), and leaves everything else as-is.
    pub fn resolve(&mut self, host: &str) -> String {
        if !is_cgnat(host) {
            return host.to_string();
        }
        if self
            .last_lookup
            .map(|t| t.elapsed() > self.ttl)
            .unwrap_or(true)
        {
            self.refresh();
        }
        self.map
            .get(host)
            .cloned()
            .unwrap_or_else(|| host.to_string())
    }

    fn refresh(&mut self) {
        self.last_lookup = Some(Instant::now());
        self.map.clear();
        #[cfg(target_os = "linux")]
        {
            use std::process::Command;
            let out = Command::new("tailscale")
                .arg("status")
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
                .unwrap_or_default();
            self.map = parse_tailscale_status(&out);
        }
        #[cfg(not(target_os = "linux"))]
        let _ = ();
    }
}

impl Default for TailscaleMap {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_status_lines() {
        let text = "\
# Health check:
100.64.0.5   jackphelps-mbp   jackphelps-mbp.tailnet.ts.net   linux   -
100.64.0.6   jetson   jetson.tailnet.ts.net   linux   -
100.101.102.103   bastion   bastion.corp.ts.net   linux   -
-  (no node)
";
        let map = parse_tailscale_status(text);
        assert_eq!(
            map.get("100.64.0.5").map(|s| s.as_str()),
            Some("jackphelps-mbp")
        );
        assert_eq!(map.get("100.64.0.6").map(|s| s.as_str()), Some("jetson"));
        assert_eq!(
            map.get("100.101.102.103").map(|s| s.as_str()),
            Some("bastion")
        );
        assert_eq!(map.len(), 3);
    }

    #[test]
    fn parses_remote_identity_from_sshd_child() {
        let cmd = "/usr/bin/ssh --remote-user=jackphelps20@gmail.com --remote-ip=100.119.117.69 --server-addr=100.80.103.86:443";
        let (email, ip) = parse_ts_identity(cmd).unwrap();
        assert_eq!(email, "jackphelps20@gmail.com");
        assert_eq!(ip, "100.119.117.69");
        assert_eq!(parse_ts_identity("bash -c 'ls'"), None);
        assert_eq!(parse_ts_identity("ssh --remote-user=noip"), None);
    }
}
