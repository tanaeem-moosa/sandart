# #47 — 2.24 — Slabs: SHIPPED in 23e48e9 (support-predicate, -95.7% divergence)

**Status:** completed

---

NOT FIXED. Reopened 2026-08-02 after the user verified 530579b4 on the Pages deployment: slabs still occur.

## SETTLED 2026-08-05: slabs are 100% SCHEDULER-INDUCED

User, having tested the perfect-simulation toggle (shipped 18878a8) on the deployment: "also confirmed that perfect simulation has no slabs."

With every in-mask block holding material simulated every tick, the artifact does not occur. So:

- **The physics is not implicated.** No change to the solver, the flux form, the repose model or the pressure work is needed to fix this. Anything proposed in that direction is off-target.
- **Any fix that gets the right blocks running will work.** The remaining question is only WHICH blocks, and whether we can afford them. That is a scheduling question end to end.
- **`diag_falling_block_slab_separation`'s row banding is definitively NOT this defect.** That is a property of the simultaneous update and would survive perfect simulation. Keep it, do not tune against it.

### This also hands us a ground truth, which this defect has never had

Slab severity is now measurable without a camera: run the same scenario twice, perfect vs adaptive, and measure the divergence (interior void count, or a direct buffer diff). Perfect-sim is the correct answer by construction, so the metric has an absolute reference instead of the void heuristics used until now, which had no target value.

Build that comparison as the regression test for whatever fix lands. It removes this defect from the "only verifiable on the Pages deployment" category.

## LEADING FIX, user's idea 2026-08-05: zero overburden == free-flowing == MUST simulate

"if a block has sand with 0 pressure, it is a must simulate block as it has free flowing sand."

**Why it addresses the actual root cause.** Every activation signal today is HISTORICAL: `last_displacements` records what moved LAST tick, and `next_displacements` only becomes `last_displacements` the tick after. A block is therefore woken one tick after the evidence appears, which is exactly how falling sand outruns its own activation. `column_depth == 0` is a STATE predicate on the current field, so it wakes a block BEFORE it moves. Categorically different signal, not a tuning of the existing one.

**Why the predicate fits.** `column_depth[i]` is the RESTING material above cell i — `in_transit_at` is subtracted, so falling material contributes nothing. Zero means "nothing supported is bearing down here": true for free fall and for free surfaces, false for buried material. Close to an exact description of the cells that are moving or about to.

**PREREQUISITE: #54 step 1.** `column_depth` is currently a side effect of the block loop, so it is STALE in exactly the blocks the scheduler skipped — the ones the predicate most needs to describe. Useless as a wake signal until it is lifted into its own unconditional pass. That work is already in flight under #54.

### Two refinements it needs

1. **Bound the cost, or it promotes most of the domain.** Every pile's entire free SURFACE has zero overburden, not just the falling stream, and a surface spans many blocks. Promoting all of them to MUST bypasses `budget_n` wholesale — the thing the previous fix deliberately avoided ("nothing escapes the frame budget"). Tighten to material that can actually MOVE: zero overburden AND somewhere to go (free capacity below or laterally adjacent). A flat bed at rest has zero overburden and nowhere to fall; simulating it every tick buys nothing.

   MEASURE FIRST, now cheap: the perfect-simulation toggle and block heat overlay give the block count directly. Report the fraction of blocks qualifying under the loose predicate and the tightened one, for a resting pile, a mid-drain hourglass, and a fresh flip. The gap between that fraction and perfect-sim's 100% is the cost saving being bought.

2. **Threshold, not exact zero.** `column_depth` is an accumulated float; test `< epsilon`. A one-cell skin of settled material on a falling body would otherwise flip the predicate off while the body below is still effectively free.

### It answers #50's open question

#50 asks how to define "potentially active" tightly enough to promote such blocks to must-simulate in gravity mode. "Has material, zero overburden, and free space to move into" IS that definition. If this works, close that part of #50 into it.

## Budget saturation: still a live sub-hypothesis, now narrower

User: "it certainly is adaptive simulation as it doesn't happen every flip." Intermittency fits `budget_n` adapting to `ema_frame_ms` — on a heavy frame the budget shrinks, Medium-tier upstream blocks lose their slot, and material above a draining block goes unscheduled.

This is no longer a competing explanation, it is a candidate MECHANISM within the settled scheduling cause. If the zero-overburden predicate promotes blocks to MUST, it bypasses the budget and this stops mattering — which is a reason to prefer it. Only instrument saturation if the predicate approach fails or proves too expensive.

## Additional descriptor to pin down before acting
User: "the behavior is almost like kinetic sand."

Do NOT infer what this means. Kinetic sand is cohesive and moves in clumps that hold together and shear as blocks — which could describe the slab motion itself, OR a separate complaint that sand generally behaves too cohesively (related to the ~5-degree repose angle against real sand's 32-35). Ask which, and ask for the measurable quantity, before designing anything. NOTE: if it describes the slabs, it is now explained — scheduling, not cohesion.

## Mechanism established so far (partial fix shipped in ee40f7c6)
`activate_neighbor` only ever woke an edge's source and destination blocks, never the block one step UPSTREAM of a cell that just lost support. Fixed at all three flow sites (`try_move`, `touched_v`, `touched_h`) via `activate_neighbor_upstream` at Medium tier (`UPSTREAM_DISPLACEMENT_HINT`, below `MUST_SIMULATE_THRESHOLD` so nothing bypasses `budget_n`), plus a side nudge. Cut peak interior void -64% at 64, -52% at 512. Cost +9.3% at 512 (9.09 -> 9.94 ms/tick).

`block_size = (grid_size / 32).max(1)` means blocks are 16 cells at 512 but only 2 at 64, while sand falls ~2 cells/tick — at 64 material crosses a whole block every tick. That is why low resolution is worse. The tiling is deliberate (lib.rs:444) and must not be changed.

## Never established
The block-boundary alignment claim. Frames were captured at 64 where `block_size` is 2 cells, so every horizontal line sits within a cell of a boundary and alignment cannot be falsified there; the quantitative check came out 52% against a 50% chance baseline. A real test needs 512.
