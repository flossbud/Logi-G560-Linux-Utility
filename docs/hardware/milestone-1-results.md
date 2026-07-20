# Milestone 1 hardware acceptance results

## Gate status

**Acceptance in progress.** The deterministic fullscreen path, USB endurance, unplug/replug recovery, lock blackout/recovery, and Ctrl-C cleanup have passed. A user-directed, non-disruptive 30-minute normal mixed-use run is in progress; capture cancellation and SIGTERM cleanup remain to be measured before the gate can close.

The planned 30-minute fullscreen four-quadrant stimulus was not run because it would make the user's primary monitor inaccessible. At the user's direction it was replaced with a 30-minute background run during ordinary mixed use. This run must not be described as continuous-update stimulus. Fullscreen/direct-scanout behavior was separately proven with the deterministic pattern for 60.72 seconds.

## Test system

- Date: 2026-07-19
- Desktop: GNOME 50.3 on Wayland
- Mutter: 50.3
- PipeWire: 1.6.8
- xdg-desktop-portal: 1.22.1
- xdg-desktop-portal-gnome: 50.0
- G560 USB ID: `046d:0a78`
- G560 reported firmware revision: `90.64`
- Physical mapping: left front `0x00`, right front `0x01`, left rear `0x02`, right rear `0x03`

## Deterministic visual and fullscreen checks

- The local `docs/hardware/quadrant-test.html` pattern drove the four intended physical zones with rotating red, green, blue, and true black.
- GNOME fullscreen direct scanout initially froze the ordinary system-memory capture path. An application-scoped DMA-BUF/OpenGL bridge fixed it; no global GNOME setting was changed.
- Fullscreen run: 60.72 seconds, 6,115 captured frames, 2,067 rendered updates, 4,048 newest-frame replacements, zero stalls, zero USB/open/reopen/blackout failures, and capture-to-write p50/p95/p99 of 0.28/95.61/107.45 ms. Changed output averaged about 12.4 updates/s.
- The user reported that fullscreen rotation worked "so very beautifully," audio remained uninterrupted, and all zones finished black.
- Ordinary video and desktop use also drove the intended edges. The user found the immediate unsmoothed effect slightly overreactive; this is recorded as later tuning feedback, not a milestone-one functional failure.

## USB pre-acceptance evidence

- A five-minute persistent-interface-claim color rotation completed with zero observed USB errors, correct physical zones, a final all-black state, and uninterrupted audio.
- Single-zone hardware checks verified every raw protocol index. Audio was uninterrupted during every interface claim and lighting write.
- Single-monitor portal capture returned exactly one aspect-correct `160x90` RGB stream and a persistent restore token. No captured image was saved.

## Thirty-minute normal mixed-use run

Command:

```bash
./target/release/logilightshow run
```

- Method: normal background use of the selected monitor; no behavior change or dedicated stimulus required
- User-directed deviation: replaces the rejected 30-minute fullscreen test pattern
- Started: 20:43:52 EDT
- Duration: in progress (required: at least 30 minutes)
- Captured frames/rate: pending final totals
- Rendered updates/rate: pending final totals
- Dropped/replaced frames: pending final totals
- Capture stalls: pending final totals; static/paused content is legitimate and no duplicate-content watchdog is used
- Spontaneous USB errors: pending final totals
- Capture-to-write p50/p95/p99: pending final totals
- Audio continuity: pending final observer confirmation
- Final blackout: pending

## Resource use

After moving the DMA-BUF limiter ahead of OpenGL conversion and adding a fixed negotiated 20 FPS cap, two consecutive 30-second `pidstat -u -r -p PID 1 30` samples measured:

- Mean CPU over 60 seconds: 7.90% of one logical CPU (segment means 7.97% and 7.83%)
- Maximum one-second CPU sample: 9.00%
- Resident memory during the sample: 103,680 KiB

The RED live sample before the negotiated cap proved that setting only the `videorate` property was insufficient: capture still reached roughly 90 FPS, CPU averaged 34.17%, and RSS was 104,204 KiB. A later compositor timestamp burst also exceeded the negotiated media rate, so the final path adds a one-buffer leaky queue and 50 ms wall-clock pacer before conversion. Final-run rate and resource behavior are reported above when complete.

## Recovery and shutdown matrix

| Scenario | Expected result | Measured result | Audio |
| --- | --- | --- | --- |
| G560 unplug and replug | Wait with bounded backoff, reconnect, never show an expired scene | Passed. Device re-enumerated from USB device 009 to 029; process stayed alive; exactly three expected write failures triggered one reopen; screen colors resumed within the first 5-second reporting window after replug with no continuing errors. | Resumed normally after the unavoidable disconnected interval |
| Ctrl-C | Attempt all-zone blackout, stop capture, exit cleanly | Passed in multiple live runs, including 71.67 s and 463.41 s diagnostics; exit status was clean and final blackout was sent. | No interruption attributable to lighting control |
| Portal capture cancellation/EOS | Attempt all-zone blackout and exit cleanly | pending live chooser-cancel check; automated EOS and cancellation regressions pass | pending |
| Display lock/capture loss | Black within 500 ms; reopen saved single-monitor capture; resume only on a fresh frame | First live test exposed process exit when GNOME destroyed the PipeWire remote node. After a RED/GREEN recovery regression and fix, the retest recorded one capture-stream failure, stayed black, reopened once, and returned to 19.8–20.2 captured FPS. | Normal; pending final observer confirmation |
| Service process termination (`SIGTERM`) | Attempt all-zone blackout, stop capture, exit cleanly | pending | pending |

## Automated verification

- Focused recovery tests: passing, including exact capped backoff, missing-device reconnect, three-write-failure blackout/reopen, expired-update suppression, 500 ms stall blackout, capture-session reopen after stream failure, capture EOS, and cancellation blackout.
- Full `cargo fmt --check`, Clippy with warnings denied, and all-target tests: pending final post-acceptance run.

## Known limits and follow-up

- SDR is the only color-accuracy guarantee. HDR remains unvalidated.
- Milestone-one hardware testing is on GNOME Wayland. The fullscreen workaround currently requires DMA-BUF plus GStreamer OpenGL elements; KDE/Bazzite validation and packaging are milestone-two work.
- The default response intentionally has no smoothing. The user's "slightly overreactive" observation belongs to later response/saturation tuning and does not justify changing the immediate milestone-one default.
- Static or paused content is valid. The service does not treat unchanged pixels as capture failure and does not add a duplicate-content watchdog.

## Acceptance decision

Pending the normal-use soak totals, capture-cancel check, SIGTERM check, final automated verification, and final observer confirmations. Milestone two must not begin until these fields are resolved.
