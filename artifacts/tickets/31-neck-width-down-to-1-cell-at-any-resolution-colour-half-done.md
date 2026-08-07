# #31 — 2.8 — Neck width down to 1 cell at any resolution (colour half DONE)

**Status:** completed

---

SHIPPED in 7387250a (pushed to main, live on Pages), alongside 2.9 — same panel, shared readout.

Neck floor 3 cells -> 0.5 (a 1-cell opening). Stored value stays a FRACTION of grid width, so the slider's min AND step become resolution-dependent: 0.5 / grid_width, recomputed by demo.js::updateNeckSliderRange() on every resolution change. The step matters — a fixed 0.005 would have skipped over the new minimum at every resolution and made the change a no-op from the UI. Coarser grid clamps the value up, pushes the clamped value to the sim, then resets the vessel, so control and geometry cannot disagree.

DELIBERATELY CHANGES n=8 GEOMETRY AT GRID 64, and only there. Tier 0's neck_cap at grid 64 is 0.30*8 = 2.4, which never reached the old 3.0 floor — so the floor won at every slider position and the neck-width slider silently did nothing to the widest tier at that resolution. Lowering the floor lets the cap bind and the slider work. That's why the bit-identity test excludes grid 64.

Worst corner measured, not assumed: test_drainage_at_narrowest_possible_neck, n=16 / grid 64 / 1-cell neck -> 98.18% into the bottom chamber in 4500 ticks, mass error 7e-8. Slow, not clogged.

Cell-count readouts on both vessel sliders, backed by physics::effective_neck_half_width_cells so the number shown is what eval_sandbox_shape actually rasterises (after cap, floor, anti-merge ceiling) rather than a second hand-copied formula.

I fixed one thing the agent got wrong: it showed the new row with style.display='block', but `.field row` is display:grid and its slider spans grid-column: 1 / -1, so an inline block would have collapsed the two-column layout. Changed to ''.

LEFT ALONE (raised, not acted on): tier 0's neck cap saturates from neck_width = 0.06 upward, so the top half of the slider is inert for the smallest chambers. A non-linear slider mapping would recover that resolution. Worth a follow-up only if it bothers you in use.

NOT VERIFIED IN A BROWSER — see 2.9.
