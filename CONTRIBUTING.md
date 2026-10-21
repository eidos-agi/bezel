# Contributing to bezel

Thanks for your interest in contributing.

## Quick start

```bash
git clone https://github.com/eidos-agi/bezel.git
cd bezel
cargo test --locked
```

Windows capture builds on Windows. The Mac listener builds on macOS. The shared tests run on either.

## Development

Run the tests before a pull request:

```bash
cargo test --locked
```

This tree has no separate lint tool. Keep the change small enough that the tests and the existing style cover it.

## Releases

A version tag `vX.Y.Z` must match `version` in `Cargo.toml`. The release workflow builds binaries and attaches them to the GitHub Release:

- `bezel-x86_64-pc-windows-msvc.exe`
- `bezel-aarch64-apple-darwin`
- `bezel-x86_64-apple-darwin`

A running `bezel --to` and a running `bezel listen` each request that release and replace themselves when the tag is newer. `--no-upgrade` keeps the copy that is already running.

## For agent developers

bezel is a headless CLI. There is no window and no MCP server.

1. **A headless path for everything.** Capture, inject, and upgrade are commands. Nothing requires a click in a GUI.
2. **Errors on stderr, data on stdout.** Diagnostics go to stderr. An agent can pipe stdout.
3. **Exit codes.** `0` is success. A non-zero exit is failure, and the stderr line says what to do next.
4. **The binary is the interface.** Agents install a release binary. They do not need a Rust toolchain on the machine that runs it.

## Pull requests

- Keep PRs focused — one feature or fix per PR
- Include tests for new functionality
- Update CHANGELOG.md with your changes
- Ensure `cargo test --locked` passes

## Reporting issues

Open an issue with:

1. What you were trying to do
2. What happened instead
3. Steps to reproduce
4. `bezel --help` output and the OS of each machine
