# Falling-liquid jitter — 2026-08-20

"help me reduce stickiness. by making falling liquid a little more stoacastic." The mechanism was
the user's; so was the quantity, chosen from three offered: **per-cell downward flow jitter, gated
on the cell being underfull.**

## What it does

`physics::fall_flow_jitter` returns a multiplier for the vertical edge below a cell:

```
1 - strength * liquidity(donor) * (1 - h/cap) * u,   u = hash(time_seed, cell) in [0,1)
```

applied through `flux_edge_candidate`'s existing `weight` slot at the two vertical candidate
sites. Three properties, each deliberate:

- **It only ever reduces.** A multiplier above 1 could push a candidate past the one-cell-per-tick
  clamp and past the donor's available mass, leaving arbitration to correct it. Scaling down keeps
  every bound that was just enforced, and mass conservation is unaffected — a smaller transfer is
  still a transfer. Measured `mass_err` stays at 1e-9 at every strength.
- **It scales with how UNDERFULL the donor is.** A cell at capacity is column, not fall; jittering
  it would make settled liquid restless, and churn at rest is a regression this project already
  watches for. A nearly-empty cell is the leading edge of a fall, which is exactly where a
  perfectly uniform front looks synthetic.
- **It scales with liquidity**, so granular material is untouched — that has `edge_share_jitter`
  already, keyed off grain size, and this is not that.

Stateless hash of `(time_seed, cell)`, the same shape as `edge_share_jitter`, so determinism holds
and COLLECT/APPLY agree within a tick.

## Cost, measured

Water, grid 512, `--ticks 300`, ceiling 16, grading on:

| jitter | ms/frame | descent | mass_err | blocks running |
|---|---|---|---|---|
| 0.0 (default) | 30.36 | 0.02418 | 1.37e-9 | 54.9 |
| 0.1 | 33.88 | 0.02414 | 7.80e-10 | 71.5 |
| 0.3 | 40.47 | 0.02400 | 1.49e-9 | 90.7 |
| 0.6 | 42.42 | 0.02102 | 5.43e-10 | 112.5 |

**Descent is unaffected up to 0.3** (-0.7%) and costs 13% at 0.6 — so the setting does not buy its
look by moving less material until it is pushed hard. **The real cost is frame time**: 30 -> 42 ms
across the range, and the reason is visible in the last column, blocks running per tick going 55 ->
112. Jitter keeps material marginally in motion, motion is what the scheduler admits, so a livelier
fall is a more expensive one. That is a genuine trade, not an implementation artifact.

Default is `0.0`, which early-outs and is bit-identical to before the feature existed. UI slider
"Falling liquid jitter", 0..0.6.

(The `spread` column was excluded deliberately: its baseline is sampled after warmup and the bottom
quarter is differently occupied at that moment in each run, so the numbers are not comparable
across rows. Lateral effect, if any, is unmeasured.)
