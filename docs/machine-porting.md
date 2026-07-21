# New-machine orientation and porting checklist

This checklist lets an agent establish facts on an unfamiliar Linux machine before changing source.

## Phase 1: Identify the workspace

```bash
pwd
git status --short
git branch --show-current
git log -1 --oneline
git remote -v
rg --files -g '!target/**' -g '!.git/**' | sort
```

Record:

- absolute repository path;
- current commit and branch;
- whether changes are tracked/untracked;
- whether a remote exists;
- whether handoff archives are evidence or active inputs.

Do not clean or reset an unfamiliar workspace.

## Phase 2: Identify OS and session

```bash
cat /etc/os-release
loginctl list-sessions --no-legend
echo "${XDG_SESSION_TYPE:-unset}"
echo "${XDG_CURRENT_DESKTOP:-unset}"
systemctl --user is-active graphical-session.target || true
systemctl --user is-active gamescope-session-plus@steam.service || true
```

Classify the target:

- mutable Fedora-like or Arch desktop;
- immutable Bazzite/Silverblue desktop;
- Gamescope Gaming Mode;
- unsupported X11/non-PipeWire context.

Do not infer Gaming Mode merely from Steam running; check the Gamescope session unit/node.

## Phase 3: Identify hardware and access

```bash
lsusb -d 046d:0a78
```

If present, derive the current bus/device and inspect:

```bash
getfacl /dev/bus/usb/BUS/DEVICE
fuser /dev/bus/usb/BUS/DEVICE 2>/dev/null || true
```

Record:

- device presence;
- current user ACL;
- any competing owner;
- whether the udev rule exists.

Do not run as root. Install the rule with `scripts/install-udev-rule.sh` only when authorized.

## Phase 4: Identify build environment

```bash
rustc --version || true
cargo --version || true
pkg-config --modversion gstreamer-1.0 || true
pkg-config --modversion libpipewire-0.3 || true
pkg-config --modversion libusb-1.0 || true
command -v distrobox || true
command -v toolbox || true
podman ps -a --format '{{.Names}} {{.Status}}' 2>/dev/null || true
```

On immutable systems, choose an existing compatible development container before installing anything. Confirm its OS and packages; container runtime libraries must remain compatible with the host where the binary executes.

## Phase 5: Baseline source verification

Run the complete automated gate from `docs/testing.md`. Record exact failures before modifying code.

If compilation fails in a native `-sys` crate, establish whether development metadata is missing before diagnosing Rust source.

## Phase 6: Identify capture facilities

Desktop:

```bash
systemctl --user status xdg-desktop-portal.service --no-pager || true
systemctl --user status xdg-desktop-portal-gnome.service --no-pager || true
systemctl --user status plasma-xdg-desktop-portal-kde.service --no-pager || true
gst-inspect-1.0 pipewiresrc videoconvert videoscale
```

Gaming Mode:

```bash
systemctl --user status gamescope-session-plus@steam.service --no-pager || true
pw-dump | rg -n 'gamescope|media.class|node.name' || true
```

Do not use the Desktop portal backend in a Gamescope session that does not provide the portal.

## Phase 7: Non-invasive runtime baseline

Before opening USB, test capture only:

```bash
./target/release/logig560 capture-test --frames 30
```

or, in Gaming Mode:

```bash
./target/release/logig560 capture-test --gamescope --frames 30
```

Confirm frames are non-black for visible content and dimensions are plausible. The command prints sampled colors but saves no images.

Then test static USB colors with every live instance stopped. Only after capture and USB are independently correct should you run the full engine.

## Phase 8: Live acceptance

Check:

- ordinary bright motion;
- harsh color changes;
- extremely dark/static content;
- lock/unlock;
- clean Ctrl-C/SIGTERM;
- missing/reconnected speakers if in scope;
- Gaming shell and a real game for Gamescope changes;
- continuous audio during USB stress.

Collect runtime metrics and exact journal timestamps. User visual confirmation is required for smoothness/flicker claims.

## Porting outcome categories

| Result | Meaning |
|---|---|
| Works unchanged | Document environment and evidence; avoid needless source branches |
| Environment-only fix | Update build/package/service docs; do not alter core behavior |
| Backend compatibility switch | Prefer explicit detection/override with tests and an ADR |
| New backend required | Normalize to `RgbFrame`; keep engine shared |
| Hardware protocol differs | Treat as a new verified device/firmware target, not a guess |

## Minimum handoff

Use [`handoff-template.md`](handoff-template.md). Include stable facts, exact commands, current service state, test results, remaining risks, and what not to undo. Exclude tokens, frames, unrelated logs, and credentials.
