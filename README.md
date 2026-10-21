# bezel

[![CI](https://github.com/eidos-agi/bezel/actions/workflows/ci.yml/badge.svg)](https://github.com/eidos-agi/bezel/actions)
[![License: MIT](https://img.shields.io/github/license/eidos-agi/bezel)](LICENSE)

The pointer leaves the left edge of the Windows desktop and arrives on the right side of the Mac. Tailscale carries the connection. There is no window.

Windows owns the mouse and keyboard. The Mac only injects. Clipboard, file drag, and a return from the Mac's keyboard are out of scope. Windows chooses the edge. Moving back through the Mac's facing edge returns the pointer to Windows.

## Install

Download the binary for this machine from the [latest release](https://github.com/eidos-agi/bezel/releases/latest).

`bezel listen` on the Mac and `bezel --to` on Windows each check that release. When a newer one exists they print `bezel: VERSION is available. Run bezel upgrade`. `bezel upgrade` installs it. Start bezel again after that. `--no-upgrade` stays quiet.

The release is three binaries: `bezel-x86_64-pc-windows-msvc.exe`, `bezel-aarch64-apple-darwin`, and `bezel-x86_64-apple-darwin`.

## Run

On the Mac, after Rust is installed:

```
git clone https://github.com/eidos-agi/bezel.git ~/bezel
cd ~/bezel
cargo build --release
./target/release/bezel listen
```

Grant Accessibility to the process that launches it. A Terminal launch means Terminal needs the grant. Restart `bezel listen` after granting it. The listener binds the Tailscale IPv4 address on port 47940 and accepts only `darwin-dell-xps`.

On Windows:

```
bezel --to
```

That reads the settings file. `--scale 1.0` is one Windows pixel to one Mac point. The Mac's logical size and backing scale are printed when the connection comes up.

## Settings

Windows uses `%APPDATA%\bezel\settings.toml`. The Mac uses `~/Library/Application Support/bezel/settings.toml`. A missing file is written with the defaults. A flag overrides the file.

```
host = daniels-macbook-pro.tail0ffb4a.ts.net
port = 47940
scale = 1.0
edge = left
allow = darwin-dell-xps.tail0ffb4a.ts.net
```

`edge = left` leaves any Windows screen edge that has no Windows screen beyond it on the left, and returns across the Mac's right edge. Two Windows screens that touch do not cross. `edge = right` is the other way around. Windows sends that edge when the connection comes up. Saving the Windows settings file while bezel is running sends the new edge on the same connection, including while the pointer is on the Mac. Scale is applied on Windows before each move. Host and port apply the next time bezel starts. `allow` is the Mac listener's accept list.

A listener from before this build has no edge frame. Windows sees that in the hello and keeps the edge to itself. Sending the frame to that listener would drop the session.

## Update

A published release is how a running copy updates. The controller and the Mac both notice it and wait for `bezel upgrade`. Neither machine installs a release on its own, and neither sends the new binary to the other.

`bezel update` still sends this source tree over the Tailscale connection. The Mac runs `cargo build --release` and restarts `bezel listen`. A listener from before that command closes the connection instead of building. The release binary is the path a machine uses on its own.

MIT License. See [LICENSE](LICENSE).

Ctrl becomes Command. Alt becomes Option. Shift stays Shift. The Windows key stays on Windows. Ctrl+Alt+Shift returns the pointer immediately.

## Pass

Cross, click, type a word, scroll, and cross back. Hold Shift across the edge, release it, and type a lowercase letter. Sleep the Mac and confirm Windows still owns the mouse. Wake it and cross again.

`bezel fake` stands in for the Mac on localhost and injects nothing.

A normal process cannot see keys typed into an elevated window. The secure desktop, including the UAC prompt, stays on Windows. Ctrl+Alt+Del stays on Windows.

This PC compiled the Windows binary and `bezel fake`, and the tests passed. It cannot verify CoreGraphics injection, the Accessibility grant, or a live cross onto the Mac. Smart App Control on this PC then blocked the unsigned binary.

This tree is original. Deskflow and Lan Mouse are GPL, and their source is not copied here.

License: MIT.
