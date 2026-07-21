# GUI Service Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Convert the existing one-shot capture CLI into a persistent, testable lighting service with manual state, atomic configuration, confirmed-hardware state, and a versioned user-session D-Bus API.

**Architecture:** A pure controller owns desired product state and delegates physical work to a `LightingBackend`. A single actor coalesces ordinary state changes through Tokio `watch`, while blackout requests use a priority path. The real backend reuses the current capture engine and G560 writer; D-Bus exposes typed snapshots and atomic logical-zone commands without exposing USB indexes.

**Tech Stack:** Rust 2024, Tokio 1.51, serde/TOML, zbus 5.18.0, zvariant 5.13.1, systemd user services.

## Global Constraints

- Never run G560 Linux Utility as root; Polkit remains limited to the fixed udev installer.
- Capture exactly one portal-authorized monitor in Desktop Mode; zero or multiple streams are errors.
- Never save frames, screenshots, thumbnails, sampled colors, confirmed colors, or color history.
- Ordinary state uses newest-value-only handoffs; safety blackouts preempt all normal work.
- Preserve at least 6 ms between every adjacent HID report, including across calls.
- Preserve logical `[left rear, left front, right front, right rear]` to protocol `[0x02, 0x00, 0x01, 0x03]`.
- Do not detach a non-HID interface; validate interface 2 and reattach its kernel driver on release.
- Desktop Portal and Gamescope remain separate capture backends.
- Clean shutdown stops capture, requests all-zone black, releases USB, and closes workers.
- No commit step may run unless the repository owner explicitly authorizes commits.

---

## File structure

- `crates/logig560-api/src/lib.rs`: transport-safe API constants, DTOs, enums, and validation.
- `src/config.rs`: versioned application configuration and atomic private-file persistence.
- `src/control/model.rs`: pure state transitions and manual RGB/brightness projection.
- `src/control/actor.rs`: latest-only ordinary commands, priority blackouts, snapshots, and persistence coordination.
- `src/control/backend.rs`: backend trait, confirmed-write events, and fake backend.
- `src/control/runtime.rs`: real USB/capture adapter built from existing engine components.
- `src/control/mod.rs`: focused public exports.
- `src/dbus.rs`: zbus service interface and snapshot notification publisher.
- `src/main.rs`: CLI parsing and thin wiring for diagnostics, `serve`, and `serve-gaming`.
- `tests/controller_fake.rs`: controller acceptance tests without USB or capture.
- `tests/dbus_contract.rs`: private-session-bus API contract tests.
- `systemd/logig560-desktop.service`: persistent Desktop Mode user service.
- `systemd/logig560-gaming.service`: Gaming Mode service updated to the service command.

### Task 1: Shared API crate and domain vocabulary

**Files:**
- Modify: `Cargo.toml`
- Create: `crates/logig560-api/Cargo.toml`
- Create: `crates/logig560-api/src/lib.rs`
- Modify: `src/color.rs`
- Modify: `src/lib.rs`
- Test: `crates/logig560-api/src/lib.rs`

**Interfaces:**
- Produces: `API_VERSION`, `BUS_NAME`, `OBJECT_PATH`, `ZoneId`, `LightingMode`, `CaptureBackend`, `HealthState`, `RgbColor`, `ManualZone`, `ZoneColor`, `DiagnosticCounters`, `ServiceSnapshot`, `ManualZoneUpdate`, and `ApiError`.
- Produces: conversions between `ZoneId`/`RgbColor` and the existing `Zone`/`Rgb8` types.

- [ ] **Step 1: Add failing DTO validation tests**

```rust
#[test]
fn manual_update_rejects_duplicate_zones() {
    let red = RgbColor { red: 255, green: 0, blue: 0 };
    let updates = vec![
        ManualZoneUpdate { zone: ZoneId::LeftFront, color: red, brightness: 100 },
        ManualZoneUpdate { zone: ZoneId::LeftFront, color: red, brightness: 50 },
    ];
    assert_eq!(validate_manual_updates(&updates), Err(ApiError::DuplicateZone(ZoneId::LeftFront)));
}

#[test]
fn brightness_must_be_at_most_one_hundred() {
    let update = ManualZoneUpdate {
        zone: ZoneId::RightRear,
        color: RgbColor { red: 1, green: 2, blue: 3 },
        brightness: 101,
    };
    assert_eq!(validate_manual_updates(&[update]), Err(ApiError::InvalidBrightness(101)));
}
```

- [ ] **Step 2: Run the tests and observe the missing API types**

Run: `cargo test -p logig560-api`

Expected: FAIL because the workspace member and DTOs do not exist.

- [ ] **Step 3: Add the workspace member and API crate**

Add a root workspace containing `.` and `crates/logig560-api`. Give the API crate exact dependencies `serde = "1.0"` with `derive`, `thiserror = "2.0"`, and `zvariant = "5.13.1"` with `option-as-array`.

Define `ZoneId` in logical array order and make every DTO `Clone + Debug + Eq + PartialEq + Serialize + Deserialize + zvariant::Type`. Use a fixed `Vec<ZoneColor>` in snapshots rather than raw protocol indexes. Set:

```rust
pub const API_VERSION: u32 = 1;
pub const BUS_NAME: &str = "org.logig560.Service1";
pub const OBJECT_PATH: &str = "/org/logig560/Service1";
pub const INTERFACE_NAME: &str = "org.logig560.Service1";
```

`validate_manual_updates` must reject an empty list, duplicate logical zones, and brightness above 100. It must accept any nonempty unique subset of the four logical zones.

- [ ] **Step 4: Add explicit core conversions**

Add `Zone::ALL` in `src/color.rs` as `[LeftRear, LeftFront, RightFront, RightRear]`. Implement `From<ZoneId> for Zone`, `From<Zone> for ZoneId`, `From<RgbColor> for Rgb8`, and `From<Rgb8> for RgbColor` beside the core color types. Re-export API types from `src/lib.rs` only where the service needs them; do not move USB protocol mapping into the API crate.

- [ ] **Step 5: Run focused and full library tests**

Run: `cargo test -p logig560-api && cargo test -p logig560 --lib`

Expected: all tests pass.

- [ ] **Step 6: Commit only with explicit owner authorization**

```bash
git add Cargo.toml Cargo.lock crates/logig560-api src/color.rs src/lib.rs
git commit -m "feat: define lighting service API types"
```

### Task 2: Versioned atomic application configuration

**Files:**
- Create: `src/config.rs`
- Modify: `src/lib.rs`
- Modify: `src/main.rs`
- Test: `src/config.rs`

**Interfaces:**
- Consumes: `LightingMode`, `ManualZone`, `RgbColor`, and `ZoneId` from `logig560-api`.
- Produces: `AppConfig`, `ConfigStore`, `FileConfigStore`, `config_path()`, and `ConfigLoad`.

- [ ] **Step 1: Write configuration red tests**

Cover these exact cases in `src/config.rs`: `fresh_config_is_manual_cyan_and_lights_on` compares all four logical zones with `RgbColor { red: 0x14, green: 0xc8, blue: 0xf4 }` at brightness 100; `save_replaces_atomically_with_mode_0600` saves into a `tempfile::TempDir`, reloads an identical value, checks permission bits `0o600`, and asserts the directory contains only `config.toml`; `failed_parse_preserves_invalid_file_and_returns_safe_defaults` writes `not = [valid`, matches `ConfigLoad::Recovered`, reads the preserved invalid bytes back unchanged, and compares the returned config with `AppConfig::default()`; `v1_capture_config_migrates_restore_token_without_losing_it` writes `version = 1\nrestore_token = "token"`, then asserts schema 2 and the exact token.

- [ ] **Step 2: Verify the tests fail**

Run: `cargo test -p logig560 config::tests -- --nocapture`

Expected: FAIL because `config` is not defined.

- [ ] **Step 3: Implement the configuration model**

Use schema version 2. `AppConfig` contains `version`, `lights_enabled`, `mode`, `[ManualZone; 4]`, `restore_token`, and `setup_complete`. Default to Lights On, Manual, all zones `#14C8F4`, brightness 100, no token, and setup incomplete.

`FileConfigStore::save` must create the parent, create a private unique temporary file with mode `0600`, serialize TOML, `write_all`, `sync_all`, rename, and sync the parent. On error it removes only its own exact temporary path. Move the duplicate capture-config persistence code out of `src/main.rs` and keep compatibility with the current `capture.toml` input.

Malformed input is renamed in the same directory to `config.invalid-<unix-seconds>.toml`; if that collision exists, add `-1` through `-99`. Return safe defaults plus the preserved path so Diagnostics can report it.

- [ ] **Step 4: Run the red tests green**

Run: `cargo test -p logig560 config::tests -- --nocapture`

Expected: all configuration tests pass.

- [ ] **Step 5: Run existing CLI configuration tests**

Run: `cargo test -p logig560 --bin logig560`

Expected: current hex parsing, capture retry, and private restore-token tests pass after using `config.rs`.

- [ ] **Step 6: Commit only with explicit owner authorization**

```bash
git add src/config.rs src/lib.rs src/main.rs
git commit -m "feat: persist service lighting configuration"
```

### Task 3: Pure controller model

**Files:**
- Create: `src/control/mod.rs`
- Create: `src/control/model.rs`
- Test: `src/control/model.rs`

**Interfaces:**
- Consumes: `AppConfig`, service API DTOs, `Zone::ALL`, and `ZoneColors`.
- Produces: `ControllerModel::new`, `apply_manual_updates`, `set_mode`, `set_lights_enabled`, `mark_pending`, `confirm_write`, `record_failure`, `snapshot`, and `manual_target`.

- [ ] **Step 1: Write reducer tests before the model**

Add tests proving:

```rust
#[test]
fn brightness_scales_channels_with_rounding() {
    assert_eq!(scale(Rgb8 { r: 255, g: 127, b: 1 }, 50), Rgb8 { r: 128, g: 64, b: 1 });
}
```

Add `grouped_update_changes_only_named_zones_atomically` using red front updates and unchanged cyan rear settings; `content_aware_rejects_manual_updates` expecting `ApiError::ModeConflict`; `lights_off_retains_mode_and_manual_state_but_targets_black` comparing the saved mode/settings before and after while `manual_target()` returns `ZoneColors::BLACK`; and `failed_write_keeps_requested_state_pending_and_confirmed_state_unchanged` comparing an exact red request against the prior cyan confirmed snapshot.

- [ ] **Step 2: Verify failure**

Run: `cargo test -p logig560 control::model::tests`

Expected: FAIL because the controller model is missing.

- [ ] **Step 3: Implement the pure model**

Keep `ControllerModel` free of Tokio, files, D-Bus, USB, and capture types. `manual_target` iterates `Zone::ALL`, looks up each logical setting, and applies `(channel * brightness + 50) / 100` using `u16`. `set_mode(ContentAware)` does not erase manual state. `set_lights_enabled(false)` produces black but preserves mode. Only `confirm_write` changes confirmed colors.

Every public mutation returns either `ModelEffect::NoWrite`, `ModelEffect::Write(ZoneColors)`, `ModelEffect::StartContentAware`, `ModelEffect::StopContentAwareAndWrite(ZoneColors)`, or `ModelEffect::PriorityBlackout`.

- [ ] **Step 4: Run model and property tests**

Add a proptest that every channel remains `0..=255` and brightness zero produces black. Run:

`cargo test -p logig560 control::model::tests`

Expected: all reducer and property tests pass.

- [ ] **Step 5: Commit only with explicit owner authorization**

```bash
git add src/control src/lib.rs
git commit -m "feat: add authoritative lighting controller model"
```

### Task 4: Controller actor with latest-only normal state and priority safety

**Files:**
- Create: `src/control/backend.rs`
- Create: `src/control/actor.rs`
- Modify: `src/control/mod.rs`
- Create: `tests/controller_fake.rs`

**Interfaces:**
- Produces: async trait `LightingBackend`, `BackendEvent`, `ControllerHandle`, `ControllerCommand`, `ControllerError`, and `spawn_controller`.
- `ControllerHandle`: `snapshot()`, `subscribe()`, `set_manual_zones(Vec<ManualZoneUpdate>)`, `set_mode(LightingMode)`, `set_lights_enabled(bool)`, `choose_desktop_display()`, `restart_capture()`, and `shutdown()`.

- [ ] **Step 1: Write fake-backend acceptance tests**

Use a fake backend with a `Notify`-blocked first write and an `Arc<Mutex<Vec<Operation>>>` log. `rapid_manual_targets_replace_unread_targets` submits red, green, and blue while red is blocked, releases it, and compares the write log with `[Write(red), Write(blue)]`. `lights_off_preempts_a_blocked_normal_write` verifies `Blackout` is the next operation after release. `confirmed_snapshot_advances_only_after_backend_confirmation` asserts pending red with confirmed cyan before release and confirmed red after release. `shutdown_stops_capture_then_blacks_out_then_releases_backend` compares the final operations exactly with `[StopCapture, Blackout, Shutdown]`.

- [ ] **Step 2: Verify failure**

Run: `cargo test -p logig560 --test controller_fake`

Expected: FAIL because actor/backend types do not exist.

- [ ] **Step 3: Define the backend contract**

```rust
#[async_trait::async_trait]
pub trait LightingBackend: Send + 'static {
    async fn write_manual(&mut self, colors: ZoneColors) -> anyhow::Result<LightUpdateStatus>;
    async fn start_content_aware(&mut self, backend: CaptureBackend) -> anyhow::Result<()>;
    async fn stop_capture(&mut self) -> anyhow::Result<()>;
    async fn blackout(&mut self) -> anyhow::Result<()>;
    async fn shutdown(&mut self) -> anyhow::Result<()>;
    fn events(&self) -> tokio::sync::watch::Receiver<BackendEvent>;
}
```

`BackendEvent` contains lifecycle/health/counter state plus ephemeral `Confirmed(ZoneColors)`; it never contains a frame or sampled-color history.

- [ ] **Step 4: Implement the actor**

Use `watch::Sender<DesiredState>` for ordinary mode/manual changes and `mpsc::unbounded_channel<SafetyCommand>` for blackout/shutdown. In every select loop, use `biased;` with the safety receiver first. A normal command updates and saves configuration, publishes `pending = true`, and returns acceptance without waiting for hardware. The actor consumes only the newest desired state. Backend confirmation clears pending and updates confirmed colors.

Use a `watch::Sender<ServiceSnapshot>` for subscribers. Do not use a `VecDeque` for commands, colors, frames, or snapshots.

- [ ] **Step 5: Make acceptance tests green**

Run: `cargo test -p logig560 --test controller_fake`

Expected: all latest-only, priority, confirmation, and shutdown-order tests pass.

- [ ] **Step 6: Run concurrency regression tests**

Run: `cargo test -p logig560 latest engine::tests usb::device::tests`

Expected: all existing newest-only and USB priority tests pass.

- [ ] **Step 7: Commit only with explicit owner authorization**

```bash
git add src/control tests/controller_fake.rs src/lib.rs
git commit -m "feat: coordinate newest lighting state and safety commands"
```

### Task 5: Real runtime backend and confirmed-write observation

**Files:**
- Create: `src/control/runtime.rs`
- Modify: `src/engine.rs`
- Modify: `src/main.rs`
- Modify: `src/control/mod.rs`
- Test: `src/control/runtime.rs`
- Test: `tests/engine_fake.rs`

**Interfaces:**
- Consumes: `LightingBackend`, existing frame-source factories, `RecoveringLightSink`, and `run_engine_with_metrics`.
- Produces: `RuntimeBackend`, `ObservedLightSink<L>`, `DesktopCaptureFactory`, and `GamescopeCaptureFactory`.

- [ ] **Step 1: Add failing observed-sink tests**

Test that `ObservedLightSink` publishes a confirmed color only for `Rendered` or `Unchanged`, publishes black after a successful blackout or `Expired`, and publishes nothing after an error. Use `watch` and a fake `LightSink`.

- [ ] **Step 2: Verify failure**

Run: `cargo test -p logig560 control::runtime::tests`

Expected: FAIL because the runtime adapter is absent.

- [ ] **Step 3: Extract reusable factories from `main.rs`**

Move `PortalFrameSourceFactory`, `GamescopeFrameSourceFactory`, capture retry classification, and restore-token updates into `src/control/runtime.rs`. Keep CLI functions thin. Preserve the existing behavior and tests for portal cancellation, post-open retry, Gamescope startup, and source shutdown.

- [ ] **Step 4: Implement `ObservedLightSink` and `RuntimeBackend`**

Wrap the existing recovering async G560 sink so successful physical writes publish ephemeral confirmed colors. Manage one optional content-aware task with its own `CancellationToken`. Starting a mode first stops and awaits the previous task; stopping Content-Aware waits for engine cleanup before manual USB ownership begins. `shutdown` performs stop-capture, priority blackout, and worker release in that order.

Do not change report order, `REPORT_DELAY`, interface claiming, or engine safety logic.

- [ ] **Step 5: Run runtime and engine tests**

Run: `cargo test -p logig560 control::runtime::tests -- --nocapture && cargo test -p logig560 --test engine_fake`

Expected: all tests pass, including final black on cancellation.

- [ ] **Step 6: Commit only with explicit owner authorization**

```bash
git add src/control/runtime.rs src/control/mod.rs src/engine.rs src/main.rs tests/engine_fake.rs
git commit -m "feat: adapt capture and USB runtime to service controller"
```

### Task 6: Versioned D-Bus service contract

**Files:**
- Modify: `Cargo.toml`
- Create: `src/dbus.rs`
- Modify: `src/lib.rs`
- Create: `tests/dbus_contract.rs`

**Interfaces:**
- Consumes: `ControllerHandle` and all `logig560-api` DTOs.
- Produces: `ServiceInterface::new`, `serve_dbus`, and D-Bus methods/signals matching the approved spec.

- [ ] **Step 1: Add private-bus contract tests**

Start a temporary `dbus-daemon --session --print-address --nofork` for the test process. Cover `GetSnapshot`, an atomic two-zone `SetManualZones`, `SetLightsEnabled`, `SetMode`, `RestartCapture`, `ChooseDesktopDisplay`, `SnapshotChanged`, invalid brightness, duplicate zones, and API mismatch.

- [ ] **Step 2: Verify failure**

Run: `cargo test -p logig560 --test dbus_contract -- --nocapture`

Expected: FAIL because no service object is exported.

- [ ] **Step 3: Add zbus and implement the interface**

Add `zbus = { version = "5.18.0", default-features = false, features = ["tokio", "option-as-array"] }` to the root crate. Export `org.logig560.Service1` at `/org/logig560/Service1`.

Every method validates `API_VERSION`. Map domain failures to stable `ApiError` values instead of exposing arbitrary Rust backtraces. Start one notification task that watches the controller snapshot and emits only the newest complete `SnapshotChanged` value, capped at 20 Hz. The Diagnostics increment adds the report method after its privacy projection has tests.

- [ ] **Step 4: Run contract tests**

Run: `cargo test -p logig560 --test dbus_contract -- --nocapture`

Expected: all method, signal, validation, and mismatch tests pass.

- [ ] **Step 5: Inspect the live interface without hardware**

Run the service with a test-only fake-backend binary under `dbus-run-session`, then run:

```bash
busctl --user introspect org.logig560.Service1 /org/logig560/Service1
```

Expected: only the approved methods, properties, and `SnapshotChanged` signal are exposed.

- [ ] **Step 6: Commit only with explicit owner authorization**

```bash
git add Cargo.toml Cargo.lock src/dbus.rs src/lib.rs tests/dbus_contract.rs
git commit -m "feat: expose versioned lighting service API"
```

### Task 7: Service commands, user units, and current documentation

**Files:**
- Modify: `src/main.rs`
- Create: `systemd/logig560-desktop.service`
- Modify: `systemd/logig560-gaming.service`
- Modify: `README.md`
- Modify: `docs/architecture.md`
- Modify: `docs/build-and-run.md`
- Modify: `docs/operations.md`
- Modify: `docs/project-status.md`
- Modify: `docs/testing.md`
- Modify: `docs/usb-and-safety.md`
- Modify: `HANDOFF_BAZZITE.md`
- Create: `docs/decisions/0009-service-controller-and-dbus.md`

**Interfaces:**
- Produces: `logig560 serve` and `logig560 serve-gaming`.
- Preserves: every existing diagnostic CLI subcommand.

- [ ] **Step 1: Add CLI parsing tests**

Add tests that `serve` selects Desktop Portal, `serve-gaming` selects Gamescope, and neither accepts root execution. Test the root guard through a small pure function that accepts an effective UID so the test itself never runs as root.

- [ ] **Step 2: Verify failure**

Run: `cargo test -p logig560 --bin logig560 parses_serve`

Expected: FAIL because service commands do not exist.

- [ ] **Step 3: Wire service lifecycle**

`serve` and `serve-gaming` load configuration, create `RuntimeBackend`, spawn the controller, export D-Bus, wait for SIGINT/SIGTERM, request controller shutdown, await it, and then release the bus. Refuse effective UID 0 before opening configuration, portals, or USB.

- [ ] **Step 4: Update systemd units**

Desktop uses `ExecStart=logig560 serve`, `Restart=on-failure`, and the graphical session target appropriate to the maintained installation script. Gaming uses `serve-gaming` and retains its Gamescope `PartOf`/`WantedBy` relationship. Do not silently change the accepted `%h/Documents/G560 Linux Utility` path; if execution changes that path, update the unit and all current documentation in the same task.

- [ ] **Step 5: Document the new architecture and operational checks**

Record the controller/D-Bus decision, manual persistence, mode transitions, exact process ownership, startup behavior, and color-free diagnostics. Add live-operation steps that inspect `systemctl --user` and `pgrep -af logig560` before starting another instance.

- [ ] **Step 6: Run full automated validation**

On Fedora/Arch host or inside the Bazzite `logig560` container as required:

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
cargo build --workspace --release
bash -n scripts/*.sh
systemd-analyze --user verify systemd/logig560-desktop.service systemd/logig560-gaming.service
git diff --check
```

Expected: all commands pass. A missing hard-coded Bazzite executable in a different checkout is an environment failure; verify the unchanged unit on the accepted Bazzite path before acceptance.

- [ ] **Step 7: Perform a no-hardware service smoke test**

Run the controller with the fake backend in a private D-Bus session. Exercise Manual, Content-Aware, Lights Off, and shutdown. Expected: operation log ends with stop-capture, blackout, release; diagnostics contain zero colors.

- [ ] **Step 8: Commit only with explicit owner authorization**

```bash
git add src/main.rs systemd README.md docs HANDOFF_BAZZITE.md
git commit -m "feat: run G560 Linux Utility as a persistent control service"
```

## Plan acceptance

This increment is complete only when the fake-controller and D-Bus contract tests pass, both service commands shut down cleanly, all current CLI diagnostics remain available, and the required repository validation passes. Do not begin the Tauri lighting application until the service snapshot and command contract are stable.
