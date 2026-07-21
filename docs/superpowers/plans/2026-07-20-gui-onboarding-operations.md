# GUI Onboarding and Operations Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the five-step first-run setup, safe udev authorization, user-service installation and maintenance, capture readiness, and actionable recovery banners.

**Architecture:** Tauri owns a small set of fixed operating-system adapters behind traits and exposes semantic setup commands; the browser never submits executables or arguments. A pure frontend setup reducer combines local system checks with authoritative service snapshots. Existing distro-specific scripts and units remain the source of operational truth.

**Tech Stack:** Rust 2024, Tauri 2.11.5, zbus 5.18.0, fixed `std::process::Command` adapters, Polkit/pkexec, systemd user units, vanilla ES modules.

## Global Constraints

- G560 Linux Utility itself never runs as root; only the fixed udev installer uses Polkit.
- No generic shell/process Tauri plugin or frontend-provided command, path, environment, or argument is allowed.
- Existing service configuration and working permissions are preserved by repair operations.
- The GUI must not start a second service or hardware owner.
- Desktop setup uses the portal and requires exactly one monitor; Gaming setup verifies Gamescope and never falls back to the portal.
- Fedora and Arch install development dependencies on the host; Bazzite builds in the `logig560` container and runs in the host user session.
- The accepted Gaming unit and current documentation change together if its path changes.
- No commit step may run unless the repository owner explicitly authorizes commits.

---

## File structure

- `src-tauri/src/setup/types.rs`: setup snapshots, steps, actions, and stable errors.
- `src-tauri/src/setup/system_probe.rs`: `/etc/os-release`, runtime, session, USB, and capture-backend checks.
- `src-tauri/src/setup/service_manager.rs`: fixed user-unit install/start/stop/restart/status operations.
- `src-tauri/src/setup/udev.rs`: fixed Polkit-backed udev installer adapter.
- `src-tauri/src/setup/mod.rs`: orchestration and Tauri command entry points.
- `src-tauri/tests/setup_fake.rs`: fake probe/process/service acceptance tests.
- `gui/js/setup-state.js`: pure five-step wizard reducer.
- `gui/js/setup-render.js`: wizard and maintenance dashboard rendering.
- `gui/styles/setup.css`: stepper, check rows, instructions, and confirmations.
- `scripts/install-desktop-service.sh`: source-build desktop user-unit installer.

### Task 1: Setup types and pure wizard reducer

**Files:**
- Create: `src-tauri/src/setup/mod.rs`
- Create: `src-tauri/src/setup/types.rs`
- Create: `gui/js/setup-state.js`
- Create: `gui/js/setup-state.test.mjs`
- Modify: `gui/js/state.js`

**Interfaces:**
- Produces: `SetupStep`, `CheckState`, `Distribution`, `SessionKind`, `SetupSnapshot`, `SetupAction`, `SetupError`, `createSetupState`, `reduceSetupSnapshot`, `canAdvance`, and `nextStep`.

- [ ] **Step 1: Write wizard reducer tests**

Test the exact ordered steps `system`, `speaker`, `service`, `capture`, `ready`; blocked checks cannot advance; informational dependency instructions can advance only after explicit acknowledgement; a fresh configuration opens the overlay; completed setup does not; and rerun starts at System without erasing lighting state.

- [ ] **Step 2: Verify failure**

Run: `node --test gui/js/setup-state.test.mjs && cargo test -p logig560-gui setup::types`

Expected: FAIL because setup modules do not exist.

- [ ] **Step 3: Implement shared setup vocabulary**

Use closed enums for supported distributions and session types. `SetupSnapshot` contains each check's stable ID, state, summary, detail, and permitted semantic actions. It contains no raw shell command supplied by the frontend. Unsupported distributions produce `Distribution::Unsupported` plus maintained manual instructions; they do not guess package-manager commands.

- [ ] **Step 4: Implement the pure wizard reducer**

The reducer owns only active step, acknowledgements, confirmation dialogs, and visibility. It never marks a check successful without a new Rust snapshot. Ready can finish only when speaker permission, service running, and the active environment's capture check are successful.

- [ ] **Step 5: Run reducer/type tests**

Run: `node --test gui/js/setup-state.test.mjs && cargo test -p logig560-gui setup::types`

Expected: all tests pass.

- [ ] **Step 6: Commit only with explicit owner authorization**

```bash
git add src-tauri/src/setup gui/js/setup-state.js gui/js/setup-state.test.mjs gui/js/state.js
git commit -m "feat: model guided G560 setup state"
```

### Task 2: Distribution, session, dependency, and G560 probes

**Files:**
- Create: `src-tauri/src/setup/system_probe.rs`
- Modify: `src-tauri/src/setup/mod.rs`
- Create: `src-tauri/tests/setup_fake.rs`

**Interfaces:**
- Produces: trait `SystemProbe`, `LinuxSystemProbe`, and `probe_setup()`.
- Consumes: fixed filesystem paths `/etc/os-release`, `/sys/bus/usb/devices`, `XDG_CURRENT_DESKTOP`, and `XDG_SESSION_TYPE`.

- [ ] **Step 1: Write probe tests with injected files/environment**

Cover Fedora, Bazzite, Arch, unsupported Linux, GNOME Wayland, KDE Wayland, Gamescope, missing WebKitGTK runtime metadata, G560 present, G560 absent, and permission denied. Test USB matching using exact lower-case vendor `046d` and product `0a78` files.

- [ ] **Step 2: Verify failure**

Run: `cargo test -p logig560-gui --test setup_fake system_probe`

Expected: FAIL because `SystemProbe` is absent.

- [ ] **Step 3: Implement read-only probes**

Parse `ID` and `ID_LIKE` without executing them. Enumerate only `/sys/bus/usb/devices/*/{idVendor,idProduct}` and check access without opening or claiming USB. Select expected capture backend from the session environment: Gamescope only for Gaming Mode, otherwise Desktop Portal. Dependency probes use fixed `pkg-config --exists` names selected in Rust; frontend data cannot add names or arguments.

- [ ] **Step 4: Return maintained instructions**

Map each supported distribution to instruction IDs whose displayed text is kept in `gui/js/setup-copy.js`. Rust returns IDs and facts, not localized prose or shell fragments. Bazzite instructions explicitly distinguish container build dependencies from host execution.

- [ ] **Step 5: Run probe tests**

Run: `cargo test -p logig560-gui --test setup_fake system_probe`

Expected: all probe cases pass without touching real USB.

- [ ] **Step 6: Commit only with explicit owner authorization**

```bash
git add src-tauri/src/setup src-tauri/tests/setup_fake.rs gui/js/setup-copy.js
git commit -m "feat: probe supported Linux setup environments"
```

### Task 3: Safe user-service manager

**Files:**
- Create: `src-tauri/src/setup/service_manager.rs`
- Modify: `src-tauri/src/setup/mod.rs`
- Create: `scripts/install-desktop-service.sh`
- Modify: `scripts/install-gaming-service.sh`
- Modify: `src-tauri/tests/setup_fake.rs`

**Interfaces:**
- Produces: trait `ServiceManager`, `SystemdUserServiceManager`, `ServiceAction::{Install,Start,Stop,Restart,Enable}`, and `ServiceStatus`.

- [ ] **Step 1: Write allowlist and state tests**

Use a fake runner to prove every action maps to exact argv, unknown units cannot be selected, start/restart checks current state first, the GUI unit and Gaming unit cannot be activated together in Desktop Mode, and failures return sanitized stderr capped at 4 KiB.

- [ ] **Step 2: Verify failure**

Run: `cargo test -p logig560-gui --test setup_fake service_manager`

Expected: FAIL because the manager is missing.

- [ ] **Step 3: Implement fixed service operations**

Allow only `logig560-desktop.service` and `logig560-gaming.service`. Use direct `systemctl --user` argv without a shell. Query `LoadState`, `ActiveState`, and `UnitFileState` before mutation. A restart of the active correct unit is allowed; starting a second conflicting owner is refused with `SetupError::CompetingInstance`.

The desktop installer script resolves the current release binary once, writes a user unit with an absolute validated path, runs daemon-reload, and enables the unit. It rejects root and paths outside a regular executable file. Keep the Gaming installer behavior and documentation synchronized.

- [ ] **Step 4: Run service-manager and shell tests**

Run: `cargo test -p logig560-gui --test setup_fake service_manager && bash -n scripts/*.sh`

Expected: all tests pass.

- [ ] **Step 5: Commit only with explicit owner authorization**

```bash
git add src-tauri/src/setup/service_manager.rs src-tauri/src/setup/mod.rs src-tauri/tests/setup_fake.rs scripts
git commit -m "feat: manage G560 Linux Utility user services safely"
```

### Task 4: Fixed Polkit-backed udev repair

**Files:**
- Create: `src-tauri/src/setup/udev.rs`
- Modify: `src-tauri/src/setup/mod.rs`
- Modify: `scripts/install-udev-rule.sh`
- Modify: `src-tauri/tests/setup_fake.rs`

**Interfaces:**
- Produces: trait `UdevInstaller`, `PolkitUdevInstaller`, `UdevStatus`, and semantic command `install_g560_access`.

- [ ] **Step 1: Write exact-command security tests**

Prove that the adapter invokes one compiled/configured installer path through `pkexec`, passes no frontend strings, refuses effective UID 0, reports cancellation separately from failure, and rerunning with the exact installed rule is idempotent.

- [ ] **Step 2: Verify failure**

Run: `cargo test -p logig560-gui --test setup_fake udev`

Expected: FAIL because the adapter is absent.

- [ ] **Step 3: Harden and expose the existing installer**

Keep rule contents restricted to `046d:0a78`. The script validates its source rule, installs to the one documented target, reloads udev rules, and triggers only that device match. It refuses to proceed when called as the normal user without Polkit elevation and never launches the application as root.

The Tauri command asks for explicit UI confirmation before calling `PolkitUdevInstaller`. After success it reruns the read-only permission probe; it does not claim success from process exit alone.

- [ ] **Step 4: Run security and syntax tests**

Run: `cargo test -p logig560-gui --test setup_fake udev && bash -n scripts/install-udev-rule.sh`

Expected: all tests pass.

- [ ] **Step 5: Commit only with explicit owner authorization**

```bash
git add src-tauri/src/setup/udev.rs src-tauri/src/setup/mod.rs src-tauri/tests/setup_fake.rs scripts/install-udev-rule.sh
git commit -m "feat: repair G560 access through fixed Polkit action"
```

### Task 5: Five-step setup overlay

**Files:**
- Create: `gui/js/setup-copy.js`
- Create: `gui/js/setup-render.js`
- Create: `gui/js/setup-render.test.mjs`
- Create: `gui/styles/setup.css`
- Modify: `gui/index.html`
- Modify: `gui/js/main.js`
- Modify: `gui/js/api.js`

**Interfaces:**
- Consumes: `probe_setup`, `run_setup_action`, `install_g560_access`, `choose_desktop_display`, and `complete_setup` semantic commands.
- Produces: accessible modal stepper and exact per-step actions.

- [ ] **Step 1: Add failing render/action tests**

Test the exact five step labels, focus entry and return, Escape behavior before/after irreversible actions, Fedora/Bazzite/Arch instruction copy, Polkit confirmation showing the fixed rule target, single-monitor portal explanation, Gamescope wording, and Ready gating.

- [ ] **Step 2: Verify failure**

Run: `node --test gui/js/setup-render.test.mjs`

Expected: FAIL because setup rendering is absent.

- [ ] **Step 3: Implement the overlay**

Use `<dialog>` or an equivalent focus-trapped modal with heading association, ordered step indicator, check list, Back/Continue, Retry, and explicit action buttons. The underlying application remains visible but inert. Copy explains that G560 Linux Utility never runs as root and that Content-Aware always uses all four zones.

- [ ] **Step 4: Complete setup through the service**

Add `CompleteSetup` to the D-Bus service and Tauri client. It persists `setup_complete = true` only after the GUI submits a successful current `SetupSnapshot` token returned by `probe_setup`; stale readiness cannot complete onboarding.

- [ ] **Step 5: Run JS and D-Bus contract tests**

Run: `node --test gui/js/setup-*.test.mjs && cargo test -p logig560 --test dbus_contract complete_setup`

Expected: all tests pass.

- [ ] **Step 6: Commit only with explicit owner authorization**

```bash
git add gui src-tauri/src src/dbus.rs tests/dbus_contract.rs crates/logig560-api
git commit -m "feat: guide users through first-run setup"
```

### Task 6: Setup & Service maintenance dashboard

**Files:**
- Modify: `gui/js/setup-render.js`
- Modify: `gui/js/setup-state.js`
- Modify: `gui/js/setup-render.test.mjs`
- Modify: `gui/styles/setup.css`
- Modify: `gui/js/main.js`

**Interfaces:**
- Consumes: live setup snapshots and semantic Start, Stop, Restart, Enable, Repair, Re-run Setup, Choose Display, and Restart Capture actions.
- Produces: maintenance page and context-sensitive recovery banners.

- [ ] **Step 1: Add dashboard state tests**

Cover installed/stopped, enabled/running, permission missing, device absent, Desktop capture unauthorized, Desktop stalled, Gamescope unavailable, competing instance, unsupported distribution, and successful states. Each failure must expose one primary action and a Diagnostics link where useful.

- [ ] **Step 2: Verify failure**

Run: `node --test gui/js/setup-render.test.mjs`

Expected: new dashboard tests fail.

- [ ] **Step 3: Implement dashboard and banners**

Render service, speaker, Desktop/Gaming integration, and capture cards. Disable impossible/conflicting actions. A service/device/capture banner in the global shell deep-links to the exact card. Rerun Setup preserves all service configuration and manual colors.

- [ ] **Step 4: Run fake-operations acceptance**

Run: `node --test gui/js/*.test.mjs && cargo test -p logig560-gui --test setup_fake`

Expected: all state, rendering, allowlist, probe, and process tests pass.

- [ ] **Step 5: Commit only with explicit owner authorization**

```bash
git add gui
git commit -m "feat: add service maintenance and recovery dashboard"
```

### Task 7: Onboarding verification and documentation

**Files:**
- Modify: `README.md`
- Modify: `docs/build-and-run.md`
- Modify: `docs/dependencies.md`
- Modify: `docs/machine-porting.md`
- Modify: `docs/operations.md`
- Modify: `docs/project-status.md`
- Modify: `docs/testing.md`
- Modify: `docs/troubleshooting.md`
- Modify: `HANDOFF_BAZZITE.md`

- [ ] **Step 1: Synchronize GUI copy and current docs**

Document every automated action, exact fallback instruction, Desktop/Gaming distinction, Polkit boundary, unit installation path, and Bazzite container/host split. Add a test or script that verifies the displayed command snippets match the maintained documentation source.

- [ ] **Step 2: Run full validation**

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
cargo build --workspace --release
node --test gui/js/*.test.mjs
bash scripts/verify-gui-assets.sh
bash -n scripts/*.sh
systemd-analyze --user verify systemd/logig560-desktop.service systemd/logig560-gaming.service
git diff --check
```

Expected: all checks pass in the appropriate target environment.

- [ ] **Step 3: Run safe environment acceptance**

With fakes, exercise every setup success, cancellation, failure, repair, and stale-token path. On each real supported environment, inspect current processes before starting; verify detection and instructions without running a hardware pulse. Invoke Polkit only with the user's explicit confirmation.

- [ ] **Step 4: Commit only with explicit owner authorization**

```bash
git add README.md docs HANDOFF_BAZZITE.md
git commit -m "docs: add guided setup and service maintenance"
```

## Plan acceptance

This increment is complete only when a fresh supported environment can reach Ready through safe actions or exact maintained instructions, rerunning setup preserves lighting settings, competing service ownership is prevented, and the user confirms the real Polkit/service/capture flow on each available target environment.
