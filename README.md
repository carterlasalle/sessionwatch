# sessionwatch

A live TUI for watching what connected SSH and local terminal sessions are
doing: who is logged in, what they are running right now, and every process
they spawn or kill — as it happens.

```
◉ SESSIONWATCH live terminal session monitor [demo]boot 1d 0h 0m  13:44:37  08-04
sessions:5  procs:18  orphans:0  load 0.60 1.30 0.80  0.2s FOLLOW ON  │ 2ssh 1local 2tmux/screen
┌ SESSIONS ─────────────────────────┐┌ PROCESSES   bob@localhost  pts/1  ────────────────┐
│● alice@10.20.30.5  [SSH]  ████▆▆▆▅││PID       USER       CPU%    RSS      ELAPSED  S  COMMAND
│   /dev/pts/0 · up 10m 0s · 5 proc ││▸1011     bob        12.8   19M        6s    S  zsh
│○ bob@localhost  [LOCAL]  ████▇▇▇▇▇││ 1012     bob        12.1   6M         6s    S  cargo build --release
│   /dev/pts/1 · up 25m 0s · 4 proc ││ 1013     bob        11.3   25M        6s    S  docker compose up -d
└───────────────────────────────────┘└────────────────────────────────────────────────────┘
┌ LIVE ACTIVITY ─────────────────────────────────────────────────────────┐
│[SPAWN] alice@pts/0  spawned `screen -ls`   (0s ago)                    │
└────────────────────────────────────────────────────────────────────────┘
 sessionwatch   ↑↓ select session  •  → processes  •  f follow  •  +/- speed  •  h help  •  q quit
```

## Features

- **Session radar** — every logged-in terminal from utmpx, tagged
  `[SSH]` / `[LOCAL]` / `[TMUX]` / `[SCREEN]`, with login time, live process
  count, and a per-session activity sparkline
- **Live process table** — everything running on the selected tty: PID, user,
  CPU%, RSS, elapsed time, state; `▸` marks the newest command the user just
  launched (newest-first so you see what they did last)
- **Activity ticker** — a scrolling feed of every process they spawn and kill,
  plus logins/logouts, in real time
- **Follow mode** — auto-pins the busiest session (toggle with `f`)
- **Demo mode** — synthetic data so the UI runs anywhere, no permissions needed

## Install

### Ubuntu / any Linux

```bash
# 1. Rust toolchain (skip if you already have cargo)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"

# 2. Install
cargo install --git https://github.com/carterlasalle/sessionwatch.git

# 3. Run — watching other users' sessions requires reading their /proc, so use sudo
sudo $(which sessionwatch)
```

Or build from source:

```bash
git clone https://github.com/carterlasalle/sessionwatch.git
cd sessionwatch && cargo build --release
./target/release/sessionwatch
```

### macOS

```bash
brew install rust
cargo install --git https://github.com/carterlasalle/sessionwatch.git
sessionwatch --demo
```

> ⚠️ The live data path is Linux-only (`/proc` + utmpx). On macOS the tool
> automatically runs in **demo mode** with synthetic sessions — the full UI,
> but no real session watching.

## Usage

```
sessionwatch                 live mode (Linux; run as root for all users' visibility)
sessionwatch --demo          synthetic data, no permissions needed
sessionwatch -i 2            refresh every 2 seconds
sessionwatch --help          full option and key reference
```

| Key          | Action                                  |
|--------------|-----------------------------------------|
| `↑` / `↓` / `j` / `k` | move selection (in the focused panel) |
| `←` / `→` / `Tab` | switch between sessions and processes |
| `f`          | toggle FOLLOW — auto-pin the busiest session |
| `space` / `r` | refresh snapshot immediately         |
| `+` / `-`    | speed up / slow down auto-refresh       |
| `h` / `?`    | help overlay                            |
| `q` / `Ctrl-C` | quit                                 |

## How it works

- **Sessions**: parsed directly from `/var/run/utmp` (no `getutxent()` — musl
  ships those as stubs, so this works on glibc *and* musl static builds).
- **Processes**: `/proc/<pid>/stat` is decoded with the kernel `tty_nr` device
  encoding and matched against each session's terminal device, so every process
  is attributed to the tty (and user) running it. CPU% is computed from
  utime/stime deltas between polls.
- **Events**: each refresh diffs the process set and emits spawn/exit events.

## Permissions

Processes of other users are visible only if you can read their
`/proc/<pid>` entries — run as **root** (or an admin with `ptrace_scope`
relaxed) for full visibility. Without it you'll see your own sessions only.

## License

MIT
