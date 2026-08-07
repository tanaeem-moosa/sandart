# #10 — Add mass-distribution quantile lines overlay

**Status:** completed

---

Deferred until Phase 5 lands and is committed (agent is live in physics.rs).

Feature: horizontal lines showing where cumulative mass fractions sit, descending as sand/liquid falls, so you can see how much weight has moved.

DECIDED with user:
- Sand-fall mode only. Not shown in Sandbox.
- Quantile count is configurable (quartiles / deciles) via UI control.
- Recompute every 5 ticks is acceptable.
- User wants the motion to look good ("could be mesmerizing"), so smoothness is a
  requirement, not a nice-to-have. See below.

Sketch:
- sandart-sim: row-sum the heightmap top-to-bottom, walk the running total to find
  where it crosses each quantile. O(w*h) for sums, O(rows) for search. Every 5 ticks.
- sandart-wasm: expose quantile y-positions (normalized) to uniforms.
- sandart-render/shader.wgsl: draw in the fragment shader, tinting pixels near each
  quantile row. An HTML overlay will NOT work - 3D perspective camera means 2D lines
  wouldn't stay aligned when orbiting.

SMOOTHNESS (needed for the "mesmerizing" motion, both cheap):
1. Sub-row precision: interpolate the crossing point WITHIN the crossing row using the
   cumulative mass either side, instead of returning an integer row index. Without this
   the lines snap by whole grid rows.
2. Frame-to-frame lerp: with a 5-tick recompute the lines would visibly step. Hold
   previous + target quantile positions and ease toward the target each rendered frame.

Weight by heightmap value (mass), not occupied-cell count.
