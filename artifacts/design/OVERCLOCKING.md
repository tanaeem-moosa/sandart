# Over/underclocking — measurements, 2026-08-20

Build step 4 of `artifacts/design/HIERARCHICAL-PRESSURE.md` (§7b). Written by the main thread; the
implementing agent hit its session limit before reporting, so everything here was measured directly
against the committed tree.

## Why this exists

Transport is clamped at `.clamp(-1.0, 1.0)` in `flux_edge_candidate` — **one cell per step, at every
resolution**. A driving potential can only change *where* the <=1 cell/tick of movement happens; it
cannot make more of it happen. That is why the coarse `eta` coupling measured ~10% on pool levelling
for 67% of frame time. **Sub-stepping is the only lever that raises the transport rate.**

## What it does

Per-block clock rate `2^idx`, `idx` clamped to give rates in `[1/8, 8]`, derived from
`|Delta[b]|/capacity[b]` (coarse-fine disagreement, one coarse cell per LOD block since
`block_size == grid_size/64`) and staleness. Powers of two so clock domains nest rather than beat,
one octave of change per tick, hysteresis of 2 octaves so a block sitting near a level boundary
cannot bounce. Staleness can only ever push a rate UP, never suppress one.

`coarse_pressure_coupling` was split: it now gates **only** the driving-potential half, and defaults
OFF. The coarse level's own tick (restrict / anchor / advance / export `eta` + `Delta`) runs
unconditionally, because the scheduler needs `|Delta|` whether or not the potential coupling is on.

`overclocking_enabled` is a separate debug toggle, default OFF.

## Measured, grid 512, `diag_blocks --ticks 300`

| material | clocking | ms/frame | centre-of-mass descent | blocks run | mass_err |
|---|---|---|---|---|---|
| Water | off | 23.17 | 0.00738 | 406.7 | 2.20e-9 |
| Water | **on** | **120.10** | **0.06142** | 492.0 | 8.95e-10 |
| DrySand | off | 13.60 | 0.03653 | 417.3 | 2.71e-8 |
| DrySand | **on** | **81.88** | **0.07450** | 411.5 | 5.78e-9 |

**It works.** Water moves material **8.3x faster** per tick. DrySand 2.0x. This is the first change
in this whole effort that moves material substantially faster, and it confirms the clamp analysis:
the constraint was never the driving head, it was the one-cell-per-tick transport limit, and only
sub-stepping lifts it.

**Mass conservation is intact** — `mass_err` did not degrade under multi-rate scheduling; it
improved slightly in both materials. This was the failure mode most likely to bite (§7b's S3: edges
are owned by their lower-index cell, so half of every clock-domain boundary would otherwise run at
the slow rate by grid geometry rather than physics).

**The cost is the problem, exactly as the user predicted.** 120 ms/frame is ~8 fps. Note the block
COUNT barely moved (406.7 -> 492.0, +21%) while frame time went up 5.2x — the cost is sub-steps
within blocks, not more blocks admitted. Also note the stale tier collapsed to 0.0, because
overclocked blocks are running often enough that nothing reaches the staleness floor.

Throughput is favourable — 8.3x the movement for 5.2x the time, so ~1.6x more work per millisecond —
but the absolute frame time is not shippable. **This is what the performance stage exists to fix**,
and the target is now concrete: roughly 4-5x on the per-block sub-step.

## Not measured

- Pool levelling ticks-to-50% and U-tube riser rate with clocking on (`diag_overclock_ab` exists but
  its run was killed; centre-of-mass descent above is the proxy that stands in for it).
- Settled churn with clocking on — the oscillation guard (baseline 0.000025/cell/tick).
- The rate distribution across blocks: how many sit at 8x, 1x, 1/8x. `diag_blocks` reports only
  totals. This matters because it says whether underclocking is doing anything at all, and the
  project has already measured 100% of material-bearing blocks sitting in the MUST tier — meaning
  there may be nothing to underclock in an active scene.

## Verification

Lib suite 102 passed / 10 failed (the same ten named failures). All eight integration suites pass,
including the new `overclocking_toggle` and `perfect_simulation_determinism`. `sandart-render`, the
wasm32 check, `cargo check -p sandart` (desktop) and `node scripts/check_js.js` all clean.
