# Project status

Last updated: 2026-07-20.

## One-sentence status

LogiLightShow is an accepted command-line prototype for low-latency, four-zone Logitech G560 screen matching on Fedora/Bazzite GNOME and Gamescope, now with a persistent Unix-socket lighting service and a Tauri 2 GUI (`crates/logilightshow-gui/`) driving Manual, Content-Aware, setup, and diagnostics workflows; it is not yet a packaged end-user application.

## GUI increment (2026-07-20)

- `crates/logilightshow-api/` — shared DTOs and the newline-delimited JSON protocol used by both service and GUI.
- `src/service/` — Unix-socket service (`$XDG_RUNTIME_DIR/logilightshow.sock`) exposed by the existing `run`/`run-gaming` commands. Reuses the accepted engine, capture backends, and USB writer unchanged.
- `crates/logilightshow-gui/` — Tauri 2 GUI with a framework-free HTML/CSS/JS frontend. Ships the mockup speakers, four rail pages, first-run overlay, `pkexec` udev installer, systemd user-unit installer, and color-free diagnostics.
- `systemd/logilightshow-desktop.service.in` — template used by the GUI to install a per-user Desktop service against the current binary path.

## Proven working

- Logitech G560 USB device `046d:0a78` on the original Fedora/Bazzite test machine.
- Four physically verified lighting zones with logical order left rear, left front, right front, right rear.
- Fedora GNOME Wayland single-monitor portal capture.
- Bazzite 43 GNOME Desktop Mode through the saved portal authorization and GStreamer system-memory path.
- Bazzite Gaming Mode and real games through direct capture of Gamescope's PipeWire node.
- Arch Linux KDE Plasma 6 Wayland through `xdg-desktop-portal-kde` and the same system-memory GStreamer Desktop backend.
- Smooth normal color transitions, deterministic low-light behavior, and a longer normal fade when a zone goes fully black.
- Immediate safety blackouts for capture stall/loss, cancellation, shutdown, and USB recovery.
- Automatic bounded recovery for missing speakers, USB write failures, and retryable capture failures.
- Clean Ctrl-C and SIGTERM handling in code and repeated service restarts during Bazzite bring-up.
- Color-free five-second runtime metrics.

The final Bazzite user acceptance was: operation worked “extremely well,” the final dark fade “looks good,” and the prototype was approved for wrap-up.

## Current defaults

| Behavior | Value |
|---|---:|
| Capture target width | 160 pixels, proportional height |
| Capture ceiling/pace | approximately 20 FPS |
| Engine write tick | 20 ms |
| Normal transition | 90 ms, per-zone OKLab smoothstep |
| Normal transition to fully black | 200 ms for affected zone |
| Desktop healthy-silence hold | emit last dimension-valid frame after 200 ms |
| Engine capture-stall timeout | 500 ms |
| USB report spacing | 6 ms between every adjacent report |
| USB control transfer timeout | 100 ms |
| USB reopen threshold | 3 consecutive write failures |
| Recovery backoff | 250 ms doubling to a 5 s cap |
| Safety-blackout acknowledgement bound | 1 s |

The 2026-07-20 Arch KDE validation captured 30/30 non-black frames at 160x90, successfully reused Plasma's saved portal permission for another 30/30 non-black frames, then completed a short live run with 57 captured frames, 30 rendered hardware updates, zero USB/capture failures, and clean signal teardown. Extended lock/suspend/audio and subjective physical-speaker acceptance remain outstanding.

## Repository and artifact state

- Git branch: `main`.
- Accepted Bazzite baseline: `348e08c` (`feat: add accepted Bazzite desktop and gaming support`).
- KDE work starts from that committed baseline; the Bazzite source/runtime changes are no longer an uncommitted working-tree payload.
- There is no configured remote in the accepted workspace.
- `LogiLightShow-Bazzite-handoff.zip` is the original transferred snapshot.
- `LogiLightShow-Bazzite-prototype-final.zip` is the pre-documentation accepted Bazzite snapshot.
- `LogiLightShow-Bazzite-agent-handoff.zip` is the current source-and-documentation handoff. The two earlier archives are preserved as immutable evidence rather than overwritten.

Agents must inspect `git status --short` and preserve this state. Never use `git clean`, `git reset --hard`, or a blanket checkout here.

## Runtime integration state on the accepted machine

- Desktop testing uses a transient user unit named `logilightshow-desktop-live.service`.
- The Gaming Mode unit is installed by symlink from the repository, enabled under `gamescope-session-plus@steam.service.wants`, and expected to be inactive while GNOME Desktop Mode is active.
- The release binary is loaded from `target/release/logilightshow`.
- The saved desktop portal token is at `~/.config/logilightshow/capture.toml`, mode `0600`.
- The repository currently lives at `/home/jaret/Documents/LogiLightShow`; the Gaming Mode unit's `ExecStart` depends on that location.

Runtime state is ephemeral. Verify it rather than assuming it from this document.

## Verification baseline

At prototype wrap-up:

- 95 library unit/property tests passed.
- 8 CLI/config tests passed.
- 3 integration tests passed.
- 106 total tests passed.
- `cargo fmt --check` passed.
- `cargo clippy --all-targets -- -D warnings` passed.
- `cargo build --release` passed.
- `cargo doc --no-deps` generated the Rust API documentation without warnings.
- All shell installers passed `bash -n`.
- The Gaming Mode unit passed `systemd-analyze --user verify`.
- The current agent-handoff archive passed `unzip -t` and contains no `.git`, `target`, portal token, or local user configuration.

Counts will change as tests are added; the important baseline is zero failures and zero Clippy warnings.

## Not complete

- No GUI, D-Bus control plane, pause control, tray icon, or monitor-selection UI beyond the system portal.
- No distributable RPM/Flatpak/ujust package.
- No persistent Desktop Mode unit or autostart installer; the accepted Desktop run is transient.
- No configuration UI for geometry, darkness, transition durations, or capture backend.
- No HDR accuracy guarantee.
- No broad GPU/compositor/device support matrix.
- No CI workflow or configured upstream remote in this workspace.
- No automated documentation-link checker.

See [`known-limitations.md`](known-limitations.md) for consequences and future-work guidance.
