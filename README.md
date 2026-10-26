# bezel

[![CI](https://github.com/eidos-agi/bezel/actions/workflows/ci.yml/badge.svg)](https://github.com/eidos-agi/bezel/actions)
[![License: MIT](https://img.shields.io/github/license/eidos-agi/bezel)](LICENSE)

The pointer leaves one outer edge of the Windows desktop and arrives on the facing edge of another machine. That machine can be a Mac or a Linux desktop. Tailscale carries the connection. There is no window.

Windows owns the mouse and keyboard. The other side only injects. Clipboard, file drag, and a return from the other keyboard are out of scope. Windows chooses the edge. Moving back through the facing edge returns the pointer to Windows.

An agent runs it. A person allows one operating-system prompt. `bezel host` prints the MagicDNS name. `bezel status` prints what the running copy last saw. `bezel install` writes the service.

## Install

Download the binary for this machine from the [latest release](https://github.com/eidos-agi/bezel/releases/latest).

`bezel listen` on the Mac and `bezel --to` on Windows each check that release. When a newer one exists they print `bezel: VERSION is available. Run bezel upgrade`. `bezel upgrade` installs it. Start bezel again after that. `--no-upgrade` stays quiet.

The release binaries are `bezel-x86_64-pc-windows-msvc.exe`, `bezel-aarch64-apple-darwin`, `bezel-x86_64-apple-darwin`, and `bezel-x86_64-unknown-linux-gnu`.

On Linux, `bezel listen` injects into the logged-in desktop through the desktop portal. The first start asks that desktop to allow keyboard and pointer control, and to share a monitor so the pointer can be placed. A later start reuses that permission. Settings are `~/.config/bezel/settings.toml`. `allow` is the Windows machine. The listener has to run in the graphical session, where the session bus is available. `bezel install` writes that user service and starts it when nothing is already listening.

`bezel host` prints this machine's MagicDNS name. Put that name in the other machine's `host` or `allow`. `bezel status` prints the running copy's release, the peer release, the link, the edge, the screen rectangles captured at start, and the last cross. It does not move the pointer.

## Windows and Smart App Control

Smart App Control blocks an unsigned Windows binary. Other machines can run a release only when that file is signed by a certificate they trust. [SignPath Foundation](https://signpath.org/) signs open-source releases. The publisher name on that signature is theirs.

This development PC also trusts a local signer, `CN=eidos-agi bezel`, through supplemental policy `{b3e2e1a0-7c4d-4f11-9a2e-be2e10001010}`. The private key stays in the user store and is not in this repository. That policy is how this PC runs the release. It does not help another PC. The copy that is already running keeps its hash and still starts.

## Run

On the machine that should receive the pointer:

```
bezel install
```

On a Mac, grant Accessibility to the process that launches it. A Terminal launch means Terminal needs the grant. Start `bezel listen` again after granting it. On Linux, allow keyboard, pointer, and a monitor on the desktop dialog. The listener binds the Tailscale IPv4 address on port 47940 and accepts only the machine named in `allow`.

On Windows:

```
bezel install
```

That starts `bezel --to` with no window, and registers a task that runs `bezel keep` at logon and every two minutes. `bezel keep` starts the capture when no bezel is running. A link that stays quiet for 30 seconds is started again. A connected capture is left alone. `--scale 1.0` is one Windows pixel to one peer point. The peer's logical size and backing scale are printed when the connection comes up. Host and port apply the next time that process starts.

## Settings

Windows uses `%APPDATA%\bezel\settings.toml`. The Mac uses `~/Library/Application Support/bezel/settings.toml`. A missing file is written with the defaults. A flag overrides the file.

```
host = other-machine.tailnet.ts.net
port = 47940
scale = 1.0
edge = left
allow = this-windows-machine.tailnet.ts.net
report =
```

A missing file is created with `host` and `allow` empty. Bezel does not guess a machine. `bezel --to` stops until `host` is set. `bezel listen` stays up and refuses every peer until `allow` is set.

`edge = left` leaves any Windows screen edge that has no Windows screen beyond it on the left, and returns across the other machine's right edge. Two Windows screens that touch do not cross. `edge = right` is the other way around. Windows sends that edge when the connection comes up. Saving the Windows settings file while bezel is running sends the new edge on the same connection, including while the pointer is on the other machine. Scale is applied on Windows before each move. Host and port apply the next time bezel starts. `allow` is the listener's accept list.

`report` is a GitHub `owner/name`. Leave it empty. Bezel then does not file issues, even when a GitHub token is present. Set it, and keep a token in `GH_TOKEN`, `GITHUB_TOKEN`, or `gh auth token`, when this machine should file on that repo.

A listener from before this build has no edge frame. Windows sees that in the hello and keeps the edge to itself. Sending the frame to that listener would drop the session.

## Inbox

`bezel version` prints the release, such as `0.1.15`. `bezel who` prints this machine's stable name. The name is hashed from the operating system id, so the same computer keeps it. `bezel host` prints the MagicDNS name. `bezel status` prints the record the running copy writes beside the settings file: its release, the peer's release once that peer asks for it, whether the link is waiting, quiet, or connected, the edge, the screen rectangles from process start, the last pointer sample, and the last cross. Quiet means the socket is open and a heartbeat was missed; the next heartbeat on that socket marks the link connected. A closed socket says disconnected and the capture redials. An older running copy leaves `status: missing`.

`bezel doctor` says who is connected, whether a move off the chosen edge could hand the pointer to the other machine, and lists the shared messages. `bezel doctor add`, `update`, and `delete` change those messages. The running bezel sends the change. Doctor does not move the pointer and it does not open a connection. `bezel inbox` is the same message file. While `bezel --to` and `bezel listen` are connected, each side picks up the other's changes. A message records who wrote it and who it is for. An older bezel on the other end never sees these frames.

Each side sends its release version only after the other side sets that capability on hello. Hello itself stays one byte. An old peer never receives the version frame.

## Problems

When `report` is set to a GitHub `owner/name`, an Enter that gets no answer, a hello that gets no answer, a dropped connection, and a refused peer each file under a stable title, `bezel: <kind>`. An open issue with that title gets a comment. Each running copy files a kind once. `bezel report <text>` files what this machine just saw.

The request uses `GH_TOKEN`, `GITHUB_TOKEN`, or `gh auth token`. The token is not printed. With `report` empty, bezel stays quiet. With a repo and no token, it prints `bezel: problem not filed (no GitHub token)` and keeps running.

## Update

A published release is how a running copy updates. The controller and the Mac both notice it and wait for `bezel upgrade`. Neither machine installs a release on its own, and neither sends the new binary to the other.

`bezel update` still sends this source tree over the Tailscale connection. The listener runs `cargo build --release` and restarts `bezel listen`. A listener from before that command closes the connection instead of building. The release binary is the path a machine uses on its own.

MIT License. See [LICENSE](LICENSE).

On a Mac, Ctrl becomes Command and Alt becomes Option. Shift stays Shift. The Windows key stays on Windows. Ctrl+Alt+Shift returns the pointer immediately. A held key repeats.

`--debug` (or `BEZEL_DEBUG=1`) prints each key and whether it was sent. A key that was typed and not transferred says why: `local`, `pending`, `unmapped`, `queue`, or `link`. The lines stay on the console. They are not written to a file and they are not filed as issues.

## Pass

Cross, click, type a word, scroll, and cross back. Hold Shift across the edge, release it, and type a lowercase letter. Sleep the Mac and confirm Windows still owns the mouse. Wake it and cross again.

`bezel fake` stands in for the Mac on localhost and injects nothing.

A normal process cannot see keys typed into an elevated window. The secure desktop, including the UAC prompt, stays on Windows. Ctrl+Alt+Del stays on Windows.

This PC compiled the Windows binary and `bezel fake`, and the tests passed. It cannot verify CoreGraphics injection, the Accessibility grant, or a live cross onto the Mac. An unsigned new build is blocked here. The signed build described above is the one that starts.

This tree is original. Deskflow and Lan Mouse are GPL, and their source is not copied here.

License: MIT.
