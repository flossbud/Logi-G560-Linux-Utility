# Testing and verification

## Final automated gate

Run from a development environment with all native headers:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
cargo build --release
bash -n scripts/install-udev-rule.sh \
  scripts/install-gaming-service.sh \
  scripts/uninstall-gaming-service.sh
systemd-analyze --user verify systemd/logig560-gaming.service
git diff --check
```

On the accepted Bazzite machine:

```bash
distrobox enter logig560 -- bash -lc \
  'cd /home/jaret/Documents/G560 Linux Utility && \
   cargo clippy --all-targets -- -D warnings && \
   cargo test --all-targets && \
   cargo build --release'
```

Host `cargo test` can fail before compiling project code if PipeWire development `.pc` files are absent. Use the container.

## Test organization

| Area | Location | Coverage style |
|---|---|---|
| Frame/layout | `src/frame.rs` tests | exact ownership, symmetry, complete/disjoint masks |
| Sampler | `src/sampler.rs` tests | deterministic examples, low-light ramps, proptest no-panic |
| Transitions | `src/transition.rs` tests | exact time points, OKLab, interruption, per-zone duration, proptest |
| Latest channel | `src/latest.rs`, `tests/engine_fake.rs` | replacement/readiness semantics |
| Engine | `src/engine.rs` tests | paused Tokio time, fake sources/sinks, race/safety/recovery behavior |
| Desktop capture | `src/capture/gstreamer.rs` tests | real in-process GStreamer pipelines, caps/stride/hold/error paths |
| Portal | `src/capture/portal.rs` tests | selection options, cancellation, cleanup at every failure stage |
| Gaming capture | `src/capture/gamescope.rs` tests | non-drifting pacing and late callbacks |
| USB | `src/usb/` tests | fake transport/delay, encoding, queue ordering, pacing, cleanup |
| CLI/config | `src/main.rs` tests | parsing, retry classification, atomic private token writes |

At prototype wrap-up the suite contained 95 library tests, 8 CLI tests, and 3 integration tests (106 total).

## Focused commands

```bash
cargo test capture:: -- --nocapture
cargo test frame::tests sampler::tests -- --nocapture
cargo test transition::tests engine::tests -- --nocapture
cargo test usb:: -- --nocapture
cargo test --test engine_fake -- --nocapture
```

## Change-specific matrix

| Change | Minimum additional evidence |
|---|---|
| Portal options/config | portal unit tests; live chooser/cancel/save/restore |
| GStreamer pipeline/caps | capture tests; portrait/ultrawide/caps change; live non-black Desktop capture |
| Gamescope worker/format | pacing tests; real `capture-test --gamescope`; Gaming shell and in-game |
| Geometry | coverage/symmetry tests; quadrant page; physical mapping confirmation |
| Sampler | low-light tests; representative fixtures; user visual confirmation |
| Transition/writer | paused-time tests; retarget and safety-preemption tests; physical smoothness |
| Stall/recovery | fake stall/error tests; journal observation; lock/suspend/reopen |
| USB protocol/index | fake encoding plus deliberate physical zone pulse |
| USB cadence/worker | queue/pacing tests; audio-playing hardware soak/calibration |
| Service unit/scripts | shell syntax; systemd verify; actual Desktop↔Gaming lifecycle |

## Non-invasive capture tests

Desktop:

```bash
./target/release/logig560 capture-test --frames 30
```

On Arch/KDE, first verify that both layers needed by the same Desktop command are present:

```bash
systemctl --user is-active plasma-xdg-desktop-portal-kde.service
gst-inspect-1.0 pipewiresrc
```

Saved selection:

```bash
./target/release/logig560 capture-test --saved-permission --frames 30
```

Gaming Mode:

```bash
./target/release/logig560 capture-test --gamescope --frames 30
```

These save no images. They do print one sampled zone-color set, so do not paste output into public logs without considering that disclosure.

Pass evidence should include:

- expected dimensions near 160 pixels wide;
- non-black frames for visible content;
- meaningful peak component/visible pixel counts;
- approximately intended FPS;
- clean source shutdown.

## Hardware checks

Hardware tests are state-changing and must be deliberate.

### Static zone check

Use `set-zones` to isolate USB/protocol from capture. Confirm each physical surface receives the intended logical color.

### Quadrant visual check

```bash
xdg-open docs/hardware/quadrant-test.html
```

Move it to the selected monitor and optionally fullscreen. Confirm screen quadrants map to the expected speaker zones.

### Lock/suspend safety

Confirm:

1. Lights are active before lock.
2. Lock/capture loss produces immediate black.
3. Unlock resumes only after a fresh frame.
4. `stalls`/capture recovery logs match the event.

### Gaming lifecycle

Confirm:

1. Gaming service is inactive in Desktop.
2. Entering real Gaming Mode starts it.
3. Steam shell content responds.
4. A real game responds.
5. Exiting Gaming Mode stops it and blackouts cleanly.

### USB/audio soak

Keep G560 audio playing while lights change. Any audible interruption, transfer error, wrong mapping, or interface conflict fails the check.

## Safety regression rules

Never accept a change solely because colors look better. It must retain:

- immediate safety priority;
- no stale relight after blackout;
- final blackout on clean stop;
- cancellation/backoff preemption;
- expired target suppression;
- FIFO report order and inter-call 6 ms pacing;
- kernel-driver reattach behavior.

## Documentation validation

There is no dedicated link checker yet. At minimum:

```bash
git diff --check
rg -n '\]\([^)]*\.md' README.md AGENTS.md HANDOFF_BAZZITE.md docs
```

Manually verify new relative links and update `docs/README.md` when adding a current document.
