# Known limitations and future work

## Product surface

- CLI prototype only.
- No GUI, tray application, D-Bus API, pause toggle, or user-facing status model.
- No permanent Desktop autostart service; Desktop acceptance uses a foreground or transient user unit.
- No installer/package that combines binary, user units, desktop integration, and udev setup.
- Gaming service assumes the repository remains at `%h/Documents/G560 Linux Utility`.

## Platform coverage

- Accepted primarily on GNOME Wayland and Bazzite's Gamescope session.
- Arch Linux KDE Plasma 6 Wayland has non-black portal-capture and short live-engine evidence, but still lacks the longer lock/unlock, suspend, audio-soak, and subjective visual acceptance performed on Bazzite.
- X11 is not a target.
- Other compositors may expose different portal or PipeWire behavior.
- Other G560 firmware revisions and USB layouts are not broadly tested.
- No other Logitech device is supported.

## Color and display

- SDR behavior only; HDR accuracy is unvalidated.
- No ICC/profile-aware color management.
- No tone mapping, gamut selection, gamma control, brightness multiplier, or user calibration UI.
- Nearest-neighbor downscale is used in the direct Gamescope path.
- Geometry is fixed to one tested four-polygon layout.
- Sampler and transition constants are compiled defaults, not configuration.

## Capture

- Exactly one monitor; no multi-monitor composition.
- Cursor intentionally hidden.
- Desktop relies on the portal returning/restoring an appropriate monitor.
- Bazzite Desktop defaults to system-memory GStreamer because the tested GL download produced black frames.
- The optional DMA-BUF path is compatibility code with host-specific behavior.
- A healthy Desktop frame may be held through sparse delivery. An unreported indefinitely wedged pipeline is difficult to distinguish from a valid static source until some other health signal occurs.
- Gaming capture relies on a PipeWire node named exactly `gamescope` and BGRx CPU-mappable negotiation.

## Safety and lifecycle

- Clean signals can request blackout; crash, power loss, kernel failure, or `SIGKILL` cannot guarantee a final USB command.
- Every new live start begins with black to mitigate prior unclean exits.
- Startup can increment the stall metric before the first frame while the device is already black; the metric does not currently distinguish startup waiting from a runtime stall.
- Portal peer disconnect during whole-session teardown can make the process report a cleanup error even after safety black was attempted.
- System suspend freezes the user process; safety timers cannot run while the entire user slice is frozen. On thaw, the engine may record a stall and black immediately.

## Performance and observability

- Metrics print to stdout/journal only.
- Latency percentiles are cumulative rather than rolling.
- Dropped count combines multiple replacement/staleness causes.
- Held versus genuinely captured Desktop frames are not separately counted.
- No structured JSON logs, tracing export, health socket, or status API.
- No CPU/RSS metrics in the application output.

## Development and delivery

- The accepted Bazzite port is committed locally, but the workspace still has no configured upstream remote.
- No CI workflow.
- No automated dependency/security audit.
- No automated Markdown link/lint gate.
- Native dependency compatibility between build container and host must be checked manually.
- Final snapshot archives are manually rebuilt and checksummed.

## Suggested next milestones

1. Add CI for format, Clippy, tests, shell syntax, and systemd verification.
2. Complete extended KDE lock/suspend/audio and user visual acceptance.
3. Create relocatable packaging and a persistent Desktop user service.
4. Add a small control/status API and UI without moving capture/USB into privileged code.
5. Expose safe user configuration for monitor selection, brightness, darkness behavior, and transitions.
6. Add structured health metrics, including held-frame and stall-cause visibility.
7. Validate additional GPUs, HDR behavior, and other firmware revisions.
8. Define an explicit PipeWire liveness signal that distinguishes valid static content from a silently wedged source.

Future work must retain the invariants in `AGENTS.md`, especially one-monitor authority, no-root runtime, no saved frames, latest-only flow, USB cadence, and immediate safety blackout.
