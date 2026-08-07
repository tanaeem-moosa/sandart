# #63 — 2.40 — Make material flow rate sensitive to pressure: slow down at LOW pressure rather than speed up at high

**Status:** in_progress

---

REOPENED 2026-08-07. The rate law shipped and works; the user's follow-up requirement is BLOCKED by #67, not by anything in this task.

USER REQUIREMENT, 2026-08-07: "I want 20 depth to have higher flow than 10 but it doesn't have to be linear."

## Shipped (local commits 73adbf63 + follow-up, not yet pushed)

Default-off "Pressure-sensitive flow rate" checkbox (`DrawingSimulation::pressure_sensitive_flow`). A LIQUID-ONLY edge's flux is scaled by `pressure_rate_factor(donor head in reference rows)` = `min(1, sqrt(rows / 20))`, at both the vertical and lateral edge sites.

Three design decisions, each measured rather than assumed:

**SQUARE ROOT, not linear.** Torricelli: `v = sqrt(2*g*h)`, so flow scales as the square root of depth. Also the affordable shape -- a linear ramp to 20 rows would run a one-row film at 5% of rate and a five-row puddle at 25%, near-freezing every free surface. `sqrt` gives the same 10-vs-20 ordering (0.71 against 1.00) while leaving the shallow end alive (0.22 at one row, 0.50 at five).

**REFERENCE ROWS, not local cells.** A local cell is `REFERENCE_GRID_HEIGHT / w` reference rows tall, so a cells-based threshold would mean a different physical depth at every resolution -- "20 deep" is a third of the vessel at w=64 and a twentieth at w=512. `rows_of_head_at` returns `head - z` directly. 20 rows = 20 cells at w=512, which is the unit the user's numbers were given in.

**THE FLUX, not `c_sq`.** The first version attenuated `c_sq` (momentum spin-up), which is the obvious reading of "flow rate". MEASURED, and it cannot produce a depth ordering: wherever an edge has real room to move into, the driving head is large enough that `v` hits the donor-mass/acceptor-room clamp within a tick or two whatever `c_sq` is, so the realised flux is MASS-limited and `c_sq` drops out. A draining vessel measured 127.1 (10 deep) against 124.5 (20 deep) -- no ordering. Scaling the realised flux (via `flux_edge_candidate`'s existing `weight` parameter) is also the more faithful reading of the law: Torricelli's `Q = C_d * A * sqrt(2*g*h)` puts the head dependence in a discharge coefficient on the flux.

Free fall stays exempt by construction: `advance_head_field` WRITES `head = z` at unsupported cells, so `rows_of_head_at` returns exactly `0.0` and the factor returns exactly `1.0`. `spec_task63_free_fall_is_bit_identical` asserts a falling slab is BIT-identical across the toggle at w=64 and w=512.

## BLOCKED: the 10-vs-20 ordering, by #67

`spec_task63_deeper_water_discharges_faster` is written, correct, and PARKED `#[ignore]`d. It measures `ratio_on = 0.9998` against a required `>= 1.10`. Nothing in it has been weakened.

The cause is not in this task. `diag_task63_orifice_head` shows the head field pins the ENTIRE column above an orifice to zero head -- orifice cell, the cell above it, and the top of the column all read 0.00, at both fills -- while an off-centre column of the same water reads 10.00 and 20.00 correctly at the same instant. So the rate law takes its free-fall exemption at every depth in a draining column and no ordering is possible there. Filed as #67.

**A levelling step cannot substitute for the drain scenario.** Measured: 133.60 flow at BOTH 10 and 20 deep, toggle on and off. In a saturated body every interior acceptor is at capacity, so `flux_edge_candidate` clamps those edges to zero however much head their donors carry, and only the free surface can move -- where head is set by the cell's own fill, not by the depth beneath it. Depth cannot influence a levelling rate in this solver at all. Do not "fix" the parked spec by moving it to a levelling scenario; it would pass on float noise, which is exactly what the first draft did.

## What still holds from the original ticket

`PRESSURE_RATE_FULL_AT_ROWS_OF_HEAD = 20.0` is a stated design choice, not a derived quantity: the top of the range is already at the CFL bound, so a depth ordering can only be produced by slowing everything below some reference depth, and this names it. Raising it grades a deeper range and slows more of the simulation; lowering it flattens the ordering. Both are real trades.

Saturation is still upstream of everything here and this task cannot fix a full vessel -- see the levelling measurement above for the direct evidence.

Cost: +219% ms/tick at w=512, effectively all of it the head-field advance rather than this feature. Filed as #66.
