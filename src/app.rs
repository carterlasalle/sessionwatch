//! Application state, refresh/diffing logic, and the key-driven event loop.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crossterm::event::{Event as TermEvent, KeyCode, KeyEvent, KeyModifiers};

use crate::collect::Collector;
use crate::model::{Event, Proc, Snapshot};

pub const MAX_EVENTS: usize = 64;
const HISTORY_LEN: usize = 42;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Sessions,
    Processes,
    History,
}

/// Which panel the right-hand side shows.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum View {
    Processes,
    History,
}

pub struct App {
    pub collector: Box<dyn Collector>,
    pub snap: Snapshot,
    prev: Option<Snapshot>,
    pub events: VecDeque<Event>,
    /// per-session activity history (proc count) for sparklines.
    pub history: Vec<Vec<f64>>,
    /// unix time of the last observed event per session — drives FOLLOW.
    pub last_activity: Vec<i64>,
    pub selected: usize,
    pub proc_sel: usize,
    pub conn_sel: usize,
    pub focus: Focus,
    pub view: View,
    pub follow: bool,
    pub interval: Duration,
    pub last_refresh: Instant,
    pub phase: u64,
    pub show_help: bool,
    pub rows: (u16, u16),
}

impl App {
    pub fn new(collector: Box<dyn Collector>, interval: Duration) -> Self {
        let mut app = App {
            collector,
            snap: Snapshot::default(),
            prev: None,
            events: VecDeque::new(),
            history: Vec::new(),
            last_activity: Vec::new(),
            selected: 0,
            proc_sel: 0,
            conn_sel: 0,
            focus: Focus::Sessions,
            view: View::Processes,
            follow: false,
            interval,
            last_refresh: Instant::now(),
            phase: 0,
            show_help: false,
            rows: (24, 80),
        };
        app.refresh();
        app
    }

    pub fn refresh(&mut self) {
        let new_snap = self.collector.collect();
        self.diff(&new_snap);
        self.snap = new_snap;
        self.last_refresh = Instant::now();

        // Per-session activity history (proc count) for sparklines.
        while self.history.len() < self.snap.sessions.len() {
            self.history.push(Vec::new());
        }
        for (i, hist) in self.history.iter_mut().enumerate() {
            let n = self
                .snap
                .procs
                .iter()
                .filter(|p| p.session_idx == Some(i))
                .count() as f64;
            hist.push(n);
            if hist.len() > HISTORY_LEN {
                let overflow = hist.len() - HISTORY_LEN;
                hist.drain(0..overflow);
            }
        }
        // Baseline activity so FOLLOW doesn't treat the first frame specially.
        while self.last_activity.len() < self.snap.sessions.len() {
            self.last_activity.push(self.snap.taken_at);
        }

        if self.snap.sessions.is_empty() {
            self.selected = 0;
        } else {
            self.selected = self.selected.min(self.snap.sessions.len() - 1);
            if self.follow {
                self.selected = self.most_active();
            }
            self.proc_sel = self
                .proc_sel
                .min(self.session_proc_len().saturating_sub(1));
            self.conn_sel = self.conn_sel.min(self.snap.connections.len().saturating_sub(1));
        }
    }

    /// Session with the most recent activity event; keeps the current
    /// selection on ties so FOLLOW never yanks the cursor around idly.
    fn most_active(&self) -> usize {
        let mut best = self.selected;
        let mut best_at = self.last_activity.get(self.selected).copied().unwrap_or(0);
        for (i, at) in self.last_activity.iter().enumerate() {
            if *at > best_at {
                best_at = *at;
                best = i;
            }
        }
        best
    }

    /// Emit spawn/exit/login events vs. the previous snapshot.
    fn diff(&mut self, new: &Snapshot) {
        let Some(prev) = self.prev.clone() else {
            // Baseline: seed only the most recent handful so the opening screen
            // isn't a wall of "spawned" lines.
            let mut seed: Vec<Event> = new
                .procs
                .iter()
                .map(|p| self.note(new.taken_at, p, format!("spawned `{}`", p.cmdline), "spawn"))
                .collect();
            seed.truncate(MAX_EVENTS);
            self.events = seed.into_iter().collect();
            self.prev = Some(new.clone());
            return;
        };

        let prev_pids: std::collections::HashSet<u32> = prev.procs.iter().map(|p| p.pid).collect();
        let new_pids: std::collections::HashSet<u32> = new.procs.iter().map(|p| p.pid).collect();

        for p in &new.procs {
            if !prev_pids.contains(&p.pid) {
                self.push(self.note(new.taken_at, p, format!("spawned `{}`", p.cmdline), "spawn"));
                if let Some(i) = p.session_idx {
                    self.mark_active(i, new.taken_at);
                }
            }
        }
        for p in &prev.procs {
            if !new_pids.contains(&p.pid) {
                self.push(self.note(new.taken_at, p, format!("ended `{}`", p.cmdline), "exit"));
                if let Some(i) = p.session_idx {
                    self.mark_active(i, new.taken_at);
                }
            }
        }

        for s in &new.sessions {
            if !prev.sessions.iter().any(|o| o.line == s.line) {
                self.push(Event {
                    at: new.taken_at,
                    user: s.user.clone(),
                    session_line: s.line.clone(),
                    text: format!("logged in on {}", s.line),
                    kind: "note",
                });
                if let Some(i) = new.sessions.iter().position(|o| o.line == s.line) {
                    self.mark_active(i, new.taken_at);
                }
            }
        }
        for s in &prev.sessions {
            if !new.sessions.iter().any(|o| o.line == s.line) {
                self.push(Event {
                    at: new.taken_at,
                    user: s.user.clone(),
                    session_line: s.line.clone(),
                    text: format!("disconnected from {}", s.line),
                    kind: "exit",
                });
            }
        }

        self.prev = Some(new.clone());
    }

    fn note(&self, at: i64, p: &Proc, text: String, kind: &'static str) -> Event {
        let line = match p.session_idx {
            Some(i) => self
                .snap
                .sessions
                .get(i)
                .map(|s| s.line.clone())
                .unwrap_or_else(|| "?".into()),
            None => "?".into(),
        };
        Event {
            at,
            user: p.user.clone(),
            session_line: line,
            text,
            kind,
        }
    }

    fn push(&mut self, e: Event) {
        self.events.push_back(e);
        while self.events.len() > MAX_EVENTS {
            self.events.pop_front();
        }
    }

    fn mark_active(&mut self, session: usize, at: i64) {
        if self.last_activity.len() <= session {
            self.last_activity.resize(session + 1, at);
        }
        self.last_activity[session] = at;
    }

    pub fn run(&mut self, terminal: &mut crate::ui::Terminal) -> std::io::Result<()> {
        loop {
            terminal.draw(|f| crate::ui::draw(f, self))?;
            self.phase = self.phase.wrapping_add(1);

            let interval_ms = self.interval.as_millis() as u64;
            let elapsed = self.last_refresh.elapsed().as_millis() as u64;
            let until_refresh = interval_ms.saturating_sub(elapsed);
            let timeout = until_refresh.min(100).max(1);

            if crossterm::event::poll(Duration::from_millis(timeout))? {
                match crossterm::event::read()? {
                    TermEvent::Key(k) => self.handle_key(k)?,
                    TermEvent::Resize(w, h) => {
                        self.rows = (h, w);
                    }
                    _ => {}
                }
            }

            if self.last_refresh.elapsed() >= self.interval {
                self.refresh();
            }
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> std::io::Result<()> {
        if self.show_help {
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?') | KeyCode::Enter => {
                    self.show_help = false;
                }
                _ => {}
            }
            return Ok(());
        }

        match key.code {
            KeyCode::Char('q') | KeyCode::Char('Q') => Err(quit_err()),
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => Err(quit_err()),
            KeyCode::Char('?') | KeyCode::Char('h') | KeyCode::Char('H') => {
                self.show_help = true;
                Ok(())
            }
            KeyCode::Tab | KeyCode::BackTab => {
                self.focus = match self.focus {
                    Focus::Sessions => Focus::Processes,
                    Focus::Processes => Focus::History,
                    Focus::History => Focus::Sessions,
                };
                if self.focus == Focus::Processes {
                    self.view = View::Processes;
                }
                if self.focus == Focus::History {
                    self.view = View::History;
                }
                Ok(())
            }
            KeyCode::Left => {
                self.focus = Focus::Sessions;
                Ok(())
            }
            KeyCode::Right => {
                self.focus = match self.view {
                    View::Processes => Focus::Processes,
                    View::History => Focus::History,
                };
                Ok(())
            }
            KeyCode::Char('1') => {
                self.focus = Focus::Sessions;
                Ok(())
            }
            KeyCode::Char('2') | KeyCode::Char('p') => {
                self.focus = Focus::Processes;
                self.view = View::Processes;
                Ok(())
            }
            KeyCode::Char('3') | KeyCode::Char('t') => {
                self.focus = Focus::History;
                self.view = View::History;
                Ok(())
            }
            KeyCode::Down | KeyCode::Char('j') => {
                match self.focus {
                    Focus::Sessions => {
                        if !self.snap.sessions.is_empty() {
                            self.selected = (self.selected + 1) % self.snap.sessions.len();
                            self.follow = false; // manual choice wins
                            self.proc_sel = 0;
                        }
                    }
                    Focus::Processes => {
                        let n = self.session_proc_len().saturating_sub(1);
                        self.proc_sel = self.proc_sel.saturating_add(1).min(n);
                    }
                    Focus::History => {
                        let n = self.snap.connections.len().saturating_sub(1);
                        self.conn_sel = self.conn_sel.saturating_add(1).min(n);
                    }
                }
                Ok(())
            }
            KeyCode::Up | KeyCode::Char('k') => {
                match self.focus {
                    Focus::Sessions => {
                        if !self.snap.sessions.is_empty() {
                            self.selected = (self.selected + self.snap.sessions.len() - 1)
                                % self.snap.sessions.len();
                            self.follow = false;
                            self.proc_sel = 0;
                        }
                    }
                    Focus::Processes => {
                        self.proc_sel = self.proc_sel.saturating_sub(1);
                    }
                    Focus::History => {
                        self.conn_sel = self.conn_sel.saturating_sub(1);
                    }
                }
                Ok(())
            }
            KeyCode::Char('f') | KeyCode::Char('F') => {
                self.follow = !self.follow;
                Ok(())
            }
            KeyCode::Char('+') | KeyCode::Char('=') => {
                self.interval = Duration::from_millis(
                    (self.interval.as_millis() as u64 + 500).min(30_000),
                );
                Ok(())
            }
            KeyCode::Char('-') | KeyCode::Char('_') => {
                self.interval = self
                    .interval
                    .saturating_sub(Duration::from_millis(500))
                    .max(Duration::from_millis(250));
                Ok(())
            }
            KeyCode::Char(' ') | KeyCode::Char('r') => {
                self.refresh();
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn session_proc_len(&self) -> usize {
        self.snap.procs.iter().filter(|p| p.session_idx == Some(self.selected)).count()
    }
}

fn quit_err() -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::Other, "quit")
}
