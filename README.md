# G560 Linux Utility

G560 Linux Utility is an experimental low-latency screen matcher for the Logitech G560 on Linux. The CLI captures one display, samples four polygons shaped for the speakers' front/rear light layout, and drives the four verified lighting zones. Desktop Mode uses the system ScreenCast portal; Bazzite Gaming Mode captures Gamescope's native PipeWire output. The prototype is accepted on Fedora/Bazzite GNOME and Gamescope, and its unchanged Desktop backend has been operationally validated on Arch Linux KDE Plasma 6 Wayland.

The `run` and `run-gaming` commands now act as a persistent lighting service that also exposes a newline-delimited JSON API on the user-session Unix socket `$XDG_RUNTIME_DIR/logig560.sock`. A Tauri 2 GUI (`crates/logig560-gui/`) drives the service over that socket to control manual color, brightness, master power, Content-Aware mode, capture backend, setup, and diagnostics. Distributable packaging remains future work.

## Documentation

- [Agent orientation and repository invariants](AGENTS.md)
- [Documentation map](docs/README.md)
- [Current project status](docs/project-status.md)
- [Architecture](docs/architecture.md)
- [Build and run guide](docs/build-and-run.md)
- [Operations runbook](docs/operations.md)
- [Troubleshooting](docs/troubleshooting.md)
- [Bazzite prototype handoff](HANDOFF_BAZZITE.md)

Fedora milestone hardware evidence lives under `docs/hardware/`.

## Prerequisites

- Logitech G560 (`046d:0a78`)
- A Wayland desktop with PipeWire and an XDG ScreenCast portal (GNOME and KDE are the intended targets)
- Rust 1.97.1 (the repository's `rust-toolchain.toml` pins it)
- GStreamer 1.x, PipeWire, and libusb development files and runtime libraries/plugins

For a Fedora development host:

```bash
pkexec dnf install -y gcc pkgconf-pkg-config gstreamer1-devel gstreamer1-plugins-base-devel gstreamer1-plugins-bad-free-devel pipewire-devel libusb1-devel
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
~/.cargo/bin/cargo build --release
```

For Arch Linux with KDE Plasma Wayland:

```bash
sudo pacman -S --needed \
  base-devel pkgconf rustup \
  gstreamer gst-plugins-base gst-plugins-bad gst-plugin-pipewire \
  pipewire libusb xdg-desktop-portal xdg-desktop-portal-kde
rustup toolchain install 1.97.1
cargo build --release
```

`gst-plugin-pipewire` is required at runtime; without it, `gst-inspect-1.0 pipewiresrc` and Desktop capture fail even when Plasma's portal and PipeWire services are healthy.

Bazzite's immutable host should not be modified just to assemble a development dependency collection. Build in a Fedora Distrobox/Toolbox, then run the host-mounted release binary in the user's real Wayland/PipeWire session. See [the build guide](docs/build-and-run.md) for the tested container command and required packages.

## One-time USB permission

Install the narrowly scoped udev rule. The script uses Polkit only to install the `046d:0a78` rule and reload udev; G560 Linux Utility itself stays unprivileged.

```bash
./scripts/install-udev-rule.sh
```

Unplug and reconnect the G560 if the current USB node does not receive an access ACL after installation. Never run G560 Linux Utility with `sudo`.

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

In Bazzite Gaming Mode, use the direct Gamescope backend:

```bash
./target/release/logig560 run-gaming
```

Normally it is launched by the checked-in user service:

```bash
./scripts/install-gaming-service.sh
```

The Gaming service is expected to be enabled but inactive in Desktop Mode and to start only with `gamescope-session-plus@steam.service`.

The first run opens the monitor chooser. A returned restore token is atomically stored at `~/.config/logig560/capture.toml` with mode `0600`; later runs ask the portal to restore that same authorization. Ctrl-C, service SIGTERM, and normal capture termination enter the clean-stop path and request a final all-zone blackout.

While running, the CLI prints five-second captured-FPS and rendered-update rates, cumulative dropped-frame and capture-stall counts, cumulative capture-to-write p50/p95/p99 latency, and USB/capture recovery counters. It never logs sampled colors. Capture is paced to approximately 20 frames per second and every inter-stage handoff keeps only the newest value. In the final Fedora fullscreen hardware run, the calibrated USB cadence delivered 18.22 complete lighting updates/s from an 18.35 FPS capture stream without queuing stale states.

The production USB driver spaces every adjacent HID report by the hardware-calibrated 6 ms interval, including across four-zone update boundaries. This value comes from error-free 15-second stages at 18, 16, 14, 12, 10, 8, 6, and 4 ms, followed by a two-minute confirmation at the selected 6 ms value (the fastest passing stage plus a 2 ms safety margin). The diagnostic can be repeated without changing saved settings:

```bash
~/.cargo/bin/cargo run --release -- calibrate-pacing --delay-ms 6 --seconds 15
```

Keep audio playing and observe the speakers during calibration. The command stops on the first USB transfer error, attempts a paced all-zone blackout, exits nonzero on failure, and never writes the diagnostic delay to configuration.

For a short deterministic zone check, open the local test page, move it to the selected monitor, and optionally press F11:

```bash
xdg-open docs/hardware/quadrant-test.html
```

The pattern is a diagnostic aid, not something that must remain open during normal use.

## Privacy and color behavior

- Capture is local and restricted to the one monitor authorized in the system chooser.
- Frames are reduced in memory and discarded. No screenshot, thumbnail, pixel buffer, sampled color, or color history is written to disk or sent over a network.
- The only persisted capture data is the portal restore token.
- Dark regions turn fully off.
- Normal content changes follow an interruptible 90 ms OKLab smoothstep fade. A normally sampled zone turning fully black uses a 200 ms fade so momentary near-black threshold crossings do not look like an off/on flicker. New samples retarget from the currently displayed color, so old transitions never queue. Safety blackouts remain immediate.
- Lock-related capture loss/stall, shutdown, and USB recovery cleanup bypass the fade and request immediate black.
- Milestone one guarantees color behavior for SDR content only. HDR color accuracy is not yet validated.

## Recovery and troubleshooting

If the speakers are missing or busy, `run` retries opening them with bounded exponential delays: 250 ms, 500 ms, 1 s, 2 s, 4 s, then at most 5 s between attempts. After three consecutive USB write failures it attempts blackout, releases the old interface, and reopens the device. An update that becomes older than 500 ms during recovery is replaced with black rather than shown late.

If the engine receives no frame for 500 ms, all currently connected zones are blacked out. A fresh frame resumes matching immediately. The Desktop backend holds its last dimension-valid frame after 200 ms of healthy portal silence so static/low-motion content does not create a false stall; bus errors, EOS, startup, and caps changes bypass that hold. GNOME can destroy the PipeWire node when the session locks; `run` keeps the lights black, reopens the saved single-monitor authorization with bounded backoff, and resumes only after a fresh frame arrives. Capture EOS exits cleanly after blackout. Cancelling the system monitor chooser also exits cleanly.

Useful checks:

```bash
lsusb -d 046d:0a78
getfacl /dev/bus/usb/BUS/DEVICE
~/.cargo/bin/cargo run -- capture-test --frames 30
~/.cargo/bin/cargo run --release -- run
```

Resolve `BUS` and `DEVICE` from `lsusb`; do not copy a stale device number. If static control is needed to isolate capture from USB behavior, rerun the `set-zones` command above. If colors address the wrong physical location, consult [the verified zone map](docs/hardware/g560-zone-map.md).

Desktop capture defaults to GStreamer system memory on the accepted Bazzite machine. The Fedora-tested DMA-BUF/OpenGL bridge remains available only through `LOGIG560_ENABLE_DMABUF=1`; it returned black downloaded pixels on this Bazzite stack and must not be made the default without new live evidence. Gaming Mode uses a separate direct PipeWire backend because Gamescope does not provide the normal desktop portal.

Fedora milestone-one acceptance measurements are tracked in [the hardware results](docs/hardware/milestone-1-results.md). Current Bazzite acceptance and unresolved prototype boundaries are tracked in [the Bazzite handoff](HANDOFF_BAZZITE.md) and [known limitations](docs/known-limitations.md).
