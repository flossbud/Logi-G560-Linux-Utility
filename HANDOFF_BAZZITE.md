# G560 Linux Utility — Bazzite handoff

For a new agent, begin with [`AGENTS.md`](AGENTS.md), [`docs/project-status.md`](docs/project-status.md), and the [`docs` index](docs/README.md). This file is the machine-specific Bazzite acceptance record.

## Prototype acceptance — 2026-07-20

The Bazzite prototype is accepted on the same physical machine used for the original Fedora implementation. The user confirmed Desktop Mode, the real Gaming Mode session, and in-game operation. After the final capture-hold and near-black fade refinements, the user described operation as “extremely well” and approved wrapping up the prototype.

Final behavior on this machine:

- Desktop Mode uses the saved single-monitor GNOME portal authorization and the system-memory GStreamer path.
- Gaming Mode uses the enabled `logig560-gaming.service` and direct Gamescope PipeWire capture; it is inactive in Desktop Mode by design.
- Brief healthy portal silence holds the last valid frame after 200 ms, preventing false 500 ms stall blackouts. Terminal capture errors and EOS retain the safety path.
- The low-light sampler uses a soft visibility ramp and deterministic blending.
- Normal color changes use 90 ms per-zone OKLab transitions. A normally sampled zone going fully black uses a 200 ms fade. Safety blackouts remain immediate.
- Final automated verification: 95 library tests, 8 CLI tests, and 3 integration tests passed (106 total); Clippy passed with warnings denied; the optimized release build succeeded.

At handoff, the transient Desktop test service is running and the Gaming Mode service is enabled. Source changes are intentionally left uncommitted for the owner to review and commit.

`G560 Linux Utility-Bazzite-agent-handoff.zip` is the current portable source-and-documentation snapshot. The older `G560 Linux Utility-Bazzite-handoff.zip` and `G560 Linux Utility-Bazzite-prototype-final.zip` files are preserved historical snapshots; do not silently replace them.

## What is here

This repository contains the completed prototype screen-matching engine for Logitech G560 speakers. It captures one selected display, computes four speaker-specific colors, and sends them over USB with smooth OKLab transitions. Desktop Mode and a Bazzite Gaming Mode user service are implemented; a polished GUI and distributable packaging are still future work.

Hardware facts established during testing:

- Logitech G560 USB ID: `046d:0a78`
- Zones: `0x00` left front, `0x01` right front, `0x02` left rear, `0x03` right rear
- Capture geometry is the asymmetric four-polygon layout in `src/frame.rs` (front zones meet at center; rear zones occupy the outer regions).
- USB worker is FIFO/non-blocking from the capture loop; control writes have a 100 ms timeout and recovery logic.
- Production defaults are approximately 18–20 FPS capture, 6 ms USB report spacing, 90 ms normal transitions, and 200 ms normal fades to exact black. Do not lower USB spacing without re-running the hardware cadence test.

Desktop Mode defaults to GStreamer's system-memory PipeWire path on Bazzite. The Fedora-tested DMA-BUF/GL bridge remains available with `LOGIG560_ENABLE_DMABUF=1`, but Bazzite's current Mesa/GStreamer GL download produced black pixels on this machine.

GNOME's portal capture can leave a healthy PipeWire stream temporarily silent during static or low-motion scenes. The Desktop capture backend now holds the last dimension-validated frame after 200 ms without a new buffer. This keeps the engine below its 500 ms capture-stall cutoff without duplicating the normal ~19 FPS feed. GStreamer bus errors, EOS, caps renegotiation, and startup without a valid frame bypass the hold.

## Bazzite constraints

Bazzite is immutable. Build and development tools belong in a Distrobox/Toolbox (or Bazzite-DX), while the USB permission rule belongs on the host. Installing the rule requires Polkit authentication. Do not attempt to make the whole host mutable with package-manager workarounds.

The accepted machine uses Bazzite 43 GNOME (`bazzite-deck-gnome`) and a Fedora 43 Distrobox named `logig560`.

## Desktop bring-up

Inside the development container, install build dependencies:

```bash
sudo dnf install -y \
  gcc pkgconf-pkg-config \
  gstreamer1-devel gstreamer1-plugins-base-devel \
  gstreamer1-plugins-bad-free-devel pipewire-devel libusb1-devel
```

Build with the repository mounted from the host:

```bash
distrobox enter logig560 -- bash -lc \
  'cd /home/jaret/Documents/G560 Linux Utility && cargo test --all-targets && cargo build --release'
```

On the Bazzite host, install the permission rule and verify the device:

```bash
cd /home/jaret/Documents/G560 Linux Utility
./scripts/install-udev-rule.sh
lsusb -d 046d:0a78
```

Unplug/replug the G560 if the current USB node did not receive a user ACL. Run the release binary on the host as the logged-in user:

```bash
./target/release/logig560 capture-test --saved-permission --frames 30
./target/release/logig560 run
```

The portal is the authority for monitor selection. Exactly one monitor is requested and accepted.

## Gaming Mode integration

Bazzite's `gamescope-session-plus` deliberately disables desktop portals, so the saved GNOME portal permission cannot be reused in Gaming Mode. `src/capture/gamescope.rs` instead connects directly to Gamescope's native PipeWire node named `gamescope`. It advertises BGRx without a DRM modifier, selecting Gamescope's CPU-mapped MemFd offer and avoiding the all-black DMA-BUF readback seen on this machine. The capture handoff is a one-slot newest-frame channel and uses a non-drifting 20 FPS deadline.

Commands:

```bash
./target/release/logig560 capture-test --gamescope --frames 5
./target/release/logig560 run-gaming
./scripts/install-gaming-service.sh
./scripts/uninstall-gaming-service.sh
```

`systemd/logig560-gaming.service` is enabled as a user unit under `gamescope-session-plus@steam.service.wants`. It is `PartOf=gamescope-session-plus@steam.service`, starts only with the Steam Gamescope session, retries failures, and receives SIGTERM on session shutdown so the engine can black out the speakers. It does not use a portal, claim audio interfaces, or require root.

A nested Gamescope test with a rendered GL scene captured non-black 160×90 frames at a steady 20 FPS, with distinct sampled zone colors. The user then confirmed the real DRM Gaming Mode service works in both the Steam shell and an in-game session and shuts down on return to Desktop Mode.

## Low-light refinements

The shared sampler treats `darkness_luma` as the midpoint of a soft visibility ramp rather than a hard cutoff. Very small visible coverage fades in proportionally, and ambiguous low-light bins blend deterministically. This prevents near-black pixels around RGB 32/33 and similarly weighted dark hues from toggling between black or unrelated dominant colors on adjacent frames.

Normal sampled transitions use 90 ms per zone, except a non-black zone whose target becomes fully black fades over 200 ms. Other zones retain their 90 ms response during that fade. Safety blackouts for capture loss/stall, lock, shutdown, and recovery bypass the transition controller and remain immediate.

## Source map and verification

- `AGENTS.md` — agent workflow, hard invariants, and change routing
- `docs/README.md` — current documentation index
- `src/` — capture, geometry, sampling, transitions, engine, and USB
- `scripts/` — udev and Gaming service installation/removal
- `systemd/` — Gaming Mode user unit
- `README.md` — user-facing usage and behavior
- `docs/hardware/` — physical zone and Fedora acceptance evidence

Before changing behavior, use the complete gate in [`docs/testing.md`](docs/testing.md). On Bazzite, run Cargo commands in the development container.

Known caveats and future work are centralized in [`docs/known-limitations.md`](docs/known-limitations.md).
