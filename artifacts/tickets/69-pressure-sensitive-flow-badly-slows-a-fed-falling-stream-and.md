# #69 — 2.46 — Pressure-sensitive flow badly slows a FED falling stream and makes it spread sideways: the free-fall exemption only covers compact unsupported slabs

**Status:** pending

---

USER-REPORTED 2026-08-07, photograph of deployed build `E4FB1163C` (origin/main e4f81163), MultiNeckHourglass, 512x512, Water, pressure heat-map on, "Pressure-sensitive flow rate" ON: "pressure sensitive flow causes significant slowdown on free fall. and cause it to spread sideways."

Photo shows the vessel's chambers filled far wider than a falling stream should make them, with a pronounced left/right asymmetry, at 78.2 ms/frame (13 fps, 420 blocks) -- the frame time is #66, not this defect.

## THE SHIPPED SPEC IS A FALSE NEGATIVE. Do not trust it.

`spec_task63_free_fall_is_bit_identical` PASSES, at w=64 and w=512, asserting a falling body is BIT-identical across this toggle. It is not lying about what it measures; it is measuring the wrong thing.

It uses `build_falling_water`: a compact slab released into empty space with NO source above it and NO material below it. Every cell of that slab is genuinely unsupported, so `advance_head_field`'s transitive-support pass pins the whole slab to `head = z`, `rows_of_head_at` returns exactly 0.0, and `pressure_rate_factor` takes its exact 1.0 branch. The exemption works perfectly on that scenario.

**A stream in an actual vessel is not that scenario.** It is continuously fed from above and it lands on standing material. Its lower cells have material beneath them, so `support_fraction` reads them as SUPPORTED, so they are NOT pinned, so they get a real head reading -- and a falling stream cell is THIN, typically a few tenths of a cell of fill. At w=512, `rows_of_head_at` for a 0.3-filled cell is 0.3 reference rows, and `pressure_rate_factor(0.3) = sqrt(0.3/20) = 0.12`. That is an ~8x attenuation applied to material that is, physically, in free fall.

Slowed descent backs the stream up; the backed-up material is at capacity and pushes laterally; hence "spread sideways". Both halves of the user's report follow from the one cause.

## The real defect

The free-fall exemption is keyed on the head field's unsupported classification, and that classification does not mean "is in free fall". It means "has nothing directly beneath it, transitively upward through the column". Those coincide for a slab in a vacuum and diverge for every stream in a real container.

Note this is the SAME class of defect as #67, in the opposite direction. #67: a column being extruded through an orifice is classified as free-falling when it is under pressure. This one: a stream in flight is classified as supported when it is ballistic. One predicate, wrong on both sides. **Fix them together** -- a correct answer to "is this material bearing load right now" resolves both, and two independent patches will fight each other.

## Do NOT fix this by

- Widening the free-fall exemption with a fill threshold (e.g. "exempt anything under 0.5 fill"). That would also exempt every genuine thin surface film, which is the population this feature exists to attenuate, and would make the toggle a no-op.
- Removing the attenuation from the vertical (phase 0) edge site. Depth-dependent DRAINING is the point of #63's follow-up requirement, and that is a vertical effect.
- Relaxing `spec_task63_free_fall_is_bit_identical`. Keep it -- it is a correct check of the slab case. ADD a fed-stream case beside it; that is the test that is missing.

## First step

Write the missing spec before touching the rate law: a vessel with a continuous source above it and standing material below, toggle on vs off, asserting the stream's descent rate and lateral spread are unchanged. It will fail. `MultiNeckHourglass` reproduces it on the deployed build, and `task55_head_spec.rs` already has the harness (`DynSim`, `tick_with`) to build a headless version.

Then instrument `rows_of_head_at` and `effective_support_transitive` down the stream, at several ticks, and confirm the stream cells are reading as supported with a fractional head before assuming the mechanism above is right.

## Scope note

The toggle is DEFAULT OFF and this defect only appears with it on, so nothing shipped is affected. `head_field_transport` is also off and separately broken (#68).

Cross-links: #67 (same predicate, opposite failure -- fix together), #63 (the feature), #68 (the other toggle), #66 (the 78 ms/frame in the photo).
