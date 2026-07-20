# ADR 0008: Hold the last valid Desktop frame through brief source silence

- Status: Accepted
- Date: 2026-07-20

## Context

On Bazzite Desktop, sparse PipeWire/GStreamer delivery during dark/static content produced frame gaps beyond the engine's 500 ms stall timeout. Each gap correctly invoked safety blackout, but the stream was still connected and fresh frames resumed, causing synchronized all-zone off/on flicker.

Simply raising/removing the engine stall timeout would weaken capture-loss safety. Returning a held frame immediately after every 50 ms poll also raced normal capture and inflated the observed rate to about 38 FPS.

## Decision

In the Desktop GStreamer source, after 200 ms without a new sample, return the last frame only when it still matches the current nonzero expected dimensions. Check bus errors and EOS on every 50 ms poll before using the hold. Do not hold before the first frame or during caps renegotiation.

## Consequences

- Healthy static scenes can report about 5 FPS rather than 19 FPS without triggering a stall.
- Normal moving capture stays near 19 FPS and is not duplicated.
- Terminal errors/EOS and dimension changes retain existing safety/error paths.
- Metrics do not currently distinguish held from newly captured frames.

## Evidence

Before the change, live logs showed capture falling to 4–11 FPS with rapidly increasing stalls and all-zone flicker. A first 50 ms implementation eliminated stalls but inflated capture to ~38 FPS; it was rejected. The 200 ms implementation returned to ~18.8 FPS under normal delivery with no false stalls in the reproduced condition. A regression test covers a connected pipeline that sends one frame and then remains silent. The user confirmed operation worked “extremely well.”
