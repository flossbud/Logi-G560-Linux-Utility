# ADR 0003: Desktop and Gaming Mode use separate capture backends

- Status: Accepted
- Date: 2026-07-20

## Context

Bazzite Desktop Mode provides the standard Wayland portal. Bazzite's Gamescope session deliberately disables normal desktop portal services, so a saved GNOME portal permission cannot open Gaming Mode output.

## Decision

- Desktop `run`: XDG portal authorization plus GStreamer `pipewiresrc`.
- Gaming `run-gaming`: direct PipeWire connection to the node named `gamescope`.
- Normalize both into `RgbFrame` and share the engine/sampler/USB path.

## Consequences

- There are two capture implementations and two diagnostics.
- Portal security semantics remain intact on Desktop.
- Gaming Mode has no portal chooser and depends on the session's named node.
- Backend-specific errors remain typed but recovery/metrics are shared.

## Evidence

The direct backend captured a rendered nested Gamescope scene, then worked in the real Steam shell and in-game. The portal backend remained working in Bazzite Desktop Mode on the same hardware.
