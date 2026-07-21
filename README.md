# G560 Linux Utility

A Linux alternative to Logitech G HUB for the Logitech G560 gaming speakers. Control per-zone lighting, run content-aware lighting driven by your screen, and keep it all running as a background service — on Wayland, without proprietary software.

## What it does

- **Manual RGB control** for all four speaker zones (left rear, left front, right front, right rear), with brightness and master power.
- **Content-aware lighting** that samples one selected monitor at ~20 FPS and drives each zone to match the region of the screen nearest to it. Uses the desktop's ScreenCast portal on GNOME/KDE Wayland, and Gamescope's native PipeWire output in Bazzite Gaming Mode.
- **Tauri GUI**, **CLI**, and a **background user-service** — pick whichever fits your workflow. All three drive the same engine over a local Unix socket.
- **Setup persistence.** Monitor authorization, mode, colors, and service state survive reboots.
- **Runs unprivileged.** The only privileged step is a one-time udev rule install via `pkexec`; the app itself never needs root.

## Status

- Accepted on Fedora GNOME Wayland, Bazzite 43 (Desktop Mode and Gaming Mode / Gamescope), and Arch Linux KDE Plasma 6 Wayland.
- Milestone-one color behavior guarantees are for SDR content only. HDR is not yet validated.
- Currently distributed as source only — no packaged builds yet. Build from source using the recipes below.
- Prototype-quality but in daily use by the developer.

## Requirements

- Logitech G560 speakers (USB ID `046d:0a78`)
- A Wayland session with PipeWire and an XDG ScreenCast portal (GNOME and KDE are the primary targets; Bazzite Gaming Mode uses Gamescope directly)
- Rust 1.97.1 (pinned by `rust-toolchain.toml`)
- GStreamer 1.x, PipeWire, and libusb — headers to build, runtime libraries and plugins to run

## Install

### Fedora

```bash
pkexec dnf install -y \
  gcc pkgconf-pkg-config \
  gstreamer1-devel gstreamer1-plugins-base-devel gstreamer1-plugins-bad-free-devel \
  pipewire-devel libusb1-devel \
  webkit2gtk4.1-devel libayatana-appindicator3-devel
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
~/.cargo/bin/cargo build --release
```

### Arch Linux (KDE Plasma Wayland)

```bash
sudo pacman -S --needed \
  base-devel pkgconf rustup \
  gstreamer gst-plugins-base gst-plugins-bad gst-plugin-pipewire \
  pipewire libusb xdg-desktop-portal xdg-desktop-portal-kde \
  webkit2gtk-4.1 libayatana-appindicator
rustup toolchain install 1.97.1
cargo build --release
```

`gst-plugin-pipewire` is required at runtime. Without it, Desktop capture fails even when the portal and PipeWire services are healthy.

### Bazzite (immutable host)

Bazzite's host is immutable — build inside a Distrobox/Toolbox, then run the resulting binary on the host in your real Wayland/PipeWire session. See [`docs/build-and-run.md`](docs/build-and-run.md) for the tested container command and package list.

### One-time USB permission

Install the narrowly scoped udev rule for `046d:0a78`. The script uses Polkit only for the rule install and `udevadm` reload; the utility itself stays unprivileged.

```bash
./scripts/install-udev-rule.sh
```

Unplug and reconnect the G560 if the current USB node does not receive an access ACL. **Never run this utility with `sudo`.**

## Using the GUI

Launch it after building:

```bash
cargo run -p logig560-gui --release
# or, after build: ./target/release/logig560-gui
```

The GUI has four pages:

- **Lighting** — the main page. Two tabs:
  - *Manual* — pick a color per zone, adjust brightness, toggle master power.
  - *Content-Aware* — enable screen-driven lighting, choose the monitor, switch capture backend (Desktop portal vs Gamescope).
- **Setup & Service** — first-run monitor authorization, install/enable the background service, mark setup complete.
- **Diagnostics** — live capture FPS, USB write latency, recovery counters, capture backend in use.
- **About** — version and links.

The GUI talks to the engine over the Unix socket `$XDG_RUNTIME_DIR/logig560.sock`, so if the service is already running the GUI just attaches to it.

## Using the CLI

The same binary that powers the service also exposes a CLI. Common commands:

```bash
# Start screen matching in a normal Wayland session
./target/release/logig560 run

# Start screen matching in Bazzite Gaming Mode (Gamescope backend)
./target/release/logig560 run-gaming

# Set all four zones directly
./target/release/logig560 set-zones \
  --left-rear FF0000 --left-front 00FF00 \
  --right-front 0000FF --right-rear FFFFFF

# Verify capture works without saving a frame
./target/release/logig560 capture-test --frames 300

# Re-run the USB pacing calibration (does not modify saved settings)
./target/release/logig560 calibrate-pacing --delay-ms 6 --seconds 15
```

The first `run` opens the system monitor chooser and stores a restore token at `~/.config/logig560/capture.toml` (mode `0600`). Later runs reuse that authorization. Ctrl-C, SIGTERM, and normal termination all end with an all-zone blackout.

## Running as a service

The engine is designed to run continuously in the background so the GUI and CLI can attach and detach freely.

**Desktop Mode (GNOME/KDE Wayland):** install and enable the user unit through the GUI's *Setup & Service* page, or manually via the units in `systemd/`.

**Gaming Mode (Bazzite Gamescope):**

```bash
./scripts/install-gaming-service.sh
```

`logig560-gaming.service` is a user unit under `gamescope-session-plus@steam.service.wants` — enabled but inactive in Desktop Mode, and starts automatically only when the Gamescope Steam session starts.

Do not run the Desktop and Gaming units at the same time; both open the same USB interface.

## Privacy

- Capture is local and restricted to the single monitor you authorized in the system chooser.
- Frames are reduced to zone colors in memory and discarded. **No** screenshot, thumbnail, pixel buffer, sampled color, or color history is ever written to disk or sent over a network.
- The only persisted capture-related data is the portal restore token.
- Runtime metrics (FPS, latency, recovery counters) are color-free.

## Color behavior

- Dark regions turn fully off.
- Normal changes use a 90 ms OKLab smoothstep fade per zone. A zone going fully black uses a 200 ms fade so momentary near-black threshold crossings do not look like a flicker.
- New samples retarget from the currently displayed color, so old transitions never queue.
- Safety blackouts for capture loss/stall, session lock, shutdown, and USB recovery bypass fades and are immediate.

## Troubleshooting

Quick checks when lights don't respond:

```bash
lsusb -d 046d:0a78                        # is the device present?
getfacl /dev/bus/usb/BUS/DEVICE           # did the udev rule grant your user access?
./target/release/logig560 capture-test --frames 30
```

Resolve `BUS` and `DEVICE` from `lsusb`. If capture works but colors go to the wrong physical location, consult the [verified zone map](docs/hardware/g560-zone-map.md).

The engine retries USB opens with bounded backoff (250 ms → 5 s max), blacks out after 500 ms without a frame, and reopens the device after three consecutive write failures. The Desktop backend holds the last dimension-valid frame for 200 ms of healthy portal silence so static content does not trigger a false stall.

For deeper diagnosis, see [`docs/troubleshooting.md`](docs/troubleshooting.md) and [`docs/capture-backends.md`](docs/capture-backends.md).

## For developers

- [`AGENTS.md`](AGENTS.md) — repository invariants and change routing
- [`docs/README.md`](docs/README.md) — full documentation map
- [`docs/architecture.md`](docs/architecture.md) — end-to-end design
- [`docs/color-pipeline.md`](docs/color-pipeline.md) — geometry, sampler, OKLab transitions
- [`docs/usb-and-safety.md`](docs/usb-and-safety.md) — HID protocol, 6 ms pacing, blackout semantics
- [`docs/testing.md`](docs/testing.md) — automated and hardware gates

Milestone hardware evidence lives under `docs/hardware/`.
