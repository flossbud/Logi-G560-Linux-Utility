# GUI Diagnostics and Release Acceptance Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Complete the application with color-free diagnostics, About/version support, restrained background notifications, accessibility verification, and cross-environment release acceptance.

**Architecture:** The service projects internal metrics into typed color-free counters and a bounded event model. Diagnostic report formatting occurs in Rust from approved fields only. The frontend renders snapshots and requests semantic copy actions; it never receives frames or historical colors. Release acceptance combines fake-service automation with disciplined real-hardware checks.

**Tech Stack:** Rust 2024, Tauri 2.11.5, tauri-plugin-notification 2.3.3, tauri-plugin-clipboard-manager 2.3.2, zbus 5.18.0, vanilla ES modules, systemd journal and service tooling.

## Global Constraints

- Diagnostics contain no screenshots, frames, thumbnails, sampled/manual/confirmed RGB values, or color history.
- Runtime metrics remain color-free.
- Event history is bounded state/error metadata, never a queue of frames, targets, or colors.
- Hardware pulses and calibration are not exposed in the first GUI release; existing CLI diagnostics remain explicit owner-operated tools.
- Background notifications are limited to service failure and device loss and are deduplicated.
- Physical visual behavior is not accepted until the user confirms it on the G560.
- No commit step may run unless the repository owner explicitly authorizes commits.

---

## File structure

- `src/diagnostics.rs`: typed event ring, metrics projection, and sanitized report formatter.
- `tests/diagnostics_privacy.rs`: explicit prohibited-data and bounded-history tests.
- `gui/js/diagnostics-state.js`: pure diagnostics selectors and copy state.
- `gui/js/diagnostics-render.js`: health summary, counters, timings, events, and empty/error states.
- `gui/styles/diagnostics.css`: dense readable status and counter layout.
- `gui/js/about-render.js`: product/version/privacy presentation.
- `src-tauri/src/support.rs`: semantic copy and notification commands.
- `src-tauri/tests/support_fake.rs`: clipboard and notification policy tests.
- `scripts/check-gui-static.mjs`: static accessibility/security checks.
- `docs/testing/gui-acceptance.md`: fake, distro, and physical acceptance checklist.

### Task 1: Color-free diagnostics projection and bounded events

**Files:**
- Create: `src/diagnostics.rs`
- Modify: `src/lib.rs`
- Modify: `src/control/actor.rs`
- Modify: `crates/logig560-api/src/lib.rs`
- Create: `tests/diagnostics_privacy.rs`

**Interfaces:**
- Produces: `EventKind`, `DiagnosticEvent`, `DiagnosticsState`, `DiagnosticsRecorder`, and `DiagnosticReport`.
- Consumes: `EngineSnapshot`, `RecoverySnapshot`, `CaptureRecoverySnapshot`, and controller health state.

- [ ] **Step 1: Write privacy and capacity red tests**

`event_history_keeps_only_the_newest_one_hundred_events` pushes sequence IDs 0 through 124 and compares the retained IDs with 25 through 124. `report_contains_no_manual_or_confirmed_colors` installs distinctive manual and confirmed values `1,2,3` and `253,254,255`, formats the report, and asserts their decimal triplets and `#010203`/`#fdfeff` forms are absent. `metrics_projection_contains_counts_and_timing_only` supplies fixed engine/recovery snapshots and compares every resulting counter and duration with an exact `DiagnosticCounters` value.

Also add a compile-time construction test demonstrating that `DiagnosticEvent` has no `Rgb8`, `ZoneColors`, `RgbColor`, frame, pixel, image, or thumbnail field.

- [ ] **Step 2: Verify failure**

Run: `cargo test -p logig560 --test diagnostics_privacy`

Expected: FAIL because diagnostics projection does not exist.

- [ ] **Step 3: Implement typed events**

Use only closed kinds: `ServiceStarted`, `ServiceStopping`, `ModeChanged`, `LightsChanged`, `DeviceConnected`, `DeviceLost`, `CaptureStarted`, `CaptureStalled`, `CaptureRecovered`, `UsbRecovery`, `ConfigurationRecovered`, and `OperationFailed`. Each event stores monotonic sequence, wall-clock timestamp, kind, subsystem, and a sanitized bounded message. `VecDeque` capacity is exactly 100; this queue never participates in runtime control.

- [ ] **Step 4: Implement metrics projection and report formatting**

Project frame count, newest replacements, stalls, recovery counts, capture rate, and last successful-write age. Format reports from a whitelist of DTO fields. Do not debug-format service snapshots or error chains. Cap each event message at 512 UTF-8 bytes and the entire copied report at 64 KiB.

- [ ] **Step 5: Run privacy tests**

Run: `cargo test -p logig560 --test diagnostics_privacy`

Expected: all privacy, capacity, and formatting tests pass.

- [ ] **Step 6: Commit only with explicit owner authorization**

```bash
git add src/diagnostics.rs src/control/actor.rs src/lib.rs crates/logig560-api tests/diagnostics_privacy.rs
git commit -m "feat: project color-free service diagnostics"
```

### Task 2: Diagnostic report D-Bus contract

**Files:**
- Modify: `src/dbus.rs`
- Modify: `tests/dbus_contract.rs`
- Modify: `src/main.rs`

**Interfaces:**
- Consumes: `DiagnosticsRecorder` and `DiagnosticReport`.
- Produces: `GetDiagnosticReport`, diagnostics fields in `ServiceSnapshot`, and sanitized tracing events.

- [ ] **Step 1: Add failing contract/privacy tests**

Test that report retrieval works while lighting is Manual, Content-Aware, Lights Off, disconnected, and shutting down; a report containing distinctive current/manual RGB triplets never contains those decimal or hexadecimal triplets; and event notification snapshots remain bounded.

- [ ] **Step 2: Verify failure**

Run: `cargo test -p logig560 --test dbus_contract diagnostic_report`

Expected: new report tests fail.

- [ ] **Step 3: Wire recorder updates**

Update diagnostics only at explicit controller/runtime state transitions. Keep the existing five-second CLI metric lines color-free. D-Bus returns `DiagnosticReport.text`; it never accepts a format string or arbitrary field selection from the GUI.

- [ ] **Step 4: Run service privacy and contract tests**

Run: `cargo test -p logig560 --test diagnostics_privacy && cargo test -p logig560 --test dbus_contract diagnostic_report`

Expected: all tests pass.

- [ ] **Step 5: Commit only with explicit owner authorization**

```bash
git add src/dbus.rs src/main.rs tests/dbus_contract.rs
git commit -m "feat: expose sanitized diagnostic reports"
```

### Task 3: Diagnostics page and semantic clipboard action

**Files:**
- Create: `gui/js/diagnostics-state.js`
- Create: `gui/js/diagnostics-state.test.mjs`
- Create: `gui/js/diagnostics-render.js`
- Create: `gui/js/diagnostics-render.test.mjs`
- Create: `gui/styles/diagnostics.css`
- Modify: `gui/index.html`
- Modify: `gui/js/main.js`
- Modify: `gui/js/api.js`
- Create: `src-tauri/src/support.rs`
- Create: `src-tauri/tests/support_fake.rs`
- Modify: `src-tauri/src/main.rs`

**Interfaces:**
- Produces: `diagnosticsViewModel`, `renderDiagnostics`, Tauri command `copy_diagnostic_report`, and success/failure copy status.

- [ ] **Step 1: Write diagnostics view tests**

Cover healthy, service unavailable, USB disconnected, capture stalled, no events, 100 events, last-write never, and last-write age. Assert that no view-model key contains `color`, `rgb`, `pixel`, `frame_image`, `thumbnail`, or `history`.

- [ ] **Step 2: Write clipboard security tests**

Use fake service/clipboard traits. Prove `copy_diagnostic_report` requests the report from the service and writes exactly that text; frontend-provided text is not accepted. Do the same for `copy_version_information` using embedded product/build metadata.

- [ ] **Step 3: Verify failure**

Run: `node --test gui/js/diagnostics-*.test.mjs && cargo test -p logig560-gui --test support_fake clipboard`

Expected: FAIL because diagnostics UI/support commands do not exist.

- [ ] **Step 4: Implement diagnostics rendering**

Render overall health, current mode/backend, counters, timing, recent typed events, and Copy Diagnostic Report. Use table/list semantics where appropriate, readable empty states, and text/icon status. Do not render raw journald output.

- [ ] **Step 5: Implement semantic clipboard support**

Pin `tauri-plugin-clipboard-manager = "2.3.2"` in the GUI crate but expose no generic clipboard command to JavaScript. Register only the two semantic commands and grant only the minimum plugin write capability required by Rust.

- [ ] **Step 6: Run diagnostics UI and support tests**

Run: `node --test gui/js/diagnostics-*.test.mjs && cargo test -p logig560-gui --test support_fake clipboard`

Expected: all tests pass.

- [ ] **Step 7: Commit only with explicit owner authorization**

```bash
git add gui src-tauri/src/support.rs src-tauri/src/main.rs src-tauri/tests/support_fake.rs src-tauri/Cargo.toml src-tauri/capabilities
git commit -m "feat: add color-free diagnostics page"
```

### Task 4: About page and restrained notifications

**Files:**
- Create: `gui/js/about-render.js`
- Create: `gui/js/about-render.test.mjs`
- Modify: `gui/index.html`
- Modify: `gui/js/main.js`
- Modify: `src-tauri/src/support.rs`
- Modify: `src-tauri/src/main.rs`
- Modify: `src-tauri/tests/support_fake.rs`
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/capabilities/default.json`

**Interfaces:**
- Consumes: `product-metadata.json`, GUI build version, service/API version, and health transitions.
- Produces: About page, Copy Version Information, and `NotificationPolicy`.

- [ ] **Step 1: Write About and notification-policy tests**

Assert About includes GUI version, service/API version, `046d:0a78`, license, project link, and privacy statement from metadata. For notifications, prove only transitions into `ServiceFailed` and `DeviceLost` notify, repeated identical snapshots do not, recovery resets deduplication, and color/capture values never enter title/body.

- [ ] **Step 2: Verify failure**

Run: `node --test gui/js/about-render.test.mjs && cargo test -p logig560-gui --test support_fake notification`

Expected: FAIL because About/policy is absent.

- [ ] **Step 3: Implement About from centralized metadata**

Fetch the bundled metadata once and combine it with build/service versions. Render semantic links and the no-account/no-telemetry/no-cloud/no-captured-image statement. A branding consistency test scans user-visible HTML/JS strings and rejects hard-coded `G560 Linux Utility` outside metadata and approved internal compatibility identifiers.

- [ ] **Step 4: Implement notification policy**

Pin `tauri-plugin-notification = "2.3.3"`. Rust observes complete snapshots and sends deduplicated notifications only for service failure and device loss. The frontend cannot supply arbitrary notification text or trigger notifications.

- [ ] **Step 5: Run About and notification tests**

Run: `node --test gui/js/about-render.test.mjs && cargo test -p logig560-gui --test support_fake notification`

Expected: all tests pass.

- [ ] **Step 6: Commit only with explicit owner authorization**

```bash
git add product-metadata.json gui src-tauri
git commit -m "feat: add product information and failure notifications"
```

### Task 5: Accessibility, responsive, and static security verification

**Files:**
- Create: `scripts/check-gui-static.mjs`
- Modify: `gui/styles/tokens.css`
- Modify: `gui/styles/shell.css`
- Modify: `gui/styles/lighting.css`
- Modify: `gui/styles/setup.css`
- Modify: `gui/styles/diagnostics.css`
- Modify: `gui/js/*.test.mjs`

**Interfaces:**
- Produces: one repeatable static check command and manual keyboard acceptance checklist.

- [ ] **Step 1: Write the static checker tests first**

Create temporary invalid fixtures and prove the checker rejects inline handlers/styles, missing button types, image elements without alt text, controls without names, positive tabindex, remote URLs, unsafe CSP tokens, generic process/shell commands, and hard-coded visible product names.

- [ ] **Step 2: Verify the checker detects every fixture**

Run: `node --test scripts/check-gui-static.test.mjs`

Expected: all negative-fixture tests pass while the real GUI initially reports any remaining violations.

- [ ] **Step 3: Fix the real GUI until the checker passes**

Ensure separate focus/selection styles, WCAG AA contrast for normal text and controls, keyboard-operable color presets and rail, focus trapping/return in setup, status text beside colors, reduced decorative motion, scroll/reflow below 1100 px, and unchanged image aspect ratio.

- [ ] **Step 4: Run static, state, and asset checks**

Run:

```bash
node scripts/check-gui-static.mjs gui
node --test gui/js/*.test.mjs scripts/check-gui-static.test.mjs
bash scripts/verify-gui-assets.sh
```

Expected: all checks pass.

- [ ] **Step 5: Perform keyboard/manual visual checks**

From a fresh launch, operate every rail item, tab, zone card, group, color input, brightness slider, master switch, wizard action, maintenance action, copy action, and dialog using only the keyboard. Test normal and reduced motion at 1380x860 and 980x680. Confirm speaker assets never stretch or overlap.

- [ ] **Step 6: Commit only with explicit owner authorization**

```bash
git add gui scripts/check-gui-static.mjs scripts/check-gui-static.test.mjs
git commit -m "test: enforce GUI accessibility and static security"
```

### Task 6: Release acceptance matrix and current documentation

**Files:**
- Create: `docs/testing/gui-acceptance.md`
- Modify: `README.md`
- Modify: `docs/README.md`
- Modify: `docs/architecture.md`
- Modify: `docs/build-and-run.md`
- Modify: `docs/dependencies.md`
- Modify: `docs/known-limitations.md`
- Modify: `docs/operations.md`
- Modify: `docs/project-status.md`
- Modify: `docs/testing.md`
- Modify: `docs/troubleshooting.md`
- Modify: `HANDOFF_BAZZITE.md`
- Create: `docs/decisions/0010-tauri-gui-and-service-boundary.md`

- [ ] **Step 1: Write the acceptance matrix**

Create rows for Fedora GNOME Wayland, Bazzite Desktop Mode, Bazzite Gaming Mode, and Arch KDE Plasma Wayland. Columns cover setup detection, udev, service start/login, Manual zones/groups/brightness, confirmed preview, Content-Aware backend, manual restoration, GUI-close persistence, device loss, capture loss/stall, master off, shutdown, keyboard, narrow window, and sanitized report.

- [ ] **Step 2: Update every current document affected by behavior**

Describe the two-process architecture, Tauri dependencies, D-Bus API, configuration schema/migration, manual defaults, global Content-Aware rule, setup/repair flow, diagnostics privacy, and exact run/verify commands. Add dated current notes to historical acceptance material rather than rewriting its past claims.

- [ ] **Step 3: Run the complete automated gate**

On the correct host or Bazzite container:

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
cargo build --workspace --release
node scripts/check-gui-static.mjs gui
node --test gui/js/*.test.mjs scripts/check-gui-static.test.mjs
bash scripts/verify-gui-assets.sh
bash -n scripts/*.sh
systemd-analyze --user verify systemd/logig560-desktop.service systemd/logig560-gaming.service
git diff --check
```

Expected: every command passes. Record environment-only failures separately and rerun them in the required accepted environment before release acceptance.

- [ ] **Step 4: Run fake-service failure acceptance**

Exercise service unavailable, API mismatch, permission missing/cancelled, device absent/lost/recovered, competing instance, capture unauthorized, zero/multiple streams, capture stall/recovery, Gamescope unavailable, persistence failure/retry, and GUI crash. Confirm no state leaves stale non-black preview after a safety event.

- [ ] **Step 5: Run disciplined real-hardware acceptance**

Inspect `systemctl --user` and `pgrep -af logig560` before every live run. Stop an existing instance cleanly before replacement. Correlate visible behavior with journal timestamps and counters. Never run a pulse/calibration while a service owns the G560. The user confirms physical colors, glow interpretation, blackout immediacy, and Desktop/Gaming behavior.

- [ ] **Step 6: Record results without rewriting history**

Update `docs/project-status.md`, the GUI acceptance matrix, and `HANDOFF_BAZZITE.md` with dated results and known limitations. Do not claim a physical visual issue resolved without the user's confirmation.

- [ ] **Step 7: Commit only with explicit owner authorization**

```bash
git add README.md docs HANDOFF_BAZZITE.md
git commit -m "docs: record GUI architecture and release acceptance"
```

## Plan acceptance

The full GUI effort is complete only when all four implementation plans pass their automated gates, setup and operation are accepted on the available target environments, diagnostic output is demonstrably color-free, closing the GUI leaves the service active, every safety path blacks out immediately, and the user accepts the physical and visual result.
