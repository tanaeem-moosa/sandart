# Block geometry back to a constant 8-cell block, and the budget test fixed

2026-09-02, against `7b8c63d`. Two changes, one cause: the coarse level was deleted on
2026-08-30 and two things that existed only to serve it were still standing.

## 1. `block_size = grid/64` -> a constant 8 cells

`94d7390` made the LOD block edge `grid/64`, which pins the block COUNT at 64x64 = 4096 for every
resolution and lets the block SIZE float. The stated reason was that a block and a
`coarse::CoarseGeometry` pressure tile would then be the same square (`COARSE_GRID` was also 64).
`coarse.rs` is gone, so the reason is gone.

What it cost while it stood:

- grid 64  -> `block_size = 1`. One block per cell; the LOD scheduler does nothing.
- grid 128 -> `block_size = 2`. Ships into the slab artifact `VERTICAL_PRESSURE_CAP_MULT` documents.
- grid 256 -> `block_size = 4`.

Now `DEFAULT_BLOCK_SIZE = 8` for every shipped resolution, so the block grid is 8x8 / 16x16 /
32x32 / 64x64 at grids 64 / 128 / 256 / 512. **At grid 512 the two geometries are identical**, so
this is a no-op at the shipped `GRID_SIZE` and only the smaller grids move — toward the
non-degenerate case. The divisor is clamped at `MAX_BLOCKS_PER_AXIS = 64`, a placeholder for a
future grid 1024; revisit it if 1024 ships.

Because block count is resolution-dependent again, the adaptive frame-time controller's four
throttles are derived from it (`budget_throttles`) rather than hardcoded 1024/128/16/4. The
divisors were chosen to reproduce those absolutes EXACTLY at 4096 blocks, which is what keeps grid
512 a no-op. `test_budget_throttles_match_the_old_absolutes_at_grid_512` pins that.

**The blocker that turned out not to be one.** `new_with_size`'s doc comment claimed the geometry
was load-bearing for `sandart-render`'s `update_block_heat`, which uploads into a texture of fixed
`HEAT_GRID_SIZE` (64) square with no bounds check on the source slice — so a smaller block grid
would read out of bounds. That is true of the function, but it has NO CALLERS, and its producer
`DrawingSimulation::block_heat_texels` was deleted with the overlays on 2026-08-30. The constraint
is unreachable. It is restated in `new_with_size`'s doc comment for whoever revives the overlay:
size that texture from the block grid, not from a constant.

## 2. `test_sandbox_wave_reach_is_budget_independent`

Open since the overfill window; the last handover listed it as the only real regression and
guessed it pointed at the block-clock scheduler. It did not — and note the block *activation*
scheduler was never deleted, only the block-clock overclocking layer on top of it, so the
machinery this test exercises has been live the whole time.

**The test was wrong, not the code.** Its third assertion demanded `far_peak` be equal TO THE BIT
across budgets 32 and 64. That contradicts what `budget_n` is for: it skips blocks whose
contribution is negligible, and negligible is not zero. Bit-identity across budgets is the demand
that the budget be a no-op. The design and the assertion cannot both stand.

It had only ever passed on headroom. Instrumenting the classification loop:

    budget=32   starved on 1140 / 1200 ticks   first starvation at tick 13
    budget=64   starved on 1129 / 1200 ticks   first starvation at tick 45

    tick 13,   budget=32:  must=48  cands=16  remaining=0
    tick 1198, budget=32:  must=129 cands=127 remaining=0

`remaining_budget` is zero whenever `must_simulate` alone exceeds `budget_n`. The whole 8%
divergence is seeded in a 32-tick window (ticks 13-44) where budget 64 still had 16 spare slots
and budget 32 had none; after tick 45 both are equally starved and carry different states forward.
Zero starvation in this scene needs ~253 of 256 blocks, so bit-identity was reachable only at full
simulation.

**A comment corrected along the way.** `activate_neighbor_upstream`'s doc says "the total simulated
block count stays capped by `budget_n` either way". It is not: MUST is budget-exempt by
construction and `must=129` under `budget_n=32` was measured. `budget_n` rations a marginal tier;
it does not bound frame time.

**The physics was never wrong.** Sweeping the budget gives a convergence curve toward the
full-simulation value, and that value is unchanged from when the test was written — 0.00779 then,
0.007786 now. Only low-budget fidelity had drifted.

Measured at the new geometry (`bs = 8`, 1024 blocks over 256x256), 1200 ticks:

    budget |   32      64     128     256     512     768    1024
    far    | .007097 .007097 .007178 .007198 .007230 .007655 .007786
    reach  |  245     245     245     245     245     245     245

Reach is invariant across a 32x budget range; worst-case amplitude is 8.85% under the reference at
`budget_min`. At the old `bs = 16` the same spread was 13.6%, so the geometry change tightened
scheduler fidelity on its own.

The test now asserts what is actually true and actually load-bearing: **reach EXACTLY equal across
budgets** (the #56 guard, no tolerance — where a wave gets to is physics), and **amplitude within
15% of the full-simulation reference** (fidelity — rationing negligible blocks costs a little
amplitude; losing a lot means the wavefront itself is being rationed). It samples `budget_min`,
`block_count/4` and `block_count` rather than absolute budgets, which stopped meaning the same
thing once block count became resolution-dependent.

## What is still open

Making the budget genuinely physics-neutral means completing the MUST tier so an impacted block
never depends on leftover budget. That was proposed and REJECTED once already (an earlier version
of the upstream-wake fix pushed the upstream block straight into `Fast`; the task's author
overturned it on the grounds that only measured displacement earns the budget-exempt tier). The
objection was that it would break the `budget_n` cap — and there is no cap to break. If it is
revisited, revisit it knowing that, and knowing it would make the budget tier permanently empty,
which makes budget-independence true by construction rather than by measurement.

Do not respond to a future failure of this test by widening the 15% tolerance.
