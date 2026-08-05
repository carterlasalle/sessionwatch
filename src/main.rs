//! sessionwatch — a live TUI for watching what connected SSH and local
//! terminal sessions are doing: who is logged in, what they are running
//! right now, and every process they spawn or kill.

mod app;
mod collect;
mod model;
mod ui;

use std::io;

use std::time::Duration;

use app::App;
use crossterm::cursor::{Hide, Show};
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;

const USAGE: &str = "\
sessionwatch — watch live SSH & local terminal sessions

USAGE:
    sessionwatch [OPTIONS]

OPTIONS:
    -i, --interval <SECS>   refresh interval in seconds [default: 1.0]
    -h, --help              show this help

REQUIRES LINUX: sessionwatch reads /proc and /var/run/utmp to observe
sessions, so it will not start on macOS or other Unixes.

KEYS (inside the TUI):
    ↑/↓ j/k   move selection        ←/→/Tab  switch panel
    f         toggle FOLLOW (auto-follow the most recently active session)
    + / -     speed up / slow down refresh
    space/r   refresh now           h/?  help
    q / Ctrl-C  quit
";

fn main() -> io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut interval = 1.0f64;

    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--help" | "-h" => {
                print!("{}", USAGE);
                return Ok(());
            }
            "--interval" | "-i" => {
                let v = it.next().ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidInput, "-i needs a value")
                })?;
                interval = v
                    .parse()
                    .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "bad interval"))?;
            }
            other => {
                eprintln!("unknown argument: {other}\n\n{USAGE}");
                std::process::exit(2);
            }
        }
    }

    let interval = Duration::from_secs_f64(interval.max(0.25));

    // The collector requires Linux; fail loudly before touching the terminal.
    let collector =
        collect::new().map_err(|msg| io::Error::new(io::ErrorKind::Unsupported, msg))?;

    // Raw-mode / alternate-screen terminal setup, restored on panic too.
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture, Hide)?;
    std::panic::set_hook(Box::new(|info| {
        let _ = execute!(
            io::stdout(),
            Show,
            DisableMouseCapture,
            LeaveAlternateScreen
        );
        let _ = disable_raw_mode();
        eprintln!("sessionwatch panicked: {info}");
    }));

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = ui::Terminal::new(backend)?;

    let mut app = App::new(collector, interval);

    let result = app.run(&mut terminal);

    // Tear down regardless of how the loop ended.
    let _ = crossterm::execute!(
        io::stdout(),
        Show,
        DisableMouseCapture,
        LeaveAlternateScreen
    );
    let _ = disable_raw_mode();
    let _ = terminal.show_cursor();

    match result {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::Other && e.to_string() == "quit" => Ok(()),
        Err(e) => Err(e),
    }
}
