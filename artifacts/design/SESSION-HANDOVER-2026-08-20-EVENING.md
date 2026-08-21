# Session handover — the clock scheduler becomes a scheduler, 2026-08-20 (evening)

Supersedes `SESSION-HANDOVER-2026-08-20.md` for next steps; that file is still the record of the
morning session and its §4 (the instrument problem) and §8 (process notes) still apply.

> **§5 is itself now superseded by LATERAL-COARSE-CORRECTION.md (night session, `242c942`).** The
> user proposed a different attack on lateral flow -- use the coarse level's own realised transport
> to move more material than the fine level's local CFL bound allows -- and it landed. It measured
> +41% lateral spread for +10% frame time on DrySand (against coarse coupling's +7% for +36%), and
> it retires §5 item 2's blocker: that item wanted the RECONCILING flux, at ~1,220 SOR sweeps a
> tick; the flux the coarse level ALREADY PERFORMED is the quantity the defect form needs and costs
> an accumulator. §5 items 1, 3 and 4 are untouched and still stand -- item 4 (selective coupling)
> is now cheap, because the ledger built for the correction is exactly the per-block directional
> quantity it was waiting on.

**Deployed:** `origin/main` = `17079b80`, confirmed serving from `gh-pages` (its tip names that
sha). Live at <https://tanaeem-moosa.github.io/sandart/>. Nothing is unpushed.

---

## 1. The finding that reframed this session

**A block's clock rate was never gating anything.** `force_overclocked_blocks_active` only ever
ADDED work: it raised the displacement of fast blocks and their S3 neighbours. Every other block
was still admitted on its own merits by `settle_tick`'s ordinary MUST classification, on EVERY
repetition — because a block that moved in the previous repetition is by definition above the MUST
bar. So "1x" did not mean "runs once per frame"; it meant "runs in all eight, like everything
else", and the whole rate ladder was a multiplier on work rather than a division of it. Measured:
with 34 blocks at 8x, the extra repetitions were still running ~370 blocks each.

`rate_gated_reps` fixed it. Everything else this session followed from being able to *allocate* a
budget rather than only inflate one.

## 2. The cost model, which should survive this session

**Frame time is executed block-steps, at a steady ~29-31 µs per 64-cell block-step under every
configuration measured.** It is the one model that predicted correctly across a 256x range of block
sizes (BLOCK-SIZE-SWEEP.md). Corollary: the only lever that matters for performance is running
fewer block-steps, and per-block overhead is not where the time goes.

Frame time is also strongly SUB-linear in activity — 4.1x the swept cells cost 2.1x the time — so a
large share of a grid-512 frame does not depend on how much is running. The exact split is unmeasured
and the fit is not trustworthy across materials; **deterministic phase timers remain the right next
instrument** and no profile should be believed here (morning handover §4).

## 3. What shipped, in order

| commit | what | the number |
|---|---|---|
| `0b8868c` | Max clock rate as a runtime field + slider; the ceiling sweep; underclocking control | movement/ms rises monotonically with the ceiling — no efficiency peak below it. Underclocking buys **nothing** in either material |
| `ee5be33` | Rank allocation (ladder by position, bands ∝ 1/r) + **the gate** | 92 -> 51 ms Water, +45% movement per ms |
| `67dcb08` | Block size decoupled from the coarse tile, and swept | **8 is already optimal**; 32x32 costs 52% more for 7% less movement |
| `4e544a4` | Stalled-boundary counter; a failed halo fix, deleted | stalls 791/frame at ceiling 8 vs 187 at ceiling 3 — tracks the user's visible artifact |
| `e4b69cb` | **Rate grading** (2:1 balance, enforced downward) | stalls -66/-73%, frame -41/-56%, and **no block reaches 8x** — nothing is wide enough to ramp |
| `cc242dd` | Ceiling to 16; the coarse-drift (`lambda`) hypothesis tested | drift makes spread and descent WORSE in both directions |
| `900d6cc` | Realised flow counters, lateral vs down, coarse vs fine | **coarse flow is 2.2x more lateral than fine (DrySand), 1.5x (Water)** |
| `ae6beeb` | Heat map draws executed sub-steps; band falloff removed; 30-tick floor verified | floor holds exactly, 0 block-ticks over 30, clocking on or off |
| `17079b8` | Falling-liquid jitter (per-cell, underfull-gated) | descent unaffected to 0.3; cost is frame time, 30 -> 42 ms |

## 4. Current defaults

Overclocking is still **default OFF**. With it on: rank allocation ON, gate ON, grading ON, ceiling
16, `liquid_fall_jitter` 0.0, coarse pressure coupling OFF. Every one of these is a UI toggle or
slider in the Debug panel.

**`mass_err` is no longer a concern with grading on**: 1.85e-9 at ceiling 8, 1.37e-9 at ceiling 16.
The 7.45e-8 recorded mid-session was ungraded, and grading appears to have retired it — but that is
correlation from two runs, not a diagnosis, and the underlying stall is bounded rather than fixed.

## 5. Next steps — fixing lateral flow, in the order I would take them

The evidence that frames all of this: **the coarse level's flow is ~2.2x more lateral than the fine
level's, and the transport that would reconcile the two levels is 0.327 lateral/down against the
fine level's realised 0.056 — roughly 6x under-served** (FLOW-DIRECTION.md). The signal exists and
the scheduler cannot see it, because `|Delta|` is a magnitude and the direction is discarded before
the scheduler is handed anything.

**1. A fine-grid term in the clock signal.** The cheapest and most certain of these. A pile above
its angle of repose is a fine-scale instability the coarse level has no model of — its tile masses
agree perfectly while the slope is still wrong — so no coarse-derived signal will ever schedule it.
The block's own `last_displacements` is already computed every tick and already says "this block is
avalanching". Measured supporting fact: 60-75% of blocks HOLDING MATERIAL are underclocked,
including the pile flanks. **The design decision the user should make first is how the two terms
combine** — max, sum, or fine-only-above-a-threshold. Do not pick this constant unilaterally; that
is how three wrong implementations happened.

**2. Lateral-only sub-steps ("directional overclocking"), the cheapest correct form of direction.**
A sub-step that evaluates ONLY horizontal edges. Each edge pass is independently conservative —
COLLECT/ARBITRATE/APPLY over a subset of edges is still an FCT projection — the ±1 clamp still
applies per pass, and it costs roughly half a full sub-step. A block whose disagreement is lateral
then gets lateral-only extra repetitions instead of full ones. No new solver and no new
conservation argument; it is a subset of edges per pass. **Blocker: the signal.** It needs the
lateral COMPONENT of the reconciling flux per block, which today costs ~1,220 SOR sweeps per tick
(`diag_delta_direction`) and needs a multigrid solve — a few V-cycles on 64x64 — before it could
run live.

**3. Block-level flow, the user's own idea and the largest.** "we kind of know how much is the max
flow for the block from each direction" — true: it is `1 cell/tick × block edge × per-cell
capacity`, and the FCT limiter already computes exact availability and headroom per edge. Compute
INTER-block flow first as a small network problem over ~4k nodes, then distribute within blocks.
The catch to design around, and the reason this needs a design pass rather than a patch:
distributing an inter-block flow WITHIN a block is the step that is not free, and getting it wrong
produces exactly the seams that `last_frame_stalled_boundaries` counts. This is also the same
object as sub-linear per-block stepping (§6 below).

**4. Selective coupling.** Coupling costs ~36% frame time for ~7% spread applied everywhere. The
lateral disagreement concentrates where the pile is. Couple only where the local disagreement is
predominantly lateral and most of that cost disappears. Cheap to try once §2's directional signal
exists — they need the same quantity.

## 6. The standing performance idea, unchanged and unbuilt

**Loop interchange, enabled by Osher-Sanders local time stepping.** The rep loop is
`for rep { for every block { sweep } }` — eight global passes. It should be
`for block { for rep in 0..rate { sweep } }`, so a block's 64 cells plus halo stay in L1 across all
its sub-steps and the grid is walked once per frame. Same operation count, much better locality.
The only blocker is the interface: a block cannot run ahead of its neighbours unless the shared
flux is accumulated — which IS Osher-Sanders (ARBITRATION-AND-N-STEP.md §3). So LTS is not an
alternative to per-block n-stepping, it is the enabler, and it retires the boundary stall by
construction at the same time.

Genuinely sub-linear in n needs regime detection: a freely falling column advancing n cells in n
sweeps is a shift computable in one scan; levelling inside a block converges to a water-filling
solution (sort column heights, solve for the level) in `O(c log c)` once; repose is a 1-D scan along
the surface. Iterate only the contested minority — structurally the same move as fusing uncontested
edges.

## 7. What was tried and rejected, with numbers — do not re-run these

- **Coarse drift** (`CoarseState::lambda` lowered): spread FALLS monotonically, 11.25 -> 10.05
  uncoupled and 12.07 -> 10.27 coupled, and descent roughly halves. The coarse level's opinion is
  useful because it is anchored.
- **Forcing edge owners** (left/top neighbour of every participant): 6% fewer stalls on Water, MORE
  on DrySand, for 22-30% more work. Widening the halo relocates the frontier, it does not remove it.
- **Gentle band falloff** `1/lg(1+r)`: superseded by grading and removed at the user's call.
- **Bigger blocks** (16, 32): 52% more frame time for 7% less movement at 32x32. **Smaller** (4, 2)
  loses too, for the opposite reason — the fast band is a fixed fraction of BLOCKS, so it covers
  less area.
- **A second 30-tick staleness floor**: unnecessary, the existing one holds exactly (0 block-ticks
  over 30, measured).

## 8. Instruments available — do not rebuild these

- `diag_blocks` — `--overclock --rank --gate --grade --maxrate --minrate --lambda --coupling
  --jitter --blockdiv --budget --material --ticks`. Prints ms/frame, block tiers, descent,
  `mass_err`, block-steps, **stalled edges**, and **spread** (mass-weighted std-dev of x over the
  bottom quarter — the lateral metric).
- `diag_block_steps` — executed sub-steps per frame and **µs per block-step**, which is what made
  the cost model legible.
- `diag_flow_direction` — realised mass flow, lateral vs down, fine vs coarse, split by whether the
  edge crosses a block boundary. Counters live in `physics::flux_dir_*`, off unless enabled.
- `diag_delta_direction` — the Helmholtz reconciling flux. **Now SOR with a residual test**; the
  first Gauss-Seidel version was unconverged and reported a wrong direction, so trust the printed
  residual, not the sweep count.
- `diag_overclock_ab oscillation` — `vpar` and settled churn.
- The block heat-map overlay now draws **executed sub-steps**, not planned rate.

## 9. Build and verification

There is no linker on the host: everything compiles inside
`distrobox enter sandart-dev -- /home/deck/.cargo/bin/cargo ...`.

Full check before any push: lib suite (expect **102 passed / 10 failed**, the same ten named
failures — that is the known-good state), all eight integration suites including
`perfect_simulation_determinism` and `overclocking_toggle`, the wasm32 check, `cargo check -p
sandart`, and `node scripts/check_js.js`. The user tests via the GitHub Pages deployment, so
confirm `origin/gh-pages`'s tip message names the pushed sha before reporting anything as testable.

Git identity is not configured on this host; commits need
`GIT_AUTHOR_NAME="Steam Deck User" GIT_AUTHOR_EMAIL="user@steamdeck.local"` (and the COMMITTER
equivalents) to match existing history.
