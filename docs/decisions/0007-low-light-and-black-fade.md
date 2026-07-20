# ADR 0007: Make low-light sampling continuous and normal black slower

- Status: Accepted
- Date: 2026-07-20

## Context

Near the original hard darkness and visible-coverage thresholds, adjacent dark frames could alternate between black and a selected dominant bin. At very low brightness, the G560's final transition to physically off could still look like a blink even with the normal 90 ms transition.

## Decision

- Replace the hard darkness cutoff with a smooth visibility ramp around `darkness_luma`.
- Scale sparse visible coverage continuously over the first 20% of a zone.
- Resolve low-light bin ties deterministically and blend ambiguous dark bins toward the overall mean.
- Keep normal transitions at 90 ms, but give each non-black zone a 200 ms duration when its sampled target becomes exact black.
- Leave safety blackout unchanged and immediate.

## Consequences

- Very dark content fades smoothly toward off.
- A brief threshold crossing is less likely to present as an off/on blink.
- Per-zone durations prevent one dark zone from slowing bright changes elsewhere.
- Sampler behavior remains stateless; temporal behavior stays in the transition layer.

## Evidence

Regression tests cover adjacent RGB 32/33 values, monotonic low-light ramps, ambiguous bins, per-zone transition completion, and safety preemption. The user confirmed the result “looks good.”
