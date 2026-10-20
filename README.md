# bezel

The pointer leaves the right edge of the primary Windows monitor and arrives on the Mac. Tailscale carries the connection. There is no window.

Windows owns the mouse and keyboard. The Mac only injects. Clipboard, file drag, and a return from the Mac's keyboard are out of scope. Moving back through the Mac's left edge returns the pointer to Windows.

## Run

On the Mac, after Rust is installed:

```
git clone https://github.com/eidos-agi/bezel.git
cd bezel
cargo build --release
./target/release/bezel listen
```

Grant Accessibility to the process that launches it. A Terminal launch means Terminal needs the grant. Restart `bezel listen` after granting it. The listener binds the Tailscale IPv4 address on port 47940 and accepts only `darwin-dell-xps`.

On Windows:

```
bezel --to daniels-macbook-pro.tail0ffb4a.ts.net
```

`--scale 1.0` is one Windows pixel to one Mac point. The Mac's logical size and backing scale are printed when the connection comes up.

Ctrl becomes Command. Alt becomes Option. Shift stays Shift. The Windows key stays on Windows. Ctrl+Alt+Shift returns the pointer immediately.

## Pass

Cross, click, type a word, scroll, and cross back. Hold Shift across the edge, release it, and type a lowercase letter. Sleep the Mac and confirm Windows still owns the mouse. Wake it and cross again.

`bezel fake` stands in for the Mac on localhost and injects nothing.

A normal process cannot see keys typed into an elevated window. The secure desktop, including the UAC prompt, stays on Windows. Ctrl+Alt+Del stays on Windows.

This PC compiled the Windows binary and `bezel fake`, and the tests passed. It cannot verify CoreGraphics injection, the Accessibility grant, or a live cross onto the Mac. Smart App Control on this PC then blocked the unsigned binary.

This tree is original. Deskflow and Lan Mouse are GPL, and their source is not copied here.
