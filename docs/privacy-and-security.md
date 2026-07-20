# Privacy and security

## Runtime data flow

LogiLightShow processes display pixels locally:

1. The desktop portal or Gamescope PipeWire node supplies a frame.
2. Capture scales it to a small CPU-readable RGB frame.
3. The sampler reduces it to four RGB values.
4. The frame and intermediate pixel data are dropped from memory.
5. Four colors are sent to the local USB device.

The runtime has no application-level network client, telemetry, or remote API.

## What is persisted

The only application-created persistent runtime state is:

```text
~/.config/logilightshow/capture.toml
```

It contains a format version and an opaque desktop-portal restore token. It is created with mode `0600`, written through a private create-new temporary file, fsynced, and atomically renamed.

It does not contain screenshots, monitor images, RGB histories, or geometry inferred from private content.

## What is not persisted

Normal operation does not save:

- screenshots or thumbnails;
- raw or scaled frames;
- per-frame sampled zone colors;
- color history;
- monitor content hashes;
- audio data;
- user input/cursor imagery (the portal request hides the cursor);
- network telemetry.

The `capture-test` command prints one final sampled color set and aggregate pixel counters to standard output. It still saves no image. Treat pasted diagnostic output as potentially revealing coarse screen-color information.

## Logging policy

Normal live logs contain rates, counts, latencies, and error messages. They intentionally do not contain sampled colors.

When adding diagnostics:

- prefer counts, timestamps, dimensions, and state transitions;
- make any color/pixel diagnostic explicit and short-lived;
- do not make continuous color logging the default;
- do not write frame dumps unless the user explicitly authorizes the exact capture and destination;
- remove temporary diagnostic artifacts after the issue is resolved, with user awareness.

## Portal authority

Desktop capture uses the system ScreenCast portal:

- monitor source only;
- exactly one stream;
- user/system chooser controls selection;
- cursor hidden;
- permission may be restored only through the portal's token;
- authorization can be revoked by the desktop.

The application does not bypass the portal in Desktop Mode.

Gaming Mode is different: the Gamescope session exposes its compositor output as a PipeWire node intended for session consumers. The direct backend targets only the node named `gamescope`.

## Privilege boundary

The runtime process is unprivileged. It must not be launched with `sudo`.

The only privileged workflow is installing the narrow udev rule with Polkit:

```text
046d:0a78 → TAG+="uaccess"
```

The script installs one known file, reloads udev, and triggers the matching device. It does not grant generic USB access.

## USB defensive checks

Before detaching a kernel driver, the implementation:

- finds USB interface 2;
- validates all descriptors for that interface are HID;
- remembers whether a kernel driver was active;
- attempts reattachment on claim failure and on drop.

This protects the G560's other functionality, particularly audio, from an accidental claim of a non-HID interface.

## Safety versus privacy

A stale ambient color can misrepresent the current/locked display. Therefore capture loss, lock-related stream destruction, process shutdown, and recovery use immediate black rather than leaving the last scene visible.

The Desktop healthy-silence hold repeats only an in-memory, dimension-validated frame while the GStreamer pipeline has not reported error/EOS. It exists to prevent false blackouts from brief sparse delivery. Caps changes invalidate the hold.

## Secrets and support bundles

Never include these in an archive or issue:

- `~/.config/logilightshow/capture.toml` contents;
- full environment dumps that may contain tokens;
- screenshots or frame buffers without explicit consent;
- unrelated user journals or home-directory data.

Repository handoff archives should contain source, docs, tests, scripts, and units only. They should exclude `.git`, `target`, user config, journals, and previously created archives.

## Dependency and supply-chain notes

- Rust dependencies are locked by `Cargo.lock`.
- The toolchain is pinned by `rust-toolchain.toml`.
- Native libraries come from Fedora/Bazzite packages.
- There is no CI dependency audit configured yet.
- There is no packaged sandbox/Flatpak permission manifest yet.

Future packaging should document portal, PipeWire, and USB permissions explicitly and preserve the no-root runtime model.
