# G560 Linux Utility Milestone 1 Engine Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build and measure the production Rust engine that captures exactly one Wayland monitor, selects four immediate weighted-dominant edge colors, and drives the Logitech G560's four lighting zones without disrupting audio.

**Architecture:** One Rust binary composes independently testable capture, sampling, scheduling, and USB modules. The XDG ScreenCast portal authorizes one source, GStreamer consumes its PipeWire stream into a latest-frame-only appsink, and a persistent libusb handle sends four-zone updates.

**Tech Stack:** Rust 1.97.1 (edition 2024), Tokio, ashpd 0.13.12, GStreamer Rust 0.25, rusb 0.9.4, palette 0.7, clap 4.5, serde/TOML, tracing, proptest.

## Global Constraints

- Supported milestone-1 host: Fedora 44, GNOME Wayland, Logitech USB `046d:0a78`.
- Capture requests `SourceType::Monitor` with `multiple(false)`; never request the combined desktop, a window, or multiple sources.
- Runtime processes are unprivileged; administrator authorization is limited to installing the `046d:0a78` udev rule.
- Default response has no temporal smoothing and zero minimum brightness.
- Frames and color history are never persisted or transmitted.
- Stale frames and stale color updates are replaced, never queued.
- USB audio and speaker controls must continue working throughout every hardware test.
- SDR correctness is required; HDR accuracy is explicitly not claimed in milestone 1.

---

## File map

- `Cargo.toml`: pinned crate dependencies and binary/library targets.
- `rust-toolchain.toml`: pinned Rust toolchain and quality components.
- `src/lib.rs`: public module surface.
- `src/color.rs`: RGB and four-zone value types.
- `src/frame.rs`: owned packed-RGB frame and normalized region geometry.
- `src/sampler.rs`: weighted perceptual dominant-color selection.
- `src/latest.rs`: capacity-one newest-value exchange.
- `src/usb/protocol.rs`: pure G560 feature-report encoder.
- `src/usb/device.rs`: persistent libusb interface ownership and writes.
- `src/capture/portal.rs`: one-monitor portal session and restore token.
- `src/capture/gstreamer.rs`: PipeWire/GStreamer appsink and packed RGB extraction.
- `src/engine.rs`: capture-to-sampler-to-USB coordination and statistics.
- `src/main.rs`: diagnostic commands and live engine entry point.
- `tests/engine_fake.rs`: end-to-end engine test with fake frame and light transports.
- `contrib/70-g560.rules`: narrowly scoped device-access rule.
- `scripts/install-udev-rule.sh`: Polkit-backed, explicit udev installation helper.
- `docs/hardware/g560-zone-map.md`: physical zone verification record.
- `docs/hardware/milestone-1-results.md`: repeatable latency, throughput, resource, and recovery results.

### Task 1: Reproducible Rust crate and core value types

**Files:**
- Create: `Cargo.toml`
- Create: `rust-toolchain.toml`
- Create: `.gitignore`
- Create: `src/lib.rs`
- Create: `src/color.rs`
- Create: `src/frame.rs`

**Interfaces:**
- Produces: `Rgb8`, `Zone`, `ZoneColors`, `RgbFrame`, `Region`, and `default_regions()` used by every later task.

- [ ] **Step 1: Install build prerequisites and create the failing geometry tests**

On Fedora Workstation, install Rust in the user account and the native GStreamer/libusb headers:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain 1.97.1
source "$HOME/.cargo/env"
pkexec dnf install -y gcc pkgconf-pkg-config gstreamer1-devel gstreamer1-plugins-base-devel gstreamer1-plugins-bad-free-devel libusb1-devel
```

Create the manifests and tests. `Cargo.toml` must use:

```toml
[package]
name = "logig560"
version = "0.1.0"
edition = "2024"
rust-version = "1.97.1"

[dependencies]
anyhow = "1.0"
ashpd = { version = "0.13.12", default-features = false, features = ["tokio", "screencast"] }
clap = { version = "4.5", features = ["derive"] }
futures-util = "0.3"
gstreamer = "0.25"
gstreamer-app = "0.25"
gstreamer-video = "0.25"
palette = "0.7"
rusb = "0.9.4"
serde = { version = "1.0", features = ["derive"] }
thiserror = "2.0"
tokio = { version = "1.51", features = ["macros", "rt-multi-thread", "signal", "sync", "time"] }
toml = "0.9"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }

[dev-dependencies]
proptest = "1.9"
tempfile = "3.27"
```

`rust-toolchain.toml`:

```toml
[toolchain]
channel = "1.97.1"
components = ["clippy", "rustfmt"]
profile = "minimal"
```

Put tests in `src/frame.rs` that assert a packed RGB frame rejects the wrong byte length, normalized regions clamp to frame boundaries, and defaults select non-overlapping upper-left, lower-left, lower-right, and upper-right edge areas.

- [ ] **Step 2: Run the tests and verify the crate is incomplete**

Run: `cargo test frame::tests -- --nocapture`

Expected: compilation fails because `RgbFrame`, `Region`, and `default_regions` do not exist.

- [ ] **Step 3: Implement the core types**

Implement these exact public shapes:

```rust
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Rgb8 { pub r: u8, pub g: u8, pub b: u8 }

impl Rgb8 {
    pub const BLACK: Self = Self { r: 0, g: 0, b: 0 };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Zone { LeftRear, LeftFront, RightFront, RightRear }

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ZoneColors(pub [Rgb8; 4]);

impl ZoneColors {
    pub const BLACK: Self = Self([Rgb8 { r: 0, g: 0, b: 0 }; 4]);
    pub fn get(self, zone: Zone) -> Rgb8 { self.0[zone as usize] }
}
```

`RgbFrame::new(width, height, stride, pixels)` must require `width > 0`, `height > 0`, `stride >= width * 3`, and `pixels.len() == stride * height`. `Region { x, y, width, height }` uses normalized `f32` values in `0.0..=1.0`; `pixel_bounds` clamps and returns nonempty integer bounds. `default_regions()` returns regions in `Zone` discriminant order.

- [ ] **Step 4: Verify and commit**

Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`

Expected: all checks pass.

```bash
git add Cargo.toml Cargo.lock rust-toolchain.toml .gitignore src
git commit -m "feat: establish engine domain types"
```

### Task 2: Weighted perceptual dominant-color sampler

**Files:**
- Create: `src/sampler.rs`
- Modify: `src/lib.rs`

**Interfaces:**
- Consumes: `Rgb8`, `ZoneColors`, `RgbFrame`, `Region`, `default_regions()`.
- Produces: `SamplerConfig { darkness_luma, chroma_bin, lightness_bin }` and `sample_zones(&RgbFrame, &[Region; 4], SamplerConfig) -> ZoneColors`.

- [ ] **Step 1: Write failing behavior tests**

Add tests using small generated frames for these exact outcomes:

```rust
#[test]
fn black_region_turns_fully_off() {
    assert_eq!(sample_region(&solid(8, 8, Rgb8::BLACK), FULL, cfg()), Rgb8::BLACK);
}

#[test]
fn dominant_red_ignores_one_white_pixel() {
    let mut frame = solid(10, 10, Rgb8 { r: 220, g: 10, b: 10 });
    frame.set_test_pixel(0, 0, Rgb8 { r: 255, g: 255, b: 255 });
    let got = sample_region(&frame, FULL, cfg());
    assert!(got.r > 180 && got.g < 50 && got.b < 50);
}

#[test]
fn four_regions_are_independent() {
    let frame = quadrant_frame();
    assert_eq!(sample_zones(&frame, &quadrants(), cfg()), ZoneColors([
        RED, GREEN, BLUE, YELLOW,
    ]));
}
```

Also use `proptest` to prove the sampler never panics for valid dimensions, strides, pixels, and normalized regions.

- [ ] **Step 2: Verify failure**

Run: `cargo test sampler::tests -- --nocapture`

Expected: compilation fails because `sample_region` and `sample_zones` are missing.

- [ ] **Step 3: Implement immediate weighted clustering**

For each sampled pixel, convert sRGB to CIE Lab with `palette`. Skip pixels whose relative luminance is below `darkness_luma`. Quantize `L`, `a`, and `b` by the configured bin widths. Accumulate per-bin `weight`, linear RGB sums, and count in a `HashMap<(i16,i16,i16), Accumulator>`. Use weight `0.25 + chroma / 128.0`, capped at `2.0`, so saturated scene colors beat gray without allowing a tiny highlight to win. Select the bin with greatest total weight, then return its weighted linear-RGB mean converted to `Rgb8`. Return black if fewer than 2% of region pixels survive the darkness filter.

Use defaults `darkness_luma = 0.015`, `chroma_bin = 12.0`, and `lightness_bin = 10.0`. Do not reference previous frames or add smoothing.

- [ ] **Step 4: Verify and commit**

Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`

Expected: all tests pass, including property tests.

```bash
git add src/lib.rs src/sampler.rs
git commit -m "feat: sample weighted dominant edge colors"
```

### Task 3: Latest-value scheduling and fake end-to-end engine

**Files:**
- Create: `src/latest.rs`
- Create: `src/engine.rs`
- Modify: `src/lib.rs`
- Create: `tests/engine_fake.rs`

**Interfaces:**
- Produces: `latest_channel<T>() -> (LatestSender<T>, LatestReceiver<T>)`, async `LatestReceiver::recv`, `FrameSource`, `LightSink`, `EngineStats`, and `run_engine`.

- [ ] **Step 1: Write failing replacement and engine tests**

Test that sending values `1`, `2`, `3` before receiving yields only `3`. In `tests/engine_fake.rs`, define a `FakeSource` emitting three quadrant frames and a `FakeSink` recording writes. Block the sink during the first write, emit two more frames, then unblock it and assert the next write matches only the newest frame. Assert `EngineStats.dropped_frames == 1` and that shutdown writes `ZoneColors::BLACK`.

- [ ] **Step 2: Verify failure**

Run: `cargo test --test engine_fake -- --nocapture`

Expected: compilation fails because engine traits and latest channel are undefined.

- [ ] **Step 3: Implement capacity-one coordination**

Define:

```rust
#[async_trait::async_trait]
pub trait FrameSource: Send {
    async fn next_frame(&mut self) -> anyhow::Result<Option<RgbFrame>>;
}

#[async_trait::async_trait]
pub trait LightSink: Send {
    async fn write(&mut self, colors: ZoneColors) -> anyhow::Result<()>;
}

pub struct EngineStats {
    pub captured_frames: u64,
    pub rendered_updates: u64,
    pub dropped_frames: u64,
    pub capture_to_write: hdrhistogram::Histogram<u64>,
}
```

Add `async-trait = "0.1"` and `hdrhistogram = "7.5"` to dependencies. Implement the latest channel with `tokio::sync::watch`; each send replaces the unread value. `run_engine` timestamps frames at receipt, samples them on `tokio::task::spawn_blocking`, and sends only the newest result to a single sink writer. On cancellation, it writes black once and returns final statistics.

- [ ] **Step 4: Verify and commit**

Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`

Expected: all tests pass; the fake integration test records first, newest, and black—not the stale middle update.

```bash
git add Cargo.toml Cargo.lock src tests/engine_fake.rs
git commit -m "feat: add newest-frame engine coordinator"
```

### Task 4: G560 packet encoder and persistent USB driver

**Files:**
- Create: `src/usb/mod.rs`
- Create: `src/usb/protocol.rs`
- Create: `src/usb/device.rs`
- Modify: `src/lib.rs`
- Modify: `src/main.rs`

**Interfaces:**
- Produces: `encode_solid(zone: Zone, color: Rgb8) -> [u8; 20]`, `UsbTransport`, `G560<T>`, and diagnostic `set-zones` command.

- [ ] **Step 1: Write failing protocol tests**

For `LeftRear` red, assert the exact known report:

```rust
assert_eq!(encode_solid(Zone::LeftRear, Rgb8 { r: 255, g: 0, b: 0 }), [
    0x11, 0xff, 0x04, 0x3a, 0x00, 0x01, 0xff, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
]);
```

Assert protocol indexes `[0x00, 0x02, 0x03, 0x01]` for `[LeftRear, LeftFront, RightFront, RightRear]`; this is a provisional logical map to be corrected from physical testing. With a fake `UsbTransport`, assert `G560::write` emits four reports in zone order and skips a write when colors equal the previous successful set.

- [ ] **Step 2: Verify failure**

Run: `cargo test usb:: -- --nocapture`

Expected: compilation fails because USB modules are absent.

- [ ] **Step 3: Implement protocol and persistent transport**

`LibUsbTransport::open` uses `rusb::open_device_with_vid_pid(0x046d, 0x0a78)`, claims interface `2` once, and retains the handle. Do not enable libusb auto-detach globally. If interface 2 has an active kernel driver, return a typed `InterfaceBusy` error; this prevents silently detaching a driver until the live hardware test proves interface ownership is safe. Each report uses:

```rust
handle.write_control(0x21, 0x09, 0x0211, 0x0002, &report, Duration::from_millis(100))
```

Require exactly 20 bytes written. `G560::blackout` sends four black reports. Implement `logig560 set-zones --left-rear RRGGBB --left-front RRGGBB --right-front RRGGBB --right-rear RRGGBB`; reject malformed colors without opening USB.

- [ ] **Step 4: Verify without hardware and commit**

Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo run -- set-zones --left-rear not-a-color`

Expected: tests pass; CLI exits nonzero with an invalid-color message before USB access.

```bash
git add src
git commit -m "feat: add persistent G560 USB transport"
```

### Task 5: Safe device permission installation and physical zone verification

**Files:**
- Create: `contrib/70-g560.rules`
- Create: `scripts/install-udev-rule.sh`
- Create: `docs/hardware/g560-zone-map.md`
- Modify: `src/usb/device.rs`
- Modify: `src/usb/protocol.rs`

**Interfaces:**
- Produces: unprivileged access and verified protocol-index-to-physical-zone mapping.

- [ ] **Step 1: Add and statically test the udev artifacts**

The rule contains exactly:

```udev
ACTION=="add", SUBSYSTEM=="usb", ATTR{idVendor}=="046d", ATTR{idProduct}=="0a78", TAG+="uaccess"
```

The installer must resolve its own directory, verify the source filename, then call `pkexec install -o root -g root -m 0644` for `/etc/udev/rules.d/70-g560.rules`, followed by `pkexec udevadm control --reload-rules` and `pkexec udevadm trigger --subsystem-match=usb --attr-match=idVendor=046d --attr-match=idProduct=0a78`. Test with `bash -n scripts/install-udev-rule.sh` and `udevadm test` against the current device sysfs path.

- [ ] **Step 2: Install the rule and verify user access**

Run:

```bash
./scripts/install-udev-rule.sh
udevadm info --attribute-walk --name=/dev/bus/usb/001/009
getfacl /dev/bus/usb/001/009
```

Expected: the active user has read/write ACL access. Resolve the current bus/device path with `lsusb -d 046d:0a78`; do not hard-code `001/009` if it changed.

- [ ] **Step 3: Run one-zone-at-a-time hardware mapping**

For each protocol index, send black to all zones and then white to exactly one zone for three seconds. Record the observed speaker side and front/rear light in `docs/hardware/g560-zone-map.md`, along with G560 firmware information if available. Keep audio playing quietly throughout and stop immediately if audio drops.

If claiming interface 2 reports `Busy`, inspect `lsusb -t`, `usb-devices`, and `/sys/bus/usb/devices/*/driver` before changing driver behavior. Only detach a kernel driver if evidence shows interface 2 is HID lighting and not an audio interface; detach once on open and reattach on drop.

- [ ] **Step 4: Correct the logical map and verify audio**

Update the zone-index constants from the recorded observations. Run four alternating color sequences for five minutes while playing audio. Expected: every logical screen corner lights the intended physical zone, with uninterrupted audio and no USB errors.

- [ ] **Step 5: Commit**

```bash
git add contrib scripts src/usb docs/hardware/g560-zone-map.md
git commit -m "test: verify G560 physical zone mapping"
```

### Task 6: One-monitor portal capture and latest-frame GStreamer adapter

**Files:**
- Create: `src/capture/mod.rs`
- Create: `src/capture/portal.rs`
- Create: `src/capture/gstreamer.rs`
- Modify: `src/lib.rs`
- Modify: `src/main.rs`

**Interfaces:**
- Produces: `PortalCapture::open(Option<String>)`, `PortalGrant { stream, remote_fd, restore_token }`, and a `FrameSource` implementation yielding packed RGB frames.

- [ ] **Step 1: Write failing option and buffer tests**

Extract a pure `selection_options(restore_token)` helper and assert it requests only `SourceType::Monitor`, `multiple(false)`, hidden cursor, and `PersistMode::ExplicitlyRevoked`. Test `frame_from_sample` with a padded 2x2 RGB buffer and assert width, height, stride, and pixels are preserved. Test that unsupported caps return a typed error.

- [ ] **Step 2: Verify failure**

Run: `cargo test capture:: -- --nocapture`

Expected: compilation fails because capture modules do not exist.

- [ ] **Step 3: Implement the portal grant**

Follow ashpd's official screencast flow: `Screencast::new`, `create_session`, `select_sources`, `start`, require exactly one returned stream, and `open_pipe_wire_remote`. Pass the prior restore token when present and return the new token from the start response. Treat cancellation as a user-facing `CaptureCancelled` state, not a crash.

- [ ] **Step 4: Implement the GStreamer source**

Initialize GStreamer once. Build this logical pipeline with typed element construction:

```text
pipewiresrc fd=<portal fd> path=<node id> do-timestamp=true
  ! queue leaky=downstream max-size-buffers=1
  ! videoconvert
  ! videoscale
  ! video/x-raw,format=RGB,width=160,pixel-aspect-ratio=1/1
  ! appsink max-buffers=1 drop=true sync=false
```

Keep the portal session and owned PipeWire FD alive for the pipeline lifetime. Pull samples with `AppSink::pull_sample`, read `VideoInfo` for width/height/stride, map the buffer read-only, copy one packed frame, and immediately release the sample. Translate EOS and bus errors into recoverable `FrameSource` errors.

- [ ] **Step 5: Verify interactively and commit**

Run: `cargo run -- capture-test --frames 300`

Expected: GNOME presents a monitor-only chooser that permits selecting one display; the command reports dimensions, effective frame rate, and zero saved images, then exits cleanly after 300 frames. Confirm the chooser cannot select multiple monitors.

```bash
git add src
git commit -m "feat: capture one portal-authorized monitor"
```

### Task 7: Live engine, measurements, and milestone acceptance

**Files:**
- Modify: `src/main.rs`
- Modify: `src/engine.rs`
- Create: `docs/hardware/milestone-1-results.md`
- Create: `README.md`

**Interfaces:**
- Produces: `logig560 run`, bounded reconnect behavior, printed performance statistics, and recorded acceptance results.

- [ ] **Step 1: Write failing CLI and recovery tests**

Use fake transports to assert these sequences: device missing → bounded retry → connected; three consecutive USB failures → blackout attempt → reopen; capture EOS → blackout → clean exit; Ctrl-C → blackout → clean exit. Use paused Tokio time to assert retry delays of 250 ms, 500 ms, 1 s, 2 s, capped at 5 s.

- [ ] **Step 2: Verify failure**

Run: `cargo test engine::tests -- --nocapture`

Expected: new recovery tests fail because retry state is absent.

- [ ] **Step 3: Implement the live command**

`logig560 run` opens one portal capture, writes the newly returned restore token to `~/.config/logig560/capture.toml` using mode `0600` and atomic rename, opens the G560, and calls `run_engine`. Handle Ctrl-C with a cancellation token. Every five seconds print captured FPS, rendered updates per second, dropped-frame count, and capture-to-write p50/p95/p99. On exit, print totals and attempt blackout before releasing USB.

Use `directories = "6"` and `tokio-util = { version = "0.7", features = ["rt"] }`; add them to `Cargo.toml`. Logs include timings and states only, never pixel or color-history data.

- [ ] **Step 4: Run full automated verification**

Run:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
```

Expected: all commands pass.

- [ ] **Step 5: Execute the hardware acceptance matrix**

Run `cargo run --release -- run` and record in `docs/hardware/milestone-1-results.md`:

1. Sustained update rate and USB error count over 30 minutes of rapidly changing four-quadrant video.
2. Capture-to-write p50, p95, and p99 latency reported by the engine.
3. CPU and resident memory from `pidstat -p <pid> 1 60` and `ps -o rss= -p <pid>`.
4. Unplug/replug recovery time.
5. Behavior on Ctrl-C, capture cancellation, display sleep, and service process termination.
6. Audio continuity during all tests.
7. A high-contrast video showing that each screen quadrant drives its mapped zone and black turns fully off.

Any failure blocks milestone acceptance and is fixed with a new failing regression test before repeating the affected measurement.

- [ ] **Step 6: Document operation and commit**

`README.md` must describe prerequisites, udev installation, `capture-test`, `set-zones`, `run`, privacy behavior, known SDR-only color guarantee, and recovery troubleshooting using the exact commands introduced above.

```bash
git add Cargo.toml Cargo.lock src README.md docs/hardware/milestone-1-results.md
git commit -m "feat: complete milestone one screen matcher"
```

---

## Milestone gate

Do not begin the GTK/D-Bus/service/packaging plan until all Task 7 acceptance results are populated with measured values and no unresolved failure interrupts audio, addresses the wrong physical zone, captures more than one source, queues stale frames, or leaves a stale color after a recoverable stop.
