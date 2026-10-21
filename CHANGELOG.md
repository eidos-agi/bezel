# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2026-10-21

### Added

- Windows owns the physical mouse and keyboard and the Mac only injects. The pointer crosses one outer edge of the Windows desktop over Tailscale.
- The Windows settings file chooses that edge. The connection tells the Mac, and saving the file updates the open connection.
- Release binaries on GitHub. The Windows controller and the Mac each request a newer binary from `eidos-agi/bezel` and replace themselves.
