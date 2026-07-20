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

## V1 polygon and smoothing refinement

The original rectangular edge regions were replaced with a complete, non-overlapping polygon layout tailored to the G560's physical light placement. Normalized vertices are:

- Left rear: `(0,0) (0.50,0) (0.17,0.70) (0.14,1) (0,1)`
- Left front: `(0.50,0) (0.50,1) (0.14,1) (0.17,0.70)`
- Right front: `(0.50,0) (0.83,0.70) (0.86,1) (0.50,1)`
- Right rear: `(0.50,0) (1,0) (1,1) (0.86,1) (0.83,0.70)`

Compiled masks assign every sampled pixel to exactly one zone, use deterministic boundary ownership, and remain mirror-symmetric at odd and even dimensions. Normal scene changes now use an interruptible 90 ms OKLab smoothstep transition. Retargeting starts from the last color confirmed by the hardware; stale destinations are never queued. Safety blackouts remain immediate.

### USB pacing calibration

Audio played throughout. Each 15-second stage continuously rotated four distinct colors, stopped on the first transfer error if one occurred, and attempted four paced black cleanup reports. No stage failed.

| Inter-report delay | Stimulus reports | Successful | Cleanup | USB errors |
| ---: | ---: | ---: | ---: | ---: |
| 18 ms | 824 | 824 | 4/4 | 0 |
| 16 ms | 928 | 928 | 4/4 | 0 |
| 14 ms | 1,056 | 1,056 | 4/4 | 0 |
| 12 ms | 1,228 | 1,228 | 4/4 | 0 |
| 10 ms | 1,468 | 1,468 | 4/4 | 0 |
| 8 ms | 1,464 | 1,464 | 4/4 | 0 |
| 6 ms | 1,932 | 1,932 | 4/4 | 0 |
| 4 ms | 2,440 | 2,440 | 4/4 | 0 |

The fastest passing interval was 4 ms. Applying the approved 2 ms safety margin selected a 6 ms production interval. A separate 120-second confirmation at 6 ms completed 15,416/15,416 stimulus reports and 4/4 cleanup reports with zero USB errors, uninterrupted audio, correct four-zone rotation, and final black. The diagnostic did not persist any value before this confirmation passed.

### Final perceptual acceptance

The release engine ran the updated polygon fixture fullscreen for 80.67 seconds, exceeding the required 60-second check:

- 1,480 captured frames (18.35 FPS) and 1,470 rendered lighting updates (18.22 updates/s)
- 10 newest-target replacements, zero capture stalls
- Capture-to-write p50/p95/p99: 71.30/85.82/88.06 ms
- Zero USB write errors, device open failures, reopens, blackout failures, capture stream/open/shutdown errors, or capture reopens
- Mean CPU over a 60-second `pidstat` sample: 9.70% of one logical CPU
- Resident memory: 103,940 KiB

The calibrated path nearly doubled complete hardware updates from the earlier 120 ms/~9.6 updates/s baseline. The user confirmed correct polygon-to-speaker mapping, visibly smoother and faster-feeling transitions, uninterrupted audio, immediate black on lock, resume from black after unlock with fresh content, and all four zones black after Ctrl-C. Their final assessment was that the result was “perfect and ready.” This closes the V1 polygon/smoothing refinement gate.

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
- The user's "slightly overreactive" feedback about the original snapping response is addressed by the 90 ms interruptible perceptual fade and calibrated 6 ms report cadence. Safety events intentionally remain unsmoothed.
- Static or paused content is valid. The service does not treat unchanged pixels as capture failure and does not add a duplicate-content watchdog.

## Acceptance decision

Pending the normal-use soak totals, capture-cancel check, SIGTERM check, final automated verification, and final observer confirmations. Milestone two must not begin until these fields are resolved.
