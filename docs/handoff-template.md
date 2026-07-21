# Machine handoff template

Copy this file to a dated machine-specific handoff and replace every placeholder. Do not include secrets or captured images.

```markdown
# G560 Linux Utility handoff — <machine/environment>

Last updated: <YYYY-MM-DD HH:MM TZ>
Owner/user confirmation: <what the user actually confirmed>

## Objective and outcome

- Requested objective:
- Current outcome:
- What remains:

## Repository state

- Absolute path:
- Branch:
- Commit:
- Remote:
- `git status --short` summary:
- Important uncommitted/untracked files that must be preserved:
- Snapshot/archive and checksum:

## Machine facts

- OS/image/version:
- Desktop/session/compositor:
- GPU(s) and relevant driver stack:
- GStreamer version:
- PipeWire version:
- Rust toolchain:
- Build container name/image (if any):
- Logitech device and firmware evidence:

## Build

- Packages installed:
- Exact build command:
- Release binary path:
- `ldd`/runtime compatibility result:

## USB access

- `lsusb` result:
- udev rule state:
- ACL result:
- Known competing processes/interfaces:

## Desktop capture

- Backend/path selected:
- Portal token path/mode (do not include token):
- Diagnostic command and result:
- Observed FPS/dimensions/non-black evidence:
- Known environment override:

## Gaming capture

- Gamescope node evidence:
- Diagnostic command and result:
- Service install/enable state:
- Steam shell confirmation:
- Real game confirmation:

## Live engine evidence

- Start command/unit:
- Invocation ID/time window:
- Capture FPS:
- Rendered update rate:
- Stalls:
- USB errors:
- Capture errors/reopens:
- Latency p50/p95/p99:
- Visual confirmation:
- Audio confirmation:

## Safety/lifecycle evidence

- Initial black:
- Lock/suspend:
- Capture error/recovery:
- Ctrl-C:
- SIGTERM/service stop:
- Desktop↔Gaming switch:

## Changes made

- Source:
- Tests:
- Services/scripts:
- Documentation/ADRs:

## Verification

- `cargo fmt --check`:
- `cargo clippy --all-targets -- -D warnings`:
- `cargo test --all-targets`:
- `cargo build --release`:
- `bash -n scripts/*.sh`:
- `systemd-analyze --user verify ...`:
- `git diff --check`:

## Known limitations and risks

-

## Do not undo

-

## Safe next steps

1.
```

Use precise observed language. Distinguish automated coverage, live metrics, and subjective user acceptance.
