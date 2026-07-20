# LogiLightShow

LogiLightShow is an experimental low-latency screen matcher for the Logitech G560 on Linux. The milestone-one CLI captures exactly one monitor through the desktop ScreenCast portal, samples four edge regions, and drives the speakers' four verified lighting zones. It targets Wayland sessions on Fedora and Bazzite; desktop integration and distributable Bazzite packaging are later milestones.

## Prerequisites

- Logitech G560 (`046d:0a78`)
- A Wayland desktop with PipeWire and an XDG ScreenCast portal (GNOME and KDE are the intended targets)
- Rust 1.97.1 (the repository's `rust-toolchain.toml` pins it)
- GStreamer 1.x development files and plugins, plus libusb development files

For a Fedora development host:

```bash
pkexec dnf install -y gcc pkgconf-pkg-config gstreamer1-devel gstreamer1-plugins-base-devel gstreamer1-plugins-bad-free-devel libusb1-devel
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
~/.cargo/bin/cargo build --release
```

Bazzite's immutable host should not be modified just to assemble a development dependency collection. Build in a development container until the packaged application from milestone two is available. The running binary must still have access to the user's Wayland/PipeWire portal session and the host USB device.

## One-time USB permission

Install the narrowly scoped udev rule. The script uses Polkit only to install the `046d:0a78` rule and reload udev; LogiLightShow itself stays unprivileged.

```bash
./scripts/install-udev-rule.sh
```

Unplug and reconnect the G560 if the current USB node does not receive an access ACL after installation. Never run LogiLightShow with `sudo`.

## Commands

Verify single-monitor capture without saving a frame:

```bash
~/.cargo/bin/cargo run -- capture-test --frames 300
```

The desktop chooser is the authority for display selection. The application requests one monitor only, hides the captured cursor, and rejects any response containing zero or multiple streams.

Set all four zones directly (six hexadecimal digits per color):

```bash
~/.cargo/bin/cargo run -- set-zones \
  --left-rear FF0000 \
  --left-front 00FF00 \
  --right-front 0000FF \
  --right-rear FFFFFF
```

Start screen matching:

```bash
~/.cargo/bin/cargo run --release -- run
```

The first run opens the monitor chooser. A returned restore token is atomically stored at `~/.config/logilightshow/capture.toml` with mode `0600`; later runs ask the portal to restore that same authorization. Press Ctrl-C (or send `SIGTERM`) for a clean stop and final all-zone blackout.

While running, the CLI prints five-second captured-FPS and rendered-update rates, cumulative dropped-frame and capture-stall counts, cumulative capture-to-write p50/p95/p99 latency, and USB/capture recovery counters. It never logs sampled colors. Capture is paced to at most 20 frames per second before DMA-BUF/OpenGL conversion; the G560's verified four-report pacing normally limits changed lighting states to about 12 updates per second.

For a short deterministic zone check, open the local test page, move it to the selected monitor, and optionally press F11:

```bash
xdg-open docs/hardware/quadrant-test.html
```

The pattern is a diagnostic aid, not something that must remain open during normal use.

## Privacy and color behavior

- Capture is local and restricted to the one monitor authorized in the system chooser.
- Frames are reduced in memory and discarded. No screenshot, thumbnail, pixel buffer, sampled color, or color history is written to disk or sent over a network.
- The only persisted capture data is the portal restore token.
- Dark regions turn fully off. Temporal smoothing is disabled.
- Milestone one guarantees color behavior for SDR content only. HDR color accuracy is not yet validated.

## Recovery and troubleshooting

If the speakers are missing or busy, `run` retries opening them with bounded exponential delays: 250 ms, 500 ms, 1 s, 2 s, 4 s, then at most 5 s between attempts. After three consecutive USB write failures it attempts blackout, releases the old interface, and reopens the device. An update that becomes older than 500 ms during recovery is replaced with black rather than shown late.

If capture delivers no fresh frame for 500 ms, all currently connected zones are blacked out. A fresh frame resumes matching immediately. GNOME can destroy the PipeWire node when the session locks; `run` keeps the lights black, reopens the saved single-monitor authorization with bounded backoff, and resumes only after a fresh frame arrives. Capture EOS exits cleanly after blackout. Cancelling the system monitor chooser also exits cleanly.

Useful checks:

```bash
lsusb -d 046d:0a78
getfacl /dev/bus/usb/BUS/DEVICE
~/.cargo/bin/cargo run -- capture-test --frames 30
~/.cargo/bin/cargo run --release -- run
```

Resolve `BUS` and `DEVICE` from `lsusb`; do not copy a stale device number. If static control is needed to isolate capture from USB behavior, rerun the `set-zones` command above. If colors address the wrong physical location, consult [the verified zone map](docs/hardware/g560-zone-map.md).

Milestone one has been exercised on GNOME Wayland. Its fullscreen workaround currently requires DMA-BUF capture plus the GStreamer OpenGL elements; desktops or GPUs that cannot negotiate that path will report a capture setup error. KDE/Bazzite desktop integration and distributable packaging remain milestone-two work.

Milestone-one acceptance measurements and unresolved limitations are tracked in [the hardware results](docs/hardware/milestone-1-results.md).
