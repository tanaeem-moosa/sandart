# The coarse-grid flow correction — 2026-08-20 (night)

The user's question, which is what this is: *"how about ... using the coarse flow to move more
material than would be safe otherwise? (right now lateral flow is bound by half the difference. but
we know some flow information from coarse sim)"* — and, on scope and safety: *"should we make it
specific to lateral flow or all flow? also let's have dampening param. coarse is by definition
approximation."*

## 1. Why the fine level cannot fix this by running harder

DrySand's lateral edges move `c_sq * damping ≈ 0.08 * 0.76 ≈ 6%` of a height difference per tick;
Water's move ~23% (`wave_params`, `physics.rs`). `PRESSURE_RATE_FULL_AT_ROWS_OF_HEAD`'s own doc
comment already records that these "sit at the CFL bound", so they cannot simply be raised.

**That bound is a local explicit-scheme stability limit.** It governs how fast *local relaxation*
may propagate information. It is not a statement about how much mass may legally move — conservation
is that statement, and the FCT limiter already enforces it exactly.

And local relaxation is a **smoother**. It kills short-wavelength error quickly and long-wavelength
error at a rate that degrades as the wavelength grows. Spreading a pile sideways across many blocks
is precisely the long-wavelength mode. This is the prediction that reframed the session:

> Running more lateral sub-steps will polish the local repose angle and barely move the pile.

which is consistent with everything the previous session measured — extra repetitions bought frame
time, not spread, and "no block reaches 8x, nothing is wide enough to ramp" (SESSION-HANDOVER
2026-08-20 evening §3). The mechanism was right and it was attached to the wrong error mode.

The coarse level is where the smooth mode is solvable. That is the entire reason multigrid exists,
and FLOW-DIRECTION.md had already measured the gap: the coarse level's flow is **2.2x more lateral**
than the fine level's on DrySand, 1.5x on Water.

## 2. Three designs, two of them wrong

### Design 1 — move the defect across the block face. **Wrong: visible seams.**

```
defect[face] = strength * ( coarse_flux[face] * t*t  -  fine_flux[face] )
```
applied as a limited flux across the single line of cells at the block boundary. It measured well
(+41% spread on DrySand for +10% frame time) and it was visibly broken on screen — the user: *"look
here I can see block edges"*, then *"the block boundaries are very visible"* with the pressure
overlay, where the block grid is an unmistakable lattice.

The seam metric (`diag_lateral_corr`: mean height step across block boundaries ÷ across interior
pairs, so 1.0 = invisible) confirmed it, and the axis correlation was exact:

| | off | lateral | vertical | both |
|---|---|---|---|---|
| DrySand lateral seam | 0.80 | **3.60** | 1.45 | **3.47** |
| DrySand vertical seam | 1.05 | 1.49 | **4.39** | **4.20** |
| Water lateral seam | 0.94 | **4.74** | 1.56 | **4.21** |
| Water vertical seam | 0.67 | 2.42 | **5.87** | **5.43** |

The whole face's defect landed on one cell-line; the block interior got nothing. SESSION-HANDOVER
2026-08-20 (evening) §5 item 3 predicted exactly this — *"distributing an inter-block flow WITHIN a
block is the step that is not free, and getting it wrong produces exactly the seams"*.

### Design 2 — spread the defect uniformly through the block. **Wrong, and worse.**

Seams did improve (3.60 → 1.76). Everything else got worse: spread blew up to 40–64 cells, which at
grid 512 is material smeared across the whole vessel rather than a pile spreading.

The user named the reason before the measurement finished: *"what if a block is partially full.
that is why we simulate the block. to get the flow. when we are using coarse flow to have more
material flow, we can't skip the fine simulation."*

**A block is not a homogeneous bucket.** It is partially full, it has a surface and a slope, and
working out where material can go inside it is the entire reason the fine simulation is run. Design
2 put material into cells that should have been empty. Both failed designs share one root: they let
the coarse level decide **where** mass goes.

### Design 3 — boost the fine solver's conveyance. **This is what shipped.**

The coarse level no longer moves any mass and decides no placement. It sets **one number per block
per axis**: a multiplier on that block's conveyance coefficient — `c_sq` on the flux solver's edges,
`alpha` on the granular CA's moves (sand's lateral transport runs through the CA, not the flux
solver, so boosting only `c_sq` would leave DrySand untouched).

```
shortfall[face] = clamp( (|coarse_want| - fine_flux·sign(coarse_want)) / |coarse_want|, 0, 1 )
boost[block]    = 1 + LATERAL_BOOST_MAX * strength * max(shortfall over its two faces)
```

Unit-free: the *fraction* of the coarse level's own transport the fine level failed to deliver. Zero
where the fine level kept up. The two axes are kept separate — a block starved sideways is not a
reason to drop material faster.

Everything else stays with the fine solver: availability, acceptor headroom, angle of repose,
capacity, all per cell, all unchanged. Mass conservation is inherited rather than re-argued, because
no new transport path exists — only a coefficient inside the existing FCT-limited solver changed.

**Bounded by construction.** `flux_edge_candidate` clamps its integrated velocity to ±1.0 whatever
`c_sq` is, so no boost can exceed the standing one-cell-per-tick transport limit. That clamp, not
the constant, is what bounds transport.

**The ledger, and the trap in it.** Both levels' realised flux is recorded at `flux_edge_apply`,
signed and per edge. `flux_edge_apply` is *not* the only transport path — sand's angle of repose
lives entirely in the granular CA (`try_move`). A ledger watching only the flux solver would read
DrySand's lateral transport as near zero and boost a deficit that does not exist.

## 3. Measured — `diag_lateral_corr`, grid 512, hourglass, 300 ticks

| material | strength | spread vs off | seam lat | seam vert | descent | frame ms | block-steps |
|---|---|---|---|---|---|---|---|
| DrySand | off | base (5.91) | 0.80 | 1.05 | +0.03653 | 16.80 | 417 |
| DrySand | 0.25 | **+27.4%** | 1.13 | 1.00 | +0.03707 | +48% | +93% |
| DrySand | 0.50 | +26.2% | 1.08 | 0.99 | +0.03727 | +49% | +95% |
| DrySand | 1.00 | +27.8% | 1.09 | 0.99 | +0.03780 | +57% | +99% |
| Water | off | base (8.77) | 0.94 | 0.67 | +0.00738 | 17.40 | 407 |
| Water | 0.25 | −6.8% | 0.99 | 1.00 | +0.00915 | +101% | +131% |
| Water | 0.50 | −3.5% | 1.00 | 0.99 | +0.00844 | +100% | +131% |
| Water | 1.00 | −8.4% | 1.00 | 1.00 | +0.01187 | +122% | +131% |

**The seams are gone.** Every ratio sits at ~1.00 — block boundaries are indistinguishable from
interior cell pairs, which is structural: the boost acts on every edge inside a block, so there is
no boundary line for it to concentrate on.

**DrySand gains +27% lateral spread**, descent unharmed, `mass_err` 1.8e-8–3.7e-8 against an
uncorrected baseline of 2.71e-8 (same order; the handover records up to 7.45e-8 for the uncorrected
solver).

**Three honest negatives:**

1. **Water gains nothing** (−3% to −8%) for +100% frame time. Its lateral conveyance is already
   ~0.235 per tick against a ±1.0 clamp, so there is little headroom to buy. The feature is for
   granular material.
2. **The strength slider barely matters** on spread: 27.4% at 0.25 vs 27.8% at 1.0, across a 4×
   range. Transport saturates against the ±1 clamp almost immediately. Strength is close to an
   on/off switch in practice, which was not the intent and is not yet understood.
3. **It costs ~50% frame time on DrySand**, and that is real scheduled physics — block-steps rise
   +93% in step with it. More material moves, so more blocks stay awake.

Design 1 measured better (+41% for +10%) than the correct design (+27% for +48%). It measured
better because it was cheating: moving mass by fiat is cheap, and the seams were the bill.

## 4. What shipped

- `physics::compute_lateral_boost` + `set_lateral_boost`, and the flow ledger (`lat_ledger_*`),
  recorded at both `flux_edge_apply` and `try_move`.
- Boost applied at four sites: the two horizontal flux edges (`c_sq * lateral_boost`), the two
  gravity-aligned ones (`c_sq * vertical_boost`), and the granular CA's `alpha`, gated on `ndx != 0`.
- `DrawingSimulation::{coarse_flow_correction, coarse_correction_damping, last_frame_correction}`.
  Default OFF. **The axis selector was removed at the user's request — both axes, always.**
- Debug panel: a checkbox, a strength slider, and a live readout. **Overclocking now defaults ON.**
- `diag_lateral_corr` — sweeps strength; reports spread, the **seam metric**, descent, `mass_err`,
  frame time and block-steps.
- `coarse_flow_correction_toggle.rs` — five tests: off-by-default is bit-identical, on diverges,
  `strength = 0` equals off, mass is conserved at full strength, and the boost is never below 1.0.

### The UI bug, separately

The toggle "never turned off" and the strength slider "did not work" because I never registered
`syncSettings` listeners for either control. `syncSettings()` is driven entirely by explicit
listeners, so an unregistered control changes nothing until some *other* control fires a sync — at
which point it picks up whatever the unregistered control was left at, which reads exactly as a
toggle that only ever turns on. Fixed.

## 5. Next

1. **Understand the saturation.** Strength moving 4× for 0.4% of spread means the ±1 clamp is
   binding almost immediately. Either the boost should target the clamp rather than `c_sq`, or the
   useful range is far below 0.25 and the slider should be rescaled.
2. **The frame-time cost needs attention before this is a default.** +48% on DrySand is a lot for
   +27% spread, and the sub-linear cost model (SESSION-HANDOVER §2) says the lever is block-steps.
3. **Water should probably not run this at all.** It buys nothing and costs double.
4. **Selective coupling is still cheap to try** — the ledger is the per-block directional quantity
   SESSION-HANDOVER §5 item 4 was waiting on.
