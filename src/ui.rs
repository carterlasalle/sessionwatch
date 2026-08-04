//! Rendering: header, session cards, live process table, activity ticker,
//! status bar, and help overlay.

use std::io::Stdout;

use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Cell, List, ListItem, ListState, Paragraph, Row, Table, TableState,
};
use ratatui::Frame;

use crate::app::{App, Focus, HistoryItem, View};
use crate::model::{Proc, SessionKind};

pub type Terminal = ratatui::Terminal<CrosstermBackend<Stdout>>;

const ACCENT: Color = Color::Cyan;
const DIM: Color = Color::DarkGray;
const GOOD: Color = Color::Green;
const BAD: Color = Color::Red;
const WARN: Color = Color::Yellow;
const INFO: Color = Color::Blue;
const SEL_BG: Color = Color::from_u32(0x1a1a2e);

fn kind_color(k: SessionKind) -> Color {
    match k {
        SessionKind::Ssh => Color::Cyan,
        SessionKind::Local => Color::Green,
        SessionKind::Tmux => Color::Magenta,
        SessionKind::Screen => Color::Yellow,
    }
}

pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4), // header
            Constraint::Min(6),    // body
            Constraint::Length(3), // ticker
            Constraint::Length(1), // statusbar
        ])
        .split(area);

    draw_header(f, chunks[0], app);
    draw_body(f, chunks[1], app);
    draw_ticker(f, chunks[2], app);
    draw_status(f, chunks[3], app);

    if app.show_help {
        draw_help(f, area);
    }
}

// ---- header ---------------------------------------------------------------

fn draw_header(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::default().borders(Borders::BOTTOM).border_style(Style::default().fg(DIM));
    let a = block.inner(area);
    f.render_widget(block, area);

    let title = Line::from(vec![
        Span::styled("◉ SESSIONWATCH", Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)),
        Span::styled(" live terminal session monitor", Style::default().fg(DIM)),
        Span::styled(
            format!(" [{}]", app.collector.source()),
            Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC),
        ),
    ]);
    let right = Line::from(vec![
        Span::styled(
            format!("boot {}  ", fmt_duration(app.snap.taken_at - app.snap.boot_unix)),
            Style::default().fg(Color::Gray),
        ),
        Span::styled(fmt_clock(app.snap.taken_at), Style::default().fg(Color::White)),
    ]);
    f.render_widget(Paragraph::new(two_col(title, right)), a);

    let (mut ssh, mut local, mut tmux) = (0usize, 0usize, 0usize);
    for s in &app.snap.sessions {
        match s.kind {
            SessionKind::Ssh => ssh += 1,
            SessionKind::Local => local += 1,
            _ => tmux += 1,
        }
    }
    let stats = Line::from(vec![
        Span::styled(
            format!("sessions:{}  procs:{}  orphans:{}  ", app.snap.sessions.len(), app.snap.procs.len(), app.snap.orphans.len()),
            Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("load {:.2} {:.2} {:.2}  ", app.snap.load[0], app.snap.load[1], app.snap.load[2]), Style::default().fg(Color::Gray)),
        Span::styled(format!("{:.1}s ", app.interval.as_secs_f64()), Style::default().fg(Color::Gray)),
        Span::styled(
            if app.follow { "FOLLOW ON" } else { "FOLLOW OFF" },
            Style::default().fg(if app.follow { GOOD } else { WARN }).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  │ {}ssh {}local {}tmux/screen", ssh, local, tmux),
            Style::default().fg(DIM),
        ),
    ]);
    let a2 = Rect { x: a.x, y: a.y + 1, width: a.width, height: a.height.saturating_sub(2).max(1) };
    f.render_widget(Paragraph::new(stats), a2);
}

/// Combine a left label and a right-aligned label into one padded line.
fn two_col(left: Line<'static>, right: Line<'static>) -> Line<'static> {
    let lw = left.width() as usize;
    let rw = right.width() as usize;
    // pad so the right label hugs the right edge (~74 cols)
    let pad = 74usize.saturating_sub(lw + rw);
    let mut out = left;
    if pad > 0 {
        out.spans.push(Span::raw(" ".repeat(pad)));
    }
    for s in right.spans {
        out.spans.push(s);
    }
    out
}

// ---- body -----------------------------------------------------------------

fn draw_body(f: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(area);
    draw_sessions(f, chunks[0], app);
    match app.view {
        View::Processes => draw_processes(f, chunks[1], app),
        View::History => draw_history(f, chunks[1], app),
        View::Detail => draw_detail(f, chunks[1], app),
    }
}

fn draw_sessions(f: &mut Frame, area: Rect, app: &App) {
    let focused = app.focus == Focus::Sessions;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(if focused { ACCENT } else { DIM }))
        .title(Line::from(vec![Span::styled(" SESSIONS ", style_title(focused))]));

    let mut items: Vec<ListItem> = Vec::new();
    for (i, s) in app.snap.sessions.iter().enumerate() {
        let selected = i == app.selected;
        let pulse = if selected && (app.phase / 40) % 2 == 0 { "●" } else { "○" };
        let nprocs = app.snap.procs.iter().filter(|p| p.session_idx == Some(i)).count();
        let ago = fmt_duration(app.snap.taken_at.saturating_sub(s.login_unix));
        let sp = sparkline(&app.history[i], 20);

        let head = Line::from(vec![
            Span::styled(pulse, Style::default().fg(if selected { GOOD } else { DIM })),
            Span::raw(" "),
            Span::styled(&s.user, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            Span::styled("@", Style::default().fg(DIM)),
            Span::styled(
                s.identity.as_deref().unwrap_or(&s.host),
                Style::default().fg(if s.identity.is_some() { Color::Magenta } else { Color::Gray }),
            ),
            Span::raw(" "),
            Span::styled(format!("[{}]", s.kind.label()), Style::default().fg(kind_color(s.kind)).add_modifier(Modifier::BOLD)),
        ]);

        // session name chip (tmux/screen/zellij session names)
        let mut name_line = head;
        if let Some(name) = &s.name {
            let nstyle = if selected {
                kind_color(s.kind)
            } else {
                Color::DarkGray
            };
            name_line.spans.splice(7..7, vec![
                Span::styled(format!("«{}»", name), Style::default().fg(nstyle).add_modifier(Modifier::BOLD | Modifier::ITALIC)),
            ]);
        }

        let detail = Line::from(vec![
            Span::styled("   ", Style::default()),
            Span::styled(&s.device, Style::default().fg(Color::Cyan)),
            Span::styled(" · up ", Style::default().fg(DIM)),
            Span::styled(ago, Style::default().fg(Color::Gray)),
            Span::styled(" · ", Style::default().fg(DIM)),
            Span::styled(format!("{} proc", nprocs), Style::default().fg(if selected { GOOD } else { Color::Gray })),
            Span::styled(" · pid ", Style::default().fg(DIM)),
            Span::styled(s.pid.to_string(), Style::default().fg(DIM)),
            Span::styled("  ", Style::default()),
            Span::styled(sp, Style::default().fg(if selected { kind_color(s.kind) } else { Color::DarkGray })),
        ]);

        items.push(ListItem::new(vec![name_line, detail]));
    }
    if items.is_empty() {
        items.push(ListItem::new(Line::from(Span::styled("no login sessions", Style::default().fg(DIM)))));
    }

    let mut state = ListState::default();
    state.select(if app.snap.sessions.is_empty() { None } else { Some(app.selected) });

    let list = List::new(items)
        .block(block)
        .highlight_style(Style::default().bg(SEL_BG).add_modifier(Modifier::BOLD));
    f.render_stateful_widget(list, area, &mut state);
}

fn draw_processes(f: &mut Frame, area: Rect, app: &App) {
    let focused = app.focus == Focus::Processes;
    let header_text = match app.snap.sessions.get(app.selected) {
        Some(s) => {
            let name = s
                .name
                .as_ref()
                .map(|n| format!(" «{n}»"))
                .unwrap_or_default();
            format!("  {}@{}{}  {}  ", s.user, s.host, name, s.line)
        }
        None => "  no session selected  ".to_string(),
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(if focused { ACCENT } else { DIM }))
        .title(Line::from(vec![
            Span::styled(" PROCESSES ", style_title(focused)),
            Span::styled(header_text, Style::default().fg(Color::Gray)),
        ]));

    let mut procs: Vec<&Proc> = app
        .snap
        .procs
        .iter()
        .filter(|p| p.session_idx == Some(app.selected))
        .collect();
    procs.sort_by(|a, b| b.start_unix.partial_cmp(&a.start_unix).unwrap_or(std::cmp::Ordering::Equal));

    let header = Row::new(vec![
        Cell::from("PID"),
        Cell::from("USER"),
        Cell::from("CPU%"),
        Cell::from("RSS"),
        Cell::from("ELAPSED"),
        Cell::from("S"),
        Cell::from("COMMAND"),
    ])
    .style(Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD));

    let mut rows: Vec<Row> = Vec::new();
    let newest_cmd: Option<u32> = procs.first().map(|p| p.pid);
    for p in &procs {
        let is_newest = newest_cmd == Some(p.pid);
        let marker = if is_newest { "▸" } else { " " };
        let state_color = match p.state {
            'R' => GOOD,
            'S' => INFO,
            'Z' => BAD,
            'T' => WARN,
            _ => Color::Gray,
        };
        let cpu_style = Style::default().fg(if p.cpu_pct > 50.0 { BAD } else if p.cpu_pct > 15.0 { WARN } else { Color::Gray });
        let cmd_style = if is_newest {
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        rows.push(Row::new(vec![
            Cell::from(format!("{}{}", marker, p.pid)).style(Style::default().fg(Color::Gray)),
            Cell::from(p.user.clone()).style(Style::default().fg(Color::Cyan)),
            Cell::from(cpu_fmt(p.cpu_pct)).style(cpu_style),
            Cell::from(fmt_rss(p.rss_kb)).style(Style::default().fg(Color::Gray)),
            Cell::from(fmt_elapsed(p.elapsed)).style(Style::default().fg(Color::Gray)),
            Cell::from(p.state.to_string()).style(Style::default().fg(state_color).add_modifier(Modifier::BOLD)),
            Cell::from(p.cmdline.clone()).style(cmd_style),
        ]));
    }
    if rows.is_empty() {
        rows.push(Row::new(vec![Cell::from("(no processes on this terminal)").style(Style::default().fg(DIM))]));
    }

    let mut state = TableState::default();
    state.select(Some(app.proc_sel.min(procs.len().saturating_sub(1))));

    let table = Table::new(rows, [
        Constraint::Length(7),
        Constraint::Length(10),
        Constraint::Length(6),
        Constraint::Length(7),
        Constraint::Length(8),
        Constraint::Length(2),
        Constraint::Min(12),
    ])
    .column_spacing(0)
    .header(header)
    .block(block)
    .row_highlight_style(Style::default().bg(SEL_BG));

    f.render_stateful_widget(table, area, &mut state);
}

/// Drill-down: the full command timeline of one connection, as observed live
/// journal, with Tailscale-resolved hosts and observed tmux/screen session
/// names) plus failed login attempts from btmp.
fn draw_history(f: &mut Frame, area: Rect, app: &App) {
    let focused = app.focus == Focus::History;
    let live = app.snap.connections.iter().filter(|c| c.is_live()).count();
    let total = app.snap.connections.len();
    let failed_n = app.snap.failed.len();
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(if focused { ACCENT } else { DIM }))
        .title(Line::from(vec![
            Span::styled(" CONNECTION HISTORY ", style_title(focused)),
            Span::styled(
                format!("{} sessions · {} live · {} failed", total, live, failed_n),
                Style::default().fg(Color::Gray),
            ),
        ]));

    // combined timeline: connections + failed attempts, newest first
    let rows: Vec<(i64, HistoryItem)> = app.history_rows();

    let header = Row::new(vec![
        Cell::from("USER"),
        Cell::from("FROM"),
        Cell::from("SESSION"),
        Cell::from("LOGIN"),
        Cell::from("DURATION"),
    ])
    .style(Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD));

    let now = app.snap.taken_at;
    let mut table_rows: Vec<Row> = Vec::new();
    for (_, r) in &rows {
        match r {
            HistoryItem::Conn(idx) => {
                let c = &app.snap.connections[*idx];
                let live = c.is_live();
                let marker = if live { "●" } else { " " };
                let from = c.identity.as_deref().unwrap_or(&c.host);
                let from_style = if live {
                    Style::default().fg(Color::Cyan)
                } else {
                    Style::default().fg(if c.identity.is_some() || c.kind == SessionKind::Ssh { Color::Cyan } else { Color::DarkGray })
                };
                let session_span = match &c.name {
                    Some(n) => Span::styled(
                        format!("«{}»", n),
                        Style::default().fg(Color::Magenta).add_modifier(Modifier::ITALIC),
                    ),
                    None => Span::raw(""),
                };
                let duration = if live {
                    Span::styled("live", Style::default().fg(GOOD).add_modifier(Modifier::BOLD))
                } else {
                    let secs = c.logout_unix.unwrap_or(now).saturating_sub(c.login_unix).max(0);
                    Span::styled(fmt_duration(secs), Style::default().fg(Color::Gray))
                };
                table_rows.push(Row::new(vec![
                    Cell::from(format!("{}{}", marker, c.user)).style(if live {
                        Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::Gray)
                    }),
                    Cell::from(from.to_string()).style(from_style),
                    Cell::from(Line::from(session_span)),
                    Cell::from(fmt_hms(c.login_unix)).style(Style::default().fg(Color::Gray)),
                    Cell::from(Line::from(duration)),
                ]));
            }
            HistoryItem::Failed(fi) => {
                let f = &app.snap.failed[*fi];
                table_rows.push(Row::new(vec![
                    Cell::from(format!("✗{}", f.user)).style(Style::default().fg(BAD)),
                    Cell::from(f.host.clone()).style(Style::default().fg(BAD)),
                    Cell::from(f.line.clone()).style(Style::default().fg(Color::DarkGray)),
                    Cell::from(fmt_hms(f.at)).style(Style::default().fg(Color::DarkGray)),
                    Cell::from("FAILED").style(Style::default().fg(BAD).add_modifier(Modifier::BOLD)),
                ]));
            }
        }
    }
    if table_rows.is_empty() {
        table_rows.push(Row::new(vec![Cell::from("(no history — /var/log/wtmp and /var/log/btmp empty or unreadable)").style(Style::default().fg(DIM))]));
    }

    let mut state = TableState::default();
    state.select(Some(app.conn_sel.min(table_rows.len().saturating_sub(1))));

    let table = Table::new(table_rows, [
        Constraint::Length(9),
        Constraint::Length(24),
        Constraint::Length(12),
        Constraint::Length(9),
        Constraint::Length(10),
    ])
    .column_spacing(0)
    .header(header)
    .block(block)
    .row_highlight_style(Style::default().bg(SEL_BG));

    f.render_stateful_widget(table, area, &mut state);
}

/// Drill-down: the full command timeline of one connection, as observed live
/// by sessionwatch (reconstructed from the journal).
fn draw_detail(f: &mut Frame, area: Rect, app: &App) {
    let focused = app.focus == Focus::History;
    let border = if focused { ACCENT } else { DIM };
    let Some(idx) = app.detail else { return };
    let Some(c) = app.snap.connections.get(idx) else { return };

    let who = c.identity.as_deref().unwrap_or(&c.host);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border))
        .title(Line::from(vec![
            Span::styled(" SESSION DETAIL ", style_title(focused)),
            Span::styled(format!("{}@{}  {}  ", c.user, who, c.line), Style::default().fg(Color::Gray)),
        ]));

    let mut lines: Vec<Line> = Vec::new();
    let meta1 = Line::from(vec![
        Span::styled(&c.user, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        Span::styled("@", Style::default().fg(DIM)),
        Span::styled(who, Style::default().fg(if c.identity.is_some() { Color::Magenta } else { Color::Cyan })),
        Span::styled("  ", Style::default()),
        Span::styled(format!("[{}]", c.kind.label()), Style::default().fg(kind_color(c.kind)).add_modifier(Modifier::BOLD)),
        Span::styled("  ·  ", Style::default().fg(DIM)),
        Span::styled("pid ", Style::default().fg(DIM)),
        Span::styled(c.pid.to_string(), Style::default().fg(Color::Gray)),
        match &c.name {
            Some(n) => Span::styled(
                format!("  «{}»", n),
                Style::default().fg(Color::Magenta).add_modifier(Modifier::ITALIC),
            ),
            None => Span::raw(""),
        },
    ]);
    let (dur_text, dur_style) = match c.logout_unix {
        Some(l) => (
            format!(
                "{}  →  {}  ({})",
                fmt_hms(c.login_unix),
                fmt_hms(l),
                fmt_duration(l.saturating_sub(c.login_unix).max(0))
            ),
            Style::default().fg(Color::Gray),
        ),
        None => (
            format!("{}  →  now  (live)", fmt_hms(c.login_unix)),
            Style::default().fg(GOOD).add_modifier(Modifier::BOLD),
        ),
    };
    let meta2 = Line::from(vec![
        Span::styled("  connected ", Style::default().fg(DIM)),
        Span::styled(dur_text, dur_style),
    ]);
    lines.push(meta1);
    lines.push(meta2);
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled(" COMMANDS (as observed live) ", Style::default().fg(DIM).add_modifier(Modifier::BOLD)),
        Span::styled(format!("{} recorded", c.commands.len()), Style::default().fg(Color::DarkGray)),
    ]));
    lines.push(Line::from(""));
    for (i, (t, cmd)) in c.commands.iter().enumerate() {
        let selected = i == app.detail_sel;
        lines.push(Line::from(vec![
            Span::styled(
                format!(" {}  ", fmt_hms(*t)),
                Style::default().fg(if selected { Color::White } else { Color::DarkGray }),
            ),
            Span::styled(
                cmd.clone(),
                Style::default().fg(if selected { ACCENT } else { Color::White })
                    .add_modifier(if selected { Modifier::BOLD } else { Modifier::empty() }),
            ),
        ]));
    }
    if c.commands.is_empty() {
        lines.push(Line::from(Span::styled(
            " (no commands recorded — sessionwatch wasn't watching this session yet)",
            Style::default().fg(DIM),
        )));
    }

    // scroll so the selection stays in view
    let inner_h = block.inner(area).height as usize;
    let scroll = app.detail_sel.saturating_sub(inner_h / 2) as u16;
    f.render_widget(Paragraph::new(lines).block(block).scroll((scroll, 0)), area);
}

// ---- ticker ---------------------------------------------------------------

fn draw_ticker(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(DIM))
        .title(Line::from(vec![Span::styled(" LIVE ACTIVITY ", style_title(false))]));

    let max_rows = block.inner(area).height as usize;
    let mut lines: Vec<Line> = Vec::new();
    for e in app.events.iter() {
        let ago = (app.snap.taken_at.saturating_sub(e.at)).max(0);
        let color = match e.kind {
            "spawn" => GOOD,
            "exit" => BAD,
            _ => INFO,
        };
        lines.push(Line::from(vec![
            Span::styled(format!("[{:>5}]", e.kind.to_uppercase()), Style::default().fg(color).add_modifier(Modifier::BOLD)),
            Span::raw(" "),
            Span::styled(&e.user, Style::default().fg(Color::Cyan)),
            Span::styled(format!("@{}", e.session_line), Style::default().fg(DIM)),
            Span::styled("  ", Style::default()),
            Span::styled(e.text.clone(), Style::default().fg(Color::White)),
            Span::styled(format!("   ({}s ago)", ago), Style::default().fg(DIM)),
        ]));
    }
    if lines.is_empty() {
        lines.push(Line::from(Span::styled("no activity observed yet", Style::default().fg(DIM))));
    }
    // Keep newest at the bottom; trim from the top.
    if lines.len() > max_rows {
        let drop = lines.len() - max_rows;
        lines.drain(0..drop);
    }

    f.render_widget(Paragraph::new(lines).block(block), area);
}

// ---- status bar -----------------------------------------------------------

fn draw_status(f: &mut Frame, area: Rect, app: &App) {
    let hints = if app.view == View::Detail {
        "↑↓ scroll commands · Enter/Esc back to history · q quit"
    } else {
        match app.focus {
            Focus::Sessions => {
                "1/2/3 views · ↑↓ select session · f follow · +/- speed · h help · q quit"
            }
            Focus::Processes => "↑↓ scroll processes · ← sessions · 2 processes · 3 history · space refresh · q quit",
            Focus::History => "↑↓ scroll history · Enter drill-down · ← sessions · 2/3 views · q quit",
        }
    };
    let line = Line::from(vec![
        Span::styled(" sessionwatch ", Style::default().fg(Color::Black).bg(ACCENT).add_modifier(Modifier::BOLD)),
        Span::styled(format!("  {}", hints), Style::default().fg(Color::Gray)),
    ]);
    f.render_widget(Paragraph::new(line), area);
}

// ---- help overlay ---------------------------------------------------------

fn draw_help(f: &mut Frame, area: Rect) {
    let w = (area.width.saturating_mul(2) / 3).max(40);
    let h = 24.min(area.height.saturating_sub(2)).max(12);
    let x = area.x + area.width.saturating_sub(w) / 2;
    let y = area.y + area.height.saturating_sub(h) / 2;
    let box_area = Rect { x, y, width: w, height: h };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(ACCENT))
        .title(Line::from(vec![Span::styled(" KEYS ", Style::default().fg(ACCENT).add_modifier(Modifier::BOLD))]))
        .title_alignment(Alignment::Center)
        .style(Style::default().bg(Color::from_u32(0x12121c)));

    let rows = [
        ("1 / 2 / 3", "switch views: sessions / live processes / connection history"),
        ("↑/↓ / j/k", "move selection (in focused panel)"),
        ("← / → / Tab", "switch focus between sessions and the right panel"),
        ("Enter", "in HISTORY: drill into a connection's command timeline"),
        ("Esc", "back from the session detail view"),
        ("f", "toggle FOLLOW — auto-follow the most recently active session"),
        ("space / r", "refresh snapshot immediately"),
        ("+ / -", "speed up / slow down auto-refresh"),
        ("h / ?", "this help"),
        ("q / Ctrl-C", "quit"),
    ];
    let mut lines = vec![Line::from(Span::styled("  What am I looking at?", Style::default().fg(DIM).add_modifier(Modifier::BOLD)))];
    lines.push(Line::from(Span::styled(
        "  SESSIONS: every logged-in terminal (ssh/local/tmux/screen); the",
        Style::default().fg(Color::Gray),
    )));
    lines.push(Line::from(Span::styled(
        "  sparkline shows how busy each tty has been.",
        Style::default().fg(Color::Gray),
    )));
    lines.push(Line::from(Span::styled(
        "  PROCESSES: everything currently running on the selected tty; ▸ marks",
        Style::default().fg(Color::Gray),
    )));
    lines.push(Line::from(Span::styled(
        "  the newest command the user launched. The LIVE ACTIVITY feed below",
        Style::default().fg(Color::Gray),
    )));
    lines.push(Line::from(Span::styled(
        "  logs every process they spawn and kill, and every login/logout.",
        Style::default().fg(Color::Gray),
    )));
    lines.push(Line::from(Span::styled(
        "  HISTORY: previous connections reconstructed from wtmp + sessionwatch's",
        Style::default().fg(Color::Gray),
    )));
    lines.push(Line::from(Span::styled(
        "  journal. Tailscale sessions (which never hit wtmp) appear with the",
        Style::default().fg(Color::Gray),
    )));
    lines.push(Line::from(Span::styled(
        "  person's tailnet email, and Enter shows the commands they ran.",
        Style::default().fg(Color::Gray),
    )));
    lines.push(Line::from(""));
    for (k, d) in rows {
        lines.push(Line::from(vec![
            Span::styled(format!("  {:<18}", k), Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)),
            Span::styled(d, Style::default().fg(Color::White)),
        ]));
    }

    f.render_widget(Paragraph::new(lines).block(block), box_area);
}

// ---- helpers --------------------------------------------------------------

fn style_title(focused: bool) -> Style {
    if focused {
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Gray)
    }
}

fn cpu_fmt(cpu: f64) -> String {
    format!("{:>5.1}", cpu.min(999.9))
}

fn fmt_rss(kb: u64) -> String {
    if kb >= 1024 * 1024 {
        format!("{:.1}G", kb as f64 / 1048576.0)
    } else if kb >= 1024 {
        format!("{:.0}M", kb as f64 / 1024.0)
    } else {
        format!("{}K", kb)
    }
}

fn fmt_elapsed(secs: f64) -> String {
    let s = secs as u64;
    if s >= 3600 {
        format!("{:>2}:{:02}:{:02}", s / 3600, (s % 3600) / 60, s % 60)
    } else if s >= 60 {
        format!("{:>4}m{:02}s", s / 60, s % 60)
    } else {
        format!("{:>4}s", s)
    }
}

fn fmt_duration(secs: i64) -> String {
    let s = secs.max(0) as u64;
    let (d, h, m) = (s / 86400, (s % 86400) / 3600, (s % 3600) / 60);
    if d > 0 {
        format!("{}d {}h {}m", d, h, m)
    } else if h > 0 {
        format!("{}h {}m", h, m)
    } else {
        format!("{}m {}s", m, s % 60)
    }
}

#[allow(deprecated)] // libc::time_t width differs across musl targets
fn fmt_clock(unix: i64) -> String {
    let t: libc::time_t = unix;
    let mut tm = unsafe { std::mem::zeroed::<libc::tm>() };
    unsafe {
        libc::localtime_r(&t, &mut tm);
    }
    format!(
        "{:02}:{:02}:{:02}  {:02}-{:02}",
        tm.tm_hour, tm.tm_min, tm.tm_sec, tm.tm_mon + 1, tm.tm_mday
    )
}

/// Local time-of-day only (for the history LOGIN column).
#[allow(deprecated)] // libc::time_t width differs across musl targets
fn fmt_hms(unix: i64) -> String {
    let t: libc::time_t = unix;
    let mut tm = unsafe { std::mem::zeroed::<libc::tm>() };
    unsafe {
        libc::localtime_r(&t, &mut tm);
    }
    format!("{:02}:{:02}:{:02}", tm.tm_hour, tm.tm_min, tm.tm_sec)
}

fn sparkline(data: &[f64], width: usize) -> String {
    if data.is_empty() || width == 0 {
        return String::new();
    }
    let ramp = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let max = data.iter().cloned().fold(f64::MIN, f64::max).max(1.0);
    let mut out = String::with_capacity(width);
    let n = data.len();
    for i in 0..width {
        let lo = i * n / width;
        let hi = ((i + 1) * n / width).max(lo + 1).min(n);
        let v = data[lo..hi].iter().cloned().fold(f64::MIN, f64::max);
        let idx = ((v / max) * (ramp.len() as f64 - 1.0)).round() as usize;
        out.push(ramp[idx.min(ramp.len() - 1)]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use crate::collect;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal as RatTerminal;
    use std::time::Duration;

    /// Render one full frame to a text buffer and dump it to
    /// `target/ui-snapshot.txt` for eyeballing.
    fn render_frame(w: u16, h: u16) -> String {
        let mut app = App::new(collect::test_collector(), Duration::from_millis(250));
        for _ in 0..5 {
            app.refresh();
        }
        let backend = TestBackend::new(w, h);
        let mut term = RatTerminal::new(backend).unwrap();
        term.draw(|f| draw(f, &mut app)).unwrap();
        let buf = term.backend().buffer();
        let mut out = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn renders_full_dashboard() {
        let text = render_frame(110, 30);
        std::fs::write("target/ui-snapshot.txt", &text).ok();
        // header
        assert!(text.contains("SESSIONWATCH"), "title missing");
        assert!(text.contains("SESSIONS"), "sessions panel missing");
        assert!(text.contains("PROCESSES"), "processes panel missing");
        assert!(text.contains("LIVE ACTIVITY"), "ticker missing");
        // sessions present
        for name in ["jackphelps", "alice", "carol", "dev", "ops"] {
            assert!(text.contains(name), "session {name} missing");
        }
        // session kind badges + named tmux/screen sessions
        assert!(text.contains("[SSH]"), "ssh badge missing");
        assert!(text.contains("[TMUX]"), "tmux badge missing");
        assert!(text.contains("[SCREEN]"), "screen badge missing");
        assert!(text.contains("cursor-env"), "tmux session name missing");
        assert!(text.contains("deploy-prod"), "screen session name missing");
        // process table headers
        assert!(text.contains("PID"), "PID column missing");
        assert!(text.contains("CPU%"), "CPU column missing");
        assert!(text.contains("COMMAND"), "command column missing");
        // processes visible from the fixture
        assert!(text.contains("vim src/main.rs"), "process row missing");
        // status bar hints
        assert!(text.contains("q quit"), "status hints missing");
    }

    #[test]
    fn history_view_renders() {
        let mut app = App::new(collect::test_collector(), Duration::from_millis(250));
        app.refresh();
        app.view = View::History;
        app.focus = Focus::History;
        let backend = TestBackend::new(110, 30);
        let mut term = RatTerminal::new(backend).unwrap();
        term.draw(|f| draw(f, &mut app)).unwrap();
        let text: String = term
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        std::fs::write("target/ui-history-snapshot.txt", &text).ok();
        assert!(text.contains("CONNECTION HISTORY"), "history panel missing");
        assert!(text.contains("5 sessions"), "session count missing");
        assert!(text.contains("2 live"), "live count missing");
        assert!(text.contains("2 failed"), "failed count missing");
        // tailscale identity email and session name from the journal
        assert!(text.contains("jackphelps20@gmail.com"), "tailscale identity missing");
        assert!(text.contains("«cursor-env»"), "journal session name missing");
        // a past connection with a duration
        assert!(text.contains("10.20.30.5"), "from-host missing");
        assert!(text.contains("live"), "live marker missing");
        assert!(text.contains("1h 0m"), "duration missing");
        // failed login rows
        assert!(text.contains("FAILED"), "failed marker missing");
        assert!(text.contains("203.0.113.7"), "failed from-host missing");
    }

    #[test]
    fn session_detail_renders_commands() {
        let mut app = App::new(collect::test_collector(), Duration::from_millis(250));
        app.refresh();
        // find the dev connection (index with the cursor-env commands)
        let idx = app
            .snap
            .connections
            .iter()
            .position(|c| c.name.as_deref() == Some("cursor-env"))
            .expect("fixture connection");
        app.view = View::Detail;
        app.focus = Focus::History;
        app.detail = Some(idx);
        let backend = TestBackend::new(110, 30);
        let mut term = RatTerminal::new(backend).unwrap();
        term.draw(|f| draw(f, &mut app)).unwrap();
        let text: String = term
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        std::fs::write("target/ui-detail-snapshot.txt", &text).ok();
        assert!(text.contains("SESSION DETAIL"), "detail panel missing");
        assert!(text.contains("jackphelps20@gmail.com"), "identity missing in detail");
        assert!(text.contains("cargo build --release"), "command timeline missing");
        assert!(text.contains("tegrastats --interval 1000"), "later command missing");
        assert!(text.contains("connected"), "connection window missing");
    }

    #[test]
    fn help_overlay_renders() {
        let mut app = App::new(collect::test_collector(), Duration::from_millis(250));
        app.refresh();
        app.show_help = true;
        let backend = TestBackend::new(90, 24);
        let mut term = RatTerminal::new(backend).unwrap();
        term.draw(|f| draw(f, &mut app)).unwrap();
        let text: String = term
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(text.contains("KEYS"), "help title missing");
        assert!(text.contains("FOLLOW"), "help body missing");
    }
}
