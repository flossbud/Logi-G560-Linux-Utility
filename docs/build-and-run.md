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

## Arch Linux KDE Plasma host

Arch is mutable, so install build and Desktop runtime dependencies on the host:

```bash
sudo pacman -S --needed \
  base-devel pkgconf rustup \
  gstreamer gst-plugins-base gst-plugins-bad gst-plugin-pipewire \
  pipewire libusb xdg-desktop-portal xdg-desktop-portal-kde
rustup toolchain install 1.97.1
cargo build --release
```

Run the release binary in the unprivileged Plasma Wayland user session. Verify that KDE's portal and GStreamer's PipeWire source are both available:

```bash
systemctl --user status xdg-desktop-portal.service \
  plasma-xdg-desktop-portal-kde.service --no-pager
gst-inspect-1.0 pipewiresrc videoconvert videoscale
```

On the validated Arch Plasma 6 host, `gst-plugin-pipewire` was the only missing prerequisite. The existing system-memory Desktop pipeline then produced non-black 160x90 frames and drove the live engine without a KDE-specific source branch.

Verify capture elements:

```bash
gst-inspect-1.0 pipewiresrc videoconvert videoscale
```

The optional DMA-BUF path additionally needs:

```bash
gst-inspect-1.0 glupload glcolorconvert gldownload videorate
```

## Bazzite development container

Bazzite is immutable. Do not layer a development toolchain onto the host merely to build this prototype. The tested setup is a Fedora 43 Distrobox named `logig560`.

Create it once if necessary:

```bash
distrobox create --name logig560 \
  --image registry.fedoraproject.org/fedora-toolbox:43
```

Install native development dependencies inside it:

```bash
distrobox enter logig560 -- sudo dnf install -y \
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
distrobox enter logig560 -- bash -lc \
  'cd /home/jaret/Documents/G560 Linux Utility && cargo build --release'
```

Run the binary on the host, not with `sudo`, so it sees the real Wayland/PipeWire user session and USB access:

```bash
./target/release/logig560 --help
```

The accepted host provides runtime `libgstreamer-1.0`, `libpipewire-0.3`, and `libusb-1.0`. Check portability after moving the binary:

```bash
ldd target/release/logig560 | rg 'not found|gstreamer|pipewire|usb'
```

If a host Cargo command fails while compiling `libspa-sys` with “Package libpipewire-0.3 was not found,” build in the container. Do not mistake it for a source regression.

## One-time USB permission

```bash
./scripts/install-udev-rule.sh
```

This invokes Polkit only for installing/reloading the narrow udev rule. The installer replays an add event for the matching device so an already-connected G560 receives the user ACL; reconnect the speakers if the ACL still does not appear.

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
./target/release/logig560 run
```

The first run opens the system monitor chooser. Later runs request the saved selection.

### `run-gaming`

Direct Bazzite Gamescope capture plus live lighting:

```bash
./target/release/logig560 run-gaming
```

Run this only in a Gamescope session with a `gamescope` PipeWire node. Normally the user service starts it.

### `capture-test`

New Desktop portal selection:

```bash
./target/release/logig560 capture-test --frames 30
```

Saved Desktop permission:

```bash
./target/release/logig560 capture-test --saved-permission --frames 30
```

Gamescope:

```bash
./target/release/logig560 capture-test --gamescope --frames 30
```

The diagnostic never saves images, but it does print the last sampled zone colors.

### `set-zones`

Direct four-zone RGB test; each value is exactly six hexadecimal digits:

```bash
./target/release/logig560 set-zones \
  --left-rear FF0000 \
  --left-front 00FF00 \
  --right-front 0000FF \
  --right-rear FFFFFF
```

### `verify-zone`

Black → one raw protocol index white for 12 seconds → black:

```bash
./target/release/logig560 verify-zone 2
```

Valid raw indexes are 0 through 3. Consult the zone map before interpreting them.

### `verify-soak`

Five-minute rotating four-color hardware/audio test:

```bash
./target/release/logig560 verify-soak
```

### `calibrate-pacing`

Diagnostic-only sustained report pacing; it never saves the delay:

```bash
./target/release/logig560 calibrate-pacing --delay-ms 6 --seconds 15
```

Stop all live instances first and keep audio playing.

## Desktop compatibility override

The system-memory path is the Bazzite default. Test the Fedora-style DMA-BUF/GL bridge with:

```bash
LOGIG560_ENABLE_DMABUF=1 ./target/release/logig560 capture-test --frames 30
```

Do not export this globally on the accepted Bazzite machine; it produced black captured pixels there.

## User state

| Path | Purpose | Expected permissions |
|---|---|---:|
| `~/.config/logig560/capture.toml` | portal restore token | `0600` |
| `~/.config/systemd/user/logig560-gaming.service` | symlink to repository unit | symlink |
| `~/.config/systemd/user/gamescope-session-plus@steam.service.wants/logig560-gaming.service` | Gaming enablement symlink | symlink |
| `/etc/udev/rules.d/70-g560.rules` | host USB access rule | root-owned `0644` |

The capture config contains a version and opaque token, not pixels or monitor images.

## Build the AppImage

The distributable is built on Ubuntu 22.04 for glibc portability. To
reproduce locally:

```bash
sudo apt-get install -y \
  build-essential pkg-config \
  libwebkit2gtk-4.1-dev libayatana-appindicator3-dev \
  libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev \
  libgstreamer-plugins-bad1.0-dev gstreamer1.0-pipewire \
  libpipewire-0.3-dev libusb-1.0-0-dev libgtk-3-dev libssl-dev \
  patchelf desktop-file-utils fuse libfuse2

./scripts/build-appimage.sh
# → dist/G560-Linux-Utility-<version>-x86_64.AppImage
```

Building on Bazzite (via the `logig560` distrobox) is possible but the
resulting AppImage is linked against Fedora 43's glibc and will not run
on Ubuntu 22.04 / Debian 12 hosts. Use the Ubuntu recipe (or CI) for
publishable builds.
