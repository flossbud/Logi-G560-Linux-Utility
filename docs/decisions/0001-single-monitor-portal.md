# ADR 0001: Desktop capture uses one portal-selected monitor

- Status: Accepted
- Date: 2026-07-19

## Context

Wayland clients cannot assume unrestricted screen access. G560 Linux Utility also needs an unambiguous mapping between one display and four physical speaker zones. Capturing an entire multi-monitor desktop would distort geometry and violate the user's selection expectation.

## Decision

Desktop Mode requests `SourceType::Monitor`, `multiple=false`, and `CursorMode::Hidden` through the XDG ScreenCast portal. The application accepts exactly one returned stream. It persists only the portal restore token with `PersistMode::ExplicitlyRevoked`.

## Consequences

- The system chooser is the authority; G560 Linux Utility does not enumerate and silently pick a monitor.
- Zero or multiple streams fail closed.
- First run may require user interaction.
- A saved opaque token can restore the selection without storing pixels.
- Gaming Mode needs a separate design because its session does not expose this portal.

## Evidence

Portal option and cleanup tests cover selection, cancellation, and every post-creation failure stage. Fedora and Bazzite Desktop runs successfully restored the selected monitor.
