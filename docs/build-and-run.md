# Build and run

## Requirements

- Rust 1.97.1 with `rustfmt` and `clippy` (pinned by `rust-toolchain.toml`).
- GCC and pkg-config.
- GStreamer core/base/bad-free development packages.
- PipeWire development package.
- libusb development package.
- At runtime: a Wayland/PipeWire session, relevant GStreamer plugins for Desktop capture, and a Logitech G560.

## Fedora development host

```bash
pkexec dnf install -y \
  gcc \
  pkgconf-pkg-config \
  gstreamer1-devel \
  gstreamer1-plugins-base-devel \
  gstreamer1-plugins-bad-free-devel \
  pipewire-devel \
  libusb1-devel

curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
cargo build --release
```

Verify capture elements:

```bash
gst-inspect-1.0 pipewiresrc videoconvert videoscale
```

The optional DMA-BUF path additionally needs:

```bash
gst-inspect-1.0 glupload glcolorconvert gldownload videorate
```

## Bazzite development container

Bazzite is immutable. Do not layer a development toolchain onto the host merely to build this prototype. The tested setup is a Fedora 43 Distrobox named `logilightshow`.

Create it once if necessary:

```bash
distrobox create --name logilightshow \
  --image registry.fedoraproject.org/fedora-toolbox:43
```

Install native development dependencies inside it:

```bash
distrobox enter logilightshow -- sudo dnf install -y \
  gcc \
  pkgconf-pkg-config \
  gstreamer1-devel \
  gstreamer1-plugins-base-devel \
  gstreamer1-plugins-bad-free-devel \
  pipewire-devel \
  libusb1-devel
```

The accepted container uses the host user's rustup toolchain mounted into the container, so Fedora `rust`/`cargo` RPMs are not required. Install rustup for the user if `cargo` is absent.

Build from the host-mounted repository:

```bash
distrobox enter logilightshow -- bash -lc \
  'cd /home/jaret/Documents/LogiLightShow && cargo build --release'
```

Run the binary on the host, not with `sudo`, so it sees the real Wayland/PipeWire user session and USB access:

```bash
./target/release/logilightshow --help
```

The accepted host provides runtime `libgstreamer-1.0`, `libpipewire-0.3`, and `libusb-1.0`. Check portability after moving the binary:

```bash
ldd target/release/logilightshow | rg 'not found|gstreamer|pipewire|usb'
```

If a host Cargo command fails while compiling `libspa-sys` with “Package libpipewire-0.3 was not found,” build in the container. Do not mistake it for a source regression.

## One-time USB permission

```bash
./scripts/install-udev-rule.sh
```

This invokes Polkit only for installing/reloading the narrow udev rule. Reconnect the speakers if the existing node does not receive a user ACL.

Verify:

```bash
lsusb -d 046d:0a78
```

Resolve the current bus/device numbers from that output, then:

```bash
getfacl /dev/bus/usb/BUS/DEVICE
```

Never hard-code a device number; it changes after reconnection.

## CLI reference

### `run`

Desktop portal capture plus live lighting:

```bash
./target/release/logilightshow run
```

The first run opens the system monitor chooser. Later runs request the saved selection.

### `run-gaming`

Direct Bazzite Gamescope capture plus live lighting:

```bash
./target/release/logilightshow run-gaming
```

Run this only in a Gamescope session with a `gamescope` PipeWire node. Normally the user service starts it.

### `capture-test`

New Desktop portal selection:

```bash
./target/release/logilightshow capture-test --frames 30
```

Saved Desktop permission:

```bash
./target/release/logilightshow capture-test --saved-permission --frames 30
```

Gamescope:

```bash
./target/release/logilightshow capture-test --gamescope --frames 30
```

The diagnostic never saves images, but it does print the last sampled zone colors.

### `set-zones`

Direct four-zone RGB test; each value is exactly six hexadecimal digits:

```bash
./target/release/logilightshow set-zones \
  --left-rear FF0000 \
  --left-front 00FF00 \
  --right-front 0000FF \
  --right-rear FFFFFF
```

### `verify-zone`

Black → one raw protocol index white for 12 seconds → black:

```bash
./target/release/logilightshow verify-zone 2
```

Valid raw indexes are 0 through 3. Consult the zone map before interpreting them.

### `verify-soak`

Five-minute rotating four-color hardware/audio test:

```bash
./target/release/logilightshow verify-soak
```

### `calibrate-pacing`

Diagnostic-only sustained report pacing; it never saves the delay:

```bash
./target/release/logilightshow calibrate-pacing --delay-ms 6 --seconds 15
```

Stop all live instances first and keep audio playing.

## Desktop compatibility override

The system-memory path is the Bazzite default. Test the Fedora-style DMA-BUF/GL bridge with:

```bash
LOGILIGHTSHOW_ENABLE_DMABUF=1 ./target/release/logilightshow capture-test --frames 30
```

Do not export this globally on the accepted Bazzite machine; it produced black captured pixels there.

## User state

| Path | Purpose | Expected permissions |
|---|---|---:|
| `~/.config/logilightshow/capture.toml` | portal restore token | `0600` |
| `~/.config/systemd/user/logilightshow-gaming.service` | symlink to repository unit | symlink |
| `~/.config/systemd/user/gamescope-session-plus@steam.service.wants/logilightshow-gaming.service` | Gaming enablement symlink | symlink |
| `/etc/udev/rules.d/70-logilightshow-g560.rules` | host USB access rule | root-owned `0644` |

The capture config contains a version and opaque token, not pixels or monitor images.
