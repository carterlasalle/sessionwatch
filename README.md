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

Press `3` for the connection-history view — every previous connection
reconstructed from `/var/log/wtmp` + sessionwatch's own journal. Tailscale
sessions carry the person's real identity email; failed login attempts from
`/var/log/btmp` show up red; `Enter` drills into a connection's command
timeline:

```
┌ CONNECTION HISTORY 5 sessions · 2 live · 2 failed ──────────────────────────────────────────────────────────┐
│USER     FROM                    SESSION     LOGIN    DURATION                                             │
│●jackpheljackphelps20@gmail.com             15:07:14 live                                                 │
│✗root    203.0.113.7             ssh:notty   15:03:54 FAILED                                               │
│✗root    100.64.0.9              ssh:notty   14:52:14 FAILED                                               │
│ dev     jackphelps20@gmail.com  «cursor-env»09:38:54 1h 23m                                              │
└───────────────────────────────────────────────────────────────────────────────────────────────────────────┘

┌ SESSION DETAIL  dev@jackphelps20@gmail.com  pts/3  ────────────────────────────────────────────────────────┐
│dev@jackphelps20@gmail.com  [SSH]  ·  pid 904  «cursor-env»                                                 │
│  connected 09:38:54  →  11:02:14  (1h 23m)                                                                  │
│ COMMANDS (as observed live) 4 recorded                                                                      │
│ 09:47:14  cargo build --release                                                                             │
│ 09:57:14  vim configs/trtllm-orin.yaml                                                                      │
│ 10:10:34  docker compose up -d                                                                              │
│ 10:22:14  tegrastats --interval 1000                                                                        │
└─────────────────────────────────────────────────────────────────────────────────────────────────────────────┘
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
- **Connection history** — press `3` to reconstruct every previous connection
  from `/var/log/wtmp` **plus sessionwatch's own journal**: who connected
  from where, when, and for how long — even after they've disconnected.
  **Tailscale sessions** (which never appear in wtmp) show the person's real
  tailnet email, tmux/screen session names survive in history, **failed login
  attempts** from `/var/log/btmp` appear as red `FAILED` rows, and pressing
  `Enter` drills into any connection to show the **timeline of commands they
  ran** — observed live by sessionwatch, plus the user's **shell history**
  (bash/zsh/fish) for connections that ended before sessionwatch was watching.
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

## Updating

```bash
cargo install --git https://github.com/carterlasalle/sessionwatch.git --force
```

`--force` is required because the crate version doesn't change between
releases — cargo install would otherwise skip an already-installed binary.
The new build lands in `~/.cargo/bin/sessionwatch` in place, so just:

1. quit any running instance (`q` or Ctrl-C),
2. run the update command above,
3. start it again (`sudo $(which sessionwatch)` for full visibility).

## Usage

```
sessionwatch                 watch live sessions (run as root for full visibility)
sessionwatch -i 2            refresh every 2 seconds
sessionwatch --help          full option and key reference
```

| Key          | Action                                  |
|--------------|-----------------------------------------|
| `1` / `2` / `3` | switch views: sessions / live processes / connection history |
| `↑` / `↓` / `j` / `k` | move selection (in the focused panel) |
| `←` / `→` / `Tab` | switch focus between sessions and the right panel |
| `Enter`       | in HISTORY: drill into a connection's command timeline |
| `Esc`         | back from the session detail view                     |
| `f`          | toggle FOLLOW — auto-follow the most recently active session |
| `space` / `r` | refresh snapshot immediately         |
| `+` / `-`    | speed up / slow down auto-refresh       |
| `h` / `?`    | help overlay                            |
| `q` / `Ctrl-C` | quit                                 |

## How it works

- **Sessions**: parsed directly from `/var/run/utmp` (no `getutxent()` — musl
  ships those as stubs, so this works on glibc *and* musl static builds).
- **History**: `/var/log/wtmp` is read incrementally (only the appended tail
  plus a small context window per refresh, so it stays cheap even on
  year-old files). `USER_PROCESS` records open a connection, `DEAD_PROCESS`
  records on the same tty close it. On top of that, sessionwatch keeps its
  **own journal** (`/var/log/sessionwatch/history.log` as root, else under
  `~/.local/state`) recording every session it observes live — with the
  tmux/screen session name and the resolved host — so history stays rich even
  if wtmp is rotated away. **Failed logins** come from `/var/log/btmp`.
- **Tailscale**: hosts in the CGNAT range (`100.64.0.0/10`) are resolved to
  real tailnet machine names via `tailscale status` (cached 60s, best-effort
  — if the CLI is missing the raw address is shown). The **person's identity
  email** (e.g. `jackphelps20@gmail.com`) is recovered from the Tailscale
  sshd child processes, which carry `--remote-user=` / `--remote-ip=` in
  their argv — that's how a session gets attributed to a real person even
  though Tailscale SSH never writes a normal utmp entry.
- **Command journal**: every process sessionwatch observes spawning on a
  session's tty is appended to the journal (`P` records), so the history
  drill-down can replay what each connection ran, in order, with timestamps.
  For connections that predate sessionwatch, the drill-down also shows the
  user's **shell history** (`~/.bash_history` in both plain and
  HISTTIMEFORMAT forms, `~/.zsh_history`, `~/.fish_history`), windowed to the
  connection's lifetime when timestamps exist. Collection runs on every
  refresh regardless of which view is on screen — history, the journal, and
  the wtmp/btmp tailing all keep updating while you sit on the main panel.
- **Processes**: each process is attributed to a session by matching its
  `/proc/<pid>/fd/0`, `/fd/1`, or `/fd/2` tty path to the session's `pts/N`
  line first, with kernel `tty_nr` device decoding as fallback. This avoids
  libc/container device-number mismatches that can otherwise make every
  process look like an orphan. CPU% is computed from utime/stime deltas.
- **Session names**: tmux/zellij/screen session names are parsed from the
  client processes' argv on each tty (`-s`/`-t`/`-S` flags, `zellij attach <n>`).
- **Events**: each refresh diffs the process set and emits spawn/exit events.

## Permissions

Processes of other users are visible only if you can read their
`/proc/<pid>` entries — run as **root** (or an admin with `ptrace_scope`
relaxed) for full visibility. Without it you'll see your own sessions only.
History needs read access to `/var/log/wtmp` and `/var/log/btmp` (root or the
`utmp` group), and Tailscale name resolution needs the `tailscale` CLI to be
runnable (root typically). The sessionwatch journal is written to
`/var/log/sessionwatch/` as root, else under `~/.local/state/sessionwatch/`.
Shell-history enrichment reads `~/.bash_history` / `~/.zsh_history` /
`~/.fish_history` — only for users that appear in the connection history, and
only when sessionwatch has read access to their home (i.e. root). It's a
box-owner's tool: those files stay unread for everyone else.

## Timestamped shell history

Exact command times are only available when the shell writes them. Enable this
for future commands; existing untimestamped lines cannot be backfilled exactly.

### Bash

```bash
printf '%s\n' 'export HISTTIMEFORMAT="%Y-%m-%d %H:%M:%S "' >> ~/.bashrc
source ~/.bashrc
```

Bash will then persist timestamp records alongside new history entries.

### Zsh

```zsh
printf '\nsetopt EXTENDED_HISTORY\n' >> ~/.zshrc
source ~/.zshrc
```

Fish already writes `when:` timestamps in its history format.

sessionwatch's own journal timestamps live process spawns independently, so
this shell setup is only needed to enrich connections that ended before
sessionwatch was watching them.

## Project docs

- [Contributing](CONTRIBUTING.md)
- [Security policy](SECURITY.md)
- [Code of Conduct](CODE_OF_CONDUCT.md)
- [License](LICENSE)

## License

MIT
