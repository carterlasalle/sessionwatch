# Contributing to sessionwatch

Thanks for helping improve sessionwatch.

## Before you start

- Use a current stable Rust toolchain.
- Linux is required for live collector work because sessionwatch reads
  `/proc`, utmp, wtmp, and btmp. macOS can compile the project, but the binary
  intentionally refuses to run there.
- Do not include real usernames, hostnames, Tailscale identities, commands,
  shell history, or log contents in issues or pull requests. Use fixtures and
  redacted examples.

## Development setup

```bash
git clone https://github.com/carterlasalle/sessionwatch.git
cd sessionwatch
cargo test
cargo run -- --help
```

The live collector should be exercised on a disposable Linux host. Run with
appropriate permissions when testing other users' sessions:

```bash
cargo run --release
sudo ./target/release/sessionwatch
```

## Making changes

1. Create a focused branch from `main`.
2. Keep changes small and explain the user-visible behavior.
3. Add a regression test for behavior that could break again.
4. Run formatting, tests, and the relevant target build before opening a PR.

Recommended local checks:

```bash
cargo fmt --all -- --check
cargo test --all-targets
cargo build --release
cargo build --release --target x86_64-unknown-linux-musl
cargo build --release --target aarch64-unknown-linux-musl
```

## Collector changes

The collector has separate responsibilities:

- `src/collect/linux.rs` reads live sessions and processes.
- `src/collect/tty.rs` attributes processes to `pts/N` sessions.
- `src/collect/utmp.rs` parses current login records.
- `src/collect/wtmp.rs` reconstructs completed connections.
- `src/collect/btmp.rs` parses failed login attempts.
- `src/collect/journal.rs` stores observations made while sessionwatch runs.
- `src/collect/shellhistory.rs` parses optional shell history enrichment.

Prefer parser fixtures and deterministic TestBackend UI tests. Do not make
live network calls or require a real user's history in tests.

## Pull requests

Include:

- what changed and why;
- how you verified it;
- any Linux permissions or Tailscale requirements;
- screenshots or text snapshots for meaningful TUI changes, with all private
  data redacted.

A PR should leave `cargo fmt --all -- --check`, `cargo test --all-targets`,
and the affected release build passing.

## Reporting bugs

Include the sessionwatch commit, target architecture, Linux distribution and
kernel version, the command used to launch it, and a redacted screenshot or
text reproduction. Never paste raw `/proc`, wtmp, btmp, shell history, or
Tailscale identity data.
