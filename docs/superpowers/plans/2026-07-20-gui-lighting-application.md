# Tauri Lighting Application Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the real Tauri desktop application shell and the complete Manual and global Content-Aware Lighting interfaces against the stable service API.

**Architecture:** A framework-free HTML/CSS/JavaScript frontend renders immutable UI state and invokes a narrow Rust command layer. The Tauri backend owns the zbus client and republishes only complete service snapshots. Speaker visualization uses the accepted alpha PNGs, neutral emission textures, CSS masks, and rear glows.

**Tech Stack:** Tauri 2.11.5, tauri-build 2.6.3, Rust 2024, zbus 5.18.0, vanilla ES modules, Node's built-in test runner, HTML/CSS.

## Global Constraints

- The GUI never opens USB, capture, systemd, or arbitrary shell interfaces directly.
- The background service remains the sole hardware and capture owner when the GUI closes.
- Content-Aware is global across all four zones and disables every manual control.
- The preview shows the last successfully written hardware colors, never optimistic requested colors.
- Speaker images preserve aspect ratio; the left is mirrored without a power dot and the right retains it.
- Rear light is rendered beneath the alpha cutout; front light uses neutral emission multiplied by selected RGB.
- No captured content or color history is persisted, logged, or placed in diagnostic UI state.
- Visible branding comes from `product-metadata.json`; D-Bus and desktop identifiers remain stable across a rename.
- No commit step may run unless the repository owner explicitly authorizes commits.

---

## File structure

- `product-metadata.json`: single user-facing product identity and About metadata.
- `gui/index.html`: accessible application landmarks and page hosts only.
- `gui/styles/tokens.css`: colors, typography, spacing, focus, and reduced-motion tokens.
- `gui/styles/shell.css`: top bar, rail, pages, banners, and responsive shell.
- `gui/styles/lighting.css`: zone controls, speaker stage, masks, glow, and mixed/pending states.
- `gui/js/api.js`: typed command/event facade around Tauri invoke/listen.
- `gui/js/state.js`: pure frontend reducer and selectors.
- `gui/js/render.js`: DOM rendering and event binding.
- `gui/js/main.js`: bootstrap and snapshot subscription.
- `gui/js/*.test.mjs`: Node state and rendering-model tests.
- `gui/assets/speakers/`: accepted left/right neutral speakers and front masks.
- `src-tauri/src/service_client.rs`: zbus proxy and snapshot stream.
- `src-tauri/src/commands.rs`: validated Tauri command surface.
- `src-tauri/src/main.rs`: window lifecycle and service-client state.
- `src-tauri/tests/commands_fake.rs`: fake-client command tests.

### Task 1: Tauri workspace member and locked-down shell

**Files:**
- Modify: `Cargo.toml`
- Create: `product-metadata.json`
- Create: `src-tauri/Cargo.toml`
- Create: `src-tauri/build.rs`
- Create: `src-tauri/tauri.conf.json`
- Create: `src-tauri/capabilities/default.json`
- Create: `src-tauri/src/main.rs`
- Create: `gui/index.html`
- Create: `gui/js/main.js`
- Test: `src-tauri/src/main.rs`

**Interfaces:**
- Consumes: `logig560-api` from the service-foundation plan.
- Produces: the `logig560-gui` binary, `ProductMetadata`, and one application window.

- [ ] **Step 1: Add a failing metadata test**

```rust
#[test]
fn product_metadata_has_required_user_facing_fields() {
    let metadata = ProductMetadata::load_embedded().unwrap();
    assert!(!metadata.display_name.trim().is_empty());
    assert_eq!(metadata.supported_usb_id, "046d:0a78");
    assert!(!metadata.license.trim().is_empty());
}
```

- [ ] **Step 2: Verify the GUI crate is absent**

Run: `cargo test -p logig560-gui`

Expected: FAIL because the package does not exist.

- [ ] **Step 3: Create the Tauri crate**

Add `src-tauri` to the workspace. Pin `tauri = "2.11.5"`, `tauri-build = "2.6.3"`, `serde_json = "1.0.151"`, `zbus = "5.18.0"` with Tokio, and a path dependency on `logig560-api`.

Set `frontendDist` to `../gui`, use stable identifier `org.logig560.Gui`, disable bundling for this source-built increment, and set a minimum window of 980 by 680 with default 1380 by 860. Configure a strict CSP permitting only bundled scripts, styles, fonts, images, `asset:` images, and Tauri IPC. Add no shell, filesystem, USB, HTTP, or process plugin.

`product-metadata.json` contains `display_name`, `subtitle`, `supported_usb_id`, `project_url`, and `license`. Rust embeds it with `include_str!`; the frontend fetches the same bundled file.

- [ ] **Step 4: Make the metadata test pass and build the empty shell**

Run: `cargo test -p logig560-gui && cargo build -p logig560-gui`

Expected: test and build pass; no frontend network requests or privileged capabilities exist.

- [ ] **Step 5: Commit only with explicit owner authorization**

```bash
git add Cargo.toml Cargo.lock product-metadata.json src-tauri gui/index.html gui/js/main.js
git commit -m "feat: scaffold locked-down Tauri lighting app"
```

### Task 2: Service client and narrow Tauri commands

**Files:**
- Create: `src-tauri/src/service_client.rs`
- Create: `src-tauri/src/commands.rs`
- Modify: `src-tauri/src/main.rs`
- Create: `src-tauri/tests/commands_fake.rs`

**Interfaces:**
- Produces: async trait `ServiceClient`, `DbusServiceClient`, `GuiState`, and Tauri commands `get_snapshot`, `set_manual_zones`, `set_mode`, `set_lights_enabled`, `choose_desktop_display`, and `restart_capture`.
- Emits: Tauri event `service-snapshot` containing one complete `ServiceSnapshot`.

- [ ] **Step 1: Write fake-client command tests**

`grouped_manual_update_is_forwarded_as_one_call` sends left-front and right-front updates and compares the fake client's calls with one two-element vector. `invalid_brightness_never_reaches_service_client` submits brightness 101, asserts `GuiError::InvalidBrightness(101)`, and asserts an empty fake call log. `snapshot_event_contains_complete_latest_snapshot` queues three snapshots before the forwarding task runs and compares the emitted event with the third complete snapshot.

- [ ] **Step 2: Verify failure**

Run: `cargo test -p logig560-gui --test commands_fake`

Expected: FAIL because the client and commands are missing.

- [ ] **Step 3: Define and implement `ServiceClient`**

```rust
#[async_trait::async_trait]
pub trait ServiceClient: Send + Sync {
    async fn get_snapshot(&self) -> Result<ServiceSnapshot, GuiError>;
    async fn set_manual_zones(&self, updates: Vec<ManualZoneUpdate>) -> Result<(), GuiError>;
    async fn set_mode(&self, mode: LightingMode) -> Result<(), GuiError>;
    async fn set_lights_enabled(&self, enabled: bool) -> Result<(), GuiError>;
    async fn choose_desktop_display(&self) -> Result<(), GuiError>;
    async fn restart_capture(&self) -> Result<(), GuiError>;
    async fn next_snapshot(&self) -> Result<ServiceSnapshot, GuiError>;
}
```

`DbusServiceClient` connects only to the approved user-session bus name/path. `GuiState` holds `Arc<dyn ServiceClient>`. Commands validate API version, zone uniqueness, and brightness before calling the client. Errors are short stable user-safe messages; Rust error chains go to color-free tracing only.

- [ ] **Step 4: Implement newest snapshot forwarding**

Start one task after Tauri setup. It drains any immediately ready service notifications before emitting `service-snapshot`, so a burst produces one newest event. On disconnect it emits a synthetic complete unavailable snapshot rather than partial fields.

- [ ] **Step 5: Run command tests and compile**

Run: `cargo test -p logig560-gui --test commands_fake && cargo check -p logig560-gui`

Expected: all tests pass.

- [ ] **Step 6: Commit only with explicit owner authorization**

```bash
git add src-tauri/src src-tauri/tests
git commit -m "feat: bridge Tauri to the lighting service"
```

### Task 3: Pure frontend state and zone selection

**Files:**
- Create: `gui/js/state.js`
- Create: `gui/js/state.test.mjs`
- Create: `gui/js/api.js`
- Modify: `gui/js/main.js`

**Interfaces:**
- Produces: `createState`, `reduceSnapshot`, `toggleZone`, `selectGroup`, `selectedUpdates`, `selectionSummary`, and group definitions `all`, `fronts`, `rears`, `left`, `right`.

- [ ] **Step 1: Write state tests using `node:test`**

Test independent colors, each exact quick-group membership, mixed RGB and brightness values, all-four default selection, pending versus confirmed colors, master-off lockout, and Content-Aware lockout.

```javascript
test('fronts selects only the two front zones', () => {
  const state = selectGroup(createState(snapshot), 'fronts');
  assert.deepEqual([...state.selected], ['left-front', 'right-front']);
});
```

- [ ] **Step 2: Verify failure**

Run: `node --test gui/js/state.test.mjs`

Expected: FAIL because `state.js` does not exist.

- [ ] **Step 3: Implement immutable state transitions**

Store selected zones in logical display order and service state separately from local selection. `reduceSnapshot` replaces the complete service snapshot. It never copies confirmed colors into persisted manual values. `selectedUpdates` creates one atomic array for the Rust command.

- [ ] **Step 4: Implement the API facade**

`api.js` is the only frontend module importing Tauri `invoke` and `listen`. Export semantic functions matching the six approved commands plus `onSnapshot`. Do not expose a generic `invoke(command, args)` to views.

- [ ] **Step 5: Run frontend state tests**

Run: `node --test gui/js/state.test.mjs`

Expected: all tests pass.

- [ ] **Step 6: Commit only with explicit owner authorization**

```bash
git add gui/js
git commit -m "feat: model lighting UI state and zone groups"
```

### Task 4: Application shell and responsive navigation

**Files:**
- Modify: `gui/index.html`
- Create: `gui/styles/tokens.css`
- Create: `gui/styles/shell.css`
- Create: `gui/js/render.js`
- Create: `gui/js/render.test.mjs`
- Modify: `gui/js/main.js`

**Interfaces:**
- Consumes: pure state selectors and API facade.
- Produces: `renderShell`, `renderStatusBar`, `renderNavigation`, `renderBanner`, and four persistent page hosts.

- [ ] **Step 1: Write rendering-model tests**

Test that each model includes visible text plus an icon for service/device status, exactly one active rail destination, a master-power accessible name/state, and a recovery action for unavailable service. Keep tests DOM-independent by testing returned view models and escaped text.

- [ ] **Step 2: Verify failure**

Run: `node --test gui/js/render.test.mjs`

Expected: FAIL because rendering helpers are missing.

- [ ] **Step 3: Implement semantic shell markup**

Use `<header>`, `<nav aria-label="Primary">`, `<main>`, real `<button>` elements, and one `<section>` per destination. The rail contains Lighting, Setup & Service, Diagnostics, and About. The top bar contains service state, G560 state, and the master switch. Avoid inline script and inline style so CSP remains strict.

- [ ] **Step 4: Implement G Hub-inspired styling**

Define tokens for black surfaces, cyan accent, white/gray text, amber/red/green state, borders, spacing, and focus. At widths below 1100 px reduce stage/control gaps and make the main content scroll; never scale image width and height independently. Honor `prefers-reduced-motion`.

- [ ] **Step 5: Run render tests and static checks**

Run: `node --test gui/js/*.test.mjs && rg -n 'style=|onclick=|<script(?! type="module" src=)' gui --pcre2`

Expected: tests pass and `rg` returns no inline-style/handler violations.

- [ ] **Step 6: Commit only with explicit owner authorization**

```bash
git add gui
git commit -m "feat: add responsive G560 application shell"
```

### Task 5: Speaker assets and confirmed-hardware preview

**Files:**
- Create: `gui/assets/speakers/g560-left-neutral.png`
- Create: `gui/assets/speakers/g560-right-neutral.png`
- Create: `gui/assets/speakers/g560-left-front-mask.png`
- Create: `gui/assets/speakers/g560-right-front-mask.png`
- Create: `gui/styles/lighting.css`
- Modify: `gui/index.html`
- Modify: `gui/js/render.js`
- Create: `scripts/verify-gui-assets.sh`

**Interfaces:**
- Consumes: `snapshot.confirmed_colors` only for the preview.
- Produces: `renderSpeakerPreview` and CSS custom properties `--left-front`, `--left-rear`, `--right-front`, `--right-rear`.

- [ ] **Step 1: Write a failing deterministic asset check**

The script must assert each speaker is exactly 701x1122, contains alpha, left/right aspect ratios match, mask dimensions match the speaker, left power-indicator sample is dark, right sample is bright, and neutral light samples have equal R/G/B channels.

- [ ] **Step 2: Verify the asset check fails**

Run: `bash scripts/verify-gui-assets.sh`

Expected: FAIL because destination assets do not exist.

- [ ] **Step 3: Promote the accepted companion assets**

Copy, without resampling, from:

```text
.superpowers/brainstorm/33070-1784566932/content/g560-left-front-neutral-soft-apex-v2.png
.superpowers/brainstorm/33070-1784566932/content/g560-right-front-neutral-soft-apex-v2.png
.superpowers/brainstorm/33070-1784566932/content/g560-left-front-light-coverage.png
.superpowers/brainstorm/33070-1784566932/content/g560-right-front-light-coverage.png
```

Rename them to the four destination paths above. Record SHA-256 values in the verification script so later accidental generative replacement or resampling fails loudly.

- [ ] **Step 4: Build the layered preview**

Use two independent speaker containers with `object-fit: contain`. Place rear glow at z-index 1, transparent speaker PNG at 2, and masked front tint at 3. Use normal blend for the speaker, multiply for front tint, and a blurred elliptical rear glow that remains behind the cutout. Set CSS colors only from confirmed service colors; pending requested colors appear in controls, not the stage.

- [ ] **Step 5: Run asset and state tests**

Run: `bash scripts/verify-gui-assets.sh && node --test gui/js/*.test.mjs`

Expected: all checks pass.

- [ ] **Step 6: Commit only with explicit owner authorization**

```bash
git add gui/assets gui/styles/lighting.css gui/index.html gui/js/render.js scripts/verify-gui-assets.sh
git commit -m "feat: render confirmed four-zone G560 preview"
```

### Task 6: Manual lighting controls

**Files:**
- Modify: `gui/index.html`
- Modify: `gui/js/state.js`
- Modify: `gui/js/render.js`
- Modify: `gui/js/main.js`
- Modify: `gui/styles/lighting.css`
- Modify: `gui/js/state.test.mjs`
- Modify: `gui/js/render.test.mjs`

**Interfaces:**
- Consumes: `set_manual_zones(Vec<ManualZoneUpdate>)` and complete snapshots.
- Produces: zone cards, quick groups, color input, RGB/hex input, presets, brightness, mixed state, and pending/failure presentation.

- [ ] **Step 1: Add failing interaction tests**

Test that color, RGB/hex, preset, and brightness actions produce one atomic update for the selected zones; invalid hex produces no command; selection alone produces no command; mixed selected values display `Mixed`; and rejected writes preserve pending controls while confirmed preview stays unchanged.

- [ ] **Step 2: Verify failure**

Run: `node --test gui/js/state.test.mjs gui/js/render.test.mjs`

Expected: new tests fail.

- [ ] **Step 3: Implement accessible controls**

Use real zone and group buttons with `aria-pressed`, `<input type="color">`, validated six-digit hex/RGB inputs, six preset buttons, and a brightness range `0..100`. Disable the controls when master power is off or mode is Content-Aware. Show `N zones selected`, `Mixed`, pending, and write-failure states in text.

- [ ] **Step 4: Send commands and reconcile snapshots**

Dispatch one API call per user action, not per zone. Do not debounce clicks into an accumulating queue; while a command is pending, later changes replace local pending state and the service's latest-only behavior decides the newest target. Reconcile only from complete snapshots.

- [ ] **Step 5: Run frontend and Rust bridge tests**

Run: `node --test gui/js/*.test.mjs && cargo test -p logig560-gui --test commands_fake`

Expected: all tests pass.

- [ ] **Step 6: Commit only with explicit owner authorization**

```bash
git add gui
git commit -m "feat: control manual colors and brightness by zone"
```

### Task 7: Global Content-Aware page state

**Files:**
- Modify: `gui/index.html`
- Modify: `gui/js/state.js`
- Modify: `gui/js/render.js`
- Modify: `gui/js/main.js`
- Modify: `gui/styles/lighting.css`
- Modify: `gui/js/state.test.mjs`
- Modify: `gui/js/render.test.mjs`

**Interfaces:**
- Consumes: `set_mode`, `choose_desktop_display`, `restart_capture`, and capture fields from `ServiceSnapshot`.
- Produces: global mode toggle, backend/capture status, choose-display action, and recovery action.

- [ ] **Step 1: Add failing global-mode tests**

Test that enabling Content-Aware sends exactly one global mode command; no zone assignment UI is rendered; every manual control is disabled; Desktop Portal shows Choose Display; Gamescope never shows or invokes portal selection; and disabling restores saved manual settings from the next snapshot.

- [ ] **Step 2: Verify failure**

Run: `node --test gui/js/state.test.mjs gui/js/render.test.mjs`

Expected: new mode tests fail.

- [ ] **Step 3: Implement Content-Aware presentation**

Display backend, authorization, active/stalled/recovering state, and concise errors. Keep the speaker stage visible and driven by confirmed colors. A safety-black snapshot therefore displays black immediately. Do not show captured frames, monitor thumbnails, sampled values, or history.

- [ ] **Step 4: Run complete GUI tests**

Run: `node --test gui/js/*.test.mjs && cargo test -p logig560-gui --all-targets`

Expected: all tests pass.

- [ ] **Step 5: Commit only with explicit owner authorization**

```bash
git add gui
git commit -m "feat: add global content-aware lighting controls"
```

### Task 8: Lighting-app verification and current documentation

**Files:**
- Modify: `README.md`
- Modify: `docs/build-and-run.md`
- Modify: `docs/dependencies.md`
- Modify: `docs/operations.md`
- Modify: `docs/project-status.md`
- Modify: `docs/testing.md`
- Modify: `HANDOFF_BAZZITE.md`

- [ ] **Step 1: Document Tauri dependencies and launch commands**

Add exact Fedora, Bazzite container, and Arch WebKitGTK/Tauri build dependencies verified from current official Tauri guidance. Document service-first launch, `cargo run --manifest-path src-tauri/Cargo.toml`, and the GUI/service separation.

- [ ] **Step 2: Run automated validation**

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

Expected: all commands pass in the appropriate host/container environment.

- [ ] **Step 3: Run fake-service desktop acceptance**

Launch the GUI against the fake D-Bus service and verify all zone/group changes, mixed state, master off/on, Manual/Content-Aware transition, disconnected state, capture-stall state, narrow window, keyboard navigation, and reduced motion. Confirm closing the GUI leaves the fake service alive.

- [ ] **Step 4: Run real-hardware acceptance**

First inspect `systemctl --user` and `pgrep -af logig560`. With one service owner, verify all four zones and groups, confirmed preview behavior, global Content-Aware behavior, manual restoration, GUI close, and immediate master-off blackout. The user makes the final visual-accuracy call.

- [ ] **Step 5: Commit only with explicit owner authorization**

```bash
git add README.md docs HANDOFF_BAZZITE.md
git commit -m "docs: add Tauri lighting app operation and acceptance"
```

## Plan acceptance

This increment is complete only when the GUI runs against both fake and real services, manual and Content-Aware interactions satisfy the approved behavior, the visual assets pass deterministic checks, closing the GUI leaves lighting active, and the user confirms the physical preview and colors.
