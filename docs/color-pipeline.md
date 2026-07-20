# Geometry and color pipeline

## Logical zones

Every `ZoneColors` value uses this fixed array order:

```text
[left rear, left front, right front, right rear]
```

The screen is divided into four asymmetric polygons designed around the G560's front- and rear-facing light surfaces. Rear zones occupy the outer edges; front zones converge toward the center.

```text
top
┌──────────────────────────────────────────────┐
│ left rear       ╲ left front │ right front ╱ right rear │
│                  ╲           │            ╱             │
│                   ╲          │           ╱              │
│                    ╲         │          ╱               │
│                     ╲        │         ╱                │
│                      ╲       │        ╱                 │
└──────────────────────────────────────────────┘
bottom
```

For the exact normalized vertices, read `ZoneLayout::g560_default` in `src/frame.rs` or open `docs/hardware/quadrant-test.html`.

## Mask compilation

`ZoneMasks::compile` turns polygons into four sorted pixel-index vectors for a specific frame size.

The built-in G560 layout uses an integer half-open boundary rule rather than floating-point point-in-polygon checks. This guarantees:

- every pixel belongs to exactly one zone;
- no overlaps or holes;
- horizontal mirror symmetry;
- deterministic center-line ownership, including odd widths;
- consistent results across portrait, landscape, and unusual sizes.

The engine caches masks and rebuilds them only when width or height changes.

## Sampler overview

Sampling is stateless per frame and per zone. `sample_mask` in `src/sampler.rs` performs:

1. Load each assigned RGB8 pixel.
2. Convert encoded sRGB to linear RGB.
3. Calculate linear luma.
4. Apply a soft darkness visibility ramp.
5. Convert visible color to CIE Lab.
6. Weight chromatic pixels and accumulate quantized Lab bins.
7. Calculate visible-area presence.
8. Select a deterministic dominant bin.
9. Blend toward the overall mean when low-light dominance is ambiguous.
10. Apply presence, convert back to encoded sRGB8, and return one zone color.

## Darkness behavior

Default midpoint:

```text
darkness_luma = 0.015 (linear-light luma)
```

Visibility uses smoothstep from half to one-and-a-half times that midpoint:

```text
lower = 0.0075
upper = 0.0225
visibility = smoothstep(clamp((luma - lower) / (upper - lower), 0, 1))
```

This replaced the original hard threshold. Adjacent near-black values therefore fade instead of toggling abruptly between included and excluded.

Pixels with zero visibility are skipped. A zone with no meaningful accumulated weight returns exact RGB black.

## Weighting and bins

Visible pixels use:

```text
weight = min(0.25 + Lab_chroma / 128, 2.0) * visibility
```

Default quantization:

```text
Lab L bin width = 10
Lab a bin width = 12
Lab b bin width = 12
```

This favors a coherent chromatic region over isolated white/gray highlights. Ties are resolved deterministically by weight, then count, then bin key; hash-map iteration order cannot randomly select a winner.

## Sparse and ambiguous low light

The sampler tracks `visible_pixel_mass`, not just a count of pixels passing a threshold. Zone presence ramps from zero to full over the first 20% equivalent visible coverage:

```text
presence = smoothstep(clamp(visible_mass / region_pixels / 0.20, 0, 1))
```

At low peak intensity (`peak < 0.15` in linear RGB), the dominant-bin mean blends with the whole visible-region mean based on dominant weight share. Shares from 45% to 65% move smoothly from overall blend to dominant selection. This prevents two similarly weighted dark hues from alternating as unrelated winners.

## Normal transitions

The sampler returns targets; it does not sleep or remember prior frames. The writer handles temporal smoothing.

`TransitionController` converts each zone's start and target to OKLab and applies smoothstep time easing. Retargeting starts from the currently visible, hardware-confirmed color so interrupted transitions do not jump or queue.

| Normal target | Duration |
|---|---:|
| Any ordinary color change | 90 ms |
| A non-black zone becoming exact black | 200 ms for that zone |

Durations are per-zone. One zone fading to black does not slow the other three.

The 200 ms black fade reduces visible off/on flicker when extremely dark content crosses the sampler's exact-black boundary. It applies only to ordinary sampled targets.

## Safety blackouts are different

Capture loss/stall, cancellation, shutdown, and USB recovery send an out-of-band safety command. That command bypasses `TransitionController` and writes exact black immediately. Never route safety through the normal 200 ms fade.

## Color-space boundaries

| Stage | Space/format |
|---|---|
| Capture | encoded RGB8 |
| Luma and weights | linear sRGB |
| Dominant grouping | CIE Lab |
| Normal interpolation | OKLab |
| USB target | encoded RGB8 |

Only SDR behavior is currently accepted. HDR transfer functions, metadata, gamut mapping, and tone mapping are not validated.

## Relevant tests

- Complete/disjoint/mirrored masks across resolutions.
- Representative zone ownership and center boundary rules.
- Black regions return exact black.
- Dominant color ignores isolated white pixels.
- Adjacent low-light values do not toggle black.
- Low-light gray ramp has bounded adjacent change.
- Ambiguous low-light bins blend deterministically.
- Arbitrary valid frames/masks never panic.
- OKLab endpoints, midpoint, easing, interruption, and per-zone duration.
- Engine safety blackouts still preempt normal fades.

Run focused tests with:

```bash
cargo test frame::tests sampler::tests transition::tests engine::tests -- --nocapture
```
