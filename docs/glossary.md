# Glossary

**Captured frame:** An owned, CPU-readable RGB image represented by `RgbFrame`. Desktop and Gaming backends both normalize into this type.

**Capture generation:** An even/odd engine counter used to invalidate frames around safety blackouts. Odd generations are blacked-out states; new normal work resumes on the next even generation.

**Capture stall:** The engine receiving no frame for 500 ms. It requests an immediate safety blackout. Desktop capture can hold a previously validated frame during shorter healthy portal silence.

**Desktop Mode:** The normal GNOME/KDE Wayland desktop. Capture is authorized through the XDG ScreenCast portal and consumed through GStreamer.

**DMABUF:** Linux GPU-shared buffer transport. The original Fedora path used a GStreamer GL bridge for DMA-BUF frames. It is optional on Bazzite because this machine's Bazzite GL download yielded black pixels.

**Gamescope:** Bazzite/Steam Gaming Mode compositor. It exposes a PipeWire node named `gamescope`; the prototype captures that node directly because the desktop portal is unavailable there.

**Held frame:** The last dimension-validated Desktop frame repeated after 200 ms without a new GStreamer sample. It prevents healthy static/low-motion portal behavior from crossing the engine's 500 ms stall threshold.

**Latest-only channel:** A one-value handoff. Sending a newer value replaces an unread older value and increments dropped/replacement accounting. This prevents latency accumulation.

**Logical zone order:** `[left rear, left front, right front, right rear]`, the order used by `ZoneColors` arrays throughout the code.

**MemFd:** A CPU-mappable file-descriptor-backed PipeWire buffer. Gaming Mode intentionally negotiates BGRx without a DRM modifier to select this path.

**Normal blackout / sampled black:** The sampler deciding that content for a zone is fully dark. It is a normal target and fades over 200 ms for that zone.

**Portal grant:** The one-monitor ScreenCast session, selected stream, PipeWire remote file descriptor, and optional restore token returned by the XDG portal.

**Restore token:** An opaque portal authorization token saved to `~/.config/logig560/capture.toml`. It allows later Desktop runs to request the same selection without storing monitor pixels or metadata.

**Safety blackout:** An out-of-band, priority command that immediately sends black to all zones and invalidates stale normal work. It is used for startup, capture stall/loss, cancellation, shutdown, and recovery.

**Sampled target:** Four RGB colors produced from one captured frame and its compiled zone masks.

**System-memory path:** The default Bazzite Desktop GStreamer pipeline that converts ordinary PipeWire video buffers to scaled RGB without the GL/DMA-BUF bridge.

**Zone mask:** A precompiled list of frame pixel indexes belonging to one logical G560 region. The built-in masks are complete, disjoint, and horizontally mirrored.
