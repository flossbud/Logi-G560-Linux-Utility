# Development guide

## Working model

G560 Linux Utility is a Rust 2024 project with a library plus one CLI binary. Most behavior is expressed behind small traits and tested with in-process fakes.

Start every task with:

```bash
git status --short
git log -1 --oneline
```

The accepted Bazzite port is intentionally uncommitted in the original workspace. Preserve it.

## Design principles

1. Prefer current state over completeness: latest-only channels replace obsolete frames/targets.
2. Separate normal visual behavior from safety behavior.
3. Normalize capture backends into one strict RGB frame type.
4. Keep blocking native I/O off Tokio worker threads.
5. Fail closed on ambiguous monitor selection, caps, frame layout, USB interface class, or report length.
6. Make cleanup explicit and preserve cleanup errors with primary failures.
7. Keep runtime metrics useful but color-free.
8. Back hardware constants with recorded physical evidence.

## Error conventions

- Use typed `CaptureError` and `UsbError` at subsystem boundaries.
- Use `anyhow::Context` when composing operations at process/engine boundaries.
- Preserve both primary and cleanup failures when both occur.
- Classify retryability explicitly; do not retry every error forever.
- Cancellation is a distinct control state, not a generic failure.

## Adding a capture backend

1. Implement `FrameSource` with explicit async shutdown.
2. If recovery is desired, implement a `FrameSourceFactory` in the CLI layer.
3. Produce valid `RgbFrame` values at width 160 and proportional height.
4. Bound or replace queued frames; never accumulate them.
5. Detect terminal errors promptly.
6. Make blocking native loops own a thread and provide bounded startup/shutdown.
7. Add a `capture-test` selection path.
8. Add fake/unit tests for format, pacing, disconnect, and shutdown.
9. Document the session authority and privacy boundary.

Do not add a backend by teaching the engine about compositor-specific types.

## Changing Desktop GStreamer

Preserve:

- one-buffer leaky behavior;
- about-20-FPS pacing before expensive conversion when practical;
- proportional output dimensions;
- caps-generation gate;
- stride-aware RGB copy;
- 50 ms terminal-bus polling;
- held-frame dimension validation;
- explicit pipeline Null state and portal close.

Test with in-process `appsrc` pipelines for portrait, ultrawide, and caps renegotiation. Then verify non-black frames on the target host. A pipeline that negotiates is not necessarily a pipeline that returns correct pixels.

## Changing Gamescope capture

Preserve the reason for BGRx/no-modifier negotiation: it selects CPU-mappable buffers on the accepted Bazzite stack. Validate offset, stride, type, and buffer length before reading.

Pacing must remain non-drifting. A late callback advances to the next schedule boundary rather than setting `next = now + interval`.

Test in both the Steam shell and a real game; a nested compositor test alone is insufficient acceptance.

## Changing geometry

- Update normalized polygons or the integer half-open rule in `src/frame.rs`.
- Retain complete/disjoint ownership at every tested resolution.
- Retain horizontal symmetry unless the physical design intentionally changes.
- Update `docs/hardware/quadrant-test.html` and diagrams.
- Physically verify zone placement.

Do not infer protocol index changes from geometry changes.

## Changing the sampler

Sampler changes must be deterministic and stateless per input frame. Add focused fixtures around the exact threshold/color distribution being changed.

Always test:

- exact black remains black;
- adjacent low values have bounded output changes;
- sparse content fades proportionally;
- equal/near-equal bins cannot depend on hash-map iteration;
- dominant saturated content survives neutral outliers;
- arbitrary valid frames/masks do not panic.

Do not use the sampler to implement safety blackouts.

## Changing transitions

`TransitionController` is a pure clock-driven value object. It must not sleep, spawn, perform I/O, or know why a target changed.

Retarget from `colors_at(now)`, not the previous destination. Preserve exact endpoints and per-zone completion. Test time points with fixed `Instant` arithmetic.

Safety remains in the engine writer and bypasses the controller.

## Changing the engine

The engine contains intentional biased selects. Review priority before altering branch order:

- cancellation and safety must beat normal work;
- due writes must not be starved by hot target input;
- safety must preempt an in-flight/queued normal write;
- generation checks must surround blocking sampling;
- teardown must await blackout and source shutdown.

Use paused Tokio time and fake sources/sinks for races. A happy-path integration test is not enough.

## Changing USB behavior

Protocol and cadence changes have the highest hardware risk.

- Keep interface validation and kernel-driver recovery.
- Keep exact 20-byte report encoding and short-write rejection.
- Keep 6 ms spacing across public-call boundaries.
- Keep `blackout` uncached.
- Keep the dedicated worker and safety invalidation of queued writes.
- Keep the 100 ms transfer bound.

Run fake tests first. Then perform deliberate physical mapping/audio testing. Never run calibration concurrently with live capture.

## Adding CLI commands

- Add a Clap variant in `src/main.rs`.
- Validate dangerous/invalid values before opening USB or capture.
- Keep diagnostics explicit about whether they save data or modify config.
- Ensure every opened source/sink receives cleanup on every return path.
- Add parser tests.
- Update `README.md`, `docs/build-and-run.md`, and `--help` examples.

## Service and installer changes

The Gaming unit is linked directly from the repository. Its path and enablement topology are part of the prototype.

Validate:

```bash
bash -n scripts/*.sh
systemd-analyze --user verify systemd/logig560-gaming.service
```

Then test a real Desktop→Gaming→Desktop cycle. Confirm the Gaming service is enabled/inactive on Desktop, active in Gamescope, and stopped/black after exit.

## Dependencies

When adding a Rust crate:

- justify it against existing native/Rust facilities;
- update `Cargo.lock` intentionally;
- document any new native header/runtime package;
- verify both Fedora host and Bazzite container builds;
- consider runtime library compatibility between container and host.

Do not hand-edit `Cargo.lock`.

## Documentation standard

Current docs describe observed behavior; historical plans remain historical. When a decision changes, add an ADR or mark the old one superseded. Include exact constants, paths, and commands where they materially affect operation, but label machine-specific paths.

## Definition of done

A runtime change is done only when:

- source and focused regressions are complete;
- full format/Clippy/test/release gate passes;
- relevant script/unit checks pass;
- current docs and handoff are updated;
- live hardware/session behavior is verified in proportion to risk;
- the user confirms subjective color/smoothness changes;
- the working tree contains no accidental artifacts or unrelated rewrites.
