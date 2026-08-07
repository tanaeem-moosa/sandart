# #27 — 2.4 — Water looks wrong: towers with a wide neck, and splashes violently

**Status:** pending

---

Reported from the live deployment on 2026-07-29, re-observed at fe5b4e9e (after the depth-floor fix, void total 21938 -> 12106). Two symptoms. PRIORITY IS NOW B — the user is explicitly "less worried" about A than about B.

## Symptom B — asymmetric tendrils (PRIMARY)
Liquid shoots out in thin tendrils, described as "shooting thunder". Two details from the user that are strongly diagnostic:

- **REPRODUCTION: MultiNeckHourglass + Water.** This is the shape to test on.
- **The tendrils are ASYMMETRIC and consistently biased — "usually on the left".**
- **They are SYNCHRONIZED across the three necks**, firing together rather than independently.

### Leading hypothesis: sweep-order artifact
A consistent directional bias in a structurally symmetric container is the signature of a solver that iterates x ascending and mutates as it goes, so every cell sees its left neighbour already updated this tick and its right neighbour not yet. Synchronisation across three physically separate necks says the trigger is a GLOBAL PER-TICK event, not local noise — three independent stochastic processes would not phase-lock.

Note the edge-flux rewrite made MASS CONSERVATION order-independent (flux is applied antisymmetrically), but that is not the same as making the DYNAMICS order-independent. If lateral edges are visited in x order and each flux immediately mutates `temp_heights`, the resulting motion still carries a directional bias even though nothing leaks. `column_depth` is likewise built in a top-down pass, so it inherits whatever ordering that pass has.

Places to look, in order:
1. The lateral edge loop in `settle_tick` — iteration order over x, and whether flux mutates `temp_heights` in place mid-sweep.
2. `column_depth`'s construction pass and whether left/right neighbours are read at different update states.
3. Anything with a per-tick global phase (a tick counter driving alternation, a shared RNG stream consumed in scan order) that could phase-lock the three necks.

### Second candidate: the dispersion term
The ignored test `test_liquid_splashes_on_impact` documents that on impact liquid width goes **8 -> 30 within 10 ticks**, "via the same dispersion noise as C2", while upward motion is impossible by construction (physics.rs:1162 and :1248 both `continue` when `gravity_active && gravity_dot < -0.01`). A 4x lateral blowout in 10 ticks is a plausible tendril source.

OPEN QUESTION worth resolving early: that dispersion lives on the granular CA path, which is scaled by `granular_share = 1.0 - cell_liquidity` and early-outs at `granular_share <= 0.0` (~line 2127). Water should therefore never reach it. Either the ignored test's note is wrong, or water is reaching a path it should not. Resolving that either kills this candidate or uncovers a real bug — do it before deeper work.

## Symptom A — towers (SECONDARY)
Water still stacks into a column rather than spreading level, still with a large neck width. NOT fixed by the depth-floor work, and NOT fixable by tuning `LATERAL_PRESSURE_SCALE`: #28 re-swept it and found a flat, noisy plateau (~11700-14500) across the whole valid window of ~3.5-18, bounded by failure at BOTH ends (below ~3.2 the landing pool backs up under the impact point and widens the stream; at 20 the stream fans apart). There is no headroom in that constant. So A is a different mechanism and needs its own investigation.

## Test methodology
Every bug in this project has been a small per-tick error invisible to tests measuring totals or settled states — three separate liquid defects hid behind exactly that. Any new test must measure the FLOWING state. Model: `test_liquid_flowing_liquid_does_not_stand_in_walls`.

For symptom B specifically, the natural instrument is an ASYMMETRY metric: compare mass (or max lateral extent) on the -x side against the +x side of each neck's axis, per tick, in a structurally symmetric container. A correct solver should show no persistent sign. That test would also have caught this at any point in the past, which is worth noting.

Verify each new test FAILS when its fix is reverted — that caught two vacuous tests earlier in this project.

## Must not regress
- All 78 tests pass. Do NOT weaken or relax any assertion; report numbers.
- `test_liquid_stream_stays_coherent`: max_width <= 8, peak_h >= 0.5.
- Enclosed-void total, currently 12106.
- Sand bit-identical — liquid paths are gated on `cell_liquidity > 0.0`.
- Mass exact: 883.000 -> 883.000, rel_err ~1e-9.
