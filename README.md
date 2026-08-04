# sessionwatch

A live TUI for watching what connected SSH and local terminal sessions are
doing: who is logged in (with their real usernames and tmux/screen session
names), what they are running right now, and every process they spawn or kill
— as it happens.

```
◉ SESSIONWATCH live terminal session monitor [live:/proc]boot 1d 0h 0m  14:14:02  08-04
sessions:5  procs:12  orphans:0  load 0.40 0.70 0.60  0.2s FOLLOW OFF  │ 2ssh 1local 2tmux/screen
┌ SESSIONS ────────────────────────────────┐┌ PROCESSES   jackphelps@10.20.30.5  pts/0  ────────────────────┐
│● jackphelps@10.20.30.5 [SSH]             ││PID     USER       CPU%   RSS     ELAPSED  S  COMMAND          │
│   /dev/pts/0 · up 10m 0s · 3 proc · 900  ││▸1003   jackphelps   3.0  12M        0s    S  git status       │
│○ alice@localhost [LOCAL]                 ││ 1001   jackphelps  22.5  12M        2s    S  vim src/main.rs  │
│   /dev/pts/1 · up 25m 0s · 2 proc · 901  ││ 1002   jackphelps   0.4  12M        8m20s S  bash             │
│○ carol@172.16.8.12 [SSH]                 ││                                                              │
│   /dev/pts/2 · up 40m 0s · 2 proc · 902  ││                                                              │
│○ dev@localhost [TMUX]«cursor-env»        ││                                                              │
│   /dev/pts/3 · up 55m 0s · 3 proc · 903  ││                                                              │
│○ ops@localhost [SCREEN]«deploy-prod»     ││                                                              │
│   /dev/pts/4 · up 1h 10m · 2 proc · 904  ││                                                              │
└──────────────────────────────────────────┘└──────────────────────────────────────────────────────────────┘
┌ LIVE ACTIVITY ─────────────────────────────────────────────────────────────────────────────────────────────┐
│[SPAWN] ops@pts/4  spawned `screen`   (0s ago)                                                             │
└────────────────────────────────────────────────────────────────────────────────────────────────────────────┘
 sessionwatch   ↑↓ select session  •  → processes  •  f follow  •  +/- speed  •  h help  •  q quit
```

## Features

- **Session radar** — every logged-in terminal from utmpx, tagged
  `[SSH]` / `[LOCAL]` / `[TMUX]` / `[SCREEN]`, with real usernames, login time,
  live process count, and a per-session activity sparkline
- **Session names** — tmux/zellij/screen session names are extracted from the
  multiplexer's argv (`tmux attach -t cursor-env` → `«cursor-env»`) and shown
  right on the card
- **Live process table** — everything running on the selected tty: PID, user,
  CPU%, RSS, elapsed time, state; `▸` marks the newest command the user just
  launched (newest-first so you see what they did last)
- **Activity ticker** — a scrolling feed of every process they spawn and kill,
  plus logins/logouts, in real time
- **Follow mode** — press `f` to auto-follow the most recently active session;
  your own navigation always wins and turns follow off

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

sessionwatch is **Linux-only**: it watches `/proc` and `/var/run/utmp`, which
do not exist on macOS. The binary builds fine on a Mac (e.g. to cross-compile
a Linux binary with `cargo build --target x86_64-unknown-linux-musl`) but
refuses to start there. Run it on a Linux box (or an Ubuntu VM/container).

## Usage

```
sessionwatch                 watch live sessions (run as root for full visibility)
sessionwatch -i 2            refresh every 2 seconds
sessionwatch --help          full option and key reference
```

| Key          | Action                                  |
|--------------|-----------------------------------------|
| `↑` / `↓` / `j` / `k` | move selection (in the focused panel) |
| `←` / `→` / `Tab` | switch between sessions and processes |
| `f`          | toggle FOLLOW — auto-follow the most recently active session |
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
- **Session names**: tmux/zellij/screen session names are parsed from the
  client processes' argv on each tty (`-s`/`-t`/`-S` flags, `zellij attach <n>`).
- **Events**: each refresh diffs the process set and emits spawn/exit events.

## Permissions

Processes of other users are visible only if you can read their
`/proc/<pid>` entries — run as **root** (or an admin with `ptrace_scope`
relaxed) for full visibility. Without it you'll see your own sessions only.

## License

MIT
