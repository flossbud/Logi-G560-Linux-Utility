# Architecture decision records

These records explain choices that future agents might otherwise “simplify” back into known failures.

| ADR | Status | Decision |
|---|---|---|
| [`0001-single-monitor-portal.md`](0001-single-monitor-portal.md) | Accepted | Desktop capture authority is one monitor selected through the system portal |
| [`0002-latest-value-pipeline.md`](0002-latest-value-pipeline.md) | Accepted | Frames and targets are newest-only rather than queued |
| [`0003-separate-bazzite-capture-backends.md`](0003-separate-bazzite-capture-backends.md) | Accepted | Desktop and Gaming Mode use different capture backends |
| [`0004-bazzite-system-memory-default.md`](0004-bazzite-system-memory-default.md) | Accepted | Bazzite Desktop defaults to system-memory GStreamer capture |
| [`0005-calibrated-g560-usb-path.md`](0005-calibrated-g560-usb-path.md) | Accepted | USB reports use verified indexes, one claimed HID interface, and 6 ms global pacing |
| [`0006-priority-safety-blackouts.md`](0006-priority-safety-blackouts.md) | Accepted | Safety blackouts bypass normal transitions and invalidate stale work |
| [`0007-low-light-and-black-fade.md`](0007-low-light-and-black-fade.md) | Accepted | Low-light sampling is continuous/deterministic and normal black uses a longer per-zone fade |
| [`0008-desktop-static-frame-hold.md`](0008-desktop-static-frame-hold.md) | Accepted | Desktop repeats a validated frame after 200 ms of healthy source silence |

When a decision changes, do not silently rewrite the old record. Mark it superseded and link the replacement.
