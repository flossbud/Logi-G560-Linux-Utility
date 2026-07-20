# LogiLightShow V1 Sampling Refinement Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace rectangular screen regions with the approved four-polygon G560 layout and make every normal color change follow an interruptible 120 ms OKLab transition without weakening immediate safety blackouts.

**Architecture:** A normalized `ZoneLayout` compiles into resolution-specific, disjoint pixel masks consumed by the existing weighted-dominant sampler. A clock-driven transition controller sits between latest sampled targets and the USB writer, retargeting from the current interpolated state while the existing engine bypasses it for lock, stall, shutdown, and recovery blackouts.

**Tech Stack:** Existing Rust 1.97.1 crate, palette 0.7 OKLab conversion, Tokio paused-time tests, existing fake frame/light transports, real G560 acceptance test.

## Global Constraints

- The four default polygons cover every captured pixel exactly once with deterministic shared-boundary ownership.
- Default normalized geometry uses apex `(0.50, 0.00)`, knees `(0.17, 0.70)` and `(0.83, 0.70)`, and bottom outer points at `x=0.14` and `x=0.86`.
- Zone order remains `[LeftRear, LeftFront, RightFront, RightRear]` and verified USB mapping remains `[0x02, 0x00, 0x01, 0x03]`.
- Normal target changes use a 120 ms eased OKLab transition that begins immediately and retargets from the current interpolated color.
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

### Task 2: Interruptible 120 ms OKLab transition controller

**Files:**
- Create: `src/transition.rs`
- Modify: `src/lib.rs`
- Modify: `Cargo.toml`

**Interfaces:**
- Produces: `TransitionController::new(ZoneColors, Duration)`, `retarget(ZoneColors, Instant)`, `colors_at(Instant) -> ZoneColors`, `is_complete(Instant) -> bool`, and constant `DEFAULT_TRANSITION_DURATION = 120ms`.

- [ ] **Step 1: Write failing fake-time transition tests**

Cover exact endpoints, perceptual midpoint, completion, and interruption:

```rust
let mut t = TransitionController::new(BLACK, Duration::from_millis(120));
t.retarget(RED_ZONES, start);
assert_eq!(t.colors_at(start), BLACK);
assert_midpoint_is_between(
    t.colors_at(start + Duration::from_millis(60)),
    BLACK,
    RED_ZONES,
);
assert_eq!(t.colors_at(start + Duration::from_millis(120)), RED_ZONES);

let midway = t.colors_at(start + Duration::from_millis(60));
t.retarget(BLUE_ZONES, start + Duration::from_millis(60));
assert_eq!(t.colors_at(start + Duration::from_millis(60)), midway);
assert_eq!(t.colors_at(start + Duration::from_millis(180)), BLUE_ZONES);
```

Add a test distinguishing OKLab interpolation from raw sRGB interpolation and a property test proving every output channel stays in `0..=255` for arbitrary endpoints/times.

- [ ] **Step 2: Run RED test**

Run: `cargo test transition::tests -- --nocapture`

Expected: compilation fails because `transition` does not exist.

- [ ] **Step 3: Implement perceptual easing and retargeting**

Convert `Rgb8` sRGB values to `palette::Oklab`, interpolate all components using smoothstep `p = t*t*(3-2*t)`, convert back with clamping, and force the exact target at or after 120 ms. `retarget` first evaluates the current transition at `now`, then uses that value as the next start. It replaces the target and start time; it never stores a queue.

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

- A red target produces intermediate colors before exact red at 120 ms.
- A blue target arriving at 60 ms retargets from the current red interpolation and never emits the abandoned red destination.
- Ten targets arriving while one USB write is blocked yield only the newest target after unblock.
- Capture stall, cancellation, source error, and shutdown each make the next sink operation black without waiting 120 ms.
- Recovery stays black until a fresh captured frame, then transitions from black toward that frame's target.

- [ ] **Step 2: Run RED tests**

Run: `cargo test engine::tests -- --nocapture && cargo test --test engine_fake -- --nocapture`

Expected: new transition assertions fail because sampled colors still write directly.

- [ ] **Step 3: Add latest-target transition scheduling**

Give the single sink writer ownership of `TransitionController`. Sampled targets arrive through the existing capacity-one latest channel. The writer selects between a new target and the next hardware write opportunity; new targets retarget immediately, while actual writes remain serialized through verified USB pacing. Use wall-clock elapsed time, not a fixed number of steps, so slow USB cannot stretch a 120 ms transition indefinitely.

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
- Produces: documented polygon mapping, 120 ms transition behavior, measured live result, and v1 known limitations.

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

Update README and results with exact observed values, polygon vertices, immediate-safety exception, 120 ms default, direct-scanout bridge, Fedora/Bazzite prerequisites, privacy behavior, and the user's “overreactive” feedback as resolved or as a clearly named remaining tuning concern.

- [ ] **Step 4: Commit**

```bash
git add README.md docs/hardware/milestone-1-results.md docs/hardware/quadrant-test.html
git commit -m "docs: verify refined v1 screen matching"
```

---

## Completion gate

V1 sampling refinement is complete only when polygon coverage tests, transition retargeting tests, immediate safety-blackout tests, full automated checks, and the real-hardware A/B test all pass. Any visible snap, added dead time, wrong physical zone, fullscreen capture loss, audio interruption, or delayed safety blackout blocks completion.
