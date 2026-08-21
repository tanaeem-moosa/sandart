# Block size is already at its optimum — 2026-08-20

The question, from the user: "I think the simulation got much slower when we moved to 8x8 block.
should we try moving the other way? 32x32? each will have 4 disagreement to deal with. we can
handle that right?"

We can handle it — `block_size` and the coarse tile are now decoupled, a block aggregates the
disagreement of every tile it covers (max, not sum), and it was measured. **Bigger blocks are
slower, and 8 is the peak.**

## The sweep

Water, grid 512, `diag_blocks --ticks 300`, hourglass drain. `budget_n` is scaled with the block
count so it stays the same FRACTION of the block grid at every size (256 at 4,096 blocks, 16 at
256, 4,096 at 65,536) — otherwise the budget, not the block size, is what is being measured.
Overclocking on means rank allocation plus the rate gate.

| block | blocks | overclocked ms | descent | movement per ms | unclocked ms |
|---|---|---|---|---|---|
| 2  | 65,536 | 50.72 | 0.01863 | 3.67e-4 | 24.14 |
| 4  | 16,384 | 42.81 | 0.03140 | 7.33e-4 | 21.79 |
| **8**  | **4,096** | **45.14** | **0.04800** | **1.06e-3** | **21.77** |
| 16 | 1,024  | 56.40 | 0.04651 | 8.25e-4 | 22.85 |
| 32 | 256    | 68.44 | 0.04462 | 6.52e-4 | 24.64 |

**32x32 costs 52% more frame time for 7% less movement.** The curve has a clear peak at 8 on
movement per millisecond, falling away in both directions, and the unclocked column is nearly flat
(21.8–24.6 ms) across a 256x range in block count.

## Why bigger blocks lose

The flat unclocked column is the answer: per-BLOCK overhead is not what a frame is made of. Frame
time is cells swept — `diag_block_steps` puts it at a steady ~29–31 µs per block-step of 64 cells
under every configuration measured, and a block-step over a 32x32 block is 16x the cells. Bigger
blocks do fewer, larger sweeps for the same total cell count, so they save nothing on overhead and
lose on granularity: the LOD scheduler's unit of "this region needs work" gets coarser, so more
quiet cells get swept alongside each busy one. At 32x32 only 11 blocks run per tick, and each one
drags 1,024 cells with it.

Smaller than 8 loses for the opposite reason, and only once clocking is on: the 8x band is a fixed
FRACTION of blocks, so at block 4 that band covers a quarter of the area it did at block 8, and
descent falls 0.048 -> 0.031 while frame time barely moves. The rank ladder allocates by count,
not by area.

## What this does and does not say

- The scene is one hourglass drain at grid 512. A scene with much larger contiguous active
  regions would shift the peak upward, since granularity would cost less.
- It does NOT explain any perceived slowdown at the 8x8 change: at grid 512 the unclocked cost is
  flat across block sizes, so if the sim got slower when blocks became 8x8, the block size is not
  the mechanism. Grid 128 is the case worth re-checking on its own terms — there `grid/64` gives
  `block_size = 2`, the worst point in the table above, and the handover already records grid 128
  as 50–65% slower since the resize.

## What was built to answer it

- `DrawingSimulation::new_with_block_divisor(grid, divisor)`; `new_with_size` is it at 64.
- `update_block_clock_rates` aggregates coarse disagreement over whatever tiles a block covers
  (max), in both directions — a block bigger than a tile takes the max over its tiles, a block
  smaller than a tile reads the tile it sits in.
- `expand_touched_to_tiles` maps the per-block touched mask onto the per-tile one
  `restrict_incremental` needs, returning `None` (no clone, no work) at the default geometry where
  the two coincide.
- `diag_blocks --blockdiv N`.
