# ADR 0005: Use verified indexes and globally paced HID reports

- Status: Accepted
- Date: 2026-07-19

## Context

The G560 exposes audio and lighting through a composite USB device. Unpaced multi-report lighting writes produced I/O failure and could risk audio disruption. Front/rear protocol indexes are not in logical array order.

## Decision

- Claim only interface 2 after verifying it is HID.
- Reattach any original kernel driver on release/failure.
- Encode 20-byte solid-color reports using physically verified indexes.
- Space every adjacent report by 6 ms, even across separate API calls.
- Keep synchronous USB ownership on one dedicated worker thread.
- Always send all four reports for safety blackout.

## Consequences

- A four-zone changed update takes at least the paced report sequence; exact duplicates are coalesced.
- The cadence constant cannot be tuned as a generic performance knob.
- USB changes require audio-playing hardware verification.

## Evidence

Raw zone pulses established `[left rear, left front, right front, right rear] → [0x02, 0x00, 0x01, 0x03]`. Calibration passed staged delays through 4 ms and a longer 6 ms confirmation with audio continuity, selecting a 2 ms margin.
