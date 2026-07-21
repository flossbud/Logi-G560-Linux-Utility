# G560 Linux Utility V1 Sampling Refinement Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace rectangular screen regions with the approved four-polygon G560 layout and make every normal color change follow an interruptible 90 ms OKLab transition at a calibrated USB cadence without weakening immediate safety blackouts.

**Architecture:** A normalized `ZoneLayout` compiles into resolution-specific, disjoint pixel masks consumed by the existing weighted-dominant sampler. A clock-driven transition controller sits between latest sampled targets and the USB writer, retargeting from the current interpolated state while the existing engine bypasses it for lock, stall, shutdown, and recovery blackouts.

**Tech Stack:** Existing Rust 1.97.1 crate, palette 0.7 OKLab conversion, Tokio paused-time tests, existing fake frame/light transports, real G560 acceptance test.

## Global Constraints

- The four default polygons cover every captured pixel exactly once with deterministic shared-boundary ownership.
- Default normalized geometry uses apex `(0.50, 0.00)`, knees `(0.17, 0.70)` and `(0.83, 0.70)`, and bottom outer points at `x=0.14` and `x=0.86`.
- Zone order remains `[LeftRear, LeftFront, RightFront, RightRear]` and verified USB mapping remains `[0x02, 0x00, 0x01, 0x03]`.
- Normal target changes use a 90 ms eased OKLab transition that begins immediately and retargets from the last successfully displayed hardware color.
- Sampled targets and transition targets are latest-value only; no stale color or transition may queue.
- Session lock, capture loss/stall, pause, shutdown, and USB recovery cleanup bypass transitions and send immediate black.
- Capture remains single-monitor, transient, and capped at a wall-clock 20 FPS before GL conversion.
- USB reports retain the verified cross-call 20 ms minimum pacing and must not interrupt audio.

---

### Task 1: Polygon layout and compiled masks

**Files:**
- Modify: `src/frame.rs`
- Modify: `src/lib.rs`
- Modify: `src/sampler.rs`
- Modify: `src/engine.rs`
- Modify: `tests/engine_fake.rs`

**Interfaces:**
- Produces: `Point { x: f32, y: f32 }`, `Polygon`, `ZoneLayout::g560_default()`, `ZoneMasks::compile(&ZoneLayout, width, height)`, and `sample_zones(&RgbFrame, &ZoneMasks, SamplerConfig)`.
- Preserves: `ZoneColors` order and the existing engine's newest-frame scheduling.

- [ ] **Step 1: Write failing geometry and sampling tests**

Add tests that compile masks at `160x90`, `1x1`, `2x2`, portrait, and ultrawide resolutions. For every pixel, sum the four membership booleans and assert exactly one. Assert horizontal mirror symmetry between rear masks and between front masks. Assert these representative points:

```rust
assert_eq!(masks.zone_at(0, 0), Zone::LeftRear);
assert_eq!(masks.zone_at(159, 0), Zone::RightRear);
assert_eq!(masks.zone_at(80, 1), Zone::RightFront); // deterministic center boundary
assert_eq!(masks.zone_at(79, 89), Zone::LeftFront);
assert_eq!(masks.zone_at(80, 89), Zone::RightFront);
```

Build a synthetic frame by coloring pixels according to the compiled mask and assert `sample_zones` returns the four assigned solid colors in logical zone order.

- [ ] **Step 2: Run RED tests**

Run: `cargo test frame::tests sampler::tests -- --nocapture`

Expected: compilation fails because polygon and mask APIs do not exist.

- [ ] **Step 3: Implement layout and mask compilation**

Use normalized polygon vertices:

```rust
LeftRear  = [(0.00,0.00), (0.50,0.00), (0.17,0.70), (0.14,1.00), (0.00,1.00)]
LeftFront = [(0.50,0.00), (0.50,1.00), (0.14,1.00), (0.17,0.70)]
RightFront= [(0.50,0.00), (0.83,0.70), (0.86,1.00), (0.50,1.00)]
RightRear = [(0.50,0.00), (1.00,0.00), (1.00,1.00), (0.86,1.00), (0.83,0.70)]
```

Evaluate pixel centers in normalized coordinates. Use a standard point-in-polygon test, but assign exactly once using deterministic priority `[LeftRear, LeftFront, RightFront, RightRear]`; if floating-point edge behavior leaves a pixel unmatched, assign it by left/right half and front/rear boundary rather than silently dropping it. Store each mask as a compact `Vec<usize>` of packed pixel indices. Validate finite coordinates and at least three vertices per polygon.

Delete rectangular `Region` from the production sampler API and update engine/fake call sites to pass compiled masks. Cache masks for the current frame dimensions inside the sampler stage; rebuild only on dimension change.

- [ ] **Step 4: Run GREEN verification and commit**

Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test --all-targets`

Expected: all tests pass and synthetic zone samples match exactly.

```bash
git add src/frame.rs src/lib.rs src/sampler.rs src/engine.rs tests/engine_fake.rs
git commit -m "feat: sample G560 polygon zones"
```

### Task 2: Interruptible OKLab transition controller

**Files:**
- Create: `src/transition.rs`
- Modify: `src/lib.rs`
- Modify: `Cargo.toml`

**Interfaces:**
- Produces: `TransitionController::new(ZoneColors, Duration)`, `retarget(ZoneColors, Instant)`, `colors_at(Instant) -> ZoneColors`, `is_complete(Instant) -> bool`, and a configurable default duration.

- [ ] **Step 1: Write failing fake-time transition tests**

Cover exact endpoints, perceptual midpoint, completion, and interruption:

```rust
let mut t = TransitionController::new(BLACK, Duration::from_millis(90));
t.retarget(RED_ZONES, start);
assert_eq!(t.colors_at(start), BLACK);
assert_midpoint_is_between(
    t.colors_at(start + Duration::from_millis(45)),
    BLACK,
    RED_ZONES,
);
assert_eq!(t.colors_at(start + Duration::from_millis(90)), RED_ZONES);

let midway = t.colors_at(start + Duration::from_millis(45));
t.retarget(BLUE_ZONES, start + Duration::from_millis(45));
assert_eq!(t.colors_at(start + Duration::from_millis(45)), midway);
assert_eq!(t.colors_at(start + Duration::from_millis(135)), BLUE_ZONES);
```

Add a test distinguishing OKLab interpolation from raw sRGB interpolation and a property test proving every output channel stays in `0..=255` for arbitrary endpoints/times.

- [ ] **Step 2: Run RED test**

Run: `cargo test transition::tests -- --nocapture`

Expected: compilation fails because `transition` does not exist.

- [ ] **Step 3: Implement perceptual easing and retargeting**

Convert `Rgb8` sRGB values to `palette::Oklab`, interpolate all components using smoothstep `p = t*t*(3-2*t)`, convert back with clamping, and force the exact target at or after the configured duration. It replaces the target and start time; it never stores a queue. Engine integration retargets from the last successfully displayed hardware value.

The controller is pure and does not sleep, spawn tasks, or know about USB. It represents normal visual transitions only; safety blackouts remain an engine policy.

- [ ] **Step 4: Run GREEN verification and commit**

Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test transition::tests`

Expected: transition tests pass with exact endpoints and stable retargeting.

```bash
git add Cargo.toml Cargo.lock src/lib.rs src/transition.rs
git commit -m "feat: add perceptual color transitions"
```

### Task 3: Integrate transitions without delaying safety blackouts

**Files:**
- Modify: `src/engine.rs`
- Modify: `tests/engine_fake.rs`

**Interfaces:**
- Consumes: `TransitionController` and existing latest sampled `ZoneColors`.
- Preserves: `LightSink::write`, USB pacing, explicit dropped-frame accounting, capture reopen, and unified teardown.

- [ ] **Step 1: Write failing engine behavior tests**

With paused Tokio time and a recording sink, prove:

- A red target produces intermediate colors before exact red at the configured duration.
- A blue target arriving at 60 ms retargets from the current red interpolation and never emits the abandoned red destination.
- Ten targets arriving while one USB write is blocked yield only the newest target after unblock.
- Capture stall, cancellation, source error, and shutdown each make the next sink operation black without waiting for the transition.
- Recovery stays black until a fresh captured frame, then transitions from black toward that frame's target.

- [ ] **Step 2: Run RED tests**

Run: `cargo test engine::tests -- --nocapture && cargo test --test engine_fake -- --nocapture`

Expected: new transition assertions fail because sampled colors still write directly.

- [ ] **Step 3: Add latest-target transition scheduling**

Give the single sink writer ownership of `TransitionController`. Sampled targets arrive through the existing capacity-one latest channel. The writer selects between a new target and the next hardware write opportunity; new targets retarget immediately, while actual writes remain serialized through verified USB pacing. Use wall-clock elapsed time, not a fixed number of steps, so slow USB cannot stretch the configured transition indefinitely.

Represent safety blackout as an out-of-band writer command with priority over normal targets. On safety command: discard the pending target, reset controller state to black, write black immediately, and acknowledge completion to teardown/recovery logic. The first fresh target after recovery transitions from black.

- [ ] **Step 4: Verify and commit**

Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test --all-targets`

Expected: all transition, engine, recovery, and integration tests pass.

```bash
git add src/engine.rs tests/engine_fake.rs
git commit -m "feat: smooth live lighting transitions"
```

### Task 4: Live refinement acceptance and milestone documentation

**Files:**
- Modify: `README.md`
- Modify: `docs/hardware/milestone-1-results.md`
- Modify: `docs/hardware/quadrant-test.html`

**Interfaces:**
- Produces: documented polygon mapping, calibrated transition behavior, measured live result, and v1 known limitations.

- [ ] **Step 1: Update the deterministic visual fixture**

Change the quadrant fixture to draw the approved polygon boundaries and cycle each logical polygon through unmistakable colors. Keep the page usable windowed or fullscreen and continuously animate content so GNOME direct-scanout capture is exercised.

- [ ] **Step 2: Run a short real-hardware A/B acceptance test**

Run the release engine for 90 seconds using the fixture on the selected monitor. Confirm with the user:

- All screen pixels visually belong to one intended polygon.
- Each polygon controls the verified physical zone.
- Changes visibly fade without snapping.
- The response does not feel delayed.
- Fullscreen capture remains live through the DMA-BUF/GL bridge.
- Lock triggers immediate black; unlock plus fresh content resumes with a fade from black.
- Audio is uninterrupted and Ctrl-C ends all zones black.

Record capture/render rates, transition duration, p50/p95/p99 capture-to-write latency, stalls, USB errors, CPU, and RSS. Do not repeat the rejected 30-minute fullscreen pattern. Preserve the explicitly approved non-disruptive mixed-use soak substitution and its limitations.

- [ ] **Step 3: Complete automated and documentation checks**

Run:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
bash -n scripts/install-udev-rule.sh
git diff --check
```

Update README and results with exact observed values, polygon vertices, immediate-safety exception, calibrated default, direct-scanout bridge, Fedora/Bazzite prerequisites, privacy behavior, and user acceptance feedback.

- [ ] **Step 4: Commit**

```bash
git add README.md docs/hardware/milestone-1-results.md docs/hardware/quadrant-test.html
git commit -m "docs: verify refined v1 screen matching"
```

### Task 5: Calibrate USB cadence and finalize faster smoothing

**Files:**
- Modify: `src/usb/device.rs`
- Modify: `src/main.rs`
- Modify: `src/transition.rs`
- Modify: `src/engine.rs`
- Modify: `README.md`
- Modify: `docs/hardware/milestone-1-results.md`
- Modify: `docs/hardware/quadrant-test.html`

**Interfaces:**
- Produces: diagnostic `calibrate-pacing --delay-ms N --seconds N`, a documented calibrated `REPORT_DELAY`, and `DEFAULT_TRANSITION_DURATION = 90ms`.

- [ ] **Step 1: Write failing calibration and duration tests**

Add fake-transport tests proving a caller-supplied calibration delay is used between every adjacent report across API calls, invalid delays below 1 ms are rejected before USB open, transfer failures stop the calibration immediately, and final cleanup attempts all-zone black. Update transition and engine fake-time tests to assert the 45 ms midpoint and exact 90 ms endpoint. Add an easing-specific assertion at 22.5 ms so replacing smoothstep with linear time fails.

- [ ] **Step 2: Run RED tests**

Run: `cargo test usb:: transition:: engine::tests -- --nocapture`

Expected: the calibration command/configuration and 90 ms default assertions fail.

- [ ] **Step 3: Implement bounded diagnostic calibration**

Keep production `G560::open()` tied to its compile-time `REPORT_DELAY`. Add a diagnostic constructor that accepts a validated delay and the existing verified transport. `calibrate-pacing` rotates four distinct colors continuously for the requested duration, prints attempted/successful report counts and the first error, always attempts paced black cleanup, and exits nonzero on any error. The diagnostic never writes configuration automatically.

Test delays `18, 16, 14, 12, 10, 8, 6, 4` milliseconds in descending order for 15 seconds each while audio plays. Stop after the first failed interval; do not test shorter intervals after failure. Select the fastest error-free interval plus a 2 ms safety margin, never lower than 4 ms. Run the selected production candidate for two minutes with rotating four-zone colors and require zero USB errors, uninterrupted audio, correct zone rotation, and final black.

- [ ] **Step 4: Apply the calibrated production values**

Set `REPORT_DELAY` to the confirmed safe interval and `DEFAULT_TRANSITION_DURATION` to 90 ms. Re-run exact cross-call pacing, blackout, transition, latest-target, safety-preemption, and full engine tests. Update all user-facing fixture text and documentation with measured rather than planned values.

- [ ] **Step 5: Run short perceptual acceptance**

Run the polygon fixture fullscreen for 60 seconds. Confirm with the user that mapping remains correct, transitions are visibly smoother than the 120 ms/~9.6 updates/s baseline, response feels faster, audio is uninterrupted, lock blackout remains immediate, unlock resumes from black, and Ctrl-C ends black. Record capture/render cadence, p50/p95/p99 latency, CPU, RSS, stalls, and USB errors.

- [ ] **Step 6: Verify and commit**

Run:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
bash -n scripts/install-udev-rule.sh
git diff --check
```

```bash
git add Cargo.toml Cargo.lock src README.md docs/hardware
git commit -m "perf: calibrate smooth lighting cadence"
```

---

## Completion gate

V1 sampling refinement is complete only when polygon coverage tests, transition retargeting tests, calibrated USB pacing, immediate safety-blackout tests, full automated checks, and the real-hardware A/B test all pass. Any visible snap, added dead time, wrong physical zone, fullscreen capture loss, audio interruption, USB error at the selected production interval, or delayed safety blackout blocks completion.
