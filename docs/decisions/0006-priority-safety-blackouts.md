# ADR 0006: Safety blackout is an out-of-band priority path

- Status: Accepted
- Date: 2026-07-19

## Context

Normal ambient transitions should be smooth, but fading stale content during lock, capture loss, shutdown, or recovery is undesirable. Ordinary target queues and in-progress USB writes can otherwise relight after a blackout.

## Decision

Represent safety blackout separately from normal `ZoneColors` targets. Give safety selection priority, invalidate work with capture generations, clear transition state, set a USB safety flag before enqueueing blackout, and require acknowledgement with a bounded timeout.

## Consequences

- Exact black can mean either a normal sampled target or a safety command; callers must preserve the distinction.
- Normal black may fade, safety black never does.
- Capture work must be generation-checked before and after blocking sampling.
- USB queued normal commands can complete as `Expired` rather than reaching hardware.

## Evidence

Paused-time and blocking-sink tests cover cancellation, capture stall/error/EOS, in-flight frames, queued targets, missing-device recovery, and safety acknowledgement. Live lock/session behavior was accepted on the physical system.
