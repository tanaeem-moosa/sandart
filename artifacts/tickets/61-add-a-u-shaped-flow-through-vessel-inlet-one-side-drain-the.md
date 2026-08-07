# #61 — 2.38 — Add a U-shaped flow-through vessel: inlet one side, drain the other

**Status:** completed

---

SHIPPED in 53516eb (pushed to main, deploying).

`SandboxShape::UTubeFlowThrough`, UI index 9, "U-tube, flow-through" in the Sand-fall funnels group. Geometry is a union of five rects in `physics::U_TUBE_RECTS` (fractions of w/h, resolution-invariant): reservoir/left arm, roofed bottom basin, right arm, spout off its rim, catch well. `neck_width`/`hourglass_curve` deliberately unused.

FIRST RUN RESULT — this is the value of the shape, see #55:
Water, 128 grid, perfect simulation, 4000 ticks. The reservoir drains 17 rows and fills the basin to the brim; mass conserved to +0.000%. The right arm NEVER rises — not slowly, at all, zero material across 4000 ticks, zero reaching the catch well. Stable and static, not oscillating.

An 85-row water column stands on a full basin whose only remaining outlet is upward, and nothing goes up. That is #55's core defect as a picture, with no instrumentation: the field cannot carry a free-surface elevation laterally, so a cell under the basin roof never learns there is water above and to the left of it. Same reason no siphon is possible.

Expect this shape to look inert until the hydraulic-head field lands. It is the first thing to re-run after.

Tests: `test_u_tube_is_one_connected_region` (flood-fill at 64/128/256/512), `test_u_tube_basin_has_a_roof`, `test_u_tube_reservoir_overflows_the_lip` (geometry computed from the rects: reservoir above lip 0.09 &gt; downstream capacity 0.042; catch well 0.083 &gt; spill 0.048). `SANDFALL_FUNNEL_SHAPES` extended to 7, wiring it into the existing geometry/mass-conservation suites.
