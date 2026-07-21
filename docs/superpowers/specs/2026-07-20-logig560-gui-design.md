# G560 Linux Utility GUI Design

## Status

This specification records the GUI design approved on July 20, 2026. It replaces the GTK/libadwaita control-panel direction in the July 19 design with a Tauri 2 application and expands the product from a content-aware CLI into a four-zone lighting controller. The existing capture, sampling, transition, USB, and safety invariants remain authoritative.

The visible product name is temporary. User-facing branding must be centralized, while stable internal identifiers and configuration migrations must not depend on a future rename.

## Purpose

The GUI gives Linux users a polished, device-first interface for Logitech G560 lighting. It must:

- control the left rear, left front, right front, and right rear zones individually or in groups;
- enable one global Content-Aware Lighting mode that always owns all four zones;
- guide new users through device access, the background service, and display capture;
- show the state that the service has successfully applied to the hardware;
- preserve lighting operation when the GUI is closed; and
- make failures understandable without exposing captured imagery or color history.

The first GUI release supports Fedora, Bazzite, and Arch Linux on the GNOME and KDE Wayland environments already covered by the project. Desktop Mode continues to use the portal capture backend. Bazzite Gaming Mode continues to use the direct Gamescope backend.

## Scope

The first release includes manual solid colors, per-zone brightness, a global Content-Aware mode, master power, guided setup, service maintenance, sanitized diagnostics, and an About page.

The following are outside this release:

- animation profiles and built-in lighting effects;
- audio-reactive lighting;
- user-created screen geometry;
- support for devices other than Logitech G560 USB `046d:0a78`;
- cloud accounts, telemetry, or network control;
- automatic application updates;
- a tray-only or GNOME Shell extension interface; and
- signed distribution packages. Packaging can be designed separately after the source-built GUI and service are accepted on all three target environments.

## Product structure

The application uses one persistent shell with a top status bar and a left navigation rail.

### Top status bar

The top bar contains:

- the application identity;
- background-service status;
- G560 connection status; and
- a master **Lights On/Off** control.

Turning lights off requests an immediate all-zone blackout, stops active capture, and leaves the service running. The saved mode and manual settings remain intact. Turning lights back on resumes the saved mode. Lighting controls are visibly disabled while master power is off.

### Navigation rail

The rail contains four destinations:

1. Lighting
2. Setup & Service
3. Diagnostics
4. About

Navigation preserves the current service-backed state. Draft text in non-lighting fields may remain local while switching pages, but lighting changes have no separate Apply step.

### First-run presentation

On a fresh configuration, a guided setup overlay opens above the visible application shell. The user can see the eventual product context while completing setup. The setup can be reopened later from Setup & Service.

Service or device failures after onboarding appear as concise banners with direct actions to the relevant Setup or Diagnostics section. Routine status changes do not open modal dialogs.

## Visual direction

The interface takes inspiration from Logitech G Hub without cloning it. It uses a near-black device-first workspace, compact controls, cyan interaction accents, and clear operational status. Linux integration is present but does not make the main lighting page resemble a generic settings panel.

The Lighting page centers two separate, front-facing G560 satellite assets. The left asset is horizontally mirrored and does not show a power indicator. The right asset retains the physical power indicator. The assets preserve their original aspect ratio and have deliberate horizontal spacing.

The front light surfaces use neutral, unsaturated emissive textures multiplied by the selected zone colors. A feathered luminance attenuation prevents a harsh hotspot at the lower apex without dimming the ribbed surface. Rear zones render as blurred ambient glows beneath the transparent speaker cutouts so that the speaker bodies occlude the glow.

## Lighting page

Lighting has two mutually exclusive modes: Manual and Content-Aware.

### Manual mode

The user may select zones individually or with these quick groups:

- All
- Fronts
- Rears
- Left
- Right

The color picker, hexadecimal/RGB entry, presets, and brightness slider apply to every selected zone. Each zone stores an RGB color and a brightness from 0 through 100. Hardware output is calculated by scaling each RGB channel by that zone's brightness percentage, with integer rounding and clamping to `0..=255`.

When selected zones have different colors or brightness values, the controls show a mixed state until the user chooses a replacement value. Selecting or deselecting zones never changes hardware by itself.

Manual changes are sent immediately and persisted only after the service accepts them. The speaker preview is updated from the service's last successfully written colors, not optimistic browser state. If a write fails, requested values remain visible in the controls as pending, the preview retains the last confirmed hardware state, and an error banner offers recovery.

On a fresh installation, all four manual zones default to cyan `#14C8F4` at 100 percent brightness. Fresh installations default to Manual mode with Lights On. Content-Aware capture begins only after an explicit user action.

### Content-Aware mode

Content-Aware is one global mode and always controls all four zones. It has no per-zone enablement, assignment, or capture-source mapping. Enabling it disables the manual picker, presets, brightness, zone cards, and group buttons while leaving their saved values intact.

The page displays:

- the active backend, either Desktop Portal or Gamescope;
- capture authorization and running state;
- concise capture-stall and recovery messages; and
- the four last successfully written hardware colors.

In Desktop Mode, **Choose display** invokes the portal chooser. Exactly one monitor is required, and the portal selection is authoritative. Zero streams or multiple streams are errors. In Gaming Mode, the service uses the direct Gamescope PipeWire node and does not invoke the desktop portal.

Disabling Content-Aware cancels capture, clears newest-only capture handoffs, and restores the saved manual colors. A saved Content-Aware mode may resume at login only when capture can be restored safely. Capture loss, a capture stall, cancellation, USB recovery, or shutdown causes an immediate safety blackout rather than preserving stale preview colors.

## Setup & Service

### Guided setup

The guided flow contains five steps:

1. **System check:** identify Fedora, Bazzite, or Arch and verify required runtime components.
2. **Speaker access:** detect USB `046d:0a78`, explain the udev rule, and invoke the existing narrowly scoped Polkit installer when permission is missing.
3. **Background service:** install or locate the user unit, enable it, start it, and verify its state.
4. **Display capture:** explain the one-monitor portal chooser in Desktop Mode and verify the appropriate capture backend. On Gaming Mode, verify Gamescope integration instead.
5. **Ready:** summarize service, device, permission, and capture readiness and then open Lighting.

System checks never run the application as root. Privileged operations show the exact fixed action before Polkit authorization. The GUI does not accept or construct arbitrary privileged shell commands. When a dependency cannot be installed safely by the application, the screen presents distribution-specific instructions copied from the maintained build and operations documentation.

### Maintenance dashboard

After onboarding, Setup & Service shows:

- service installation, enabled, and running state;
- Start, Stop, and Restart actions;
- G560 detection and permission state;
- Desktop Portal and Gamescope integration state;
- repair actions for known incomplete states; and
- a way to re-run the guided setup.

Before starting or restarting, the application checks the user service and running processes to avoid a second instance that could compete for the G560. Existing working configuration is preserved by repair operations.

## Diagnostics

Diagnostics presents operational data without captured content:

- overall service, USB, capture, and lighting-writer health;
- current Manual, Desktop Content-Aware, or Gamescope Content-Aware mode;
- frames processed;
- newest-value replacements;
- capture stalls;
- USB recoveries and report failures;
- capture rate and last successful hardware-write time; and
- a bounded recent-events list containing state transitions and error summaries.

The event list and copied diagnostic report never include screenshots, frames, thumbnails, sampled RGB values, manual colors, Content-Aware colors, or color history. Runtime metrics remain color-free.

**Copy Diagnostic Report** produces sanitized text containing versions, environment, operating states, counters, and errors. Hardware tests are non-destructive by default. Zone pulses or calibration require explicit confirmation and remain unavailable while another G560 Linux Utility process owns the device.

## About

About shows the GUI version, service/API version, supported USB identifier, license information, project links, and a **Copy Version Information** action. Branding strings, icon references, application title, and About metadata come from one product-metadata source to simplify the future rename.

The page states that the application uses no account, telemetry, cloud service, or captured-image storage.

## Architecture

The product contains two long-lived process roles:

1. a background Rust service that owns configuration, capture, safety state, and the G560; and
2. a separate Tauri 2 GUI that may open and close without changing service lifetime.

The frontend uses framework-free HTML, CSS, and JavaScript embedded in the Tauri binary. Tauri's Rust command layer validates frontend input and acts as a narrow client for the service. Browser code never opens USB devices, invokes shell commands, controls systemd, or talks to D-Bus directly.

### Component boundaries

| Component | Responsibility | Depends on |
| --- | --- | --- |
| Frontend view | rendering, keyboard interaction, local navigation, pending-state presentation | typed Tauri commands and events |
| Tauri command layer | input validation, D-Bus client, safe setup helpers, system integration | service API and fixed operating-system adapters |
| Service API | versioned commands, full state snapshots, newest-only state notifications | controller |
| Controller | authoritative mode, manual settings, master power, capture lifecycle, safety priorities | existing capture engine and USB writer |
| Persistence | atomic versioned configuration and migration | controller-owned validated settings |
| Diagnostics projection | color-free counters and sanitized events | existing metrics and controller state |

Each boundary uses typed data. Frontend strings are not converted into shell fragments, USB reports, or capture requests without validation in Rust.

## Service API

The first API uses the user session D-Bus name `org.logig560.Service1` and object path `/org/logig560/Service1`. These are internal compatibility identifiers and do not change solely because the visible product is renamed.

The API provides these conceptual operations:

- `GetSnapshot`
- `SetLightsEnabled`
- `SetMode`
- `SetManualZones`
- `ChooseDesktopDisplay`
- `RestartCapture`
- `GetDiagnosticReport`

`SetManualZones` accepts one atomic set of zone updates so group changes cannot be observed half-applied. Requests contain logical zone names, never raw USB indexes. The service preserves the physical mapping from logical `[left rear, left front, right front, right rear]` to protocol `[0x02, 0x00, 0x01, 0x03]`.

The service publishes a `SnapshotChanged` notification containing the newest complete state. Snapshot publication is coalesced and capped so a slow or hidden GUI cannot accumulate color or frame updates. A newly connected GUI always calls `GetSnapshot` before processing notifications.

The snapshot includes service/API compatibility, operating mode, master power, persisted manual settings, pending command state, active capture backend, health states, color-free diagnostics, and the four last successfully written colors. Confirmed colors are ephemeral service state and are never written to configuration or logs.

Unknown enum values, invalid zone sets, incompatible API versions, and commands disallowed by the current mode return explicit typed errors. Multiple GUI clients are permitted, but the service remains authoritative and serializes state transitions without creating multiple capture or USB owners.

## Controller behavior

The existing runtime is refactored behind a controller state machine; its capture and USB implementations remain reusable components rather than being duplicated in the GUI.

### Manual update

1. The frontend sends a complete selected-zone update through Tauri.
2. Tauri validates color, brightness, and zone identifiers and calls the D-Bus API.
3. The controller updates one newest manual target and requests the existing writer path.
4. The USB writer preserves report order and at least 6 ms between every adjacent HID report, including across calls.
5. The controller persists accepted manual settings atomically.
6. After a successful write, it updates the confirmed-color snapshot.

### Content-Aware update

1. The controller selects Desktop Portal or Gamescope from the active environment.
2. The existing latest-only capture, sampler, transition, and writer pipeline produces targets.
3. The controller publishes only the newest confirmed hardware state to the GUI.
4. Capture loss or stall sends an immediate priority blackout and publishes the failure state.

### Mode and power transitions

- Manual to Content-Aware starts the correct capture backend and leaves saved manual settings untouched.
- Content-Aware to Manual stops capture cleanly and writes the saved manual state.
- Lights Off stops capture, requests immediate all-zone black, and retains both the saved mode and manual settings.
- Lights On resumes the saved mode only after the required backend and USB state are ready.
- Service shutdown stops capture, requests all-zone black, releases USB, and closes portal and worker resources.

Safety blackouts preempt normal transitions, pending manual changes, reconnection writes, and GUI traffic.

## Persistence and privacy

The service owns a versioned configuration under the XDG user configuration directory. It stores:

- master Lights On/Off;
- selected Manual or Content-Aware mode;
- four manual RGB colors and brightness values;
- portal restore data already required by the capture backend; and
- setup completion and schema version.

Updates use write-to-temporary-file, flush, and atomic rename within the configuration directory. Malformed configuration is preserved for diagnosis, safe defaults are loaded, and the GUI reports the problem.

The application never saves frames, screenshots, sampled colors, thumbnails, Content-Aware color history, or confirmed-color history. It does not transmit captured data over the network. The speaker preview is generated from bundled product assets and ephemeral confirmed RGB values only.

## Failure handling

- **Service unavailable:** the GUI reports the state, may request the user service to start, and links to Setup & Service.
- **API mismatch:** read-only status is shown when possible; control actions are disabled with upgrade guidance.
- **G560 disconnected:** capture stops, an immediate blackout is requested where possible, and bounded reconnection begins.
- **USB permission missing:** the GUI links to the explicit Polkit-backed udev repair step.
- **USB owned elsewhere:** no non-HID interface is detached and no competing process is killed automatically.
- **Capture authorization missing:** lights remain black and the chooser opens only following a user action.
- **Zero or multiple portal streams:** capture fails closed and the user is asked to choose exactly one monitor.
- **Capture stall or loss:** a safety blackout is immediate, the stall counter rises, and recovery status is published.
- **Gamescope unavailable in Gaming Mode:** capture remains stopped; the desktop portal is not used as a fallback.
- **Persistence error:** the active safe state remains in memory, the failed setting is marked unpersisted, and the GUI presents a retry action.
- **GUI crash or close:** service operation continues unchanged.

## Accessibility and interaction

- Every rail item, tab, zone selector, color control, setup step, and action is keyboard accessible.
- Focus indication is visually distinct from zone selection.
- Status uses text and icons in addition to color.
- Reduced-motion preference removes decorative transitions but never delays or animates safety blackouts.
- Speaker assets keep their aspect ratio at every supported window size.
- The desktop layout becomes scrollable and rearranges controls before allowing overlap or horizontal stretching.
- Ordinary lighting changes are immediate and require no confirmation.
- Privileged operations, destructive resets, and hardware pulses require explicit confirmation.
- Desktop notifications are limited to meaningful background failures such as service failure or device loss.

## Installation and service integration

The source-built first release provides maintained Fedora, Bazzite, and Arch instructions and an in-GUI setup flow. Fedora and Arch development dependencies are installed on the host. Bazzite builds remain inside the `logig560` Distrobox/Toolbox and run the host-mounted binary in the host user session.

The GUI installs or locates a user-level service without root. Polkit is used only by the existing fixed udev installer. The application verifies the service after enabling or starting it. The checked-in Gaming Mode unit and its documentation must be updated together if their repository path assumption changes.

Desktop and Gaming Mode keep separate service/capture behavior. The Gaming Mode unit is enabled but inactive in Desktop Mode and starts with the Gamescope session. The GUI must inspect user-unit and process state before any live start or restart.

## Implementation decomposition

The design is implemented and accepted in four dependency-ordered increments:

1. **Service foundation:** controller state machine, manual state, persistence, versioned D-Bus API, and fake-service test support.
2. **Lighting application:** Tauri shell, shared product metadata, top bar, navigation, Manual mode, Content-Aware mode, and confirmed-hardware preview.
3. **Onboarding and operations:** guided setup, fixed Polkit integration, user-service maintenance, capture selection, and recovery banners.
4. **Support and release acceptance:** Diagnostics, About, accessibility passes, render coverage, documentation updates, and environment/hardware acceptance.

Each increment must pass the repository's full relevant validation before the next begins. The service foundation is usable without the GUI, and every GUI increment can run against the fake service without hardware.

## Testing

### Automated tests

- Controller tests cover Manual and Content-Aware transitions, master-off blackouts, saved-state restoration, invalid commands, newest-only updates, and recovery.
- Persistence tests cover atomic writes, schema migration, malformed files, interrupted writes, and failures that leave active state unpersisted.
- D-Bus contract tests cover every command, full snapshots, coalesced notifications, invalid zone sets, multiple clients, and API-version mismatch.
- Tauri command tests prove frontend validation and ensure browser input cannot select arbitrary executables, shell commands, USB indexes, or files.
- Frontend state tests cover individual and grouped zones, mixed values, pending versus confirmed state, Content-Aware lockout, master-off lockout, navigation, setup progress, and error banners.
- Render checks cover Manual, Content-Aware, Lights Off, disconnected device, capture failure, API mismatch, setup overlay, diagnostics, and narrow-window layouts.
- GUI integration tests run against a fake D-Bus service and fake setup adapters; routine tests never access real G560 hardware or invoke Polkit.
- Existing capture, sampler, transition, USB pacing, recovery, and clean-shutdown tests remain mandatory.

Test fixtures use generated or redistributable content. They never use images captured from the user's desktop.

### Hardware and environment acceptance

On real G560 hardware:

1. Verify every individual zone and quick group.
2. Confirm brightness and RGB output for each zone.
3. Confirm the preview follows successful hardware writes and does not advance on failure.
4. Confirm Content-Aware always owns all four zones and restores manual colors when disabled.
5. Confirm closing the GUI leaves the service operating.
6. Confirm device loss, capture loss, capture stall, cancellation, master off, and shutdown use immediate blackouts.
7. Confirm USB audio and speaker controls remain unaffected.

Environment acceptance is performed separately on Fedora GNOME Wayland, Bazzite Desktop Mode, Bazzite Gaming Mode, and Arch KDE Plasma Wayland. The user must confirm visual accuracy and physical-light behavior; automated results alone do not close visual issues.

## Success criteria

The GUI release succeeds when a new user can install device access and the user service with clear guidance, control any combination of four G560 zones, switch one global Content-Aware mode on and off, close the GUI without stopping lighting, understand capture or USB failures, and collect a color-free diagnostic report.

The preview must represent confirmed hardware state. Manual settings must survive service and login restarts. Content-Aware colors must remain ephemeral. Safety blackouts, newest-value-only handoffs, the calibrated 6 ms USB pacing, exact logical-to-physical zone mapping, HID-only interface ownership, separate Desktop/Gaming capture backends, and clean shutdown must remain intact.
