# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.10] - 2026-10-22

### Added

- `bezel doctor` lists the shared messages. `bezel doctor add`, `update`, and `delete` change them. The running bezel sends the change. Doctor does not open a connection.

## [0.1.9] - 2026-10-22

### Added

- `bezel doctor` says who is connected and whether a move off the chosen edge could hand the pointer over. It does not move the pointer or open the connection.

## [0.1.8] - 2026-10-21

### Added

- `bezel version` prints this release on its own line.

## [0.1.7] - 2026-10-21

### Added

- `--debug` prints each key and whether it was sent. A key that is typed and not transferred says why.

### Fixed

- A key the hook cannot name is no longer eaten. A key the hook misses is still read from raw input. A break scan with the high bit set counts as a key-up.

## [0.1.6] - 2026-10-21

### Fixed

- Keys typed on Windows reach the Mac. Swallowing a key stops Windows from delivering it as raw input, so the hook keeps that key. A held letter repeats. The Windows key and Ctrl+Alt+Shift still stay on Windows.
- The Mac posts keys and moves with no local-event suppression, so a stream of pointer moves does not drop the next key.

## [0.1.5] - 2026-10-21

### Added

- Windows and the Mac each file a GitHub issue when a problem happens. `bezel report` files one from either machine. An open issue with the same title gets a comment.

## [0.1.4] - 2026-10-21

### Fixed

- The Mac answers when the pointer arrives. Windows then hides its cursor and the pointer travels across the Mac. Before this, both cursors stayed on the shared edge.

## [0.1.3] - 2026-10-21

### Added

- Each machine takes a stable name from its machine id. The name is one of 10,000 first names and one of 10,000 last names, and it stays put across restarts.
- `bezel inbox` lists a shared inbox. `add`, `update`, and `delete` change it. Connected machines sync those messages. A machine that does not advertise the inbox is left on the pointer frames only.

## [0.1.2] - 2026-10-21

### Fixed

- Moving into a gap between Windows screens, or off a screen with nothing beyond it, hands the pointer to the Mac. The seam where two Windows screens touch stays on Windows.

## [0.1.1] - 2026-10-21

### Changed

- A running copy announces a newer release and waits. `bezel upgrade` installs that binary. Start bezel again afterward.

## [0.1.0] - 2026-10-21

### Added

- Windows owns the physical mouse and keyboard and the Mac only injects. The pointer crosses one outer edge of the Windows desktop over Tailscale.
- The Windows settings file chooses that edge. The connection tells the Mac, and saving the file updates the open connection.
- Release binaries on GitHub. The Windows controller and the Mac each request a newer binary from `eidos-agi/bezel` and replace themselves.
