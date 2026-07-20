# Troubleshooting

Start with evidence. Do not tune the sampler, transition, capture timeout, or USB cadence until the symptom is assigned to the correct subsystem.

## Fast triage

```bash
pgrep -af logilightshow || true
lsusb -d 046d:0a78
systemctl --user status logilightshow-desktop-live.service --no-pager || true
systemctl --user status logilightshow-gaming.service --no-pager || true
journalctl --user -u logilightshow-desktop-live.service -n 30 --no-pager || true
```

Then classify:

| Symptom | First evidence |
|---|---|
| All four zones turn off/on together | Did cumulative `stalls` or capture error counters increase? |
| Only one/some zones flicker near black | Did `stalls` remain unchanged? Inspect sampler/normal fade |
| Colors freeze but process remains active | capture FPS, held-frame cadence, PipeWire/GStreamer state |
| Capture diagnostic says black pixels | backend/memory path, not USB |
| Static `set-zones` fails | USB access/ownership/protocol, not capture |
| Desktop works, Gaming fails | Gamescope node/service/backend |
| Gaming works, Desktop fails | portal/token/GStreamer backend |

## All lights flicker off and back on

If `stalls` increments, this is an engine safety blackout, not a color target. Correlate the exact second:

```bash
journalctl --user -u logilightshow-desktop-live.service -o short-precise --no-pager
journalctl --since 'START' --until 'END' -o short-precise --no-pager
```

Check for:

- system suspend or user-slice freeze;
- screen lock/display sleep;
- portal/PipeWire disconnect;
- caps renegotiation;
- severe scheduling pause.

The accepted implementation holds a valid Desktop frame after 200 ms of healthy silence. Do not simply increase or remove the 500 ms engine stall timeout; genuine capture loss must still black.

If stalls do not increment, it is a normal sampled color transition. Reproduce with `capture-test` or an instrumented local build, but do not log continuous user colors in production.

## Extremely dark content turns off abruptly

Current normal behavior is:

- soft per-pixel darkness visibility ramp;
- deterministic ambiguous-bin blending;
- 200 ms per-zone fade when the sampled target becomes exact black.

First verify the running process uses the latest release binary; rebuilding does not replace a running process. Stop/restart its service. Then check that `stalls` did not increase. If it did, diagnose capture/suspend instead.

Sampler regressions belong in `src/sampler.rs` with adjacent-value/ramp tests. Fade regressions belong in `src/transition.rs` and writer-state tests. Do not fade safety commands.

## Desktop capture is entirely black

Run:

```bash
./target/release/logilightshow capture-test --saved-permission --frames 30
```

If peak component and non-black frame count are zero for visibly bright content:

1. Ensure `LOGILIGHTSHOW_ENABLE_DMABUF` is not exported.
2. Verify `pipewiresrc`, `videoconvert`, and `videoscale` exist.
3. Re-run without saved permission and choose the intended monitor.
4. Inspect GStreamer errors and negotiated caps.

On the accepted Bazzite machine, the optional DMA-BUF/GL path negotiated but downloaded black pixels. The system-memory path is intentional.

## Capture FPS falls to about 5 on a static scene

This can be expected Desktop behavior: the source returns one held frame every 200 ms while no new portal buffer arrives. If:

- `stalls` does not increase;
- lights remain stable;
- capture and USB errors remain zero; and
- motion immediately raises FPS/updates,

do not treat 5 FPS as a failure.

## Portal chooser or restore-token problems

Inspect the token file without printing its contents:

```bash
stat -c '%a %U %G %n' ~/.config/logilightshow/capture.toml
```

Expected mode is `600` and current user ownership.

To test a fresh chooser without destroying the token, `capture-test` without `--saved-permission` already requests a new selection. If a persistent reset is necessary, move the config to a backup rather than deleting it:

```bash
mv ~/.config/logilightshow/capture.toml \
  ~/.config/logilightshow/capture.toml.backup
```

Do this only with user intent; it changes saved authorization.

Explicit chooser cancellation is a clean, non-retryable user state.

## G560 not found or permission denied

```bash
lsusb -d 046d:0a78
```

If absent, check power/cable. If present, resolve the current bus/device and inspect ACL:

```bash
getfacl /dev/bus/usb/BUS/DEVICE
```

Install/reinstall the rule:

```bash
./scripts/install-udev-rule.sh
```

Reconnect the speakers. Never run the application with `sudo`.

Check competing instances:

```bash
pgrep -af logilightshow
fuser /dev/bus/usb/BUS/DEVICE 2>/dev/null || true
```

## Audio interruption or USB I/O errors

Stop capture and isolate with a slow static `set-zones` command. Inspect cumulative `USB_errors`. Do not lower `REPORT_DELAY`; 6 ms is the accepted calibrated value. Ensure the process did not claim a wrong interface and only one instance is active.

Any cadence change requires fake pacing tests plus a real audio-playing calibration/soak.

## Gaming Mode service does not start

From Desktop after returning:

```bash
systemctl --user is-enabled logilightshow-gaming.service
systemctl --user cat logilightshow-gaming.service
journalctl --user -u logilightshow-gaming.service -b --no-pager
ls -l ~/.config/systemd/user/logilightshow-gaming.service
ls -l ~/.config/systemd/user/gamescope-session-plus@steam.service.wants/
```

Confirm:

- release binary exists at the unit's exact `ExecStart` path;
- unit symlinks still point into the repository;
- the repository has not moved;
- Gaming session actually provides the `gamescope` PipeWire node.

Inside Gaming Mode, a missing node should appear as a PipeWire setup/retry error rather than a portal error.

## Desktop transient service will not restart

An old collected transient definition may still be unloading. Stop/reset and wait until `systemctl --user cat` no longer finds it, then reuse the name. Do not create multiple differently named instances that compete for USB.

## Build fails in `libspa-sys`

Typical message:

```text
Package libpipewire-0.3 was not found in the pkg-config search path
```

Install `pipewire-devel` in the build environment or use the prepared Bazzite development container. Host runtime `pipewire-libs` is not the same as development metadata.

## Wrong physical zone

Read `docs/hardware/g560-zone-map.md`. Use `verify-zone INDEX` to pulse a raw protocol index and `set-zones` for logical mapping. Do not “fix” the mapping from visual intuition alone; it was physically verified and front/rear order differs from numeric protocol order.

## Useful environment snapshot

```bash
cat /etc/os-release
rustc --version
cargo --version
gst-inspect-1.0 --version
pw-cli --version
lsusb -d 046d:0a78
systemctl --user --version
git status --short
git rev-parse --short HEAD
```

Include relevant command output and precise timestamps in a handoff, but do not include the portal token or private captured content.
