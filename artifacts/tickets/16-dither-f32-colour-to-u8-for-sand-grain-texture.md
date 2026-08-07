# #16 — Dither f32 colour to u8 for sand grain texture

**Status:** completed

---

USER IDEA (2026-07-28): "if we could figure out how to handle integers correctly, I think it
would look grainier which is actually good for sands. maybe after rounding use leftover the
floating part as probability to increase the value by 1?"

That is unbiased stochastic rounding / dithering. Already has a supporting data point: the
Stage B probe measured stochastic rounding taking colour error from 15.4% to 0.0125%.

PLACEMENT — this is the crux:
  DO dither at the f32 -> u8 DISPLAY conversion, in sync_cell_colors_u8 /
  sync_cell_colors_u8_dirty (sandart-sim/src/lib.rs:417 and :451, committed fe2701a8).
  Keeps f32 as the sim's source of truth, so exact accumulation is preserved. Free grain,
  no precision loss, no perf cost.

  DO NOT dither inside the simulation and revert colour storage to u8. That injects noise
  into the DYNAMICS - colour random-walks every blend, so patterns diffuse and blur over
  thousands of ticks. It would undo the slow-deformation detail the user just noticed
  (colour lines bending as sand creeps).

SEEDING — make or break:
  Seed the dither from CELL POSITION (a spatial hash), giving static grain that sits still
  on the sand and reads as texture.
  Do NOT seed from frame count / time. The same cell would flip 180<->181 every frame and
  read as crawling TV static.

  Specific interaction: sync_cell_colors_u8_dirty only reconverts CHANGED blocks. Position-
  seeded dither is stable so untouched blocks keep matching grain. Frame-seeded dither would
  leave stale blocks frozen while active blocks shimmer, making the 32x32 block grid appear
  as visible seams.

ENHANCEMENT: PROP_GRAIN_SIZE already exists per-cell. Modulating dither amplitude by it would
make CoarseSand visibly coarser and FinePowder/MoonDust smooth - grain tied to material
rather than uniform. Check the existing grain_size ranges in apply_preset before picking a
mapping.

TESTING: conservation tests will not catch a bad dither (it is unbiased, so totals hold).
Needs either a visual check by the user or a test asserting the dithered u8 view round-trips
to within 1 LSB of the f32 buffer in the mean over a region.

RELATED: see task #11's note on u16 vs f32 - this idea is a better answer to "u8 looks flat"
than downgrading precision, because it adds grain WITHOUT giving up accumulation.
