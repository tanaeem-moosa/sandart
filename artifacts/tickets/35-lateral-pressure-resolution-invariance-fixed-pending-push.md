# #35 — 2.12 — Lateral pressure resolution-invariance (FIXED, pending push)

**Status:** completed

---

RESOLVED 2026-07-30, verified independently. Uncommitted in the working copy.

## The defect
`column_depth` accumulated ONE TERM PER GRID ROW, so the lateral driving head scaled with RESOLUTION rather than physical depth. The vertical term does not (`gravity_dir.y * GRAVITY_HEAD_SCALE` is a local per-edge comparison). `LATERAL_PRESSURE_SCALE = 5.0` was swept entirely at 64x64/64x96; production is 512.

## The fix
Scale each row's `resting_above` contribution by `depth_scale = REFERENCE_GRID_HEIGHT / w` before folding it into the running sum.

**`REFERENCE_GRID_HEIGHT = 512`, NOT 64.** The reference must be where the app RUNS, not where the constant was historically tuned. With 64 the fix made production 8x weaker and materially worse (walls total 66.7M -> 98.4M) and was unshippable. With 512:
- Production (512x512) is an EXACT NO-OP — walls `34161/31718/66,730,129`, bit-identical to pre-fix. Zero shipping risk.
- Every other resolution now matches production physics instead of being up to 8x off.
- All tests pass (78 pass / 1 intentionally-red / 5 ignored), including the 64-scale tests which now run with 8x stronger lateral pressure and still hold their bounds.
- Stream at 512: max_width 49 = 0.0957 of w, peak_h 1.0000.

Uses `w` not `h` deliberately: the two tuning tests share width 64 but differ in height (64 vs 96), and production is always square. If grids ever become NON-SQUARE this must switch to `h`, since column_depth's accumulation is genuinely vertical.

## Scaled-test harness (also uncommitted)
`test_scale()` reads `SANDART_TEST_SCALE` (default 1). `test_liquid_stream_stays_coherent` and `test_liquid_flowing_liquid_does_not_stand_in_walls` scale grid, tap position/width and tick budget together. Documented in docs/ARCHITECTURE.md 11. Invocation:
  SANDART_TEST_SCALE=8 distrobox enter sandart-dev -- cargo test --release -p sandart-sim -- --nocapture <names>
Scale 8 = 512, ~20s for the walls test. Default `cargo test` unchanged: same sizes, same numbers, ~11s.

Assertion changes, classified:
- `max_width <= 8` -> RE-DERIVED as `max_width/w <= 0.125`, identical numeric strictness, now resolution-independent.
- `peak_h >= 0.5` -> unchanged (already a fraction).
- Walls void bounds -> KEPT ABSOLUTE, deliberately not fractionalised. Checked: `voids/interior_cells` is NOT stable across scale (0% -> 8.8% -> 27.7% -> 32.1%), so fractionalising would launder a real defect.
- `budget_n` 256 -> `usize::MAX` in both: bit-identical at scale 1 (256 already exceeded the block count) but removes LOD scheduling as a confound at scale 8.

## What this does NOT fix — see the follow-up task
The walls test still fails at 512 (66.7M vs a 34,000 bound). That residue is NOT this defect family and is NOT tunable: sweeping `LATERAL_PRESSURE_SCALE` from 5 to 100 at 512 bottoms out at ~66.7M and never approaches 34,000. Diagnosed as a separate time-dimension defect in `wave_params`. Production is unchanged by this fix, so this is not a regression — it is a pre-existing condition now correctly attributed.
