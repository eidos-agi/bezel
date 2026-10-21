# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
