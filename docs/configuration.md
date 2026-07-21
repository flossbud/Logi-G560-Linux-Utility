# Configuration and constants

## Persisted configuration

G560 Linux Utility currently persists only Desktop portal authorization:

```text
~/.config/logig560/capture.toml
```

Schema:

```toml
version = 1
restore_token = "<opaque portal token>"
```

The token is private and should not be printed, committed, archived, or copied between users. File mode must be `0600`.

There is no user configuration file for colors, brightness, geometry, transition durations, capture rate, USB cadence, or service paths.

## Environment variables

| Variable | Effect | Default |
|---|---|---|
| `LOGIG560_ENABLE_DMABUF` | Any presence enables the optional Desktop DMA-BUF/GL bridge | unset; system-memory Desktop path |
| `RUST_LOG` | No current effect; the binary does not initialize a tracing subscriber | unset |

The DMABUF variable is a compatibility diagnostic, not a recommended Bazzite setting.

## Compiled behavior constants

| Constant/setting | Current value | Source |
|---|---:|---|
| Rust toolchain | 1.97.1 | `rust-toolchain.toml` |
| Desktop static hold | 200 ms | `src/capture/gstreamer.rs` |
| Gamescope output width | 160 | `src/capture/gamescope.rs` |
| Gamescope pace | 50 ms / 20 FPS | `src/capture/gamescope.rs` |
| Desktop initial/scaled width | 160 | `src/capture/gstreamer.rs` |
| Darkness midpoint | 0.015 linear luma | `src/sampler.rs` |
| Chroma bin | 12 | `src/sampler.rs` |
| Lightness bin | 10 | `src/sampler.rs` |
| Normal transition | 90 ms | `src/transition.rs` |
| Normal fade to black | 200 ms | `src/engine.rs` |
| Capture stall | 500 ms | `src/engine.rs` |
| Engine write tick | 20 ms | `src/engine.rs` |
| Safety acknowledgement timeout | 1 s | `src/engine.rs` |
| Recovery initial/max | 250 ms / 5 s | `src/engine.rs` |
| USB failure reopen threshold | 3 | `src/engine.rs` |
| USB report spacing | 6 ms | `src/usb/device.rs` |
| USB transfer timeout | 100 ms | `src/usb/device.rs` |
| USB command queue | 32 | `src/usb/device.rs` |

Treat this table as an index, not a second source of truth. Update it when source constants change.

## Service configuration

`systemd/logig560-gaming.service` contains a machine-layout assumption:

```ini
ExecStart=%h/Documents/G560 Linux Utility/target/release/logig560 run-gaming
```

The install script symlinks the repository unit into the user systemd configuration. Moving the repository breaks both the unit symlink and `ExecStart`. Reinstall or make packaging relocatable as one coordinated change.

Desktop transient service parameters are documented in `docs/operations.md` but are not persisted by the repository.

## Reset behavior

- Portal selection: run `capture-test` without `--saved-permission` for a fresh diagnostic chooser.
- Persistent token reset: move `capture.toml` to a backup only with user intent.
- Gaming service: use the install/uninstall scripts.
- USB permission: rerun the idempotent udev installer and reconnect the device if needed.

No command currently writes sampler/transition/USB settings.
