# Hierarchical pressure — design

Status: **design only, nothing built. Reviewed once and substantially revised; two load-bearing
claims of the first draft were wrong.** Written 2026-08-18 against `3bb6533`. Every number is
measured on this tree unless marked derived.

**Read §0 first.** One unresolved question (§0.1) can kill this whole approach, and it is cheap to
settle before any code is written.

---

## 0. What review changed, and what must be settled first

The first draft claimed three things that carried the design. One survived.

| claim | verdict |
|---|---|
| Coarsening reduces representable depth, so hierarchy escapes the §1 budget | **WRONG.** Representable depth is invariant under coarsening (§0.1). Hierarchy buys *rate only*. |
| `P[C]` is a tile-constant that cancels within a tile, so there is no double-counting | **WRONG.** It cancels *compression* and double-counts *elevation*, producing a sawtooth at every horizontal tile seam (§0.2). |
| Coarse lateral relaxation is outside the feedback loop, so `N` is free | **WRONG.** The relaxation operator is composed into the loop; `N` is a spatial gain on the feedback (§6). |
| The overfill model derives free-fall/supported rather than imposing it, and that survives coarsening | **HOLDS.** Measured, §3. |

### 0.1 The question that decides whether this is worth building

Representable depth in *physical* rows is `o_max * unit / base_head` — **independent of tile size**.
The coarse cell's `o` is a compression *fraction*, `capacity[C]` is the sum of its fine capacities,
and the head over the column is unchanged, so the demanded `o` at the base of a 300-row column is
**2.43 at every level**, against a ceiling of `o_max = 0.90`. Coarsening does not move that.

If deep coarse tiles pin at `o_max`, then `P` is spatially constant there, `P[C] - P[D] = 0`, and
**the driving force vanishes at exactly the depth it exists to supply** — the same pinning pathology
HANDOVER §9 records at the fine level, moved up one level. `Delta` would also sit permanently at its
maximum, since `A` can never reach `M` when both are bounded by the same ceiling.

**Test this before writing any coupling** (build step 2, §9): print `max(o)` over coarse tiles for a
300-row pool. If it pins, this approach needs a different pressure law at the coarse level — one
whose potential is unbounded in compression — and the rest of this document is premature.

What hierarchy *does* buy, and it is still worth having, is **rate**: `tau ~ base_head/unit` per
level, and the coarse level covers `t` fine rows per level row. The honest speedup is **linear in
`t`** (4x/8x/16x for t=4/8/16), not the 6x/12x/25x the first draft's table claimed.

### 0.2 The coupling must supply HEAD, not pressure

The fine vertical solve already applies `base_head` on every vertical edge. If the coarse relax
applies `base_head * t` per coarse row and `P[C]` is injected piecewise-constant into `phi`, then
across a horizontal tile seam the fine solver sees

```
phi_b - phi_a = base_head - (P_D - P_C) = (1 - t) * base_head
```

which at `t = 8` is `-7 * base_head`. Net compression over one tile period is **exactly zero** (the
`t-1` interior edges and the one seam edge cancel), so the fine field becomes a **sawtooth of
amplitude `(t-1)*base_head/unit` = 0.056 in `o`, repeating every 8 rows**, locked to block
boundaries — and the seam edge is driven *upward*, a gap-opening force at `y % block_size == 0`.
This codebase already has instruments for that exact signature
(`diag_flip_release_front_and_block_alignment`).

**Fix, and it is not optional:** the coarse level supplies **hydraulic head** `eta = z + p/(rho g)`,
with elevation removed, and the fine cell reads `P_fine = eta[tile] - z_fine`. `z_fine` varies per
fine row, so the term is smooth within a tile and the elevation head is stated once, not twice.
This is the same formulation HANDOVER §3 gives for the head field — reached here from the opposite
direction, and it is the only part of that work this design reuses.

---

## 1. The invariant that forces the design

For the overfill model, with `base_head` the gravity head per row
(`gravity_dir.y * GRAVITY_HEAD_SCALE` = 1.0 at the shipped gravity):

- **Representable depth** `R = o_max * unit / base_head` — rows of depth before the compression
  that hydrostatic equilibrium demands exceeds the overfill ceiling.
- **Saturated transfer** `tau ~= base_head / unit` — cells per tick moved into already-packed
  material, from solving the equilibrium transfer with the acceptor at capacity.

```
R * tau = o_max
```

`unit`, `base_head` and grid resolution cancel. **Stiffness does not** — `o_max = 4.5/s`, so a softer
fluid gets a bigger budget; only `p_max` is stiffness-free. And both sides are small-`o`
linearisations of `phi = h/cap + unit*(o + o^2/o_max)`, each wrong in the opposite direction: the
exact head to reach the ceiling is `o_max*(1 + 2*unit)/base_head`, roughly **2R**, because the
quadratic term doubles the potential at `o = o_max`; and `tau` is quoted for the most favourable
configuration (acceptor exactly at capacity), falling ~4x once the acceptor is itself compressed.
The product is right by construction, which is why it should be read as an identity relating two
definitions, not as a conservation law. Verified against measurement:

| grid | unit | R (rows) | tau (measured) | R*tau | o_max |
|---|---|---|---|---|---|
| 512 | 125.0 | 112 | 0.00775 | 0.87 | 0.90 |
| 128 | 31.2 | 28 | 0.02838 | 0.80 | 0.90 |

**Depth and speed are the same budget.** To represent 500 rows you must accept
`tau <= 0.0018` cells/tick. To get `tau = 0.5` you get 1.8 rows. This is why the stiffness dial
fails in both directions, and why "let it compress first, then stiffen" is the only winning
schedule — it is asking to spend the budget twice, sequentially, which no constant can express.

Two corollaries worth stating because both have been tried:

- **The stiffness dial cannot change the maximum representable pressure**, over the range where
  the ceiling formula is not clamped. `p_max = unit * 2 * o_max`, and
  `overfill_ceiling_for(s) = 1 + 4.5/s` clamped to `[1.10, 3.0]`, so
  `p_max = 9 * GRAVITY_HEAD_SCALE * w/512` — stiffness-free for `s` in `[1.5, 45]`. Measured at
  w=128: 56.2 / 56.3 for stiffness 5 / 20. At `s = 60` the clamp binds (`o_max = 0.10`) and
  `p_max = 75.0`, a 33% departure — the cancellation is a property of the formula, not a law.
- **Raising the ceiling alone does nothing**, because the ceiling is not what binds — the rate is.
  Measured, same pool, ceilings 1.9 / 5.0 / 12.0: `o` at the bottom row 0.150 / 0.161 / 0.163,
  free surface unchanged, mass identical.

### What this costs today

| grid | pressure reached after 1500 ticks (fraction of hydrostatic) |
|---|---|
| 128 | 24% |
| 256 | 13% |
| 512 | **6%** |

The *demand* is resolution-invariant (equilibrium `o = 2.43` at all three). Only the rate degrades,
exactly as `1/w`. A pool at 512 is carrying 6% of its own hydrostatic pressure, so the lateral
driving head at depth is ~16x too small. That is the U-tube not rising and the pile not settling.
Establishing hydrostatic pressure costs `~unit` ticks per row of depth: **125 ticks/row at 512**,
and that is the optimistic figure — see the `tau` caveat above; at depth it is several-fold worse.

`tau` is per edge per *sweep*, so uniform sub-stepping (HANDOVER §10's own first candidate) multiplies
it without touching `R`. This identity constrains what a single level can do in one sweep; it does
not constrain every possible fix.

---

## 2. What hierarchy actually buys

**Not because it escapes the §1 budget — it does not (§0.1).** Because `tau` is per level per sweep,
and one coarse sweep advances mass across `t` fine rows. The gain is in **rate**, linear in `t`.

| pressure grid (512 sim) | tile | rows in a 300-fine-row column | rate vs today (derived, linear in t) |
|---|---|---|---|
| 512 (today) | 1x1 | 300 | 1x |
| 128 | 4x4 | 75 | 4x |
| **64** | **8x8** | **37** | **8x** |
| 32 | 16x16 | 18 | 16x |

These are derived, not measured, and they assume the coarse field is equilibrated — which §C2 below
says it will not be at `N = 8`. Treat them as an upper bound.

Coarser is strictly better for the invariant and strictly worse for geometry. The bound is that a
tile must still resolve the features that carry flow — the hourglass neck and the U-tube's tube.
At the shipped `neck_width = 0.005`, `multistage_neck_half_width` gives a half-width of
`0.005 * 512 = 2.56` cells, so the neck is **~5 cells wide** at 512. An 8-wide tile boundary sees
5 of 8 cells open, which is resolvable *provided the coarse edge carries a conveyance fraction
rather than a binary open/closed* (§4). A 16-wide boundary would read the neck as a minority of a
mostly-wall tile.

**Decision: the pressure grid is 64x64, fixed, at every render resolution.** Fixed grid rather than
fixed tile size, because `R` must be resolution-invariant — that is the entire point. The tile is
then `grid/64` fine cells: 8x8 at 512, 4x4 at 256, 2x2 at 128.

### The LOD block and the pressure cell are the same object

`block_size` is currently `grid_size/32` (32x32 = 1024 blocks at every resolution). Changing it to
`grid_size/64` makes the LOD block *identical* to the pressure tile: 8x8 cells at 512, 64x64 = 4096
blocks. One hierarchy, one restriction pass, one activity structure.

Consequences to accept deliberately:

- 4096 blocks instead of 1024. The per-block classification loop is currently 0.01 ms; 4x that is
  still negligible. `budget_n` and `BUDGET_MIN` are counts of blocks and must be rescaled 4x
  (256 -> 1024 start, 32 -> 128 floor) or they silently become a 4x tighter throttle.
- At 128 the block is 2x2 cells, and `physics.rs` already documents a slab artifact at
  `block_size = 2` ("material moving multiple cells per tick while `block_size` is 2 cells"). This is
  not only a scheduling-overhead question.
- **At grid 64 — a shipped resolution — the scheme degenerates**: `t = 1`, the coarse grid *is* the
  fine grid, `P[C]` is the cell's own overfill pressure added to its own potential (literal 2x
  double counting), and `block_size` becomes 1. The coarse level must be disabled below some grid
  size, or `t` floored at 2 with the pressure grid allowed to be finer than 64.
- **Wake reach is `block_size` cells per tick.** `activate_neighbor_upstream`/`_side` wake the
  *adjacent block*, so halving `block_size` halves how far activation travels per tick.
  `test_sandbox_wave_reach_is_budget_independent` is the existing check on this. It is a propagation
  change, not a scheduling detail.
- `BUDGET_STEP_DOWN`/`BUDGET_STEP_UP` are block counts too and must scale 4x, or the adaptive
  controller's response rate drops 4x. `budget_n = 256` appears twice (construction and `reset`).

---

## 3. The property that must survive, and the evidence it does

**Free-falling material carries exactly zero pressure today: 0 of 9,647 falling cells.** This is
structural, not tuned: `o > 0` requires `h > cap`, and material can only exceed capacity if
something stops it descending. **Compression is support.** The overfill model *derives* the
free-fall/supported distinction; the head field had to *impose* it and could not
(`test_spec_free_fall_is_pressureless_throughout`, ignored: "support is not transitive"). That is
why every previous attempt at U-tube flow broke the hourglass, and why this design keeps overfill
as the pressure model rather than replacing it.

Measured at 512, after 400 ticks, does coarsening create spurious pressure in a falling stream:

| pressure tiles | tiles holding material | pressurised | falling-dominated | **falling AND over capacity** | worst fill in a falling tile |
|---|---|---|---|---|---|
| 128 (4x4) | 3310 | 2343 | 659 | **0** | 0.9809 |
| 64 (8x8) | 888 | 605 | 181 | **0** | 0.9797 |
| 32 (16x16) | 240 | 148 | 50 | **0** | 0.9695 |

Same check on `UTubeFlowThrough` at 64x64 tiles, 800 ticks: 1673 pressurised, 345
falling-dominated, **0** spuriously pressurised.

The margin is thin — 0.981 against 1.0 — so §6 requires a deadband.

---

## 4. Coarse geometry: computed once, from the mask

Rebuilt only when `shape_mask` is rebuilt (shape, neck width, chamber count, grid size). Never per
tick. Stored alongside `shape_mask`.

For each coarse cell `C` covering a `t x t` fine tile (`t = grid/64`):

```
open_cells[C]  = count of fine cells in C with shape_mask != MASK_OUTSIDE
capacity[C]    = sum over those cells of cell_capacity_for(wetness)     // see note
inside[C]      = open_cells[C] > 0
```

For each coarse edge `e = (C, D)` sharing a `t`-cell boundary:

```
open_span[e]   = count of fine cell PAIRS (a in C, b in D) that are orthogonally adjacent
                 across the shared boundary and both mask-inside
k[e]           = open_span[e] / t                                        // in [0, 1]
```

`capacity[C]` is the exception to "computed once": `cell_capacity_for` depends on wetness, which
advection changes continuously, so it is either cached (and wrong wherever material has mixed) or
recomputed per tick (and not free). **Decide explicitly.** A defensible middle is to recompute it on
the same cadence as the saturation deciles rather than per tick, and to accept staleness bounded by
that cadence.

`k[e]` is the conveyance fraction — this is the "x percent flow" that lets a coarse edge represent
a thin neck. At 512 with `t = 8` and a 5-cell neck, `k = 5/8 = 0.625`. A fully open interior edge
has `k = 1`. A wall has `k = 0`.

**Connectivity guarantee.** `k[e] > 0` if and only if at least one fine-cell pair crosses that
boundary. So the coarse graph is connected exactly where the fine graph is: no path is invented and
none is lost. This is the property that makes the coarse solve trustworthy, and it must be asserted
in a test against a flood fill of the fine mask (§8).

**`k[e]` is necessary but NOT sufficient for necks.** Two reasons, both fatal on their own if
unaddressed:

- **Conveyance sets the rate, not the equilibrium.** TASK55-MULTIGRID states this for the deleted
  pass: "conveyance only affects the RATE of approach to equilibrium, not the equilibrium value
  itself." So a large `N` washes `k` out and the coarse field converges to the neck-blind answer —
  which is the mechanism behind `acf2da4`'s "flat surfaces across actively draining necks". `k` and
  large `N` are mutually destructive.
- **A neck interior to a tile has no coarse edge to throttle.** At `t = 8` the neck's row is
  arbitrary, so roughly `(t-1)/t` of the time the constriction lies *inside* one coarse cell and
  disappears from the coarse model entirely.

Neither is solved by this design as written. The likely shape of a fix is an intra-cell conveyance
(a per-coarse-cell throughput limit derived from the narrowest fine cross-section within it, applied
to that cell's total outflow), but that is unbuilt and unmeasured. **Do not treat necks as handled.**

**Diagonal necks.** A neck at 45 degrees can cross a tile corner with zero orthogonally-adjacent
pairs on either shared edge, giving `k = 0` on both while the fine grid is connected. The fine
solver itself only moves mass orthogonally, so a diagonal-only connection carries no fine flow
either and `k = 0` is correct. **This must be verified, not assumed** — it is the most likely place
for the connectivity guarantee to be wrong.

---

## 5. The tick

Per tick, in order. Only steps 2-4 are new.

**1. Restrict.** `A[C] = sum of fine h over tile C`. **This is not free and cannot piggyback on
COLLECT**, for four reasons in `settle_tick`: COLLECT is where `P` is *consumed*, so producing `A`
there is circular (accept a one-tick-stale `P` and say so, or pay for a pass); COLLECT is
block-scheduled (`if !will_simulate[b] { continue; }`) so a resting tile's `A` would be up to
`MAX_STALENESS = 30` ticks stale; the block loop runs once per phase with heights mutated in
between, so "during COLLECT" is ambiguous and would double-count; and the whole thing is skipped by
the quick-exit when no block is active. **Decision required:** own pass over the grid (cost it), or
one-tick-stale `A` (then the loop in §6 has an extra unit of lag and I1/I6 must be re-derived).

**2. Anchor.** The coarse mass state `M[C]` is pulled toward the real aggregated mass:

```
M[C] += lambda * (A[C] - M[C])
```

`lambda` in `(0, 1]` is **the single damping knob in the whole design** and the only place the
feedback loop can be tuned. `lambda = 1` makes the coarse level memoryless (and reintroduces the
head field's brittleness). `lambda -> 0` makes it ungrounded. See §6 for its stability bound.

**3. Relax.** Run `N` sweeps of the *existing* overfill equilibrium transfer on `M`, over the coarse
graph, with per-edge conveyance `k[e]` scaling the transfer and coarse gravity head
`base_head_coarse = base_head * t`. This moves **coarse** mass only. `M` is not real mass, so no
conservation obligation is involved and the sweeps may be as aggressive as convergence allows.

`N` is affordable: at 64x64 the coarse grid is `1/t^2` of the fine grid, so 64 coarse sweeps at 512
cost the same as one fine sweep.

**4. Read back.** Two quantities per coarse cell:

```
P[C]     = overfill pressure of M[C] against capacity[C]      // the depth term
Delta[C] = M[C] - A[C]                                        // signed disagreement
```

**5. Fine solve (existing, one term added).** The fine potential gains the coarse depth term:

```
phi_fine(h, cell) = h/cap + gain * local_overfill(h, cap) + P[tile(cell)]
```

The fine equilibrium transfer, arbitration, advection and copy-back are **unchanged**.

---

## 6. Oscillation: what is guaranteed, and what is now known not to be

The current model does not oscillate for two specific reasons, established by task #70's fix:
pressure is an **instantaneous** function of local compression (zero lag) and the transfer is
**solved to equilibrium** (zero overshoot). Memory reintroduces lag, so each must be re-established
deliberately rather than hoped for.

**I1 — No overshoot at the fine level, by construction.** `P[C]` enters the **potential**, never the
flux. `phi` stays monotone in `h`, so `overfill_equilibrium_transfer` still lands exactly on the
fixed point of the *combined* potential, and the closed-form solver still applies unchanged. This is
the load-bearing structural decision of the whole design: **the coarse field must never contribute a
flux, a velocity, or a height.** `acf2da4` (the deleted multigrid pass) moved heights directly and
was visually refuted — water too fast, falling water drifting sideways, flat surfaces across
actively draining necks.

**I0 — `P` and `Delta` are different couplings and the design must pick one.** The first draft
coupled the fine solver to `P` (§5 step 5) while arguing self-correction and anti-overshoot about
`Delta` (I2, I4). Those are not the same quantity: at perfect agreement `Delta = 0` but `P` is at
its maximum and still driving, and I4's budget would scale `P` to zero exactly when the coarse level
is correct — deleting the depth term §7 says the coarse level owns. **Unresolved. The two candidate
designs are:** (a) `P` as a persistent depth term, in which case I2/I4 do not apply and it needs its
own stability argument; or (b) `Delta` as a transient corrector, in which case §7's ownership table
is wrong and the fine level must still earn depth pressure itself. This must be decided before
anything is built.

**I2 — The forcing vanishes at agreement.** `Delta[C] = M[C] - A[C]` is a disagreement signal. As the
fine solver moves mass toward what the coarse level implies, `A -> M` and the forcing goes to zero.
Self-correction is a property of the signal's definition, not of a tuned constant.

**I3 — WITHDRAWN. The relaxation is inside the loop, and `N` is a gain on it.**

The first draft claimed the loop was

```
P[C]  ->  fine transfer  ->  fine mass  ->  A[C]  ->  (anchor)  ->  M[C]  ->  P[C]
```

with coarse relaxation outside it because it moves no real mass. That is wrong: relaxation is the
operator mapping `M` to the `M` that `P` is read from, so it is *composed into* the loop:

```
M_{k+1} = R_N( (1-lambda) M_k + lambda A_k ),    A_k = A( A_{k-1}, P(M_k) )
```

`||R_N||` grows with `N`: at `N = 0` one tile's error perturbs one tile's `P`; at large `N` it
perturbs an `O(N)`-tile neighbourhood, so one tile's fine-level overshoot moves mass across that
whole neighbourhood in the same tick and the entire region's response returns through the anchor.
**`N` is a spatial gain on the feedback, not a free parameter.**

Two further couplings the first draft missed:

- **`N` and `lambda` are jointly constrained, from both sides.** The anchor pulls `M` back toward
  `A` by `lambda` every tick, so for the coarse field to *hold* a profile `A` does not have — the
  entire point — the `N` sweeps must out-run the anchor. There is a joint constraint, not a single
  scalar bound.
- **Differential-mode gain is 2x common-mode.** Mass crossing a tile boundary changes `A` on both
  tiles in opposite directions, so `Delta` changes by `2m`. The mode the coarse gradient actually
  drives has twice the gain of a scalar analysis, so any scalar-derived `lambda` bound is 2x loose.

**I4 — Per-tile flux budget (anti-overshoot at the coupling).** The total real mass the coarse term
causes to leave tile `C` in one tick must not exceed `|Delta[C]|`. Enforced by scaling the coarse
contribution to `phi` down when a tile's realised outflow would exceed its own disagreement. Without
this, the fine level can move more than the coarse level asked for, `Delta` changes sign, and the
loop rings — the exact failure mode §9 of HANDOVER describes for the pre-#70 solver.

**I5 — Deadband, to protect the hourglass — and the measured margin is already too thin.**
`P[C] = 0` unless `M[C]/capacity[C] > 1 + eps_dead`. The worst measured falling-dominated tile sits
at **0.9809**, so the available margin is 0.0191 — *narrower* than the `eps_dead ~= 0.02` proposed to
guarantee it, and the trend runs the wrong way as tiles get smaller (0.9695 at 16x16, 0.9797 at 8x8,
0.9809 at 4x4). At the fixed 64x64 grid, `t = 4` at 256 and `t = 2` at 128, neither measured. One
scenario's worst observed value is not a bound. **The deadband must be derived from the free-fall
condition (a falling cell has room below, so its tile cannot exceed capacity unless it also contains
supported material), not fitted to an observation**, and the mixed-tile case — part stream, part
pool — is the one that decides it.

**I7 — Liquid only, and the granular yield criterion must be re-derived if not.** HANDOVER §3
records this as non-negotiable for the head field: "the field has no yield criterion, so applying it
to granular material flattens a resting pile's angle of repose." Worse here: the lateral granular
path derives its Mohr-Coulomb yield from the **fine** pressures
(`normal_p = 0.5*(p_a + p_b)`, `tau_overfill = mu * normal_p * ...`). Adding `P` to the driving
potential without adding it to `normal_p` grows the driving term by up to `p_max` while the yield
stress stays at the fine level's ~6%-of-hydrostatic value, violating the yield criterion by
construction and collapsing dry sand's repose angle at depth. Gate on
`liquidity(wetness) >= LIQUID_ELLIPTIC_THRESHOLD` at both endpoints, as the head field does, until
the granular case is designed deliberately.

**I8 — The LOD scheduler must see `P`.** A settled pile is exactly the case where blocks go
Inactive, and block activation is driven by last tick's displacement and head difference with no `P`
term. Without this, `P` builds in the coarse field while the fine cells that should respond are
asleep, waking only via the 30-tick staleness path. A coarse tile whose `|P|` exceeds a threshold
must mark its block active.

**I6 — Conservation is untouched.** Real mass moves only through the fine edge solver, which is
conservative by construction. `M` is a modelling variable. No reconciliation pass, no fixups.

---

## 7. The split: what each level owns

If both levels supply the depth term, the same physical pressure is applied twice and the symptom is
"water moves too fast" — indistinguishable from the failure that refuted `acf2da4`.

| | fine (per cell) | coarse (per tile) |
|---|---|---|
| local compression | **owns** | — |
| free-fall / support discrimination | **owns** (structural: `o > 0` requires `h > cap`) | inherits via I5 |
| depth / long-range hydrostatic | — | **owns** |
| moves real mass | **owns, exclusively** | never |
| conservation | **owns** | n/a |

The fine model keeps its local term because that is what makes free-fall pressureless and keeps the
equilibrium solve exact. The coarse level supplies only what the fine level provably cannot earn in
reasonable time: 125 ticks per row of depth at 512.

**RESOLVED, against the first draft.** The cancellation argument — that `P[C]` appears on both sides
of `phi_a - phi_b` for a same-tile edge and drops out — is correct *as algebra* and checks the wrong
quantity. No **compression** is double-counted. **Elevation** is, because the fine solver already
applies `base_head` per vertical edge and the coarse relax applies `base_head * t` per coarse row.
See §0.2 for the sawtooth this produces and the fix: the coarse level must supply **head**
(`eta`, elevation removed) and the fine cell reads `eta[tile] - z_fine`, which varies smoothly
within a tile and states the elevation head exactly once.

---

## 7b. Advection, and sub-stepping as the intra-tile half of the hierarchy

**Advection never sees the coarse level.** Real mass moves only through fine edges, and
`flux_edge_apply` calls `advect_properties` with the realised flux, so colour and the four material
properties transport at full fine resolution. This is a direct consequence of I1/I6 and it is the
reason the design does not trade visual fidelity for propagation speed. `acf2da4` moved heights at
coarse resolution, which advects colour at coarse resolution; "falling water drifted sideways" is
what that looks like.

Second order, and it points the favourable way: `advect_properties` blends by
`flow / (h_dst + flow)`, so numerical diffusion accumulates per *transfer event*, not per unit of
mass moved. Fewer, larger transfers over a journey therefore smear *less* than many small ones — a
faster solver should sharpen colour boundaries, not blur them.

Measured, two-tone column at grid 256, equal simulated progress, sub-steps 1/2/4/8: boundary smear
118 / 117 / 116 / 114 rows. **Sub-stepping does not cost colour fidelity.** (The instrument is
blunt — the boundary smears across most of the column within 25 ticks regardless — so this is
evidence of no penalty, not a resolution of small differences.)

### The intra-tile bottleneck, and why sub-stepping is its natural fix

The hierarchy splits by wavelength. The coarse field carries signal *between* tiles. *Within* a
tile, propagation is still one fine cell per tick, and a tile is `t = grid/64` cells across. So once
the coarse level is doing its job, the residual latency is intra-tile and is **bounded by `t`, not
by the domain** — 8 ticks at 512, and it does not grow as the grid grows. That is the principled
answer to "how many sub-steps", which HANDOVER §11 never had: enough to cross a tile.

### Better: the coarse field is a predictive work scheduler

The LOD scheduler is *reactive* — a block is admitted on what moved LAST tick
(`last_displacements`), which is why 100% of material-bearing blocks were measured sitting in the
MUST tier. A coarse pressure gradient is a **predictive** signal: it knows where mass wants to go
before the fine level has moved any. So sub-step only the blocks whose tile shows a large `|grad P|`
and leave the rest at one step.

This makes the selector for adaptive overclocking fall out of the physics instead of being a
heuristic, and it is also the fix for I8 (the scheduler otherwise cannot see `P` at all, so a
settled pile's blocks sleep through exactly the pressure that is supposed to wake them).

Cost, from measurement: 4 sub-steps applied to the whole MUST set is ~3.2x the tick (~34 ms against
today's 10.5). Applied only to a high-gradient subset it is a fraction of that. **Unmeasured:** how
large that subset actually is in a settling pile, which decides whether this is affordable. Measure
it at build step 2, where `P` exists but drives nothing.

## 8. Acceptance criteria

Physics, in the user's words, all four required simultaneously:

1. A pile of liquid settles fast enough to look right at 512.
2. The U-tube shows visible upward movement in the riser.
3. No oscillation — `diag_task70_rest_color_mixing_and_checkerboard`'s `vpar` column watched before
   and after, and settled churn compared against the 0.00002/cell/tick baseline in HANDOVER §9.
4. **The hourglass does not break.** No lateral spreading of a falling stream, no upward excursion at
   impact.

Mechanical:

- `cargo test -p sandart-sim --lib --release` — the same 10 documented failures, no new ones.
- **`test_dry_sand_has_angle_of_repose`**, with the coarse coupling forced ON, exactly as the head
  field's own non-regression check does (see I7). A repose collapse at depth is the predicted
  failure if the liquidity gate or `normal_p` handling is wrong.
- **No block-boundary banding.** Fill sampled down a column must show no periodic structure at
  `y % block_size == 0` (§0.2). `diag_flip_release_front_and_block_alignment` is the existing
  instrument for the related gap-alignment signature.
- All six integration suites pass, `perfect_simulation_determinism` included.
- Coarse connectivity test: `k[e] > 0` exactly where a flood fill of the fine mask says the two
  tiles are orthogonally connected. Include a diagonal neck case (§4).
- Deadband test: a falling stream at 512 produces zero pressurised tiles across the whole drain.
- Budget: the coarse level's cost must stay under 15% of the tick. At 64x64 with `N = 8` the sweep
  arithmetic is `8 * 4096 / 262144 = 12.5%` **of one fine sweep** (a tick is two phases of
  COLLECT/ARBITRATE/APPLY, so the fraction of a tick is smaller) — derived, not measured, and it
  does **not** model the LUT hazard below, which could dominate it.
- **LUT thrashing.** `cached_vertical_lut` holds a *single* entry keyed on
  `(overfill_ratio, unit, tension, gravity_head)`. If the coarse term is folded into `gravity_head`,
  the key changes per edge and every change rebuilds a 4096-entry x 64-bisection table — comparable
  to an entire fine sweep, potentially thousands of times per tick. It also newly routes lateral
  edges through the vertical LUT whenever `|dP| >= 0.5` trips its gate. Either the coarse term must
  enter somewhere the LUT is not keyed on, or the LUT needs a real cache. **Settle this before
  step 3.**
- **No bang-bang transport.** `overfill_equilibrium_transfer` returns its full mass limit when
  `st(limit) >= tau`, which then meets `flux_edge_candidate`'s `.clamp(-1.0, 1.0)`. With `P`
  differences reaching `p_max = 225` at 512, that branch fires and the edge saturates — the
  pre-#70 defect. I1 rules out overshoot in the *potential*, not saturation of the *mass limit*.

---

## 9. Build order

Each step is separately measurable and separately revertible. Do not proceed past a step whose
measurement disagrees with its prediction.

0. **Falsify §0.1 first, with no new code paths.** Aggregate the existing fine mass into 64x64
   tiles offline and print `max(o)` over coarse tiles for a 300-row pool, plus `P` down a column.
   Two pass/fail questions: does `o` pin at `o_max` (if yes, the depth term saturates and this
   approach needs a different coarse pressure law before anything else), and does
   `P[D] - P[C] ~= t * base_head` (if yes, §0.2's head reformulation is mandatory, not optional).
   Also sweep `N` and measure how many sweeps a 64-cell coarse chain actually needs to settle — if
   it is O(N^2), §10's diffusion risk is real and §8's budget is off by orders of magnitude.
1. **Coarse geometry only.** Build `open_cells`, `capacity`, `k[e]` from the mask; add the
   connectivity test against a fine flood fill. Nothing reads it yet. Confirms §4 including the
   diagonal case, and measures how often a neck falls *inside* a tile rather than on an edge.
2. **Restriction + instrumentation, no coupling.** Compute `A[C]`, `M[C]`, `P[C]`, `Delta[C]` and
   print them. `P[C]` feeds nothing. Decide `P` vs `Delta` (I0) against what the numbers show.
3. **Couple, liquid-only, with `lambda` small, the flux budget on, and the LUT hazard settled.**
   Watch acceptance criterion 4 (hourglass) and the block-banding check before 1 and 2. If the
   hourglass breaks here, the deadband or the split is wrong.
4. **Map the joint `(N, lambda)` stability region** rather than assuming `N` is free — I3 is
   withdrawn, so this is measurement, not confirmation.
5. **Then** tune within that region against criteria 1 and 2, and only then unblock granular (I7).

---

## 10. What is not settled

- **The stability bound on `lambda`** is asserted to exist (one loop, one gain) but has not been
  derived. It should be derived before step 3, not tuned during it.
- **Whether restriction preserves U-tube connectivity through the bend under partial fill.** Geometry
  dependent; measurable at step 1.
- **Whether `P[C]` cancelling within a tile removes double-counting** (§7). If it does not, the fine
  local term needs an explicit reduction, and that reduction is a second constant with its own
  stability question.
- **Whether `M` needs its own velocity/momentum state** to propagate at the rate §2 predicts, or
  whether position-only relaxation suffices. Adding momentum adds a second lag and would need I3
  re-derived.
- **Persistent defect accumulation.** Memory makes the field robust to a *transient* bad cell but
  lets a *persistent* one accumulate. `lambda` bounds the accumulation; the bound has not been
  written down.
- **Coarse relaxation is diffusion, and HANDOVER §3 says two previous attempts died on exactly
  this.** "Averaging is diffusion: the settling time over an N-cell chain is O(N^2) sweeps, not the
  O(N) wavefront-arrival time... at N=512 they differ by 512x." The coarse relax is pairwise
  equilibrium transfer, i.e. nonlinear diffusion, so settling a 64-cell coarse chain is O(64^2) ~
  4000 sweeps, not `N = 8`. Persistence rescues this only partially — convergence then takes ~500
  ticks, the same order as the problem being fixed — and `lambda` actively fights the accumulation.
  **Either adopt a max-propagation or a direct per-connected-component coarse solve (as the deleted
  pass actually did), or cost the diffusion honestly.** This is the single largest unaddressed risk
  after §0.1.
- **State lifecycle for `M`.** Undefined across mask rebuild, resolution change, `flip_hourglass`
  (which clears edge momentum — must it clear `M`?), and user painting. A stale `M` after a flip
  injects the pre-flip pressure field into the new geometry.
- **`k[e]` for `MultiStageHourglass`**, where `multistage_neck_half_width`'s cap/floor applies and
  the neck floors at 1 cell — against a 2x2 tile at grid 128.
