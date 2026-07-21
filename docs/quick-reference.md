# Quick reference

## Thirty-second summary

G560 Linux Utility captures one screen, reduces it to four zone colors, smoothly transitions the Logitech G560 lights, and immediately blacks them on safety events. Desktop uses portal+GStreamer; Bazzite Gaming uses direct Gamescope PipeWire. The accepted Bazzite changes are intentionally uncommitted in the original workspace.

## First commands

```bash
git status --short
cat AGENTS.md
sed -n '1,220p' docs/project-status.md
./target/release/logig560 --help
```

On Bazzite, build/test in:

```bash
distrobox enter logig560 -- bash -lc \
  'cd /home/jaret/Documents/G560 Linux Utility && cargo test --all-targets'
```

## Runtime commands

```bash
# Desktop
./target/release/logig560 run

# Gaming Mode (normally service-managed)
./target/release/logig560 run-gaming

# Capture only; saves no image
./target/release/logig560 capture-test --saved-permission --frames 30
./target/release/logig560 capture-test --gamescope --frames 30

# Static USB isolation
./target/release/logig560 set-zones \
  --left-rear FF0000 --left-front 00FF00 \
  --right-front 0000FF --right-rear FFFFFF
```

## Service commands

```bash
systemctl --user status logig560-desktop-live.service --no-pager
journalctl --user -u logig560-desktop-live.service -f
systemctl --user is-enabled logig560-gaming.service
journalctl --user -u logig560-gaming.service -b --no-pager
```

Gaming should be enabled/inactive on Desktop.

## Critical constants

```text
capture width                 160 px
capture pace                  ~20 FPS
Desktop held-frame interval   200 ms
capture stall timeout         500 ms
normal transition              90 ms
normal zone fade to black     200 ms
writer tick                    20 ms
USB report spacing              6 ms
USB transfer timeout          100 ms
USB reopen threshold            3 failures
recovery backoff              250 ms → 5 s cap
```

## Diagnostic split

```text
all zones off/on + stalls increased  → capture/suspend safety path
dark-zone flicker + stalls unchanged → sampler/normal transition
capture-test black                   → capture backend/memory path
set-zones fails                      → USB access/device path
Desktop-only failure                 → portal/GStreamer
Gaming-only failure                  → gamescope PipeWire/service
```

## Never do casually

- Do not reset/clean the uncommitted Bazzite work.
- Do not run the program with sudo.
- Do not save or continuously log screen pixels/colors.
- Do not add frame/target FIFO queues.
- Do not fade safety blackouts.
- Do not lower USB pacing without hardware/audio calibration.
- Do not make the DMA-BUF path the Bazzite default based only on successful negotiation.
- Do not run two instances against the G560.

## Final gate

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
cargo build --release
bash -n scripts/*.sh
systemd-analyze --user verify systemd/logig560-gaming.service
git diff --check
```

See the [documentation index](README.md) for the full guide set.
