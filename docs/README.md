# G560 Linux Utility documentation map

Use this page to choose the shortest path to the information you need.

## First-time orientation

| Document | Use it for |
|---|---|
| [`../AGENTS.md`](../AGENTS.md) | Safe repository workflow and hard invariants for coding agents |
| [`quick-reference.md`](quick-reference.md) | One-page commands, constants, diagnostic split, and “do not” list |
| [`project-status.md`](project-status.md) | Current completion state, accepted environments, limitations, and working-tree status |
| [`architecture.md`](architecture.md) | End-to-end design, tasks, channels, state transitions, and source map |
| [`glossary.md`](glossary.md) | Project-specific terms such as portal grant, Gamescope node, stall, target, and safety blackout |
| [`machine-porting.md`](machine-porting.md) | Fact-gathering and acceptance checklist for an unfamiliar machine |
| [`handoff-template.md`](handoff-template.md) | Structured template for transferring work to another agent/machine |

## Build and operation

| Document | Use it for |
|---|---|
| [`build-and-run.md`](build-and-run.md) | Dependencies, Fedora/Bazzite setup, CLI command reference, config paths |
| [`configuration.md`](configuration.md) | Persisted token, environment switch, compiled constants, and service paths |
| [`operations.md`](operations.md) | Desktop and Gaming Mode services, logs, metrics, session switching, clean stop |
| [`troubleshooting.md`](troubleshooting.md) | Symptom-first diagnosis and evidence collection |
| [`testing.md`](testing.md) | Automated suites, hardware gates, and change-specific test matrix |

## Technical internals

| Document | Use it for |
|---|---|
| [`capture-backends.md`](capture-backends.md) | Desktop portal/GStreamer and Gaming Mode direct PipeWire capture |
| [`color-pipeline.md`](color-pipeline.md) | Four-zone geometry, low-light sampler, color conversion, and fades |
| [`usb-and-safety.md`](usb-and-safety.md) | G560 HID protocol, pacing, worker, recovery, and blackout semantics |
| [`development.md`](development.md) | How to modify or extend each subsystem without violating invariants |
| [`dependencies.md`](dependencies.md) | Rust crates, native packages, runtime services, and upgrade checklist |
| [`privacy-and-security.md`](privacy-and-security.md) | Data handling, privilege boundaries, saved state, threat-conscious practices |
| [`known-limitations.md`](known-limitations.md) | Explicit prototype boundaries and future work |

## Decisions and evidence

- [`decisions/README.md`](decisions/README.md) indexes the current architecture decision records.
- [`hardware/g560-zone-map.md`](hardware/g560-zone-map.md) records the physically verified protocol-to-zone mapping.
- [`hardware/milestone-1-results.md`](hardware/milestone-1-results.md) is the Fedora milestone-one acceptance record.
- [`../HANDOFF_BAZZITE.md`](../HANDOFF_BAZZITE.md) is the Bazzite port and final prototype acceptance record.
- `superpowers/` and `../.superpowers/` preserve historical implementation plans and reviews. They explain how the original milestone was developed but may describe superseded behavior.

## Authority order

When documents disagree, use this order:

1. Current source and tests.
2. Root [`AGENTS.md`](../AGENTS.md) invariants.
3. Current topic documents in this directory.
4. `README.md` and `HANDOFF_BAZZITE.md`.
5. Historical milestone results, plans, and review artifacts.

Update the current documents when source behavior changes so agents rarely need to resolve such conflicts.
