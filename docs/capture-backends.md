# Capture backends

G560 Linux Utility has two capture backends because ordinary Wayland desktops and Bazzite Gaming Mode expose fundamentally different session facilities.

## Shared output contract

Both backends implement `FrameSource` and produce owned `RgbFrame` values:

- packed or padded RGB8 rows;
- nonzero width and height;
- `stride >= width * 3`;
- exactly `stride * height` bytes;
- target width 160 pixels with proportional height.

The engine does not know which backend produced a frame.

## Desktop: portal plus GStreamer

### Authorization

`PortalCapture::open` requests:

- source type: monitor;
- multiple selection: false;
- cursor: hidden;
- persistence: explicitly revoked;
- optional saved restore token.

It rejects zero or multiple returned streams. Any failure after session creation attempts to close the session; cleanup failure is preserved alongside the primary error.

The restore token is opaque. `src/main.rs` stores it at:

```text
~/.config/logig560/capture.toml
```

The write is create-new temporary file → `0600` → write → fsync → atomic rename → parent-directory fsync. Stale temporary files are not reused.

### Default Desktop pipeline

```text
pipewiresrc
  → queue(leaky=downstream, max-size-buffers=1)
  → identity(sleep-time=50ms)
  → videoconvert
  → videoscale
  → capsfilter(RGB, width=160, proportional height)
  → appsink(max-buffers=1, drop=true, sync=false)
```

The 50 ms wall-clock pacer limits useful work to about 20 FPS. The leaky queue and appsink ensure old frames do not accumulate. This same system-memory path is validated through the GNOME and KDE ScreenCast portal implementations; desktop-specific chooser behavior remains owned by the portal.

An upstream caps probe recalculates the proportional output height on orientation/resolution changes. While caps are changing, `expected_dimensions` is zero and samples are gated. Only a sample matching the most recently installed dimensions can pass. This prevents a stale-resolution frame from reopening the gate during asynchronous caps updates.

### Healthy static-frame hold

GNOME portal/PipeWire delivery can become sparse for static or low-motion content. The source polls the appsink and GStreamer bus every 50 ms. If no sample arrives for 200 ms and:

- a prior valid frame exists;
- expected dimensions are nonzero; and
- the prior frame still matches those dimensions,

the backend returns a clone of that frame. This prevents a healthy pause from reaching the engine's 500 ms stall cutoff without doubling the normal stream cadence.

The hold does not apply before the first frame or during caps renegotiation. Bus errors and EOS are checked before the hold on each poll.

### Optional DMA-BUF/GL pipeline

Set this only for compatibility testing:

```bash
LOGIG560_ENABLE_DMABUF=1 ./target/release/logig560 run
```

The optional path constrains DMA-BUF formats, rate-limits before conversion, uploads to GL memory, converts to RGBA, downloads to system memory, and then scales to RGB. It was needed on the original Fedora fullscreen path. On the accepted Bazzite machine, GL download negotiated successfully but returned black pixels; therefore system memory is the default.

Do not make DMA-BUF the Bazzite default without a non-black capture test on the real machine.

## Gaming Mode: direct PipeWire

Gamescope session startup intentionally does not provide the normal desktop portal. `GamescopeFrameSource` connects directly to a PipeWire node named:

```text
gamescope
```

The worker advertises raw `BGRx` without a DRM modifier. That choice intentionally negotiates a CPU-mapped `MemFd`/`MemPtr` buffer rather than DMA-BUF, avoiding the same black GPU readback seen in Desktop testing.

### Worker lifecycle

- A named standard thread owns the PipeWire main loop and stream.
- Startup is acknowledged through a synchronous channel with a 5 s timeout.
- Stream error or unexpected disconnect is sent to the async side as `CaptureError::PipeWire`.
- A PipeWire loop channel carries the stop command.
- Shutdown joins the worker through `spawn_blocking`.
- `Drop` at least sends stop if explicit shutdown did not complete.

### Frame decode and pace

The process callback:

1. Uses a non-drifting 50 ms deadline to accept at most 20 frames/s.
2. Dequeues a CPU-mapped buffer.
3. Validates BGRx, buffer type, dimensions, offset, stride, and byte range.
4. Nearest-neighbor scales to width 160 with proportional height.
5. Reorders BGRx into packed RGB.
6. Sends through a latest-only channel.

Late callbacks advance to the next long-term deadline instead of shifting the schedule, preventing slow drift.

## Backend selection

| Context | Command | Backend |
|---|---|---|
| GNOME/KDE Desktop | `logig560 run` | portal + GStreamer |
| Desktop capture diagnostic | `logig560 capture-test` | new portal selection + GStreamer |
| Saved Desktop diagnostic | `logig560 capture-test --saved-permission` | saved portal selection + GStreamer |
| Bazzite Gaming Mode | `logig560 run-gaming` | direct Gamescope PipeWire |
| Gaming capture diagnostic | `logig560 capture-test --gamescope` | direct Gamescope PipeWire |

`--gamescope` and `--saved-permission` conflict by design.

## Capture diagnostics

`capture-test` processes frames in memory and prints:

- frame count, dimensions, elapsed time, and effective FPS;
- count of non-black frames;
- maximum number of pixels with a component at least 32;
- maximum component value;
- the final four sampled zone colors;
- portal persistence status where applicable;
- `saved images: 0`.

It does not write a screenshot. Be aware that this diagnostic prints one sampled color set, unlike the normal live metrics, which never print colors.

## Common failure boundaries

| Failure | Expected behavior |
|---|---|
| Portal chooser cancelled | typed cancellation; clean exit; no retry loop |
| Portal returns multiple streams | fail closed |
| Unsupported/missing caps or invalid layout | fail; do not silently sample |
| Runtime Desktop node destroyed | safety black; bounded reopen when retryable |
| Gamescope node missing at service start | service retries via source recovery/systemd |
| PipeWire worker disconnect | error to engine, safety black, reopen |
| Static Desktop content | hold last valid frame after 200 ms |
| Resolution/caps change | invalidate hold and drop mismatched samples until new caps are installed |

See [`troubleshooting.md`](troubleshooting.md) for concrete commands.
