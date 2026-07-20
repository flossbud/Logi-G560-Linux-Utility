# Async USB I/O fix

LibUSB control transfers are synchronous and can block up to the existing 100 ms
timeout. Production now uses `AsyncG560`, a serialized adapter that runs each
operation on a dedicated bounded standard-library worker owning the G560.
Capture, recovery, and cancellation tasks therefore continue to run
while USB is stalled; report ordering and the calibrated 6 ms pacing are
unchanged. The adapter preserves the existing `LightSink` and diagnostic APIs,
and fake transports remain synchronous and deterministic. A bounded FIFO queue
and blackout invalidation prevent stale queued writes from reaching hardware.

Added a current-thread Tokio test proving an 80 ms blocking transport does not
prevent an independent async timer from progressing. No live hardware tests
were performed. SIGTERM behavior and its existing documentation caveat are
unchanged.
