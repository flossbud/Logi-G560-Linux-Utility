# LogiLightShow Design

## Purpose

LogiLightShow brings Logitech G560 screen-reactive lighting to Linux. It captures one user-authorized monitor, derives a spatially representative color for each of the speaker system's four lighting zones, and updates the hardware with the lowest reliable latency. After the screen matcher is polished, the same foundation will grow into a general G560 lighting application.

The initial supported environment is Fedora Workstation and Bazzite on Wayland, including GNOME and KDE desktops. The application must not require Logitech software, must not run as root, and must not capture more than one monitor.

## Product progression

Development is divided into three milestones:

1. **Hardware and latency proof:** Build the production Rust engine, initially controlled through a diagnostic CLI. Verify independent control of all four physical zones, single-monitor portal capture, immediate screen sampling, reliable USB behavior, and measured performance on a real G560.
2. **Polished screen matcher:** Add the user service, D-Bus API, GTK/libadwaita control panel, persistent capture authorization, login startup, and Fedora/Bazzite packaging.
3. **General lighting application:** Add static per-zone colors, built-in effects, brightness controls, profiles, and optional integrations. Screen matching becomes one profile in the broader application.

Milestone 1 uses the production engine and is not a throwaway prototype. General RGB-device support, audio-reactive effects, network control, GNOME Shell extensions, and OpenRGB integration are outside the first two milestones.

## Architecture

The engine is one Rust process with four internal components. Keeping the latency-sensitive path in one process avoids unnecessary serialization and deployment complexity while maintaining testable module boundaries.

### Capture

The capture component uses `org.freedesktop.portal.ScreenCast` to request exactly one monitor and consumes the resulting stream through PipeWire. The portal's source-selection dialog is the authority for monitor choice; the application never requests multi-monitor capture.

On first use, the desktop presents its secure monitor chooser. LogiLightShow requests persistent permission and stores the rotating restore token. On later logins it attempts to restore the same source without prompting. If restoration fails because the display is absent, desktop support differs, or permission was revoked, capture remains stopped until the user selects a monitor again.

The capture component retains only the newest usable frame. When a new frame arrives before the previous frame has completed processing, the older pending frame is discarded rather than queued. Capture buffers are never written to disk or transmitted over a network.

### Sampler

The sampler downsizes each frame before analysis and calculates a color independently for four configurable polygon regions. The default spatial mapping is:

| Screen region | Intended G560 zone |
| --- | --- |
| Upper-left edge | Left rear light |
| Lower-left edge | Left front light |
| Lower-right edge | Right front light |
| Upper-right edge | Right rear light |

Milestone 1 hardware testing verified the USB indexes as left front `0x00`, right front `0x01`, left rear `0x02`, and right rear `0x03`.

The four default polygons form a complete, non-overlapping partition of the selected monitor. They share an apex at normalized coordinate `(0.50, 0.00)`. The left and right outer knees are approximately `(0.17, 0.70)` and `(0.83, 0.70)`, and the front zones reach the bottom edge at approximately `x=0.14` and `x=0.86`. Rear zones own the outer edges and corners; front zones form inward-facing wedges that own the center and most of the lower field. Shared polygon boundaries have deterministic ownership, so every captured pixel belongs to exactly one zone.

At each negotiated capture resolution, the layout is compiled into four pixel-index masks. Masks are rebuilt only when the layout or capture resolution changes. The later control panel may expose the normalized polygon vertices as draggable control points without changing the sampler interface.

For each region, pixels are converted to a perceptual color representation and clustered by similarity. Very dark pixels do not distort hue selection, and tiny isolated highlights have limited influence. The strongest meaningful color cluster supplies the hue and chroma; source luminance supplies output brightness. If the region is below the configured darkness threshold, the output is true black.

Every normal sampled-color change uses an interruptible 90 millisecond perceptual transition. The transition begins affecting output immediately, interpolates hue and brightness in OKLab with easing, and continuously retargets to the newest sampled color. A retarget begins from the last color successfully displayed by the hardware; previous destinations never queue. Minimum brightness remains zero, so black screen content fades fully off.

The USB inter-report interval is a measured hardware parameter rather than a permanently conservative constant. Development calibration tests progressively shorter intervals with sustained four-zone changes while audio plays, stops at the first transfer error, and selects the fastest stable interval with a safety margin. The selected interval must pass a longer confirmation soak before becoming the v1 default. Increasing real hardware update cadence—not merely changing the easing curve—is what allows the transition to become both smoother and faster.

Safety blackouts bypass perceptual transitions and take effect immediately. Session lock, capture loss or stall, pause, normal exit, and USB recovery cleanup must never wait for a fade to finish.

The first milestone targets SDR correctness. HDR streams will be consumed using the color metadata and formats exposed by the capture stack when practical, but correct HDR-to-light mapping is not claimed until explicitly tested. Unsupported or ambiguous HDR input must degrade safely and be reported in diagnostics rather than silently claiming color accuracy.

### G560 driver

The hardware component addresses Logitech USB vendor ID `046d`, product ID `0a78`. It adapts the known command structure demonstrated by `g560-led`, which identifies four zone indexes, but it does not copy that tool's attach/send/detach cycle.

While matching is active, the driver claims only the USB interface needed for lighting and keeps it open. Each update sends the four zone colors with minimal calls permitted by the verified protocol. The driver must not disturb USB audio playback or speaker controls. A narrowly scoped udev rule grants the active local user access to this single product ID, so neither the service nor GUI runs as root.

Milestone 1 determines the maximum reliable update rate empirically. The engine prioritizes the newest color set and may coalesce updates when frames arrive faster than the hardware can accept them. It does not promise a fixed frame rate before this measurement.

### Coordinator

The coordinator moves the newest frame through sampling to the driver without an accumulating queue. It manages state transitions for pause/resume, lock/unlock, display sleep/wake, capture loss, service shutdown, and speaker reconnect. It publishes runtime status and accepts configuration changes through an internal interface that later backs D-Bus.

## Runtime behavior

The polished application runs as a user-level systemd service enabled at login. It attempts to restore the previously authorized monitor and starts screen matching immediately. The settings application is a separate GTK/libadwaita process that communicates with the service over the user session D-Bus. Closing the settings window does not stop matching.

The service sends black to all zones before releasing the interface whenever matching is paused, the session locks, the selected display sleeps, capture stalls or ends, or the service shuts down normally. Because a killed or crashed process cannot guarantee a final command, every service start resets the zones before capture begins.

Stock GNOME does not provide a traditional application tray, so core operation cannot depend on one. The control panel is always available from the desktop application launcher. KDE may additionally receive a StatusNotifier item. A GNOME Quick Settings extension is a possible later add-on, not part of the first polished release.

## Control panel

The milestone 2 control panel provides:

- A master enable/pause control.
- G560 connection and capture-permission status.
- A **Choose display** action that invokes the portal source chooser.
- A preview of the four sample regions with editable geometry.
- Live color indicators for all four zones.
- A login-startup preference, enabled by default.
- Minimum-brightness and transition-duration controls, defaulting to zero and 90 milliseconds respectively.
- Reset-to-defaults and concise diagnostic information.

The interface must clearly distinguish these states: active, paused, waiting for speakers, waiting for monitor authorization, capture stalled, and USB interface conflict.

## Configuration and privacy

Versioned, human-readable configuration is stored below `~/.config/logilightshow/`. It includes sampling geometry, color-response settings, service preferences, and the latest portal restore token. The service applies safe configuration changes live. Versioned parsing enables future migration without discarding user settings.

No captured image, thumbnail, pixel buffer, or color history is persisted. No telemetry or network service is required. Logs contain state transitions and performance measurements, never frame content.

## Failure handling

- **Speaker disconnected:** Stop USB writes, expose a waiting state, and reconnect automatically when the matching device returns.
- **USB write failure:** Retry with bounded backoff. After repeated failures, release and reopen the lighting interface.
- **Interface owned elsewhere:** Report a conflict and retry conservatively; do not forcibly detach an unrelated userspace owner.
- **Capture permission lost:** Send black and expose an authorization-required state. Reopen the chooser only following a user action.
- **Capture stalled:** Send black after a short safety timeout rather than leaving stale scene colors indefinitely.
- **Monitor topology changed:** Attempt restoration. If the authorized source is unavailable, require a new single-monitor selection.
- **Malformed or incompatible configuration:** Preserve the invalid file for diagnosis, load safe defaults, and report the configuration error.

Exact retry intervals and the capture-stall timeout are implementation constants selected during milestone 1 testing; they do not alter user-visible product behavior and may later become advanced settings if evidence warrants it.

## Installation and packaging

Installation must work with Bazzite's immutable base system and must not require layering a runtime dependency collection. The packaged application installs its user service and desktop files in locations appropriate to the chosen package format. A one-time Polkit-authorized helper installs a narrowly scoped udev rule for `046d:0a78` and reloads device rules. All routine processes run unprivileged.

The first supported package target will be selected during implementation planning after validating the Rust, GTK, portal, and service deployment constraints. Regardless of package format, GNOME and KDE must use their native XDG portal backend instead of desktop-specific capture shortcuts.

## Verification

### Automated tests

- Unit tests cover polygon geometry, monitor-size changes, perceptual clustering, highlight rejection, darkness behavior, transitions, configuration parsing/migration, and newest-frame-wins scheduling.
- Polygon tests prove complete single-zone pixel coverage, deterministic boundary ownership, mirror symmetry, and expected sampling from synthetic zone images.
- Transition tests use a fake clock to verify the 45 millisecond midpoint, exact 90 millisecond completion, interruption from the last successfully displayed value, newest-target replacement, OKLab interpolation, and immediate safety blackout.
- USB calibration tests verify cross-call pacing at the selected interval, failure detection at an unstable interval, the configured safety margin, and a sustained real-hardware confirmation with uninterrupted audio.
- A fake capture source feeds generated and recorded test patterns through the complete sampler path.
- A fake four-zone USB transport verifies zone ordering, write coalescing, reconnect behavior, retry limits, and shutdown blackouts without requiring hardware.
- D-Bus integration tests cover service state and live configuration changes in milestone 2.

Test fixtures containing screen imagery must be purpose-built or redistributable assets and must not be captured from the user's desktop.

### Hardware acceptance tests

On the user's G560 and Fedora Wayland system:

1. Identify and document the physical light controlled by every zone index.
2. Confirm lighting control does not interrupt USB audio or speaker controls.
3. Measure the reliable sustained update ceiling and behavior under rapid four-zone changes.
4. Estimate capture-to-light latency using alternating high-contrast edge patterns, recording the test method and result.
5. Measure idle and active CPU/memory use.
6. Verify pause, lock, display sleep, normal shutdown, unplug/replug, USB error recovery, and service restart.
7. Verify the portal requests one monitor only and restores the selected source after logout/login when the desktop grants persistent permission.

Milestone 2 additionally verifies installation, login startup, portal behavior, and control-panel operation on supported GNOME and KDE environments, including Bazzite where available.

## Success criteria

Milestone 1 succeeds when one authorized monitor drives four independently verified physical zones using the default polygon layout and weighted dominant colors; normal changes follow interruptible 90 millisecond OKLab transitions at a calibrated hardware-safe update cadence; safety blackouts remain immediate; black regions turn fully dark; stale frames and transition targets never queue; audio remains unaffected; unplug/replug recovers automatically; and measured latency and reliable update rate are documented. The user must confirm that transitions are visibly smooth and do not feel delayed.

Milestone 2 succeeds when a nontechnical user can install the application with one administrator authorization for device access, select one monitor once, have matching resume at later logins when portal restoration is available, control the service from a clear desktop UI, and run the application without root privileges on Fedora Workstation and Bazzite.
