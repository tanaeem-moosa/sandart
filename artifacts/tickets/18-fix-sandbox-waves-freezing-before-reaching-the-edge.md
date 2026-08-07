# #18 — Fix sandbox waves freezing before reaching the edge

**Status:** completed

---

USER-REPORTED, STILL OPEN: "one issue I am seeing with waves in sandbox, they don't continue
to the edge. they freeze half way through."

Fully diagnosed and measured by the edge-sleeping agent, which deliberately left it out of
scope as cleanly separable. Measured cost of the fix: ZERO on both bench_sandfall and
profile_sim.

MECHANISM (two parts, must fix BOTH — see below):
physics.rs `let active_threshold = if gravity_active { 1e-4 } else { MUST_SIMULATE_THRESHOLD }`
where MUST_SIMULATE_THRESHOLD = 0.1. Sandbox needs displacement >= 0.1 to be must-simulate,
1000x coarser than gravity's 1e-4. A ripple's recorded displacement is order 1e-3, so
wavefront blocks never qualify; they fall to rest_candidates prioritised by
staleness * displacement, near the bottom of the queue, so the front only advances when a
block hits MAX_STALENESS.

Measured at the app's real bed level (DEFAULT_SAND_HEIGHT = 0.35), 256x256, block 16, crest
near left wall, 1200 ticks, reach = furthest column ever deviating > 2e-3:
  budget 64  -> reach 199/245 (far column deviation EXACTLY 0.00000 for all 1200 ticks)
  budget 32  -> reach 144
  budget 256 -> full reach
So propagation speed tracks the budget, which is the invariant that is broken.

DO NOT JUST LOWER THE THRESHOLD. The liquid wake magnitude is
`disturbance = |h - DEFAULT_SAND_HEIGHT|` (physics.rs, in the !gravity_active branch), an
ABSOLUTE-LEVEL measure. At threshold 1e-4 any pool not sitting at exactly 0.35 is permanently
MUST — measured 7680/7680 block-ticks, the whole domain, forever. That is what the 0.1 bar was
really protecting against. Edge sleeping does NOT make it safe: sleeping and waking are
independent (must-counts were bit-identical before/after edge sleeping).

THE FIX IS BOTH HALVES:
  1. Replace the liquid-branch wake magnitude with the HEAD DIFFERENCE across the cell's owned
     edges - the same quantity edge_sleeps' branch-2 driving term already uses.
  2. Then lower the sandbox threshold.
With both: wave reaches the far wall at ANY budget, and reach + far-column peak become
byte-identical at budget 32 and 64.
  bed 0.35, threshold 1e-4, wake = max|H_a - H_b| -> reach 245, settled_must 1848
  bed 0.50, threshold 1e-4, wake = max|H_a - H_b| -> reach 245, settled_must 2185

TEST GAP: the four test_sandbox_wave_* tests CANNOT catch this. They force
last_displacements.fill(1.0) at setup AND their pools sit at 0.50, which is 0.15 above
DEFAULT_SAND_HEIGHT i.e. above the 0.1 bar — so every block is permanently MUST for the whole
run (7680/7680) and they never enter the scheduler path at all. Needs a NEW test at the real
bed level with only the disturbed blocks armed, asserting the disturbance reaches the boundary
and that reach is budget-independent.

RELATED SEPARATE DEFECT (found while measuring, not this task): a settled pool's MUST count
decays 64 -> 8 and then sits at exactly 8 forever (verified to 20,000 ticks). Those 8 blocks
are the full width of the free-surface row. Water's (c_sq, damping) = (0.24, 0.98) is lightly
damped, so an edge's momentum settles at c_sq*damping*d/(1-damping) ~= 12*d; a 0.02 surface
film ping-pongs its entire contents between two adjacent cells every tick, forever, far above
the 1e-4 MUST bar. Visually invisible, genuinely moves mass so sleeping cannot touch it. It is
the largest remaining must-count lever for liquid, and matters more at Stage B scale (~32
blocks per 512-wide surface; a granular heap has far more surface than a pool). A real tau > 0
may fix it for free, since a yield stress is exactly what stops a sub-threshold film sloshing.
