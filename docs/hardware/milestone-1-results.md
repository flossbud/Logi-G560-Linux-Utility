# Milestone 1 hardware acceptance results

## Gate status

**V1 screen matching is accepted on the tested GNOME Wayland system at commit `fa4c07f`.** Production uses a calibrated 6 ms USB report interval and a 90 ms interruptible perceptual transition. The final 80.67-second fullscreen run delivered 18.22 complete lighting updates/s with zero USB, capture, recovery, or blackout errors. The user accepted mapping, smoothness, response, audio continuity, lock/unlock behavior, and final Ctrl-C blackout as “perfect and ready.”

A live `SIGTERM` hardware observation was not performed and is not claimed by this acceptance result. Ctrl-C and normal capture termination are the verified shutdown paths. The historical pre-refinement working notes below retain earlier `pending` fields for auditability; those fields are not the current gate status.

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

## Historical pre-refinement evidence

This section records the rectangular, unsmoothed baseline before the final polygon and cadence refinement. Its measurements are retained for comparison, not presented as the final V1 result.

### Deterministic visual and fullscreen baseline

- The local `docs/hardware/quadrant-test.html` pattern drove the four intended physical zones with rotating red, green, blue, and true black.
- GNOME fullscreen direct scanout initially froze the ordinary system-memory capture path. An application-scoped DMA-BUF/OpenGL bridge fixed it; no global GNOME setting was changed.
- Fullscreen run: 60.72 seconds, 6,115 captured frames, 2,067 rendered updates, 4,048 newest-frame replacements, zero stalls, zero USB/open/reopen/blackout failures, and capture-to-write p50/p95/p99 of 0.28/95.61/107.45 ms. Changed output averaged about 12.4 updates/s.
- The user reported that fullscreen rotation worked "so very beautifully," audio remained uninterrupted, and all zones finished black.
- Ordinary video and desktop use also drove the intended edges. The user found the immediate unsmoothed effect slightly overreactive; this is recorded as later tuning feedback, not a milestone-one functional failure.

## Final V1 acceptance (`fa4c07f`)

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

### Final fullscreen and perceptual acceptance

The release engine ran the updated polygon fixture fullscreen for 80.67 seconds, exceeding the required 60-second check:

- 1,480 captured frames (18.35 FPS) and 1,470 rendered lighting updates (18.22 updates/s)
- 10 newest-target replacements, zero capture stalls
- Capture-to-write p50/p95/p99: 71.30/85.82/88.06 ms
- Zero USB write errors, device open failures, reopens, blackout failures, capture stream/open/shutdown errors, or capture reopens
- Mean CPU over a 60-second `pidstat` sample: 9.70% of one logical CPU
- Resident memory: 103,940 KiB

The calibrated path nearly doubled complete hardware updates from the earlier 120 ms/~9.6 updates/s baseline. The user confirmed correct polygon-to-speaker mapping, visibly smoother and faster-feeling transitions, uninterrupted audio, immediate black on lock, resume from black after unlock with fresh content, and all four zones black after Ctrl-C. Their final assessment was that the result was “perfect and ready.” This closes the V1 screen-matching acceptance gate on the tested system.

## Historical USB pre-acceptance evidence

These checks preceded the final 6 ms/90 ms run and remain supporting hardware evidence:

- A five-minute persistent-interface-claim color rotation completed with zero observed USB errors, correct physical zones, a final all-black state, and uninterrupted audio.
- Single-zone hardware checks verified every raw protocol index. Audio was uninterrupted during every interface claim and lighting write.
- Single-monitor portal capture returned exactly one aspect-correct `160x90` RGB stream and a persistent restore token. No captured image was saved.

## Historical mixed-use plan (superseded)

The planned 30-minute fullscreen stimulus was rejected because it would make the user's primary monitor inaccessible. A non-disruptive ordinary-use run was proposed in its place, but the working document never received accepted totals. The approved final V1 procedure instead used the deterministic fullscreen check plus the dedicated USB calibration and recovery checks. The uncollected fields below are retained only as a historical audit trail.

Command:

```bash
./target/release/logig560 run
```

- Method: normal background use of the selected monitor; no behavior change or dedicated stimulus required
- User-directed deviation: replaces the rejected 30-minute fullscreen test pattern
- Started: 20:43:52 EDT
- Duration: historical status was `in progress`; no accepted total was recorded
- Captured frames/rate: historical pending field; not used for final acceptance
- Rendered updates/rate: historical pending field; not used for final acceptance
- Dropped/replaced frames: historical pending field; not used for final acceptance
- Capture stalls: historical pending field; static/paused content remained valid
- Spontaneous USB errors: historical pending field; not used for final acceptance
- Capture-to-write p50/p95/p99: historical pending field; not used for final acceptance
- Audio continuity: historical pending field; not used for final acceptance
- Final blackout: historical pending field; not used for final acceptance

## Historical capture-rate optimization baseline

After moving the DMA-BUF limiter ahead of OpenGL conversion and adding a fixed negotiated 20 FPS cap, two consecutive 30-second `pidstat -u -r -p PID 1 30` samples measured:

- Mean CPU over 60 seconds: 7.90% of one logical CPU (segment means 7.97% and 7.83%)
- Maximum one-second CPU sample: 9.00%
- Resident memory during the sample: 103,680 KiB

The RED live sample before the negotiated cap proved that setting only the `videorate` property was insufficient: capture still reached roughly 90 FPS, CPU averaged 34.17%, and RSS was 104,204 KiB. A later compositor timestamp burst also exceeded the negotiated media rate, so the final path adds a one-buffer leaky queue and 50 ms wall-clock pacer before conversion. Current final-run rate and resource behavior are reported in the `fa4c07f` acceptance section above.

## Recovery and shutdown matrix

| Scenario | Expected result | Measured result | Audio |
| --- | --- | --- | --- |
| G560 unplug and replug | Wait with bounded backoff, reconnect, never show an expired scene | Passed. Device re-enumerated from USB device 009 to 029; process stayed alive; exactly three expected write failures triggered one reopen; screen colors resumed within the first 5-second reporting window after replug with no continuing errors. | Resumed normally after the unavoidable disconnected interval |
| Ctrl-C | Attempt all-zone blackout, stop capture, exit cleanly | Passed in multiple live runs, including the final 80.67 s acceptance; exit status was clean and all four zones ended black. | No interruption attributable to lighting control |
| Portal capture cancellation/EOS | Attempt all-zone blackout and exit cleanly | Automated EOS and cancellation cleanup regressions pass. A separate live chooser-cancel observation was not required for the final hardware gate. | Not separately observed live |
| Display lock/capture loss | Black within 500 ms; reopen saved single-monitor capture; resume only on a fresh frame | An initial failure led to a recovery regression and fix. In final acceptance the user confirmed immediate black on lock and resume from black after unlock with fresh content. | Normal and uninterrupted |
| Service process termination (`SIGTERM`) | Attempt all-zone blackout, stop capture, exit cleanly | Not live-verified; pending outside the V1 hardware acceptance scope and not claimed in the README. | Not observed |

## Automated verification

- Focused recovery tests: passing, including exact capped backoff, missing-device reconnect, three-write-failure blackout/reopen, expired-update suppression, 500 ms stall blackout, capture-session reopen after stream failure, capture EOS, and cancellation blackout.
- Final `cargo fmt --check`, Clippy with warnings denied, and all-target tests passed at `fa4c07f`: 93 tests passed, 0 failed.

## Known limits and follow-up

- SDR is the only color-accuracy guarantee. HDR remains unvalidated.
- Milestone-one hardware testing is on GNOME Wayland. The fullscreen workaround currently requires DMA-BUF plus GStreamer OpenGL elements; KDE/Bazzite validation and packaging are milestone-two work.
- The user's "slightly overreactive" feedback about the original snapping response is addressed by the 90 ms interruptible perceptual fade and calibrated 6 ms report cadence. Safety events intentionally remain unsmoothed.
- Static or paused content is valid. The service does not treat unchanged pixels as capture failure and does not add a duplicate-content watchdog.

## Acceptance decision

Accepted for V1 on the tested GNOME Wayland system at `fa4c07f`. The evidence comprises the 6 ms production cadence, 90 ms transition, complete polygon masks, 120-second pacing confirmation, final 80.67-second fullscreen run at 18.22 lighting updates/s, zero runtime errors, lock/unlock recovery, uninterrupted audio, Ctrl-C final black, 93 passing tests, and explicit user acceptance. The superseded mixed-use placeholders and the unperformed live `SIGTERM` observation are historical/out-of-scope notes, not unresolved claims in this acceptance decision.
