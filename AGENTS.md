# G560 Linux Utility agent guide

This file is the fastest safe entry point for an agent opening the repository on any machine.

## Read this first

Read these documents in order before changing runtime behavior:

1. [`docs/project-status.md`](docs/project-status.md) — what is complete, what is intentionally uncommitted, and what is not built yet.
2. [`docs/architecture.md`](docs/architecture.md) — end-to-end data flow, concurrency, recovery, and safety paths.
3. [`docs/build-and-run.md`](docs/build-and-run.md) — Fedora, Bazzite, and Arch dependencies and exact commands.
4. [`docs/testing.md`](docs/testing.md) — required automated and hardware checks.
5. The topic-specific document linked from [`docs/README.md`](docs/README.md).

`README.md` is the user-facing overview. `HANDOFF_BAZZITE.md` records the machine-specific port and acceptance.

## Current repository state

- Product: an experimental Rust CLI that matches one Linux display to the four lighting zones on Logitech G560 speakers.
- Tested hardware: Logitech G560 USB `046d:0a78`, firmware `90.64` during protocol verification.
- Tested environments: Fedora GNOME Wayland; Bazzite 43 GNOME Desktop Mode; Bazzite Gaming Mode/Gamescope; real in-game use; Arch Linux KDE Plasma 6 Wayland portal capture and live engine operation.
- The accepted Bazzite port is committed as `348e08c` on `main`.
- The three Bazzite snapshot archives remain intentionally untracked evidence. Do not discard, reset, clean, or treat them as generated junk.
- The original Fedora-transfer archive and the later final snapshot archive are evidence/artifacts, not build inputs.
- There is no configured Git remote in the accepted workspace. Do not publish or commit unless the owner explicitly asks.

Always run `git status --short` before editing. Preserve unrelated changes.

## Non-negotiable behavior

Do not weaken these invariants:

- Capture exactly one monitor in Desktop Mode. The portal chooser is authoritative; zero or multiple streams are errors.
- Never run G560 Linux Utility as root. Polkit is used only by the udev installer.
- Never save screenshots, frames, sampled colors, thumbnails, or color history. Runtime metrics must remain color-free.
- Use newest-value-only handoffs. Do not introduce an accumulating frame or color queue.
- Safety blackouts preempt normal transitions for capture loss/stall, cancellation, shutdown, and USB recovery.
- Normal sampled darkness may fade; safety blackouts must remain immediate.
- Keep USB report ordering and the calibrated 6 ms delay across every adjacent HID report, including across calls.
- Do not detach a non-HID interface. The USB driver validates interface 2 before claiming it and reattaches the kernel driver on release.
- Preserve the physical zone mapping: logical `[left rear, left front, right front, right rear]` maps to protocol indexes `[0x02, 0x00, 0x01, 0x03]`.
- Preserve clean shutdown: stop capture, request all-zone black, release USB, and close the portal/worker.
- Desktop and Gaming Mode intentionally use different capture backends. Do not try to force the desktop portal into Gamescope.

## Environment rules

On mutable Fedora, install development packages on the host. On immutable Bazzite, build in the `logig560` Distrobox/Toolbox and run the resulting host-mounted binary in the host user session.

The accepted Bazzite workspace uses:

```text
/home/jaret/Documents/G560 Linux Utility
```

Portable documentation must not assume that path. The checked-in Gaming Mode unit currently does assume `%h/Documents/G560 Linux Utility`; update the unit and documentation together if the repository moves.

Use this on the accepted Bazzite machine:

```bash
distrobox enter logig560 -- bash -lc \
  'cd /home/jaret/Documents/G560 Linux Utility && cargo test --all-targets'
```

The host lacks PipeWire development metadata, so a direct host `cargo build` or `cargo test` can fail in `libspa-sys`. That is an environment error, not a Rust regression.

## Source orientation

| Area | Primary files | Responsibility |
|---|---|---|
| CLI and process lifecycle | `src/main.rs` | Commands, config token, recovery factories, metrics, signals |
| Engine | `src/engine.rs` | newest-frame pipeline, sampling, transitions, writer, stalls, recovery |
| Desktop capture | `src/capture/portal.rs`, `src/capture/gstreamer.rs` | portal authorization, PipeWire/GStreamer frames, caps, static-frame hold |
| Gaming capture | `src/capture/gamescope.rs` | direct Gamescope PipeWire node, MemFd BGRx decode, 20 FPS pacing |
| Geometry | `src/frame.rs` | RGB frame validation, polygons, disjoint zone masks |
| Color sampling | `src/sampler.rs` | darkness ramp, weighted Lab bins, deterministic low-light blending |
| Transitions | `src/transition.rs` | interruptible per-zone OKLab smoothstep interpolation |
| Latest-only channel | `src/latest.rs` | one unread value, replacement accounting |
| USB | `src/usb/device.rs`, `src/usb/protocol.rs` | device claim, HID encoding, 6 ms pacing, async worker, blackout |
| Integration | `systemd/`, `scripts/`, `contrib/` | Gaming service and udev access |

## Change routing

- Capture problems: start with `docs/capture-backends.md` and `docs/troubleshooting.md`.
- Flicker/color problems: first distinguish `capture_stalls` from normal sampled black; then read `docs/color-pipeline.md`.
- USB/audio/device problems: read `docs/usb-and-safety.md` before touching pacing or interface code.
- Service/session problems: read `docs/operations.md`.
- Architecture changes: add or update a record under `docs/decisions/`.

## Required validation

For documentation-only edits:

```bash
git diff --check
bash -n scripts/*.sh
systemd-analyze --user verify systemd/logig560-gaming.service
```

For Rust, dependency, runtime, service, or script changes:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
cargo build --release
bash -n scripts/*.sh
systemd-analyze --user verify systemd/logig560-gaming.service
git diff --check
```

Run Cargo commands inside the development container on Bazzite. Hardware checks are additional, not substitutes for automated checks. Never run destructive calibration or zone pulses while another G560 Linux Utility process owns the G560.

## Live-operation discipline

- Inspect `systemctl --user` and `pgrep -af logig560` before starting a second instance.
- Desktop prototype runs may use a transient `logig560-desktop-live.service`.
- `logig560-gaming.service` should be enabled but inactive in Desktop Mode; it starts with `gamescope-session-plus@steam.service`.
- Stop an existing instance cleanly before replacing `target/release/logig560` for live verification.
- Correlate visual reports with journal timestamps. All-zone off/on plus a rising `stalls` counter is a capture safety event; a low-light target without a stall is a sampler/transition event.
- Do not claim a visual issue fixed until the user confirms it on the physical speakers.

## Documentation maintenance

When behavior changes, update all affected current documents:

- `README.md` for user-visible behavior or commands.
- `docs/project-status.md` for completion/limitations.
- The relevant subsystem document.
- `HANDOFF_BAZZITE.md` for machine-specific behavior.
- `docs/decisions/` when a design choice or invariant changes.

Keep historical acceptance documents intact unless correcting a factual error; add a clearly dated current note instead of rewriting the past.
