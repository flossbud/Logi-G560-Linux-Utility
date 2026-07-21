# ADR 0004: Bazzite Desktop defaults to system-memory capture

- Status: Accepted
- Date: 2026-07-20

## Context

The Fedora fullscreen workaround used a DMA-BUF → GL upload/convert/download bridge. On the accepted Bazzite Mesa/GStreamer stack, that bridge negotiated but the downloaded pixels were entirely black. A technically “playing” pipeline was therefore not sufficient evidence of correct capture.

## Decision

Use ordinary system-memory PipeWire buffers and GStreamer conversion/scaling by default on Desktop. Retain the DMA-BUF/GL bridge behind `LOGIG560_ENABLE_DMABUF=1` for explicit compatibility testing.

Gaming Mode separately advertises BGRx without a DRM modifier to obtain CPU-mapped MemFd buffers.

## Consequences

- Bazzite correctness takes precedence over the original Fedora optimization.
- The optional path increases maintenance surface but preserves compatibility evidence.
- Any attempt to restore DMA-BUF as default requires a live non-black pixel diagnostic, not just successful caps negotiation.

## Evidence

Desktop `capture-test` and live lighting returned correct visible content on the system-memory path. The user confirmed Desktop operation; the optional GL path had produced black capture on the same machine.
