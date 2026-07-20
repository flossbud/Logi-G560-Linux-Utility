# ADR 0002: Use newest-value handoffs instead of queues

- Status: Accepted
- Date: 2026-07-19

## Context

Screen matching is useful only when it reflects current content. A conventional FIFO between capture, sampling, transitions, and USB can build latency during load; rendering every stale frame makes the lights visibly trail the display.

## Decision

Use one-slot/newest-only behavior at capture appsinks, backend handoffs, captured-frame handoff, and sampled-target handoff. Replacing unread work is counted as dropped. USB commands remain ordered, but safety invalidates queued normal writes.

## Consequences

- `dropped` is expected under motion or backpressure.
- Throughput is not the goal; freshness is.
- Work must carry timestamps/generations so old work can be rejected after recovery or blackout.
- Adding a larger queue is an architecture regression unless a new requirement explicitly values replay over latency.

## Evidence

Latest-channel and fake-engine tests prove unread replacement. Engine tests prove blocked writes use the newest target and do not replay ten old targets. Hardware acceptance reported responsive transitions without stale-state accumulation.
