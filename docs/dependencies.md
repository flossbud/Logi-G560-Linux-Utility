# Dependency map

## Rust crates

| Crate | Role |
|---|---|
| `anyhow` | contextual errors at application/engine composition boundaries |
| `async-trait` | async capture/sink/factory traits |
| `ashpd` | XDG Desktop Portal ScreenCast client |
| `clap` | CLI parsing and validation |
| `directories` | XDG-compatible user config location |
| `futures-util` | declared legacy dependency; no direct source reference in the current prototype |
| `gstreamer` | Desktop PipeWire pipeline and bus/caps management |
| `gstreamer-app` | newest-frame appsink access |
| `gstreamer-video` | video info, format, stride, and mapped frame access |
| `hdrhistogram` | cumulative capture-to-write latency percentiles |
| `palette` | sRGB/linear RGB, Lab sampling, and OKLab transitions |
| `pipewire` | direct Gamescope node connection and SPA buffer negotiation |
| `rusb` | G560 interface claim and HID control writes |
| `serde`, `toml` | private portal restore-token config |
| `thiserror` | typed capture and USB errors |
| `tokio`, `tokio-util` | async runtime, channels, signals, time, cancellation |
| `tracing`, `tracing-subscriber` | declared logging dependencies; no subscriber is currently initialized and metrics use stdout/stderr |

Development:

| Crate | Role |
|---|---|
| `proptest` | arbitrary frame/color safety properties |
| `tempfile` | isolated config persistence tests |

Versions are authoritative in `Cargo.toml`/`Cargo.lock`.

## Native build dependencies

Fedora package names:

```text
gcc
pkgconf-pkg-config
gstreamer1-devel
gstreamer1-plugins-base-devel
gstreamer1-plugins-bad-free-devel
pipewire-devel
libusb1-devel
```

The accepted Fedora 43 container used GStreamer 1.26.11, PipeWire development 1.4.11, and libusb development 1.0.30. The Bazzite host runtime versions can differ slightly; verify dynamic linking and live behavior after upgrades.

## Runtime services/libraries

Desktop requires:

- Wayland compositor;
- `xdg-desktop-portal` plus a desktop backend such as GNOME;
- PipeWire;
- GStreamer core, PipeWire source, base video conversion/scaling plugins;
- optional GL plugins only for the opt-in DMA-BUF path.

Gaming requires:

- Gamescope session with PipeWire node `gamescope`;
- PipeWire runtime library;
- user systemd session.

Both require libusb runtime and udev-granted access to `046d:0a78`.

## Why native headers are needed even when one backend is unused

Both capture modules compile into the single binary. Cargo therefore builds the GStreamer and PipeWire bindings on every target build, even if a particular run uses only Desktop or only Gaming capture. Missing `libpipewire-0.3.pc` prevents compilation before runtime backend selection.

Feature-gating backends could change this in future, but it would increase build/test combinations and must not accidentally ship a Gaming binary without Desktop support or vice versa.

## Upgrade checklist

When upgrading Rust crates or native stacks:

1. Read API/changelog impact for caps, PipeWire buffer ABI, portal persistence, and Tokio scheduling.
2. Update lockfile through Cargo, never manually.
3. Run the full automated suite.
4. Verify Desktop non-black capture, caps changes, and static hold.
5. Verify Gamescope BGRx MemFd negotiation in real Gaming Mode.
6. Verify USB/audio behavior if rusb or runtime libusb changes.
7. Update this document and environment handoff with observed versions.
