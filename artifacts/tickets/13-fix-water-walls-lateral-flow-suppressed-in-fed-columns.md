# #13 — Fix water walls: lateral flow suppressed in fed columns

**Status:** completed

---

USER REPORT: "water doesn't move sideways enough. this creates unrealistic water walls."

BLOCKED until the perf agent finishes - it is live in physics.rs right now.

ROOT CAUSE (diagnosed against committed main, physics.rs:1417-1440):

  let avail_a = (temp_heights[center_idx] - edge_vel_v[center_idx - w].max(0.0)).max(0.0);
  let avail_b = (temp_heights[nb_idx]     - edge_vel_v[nb_idx - w].max(0.0)).max(0.0);

edge_vel_v[i - w] is the flux that arrived from the cell above this tick. It is
subtracted from what the cell may donate sideways, on the reasoning (flux_edge doc,
~line 231-234) that in-transit mass is unsupported and exerts no hydrostatic pressure.

That is right for a free-falling parcel and WRONG for a supported column. It is applied
unconditionally, so in a continuously fed column - a pour, or an upper chamber draining
into a pool - every cell receives from above every tick and avail_a is permanently
suppressed by the inflow rate. When inflow approaches the cell's content per tick,
avail_a -> 0 and lateral flow stops at every depth. Vertical water walls.

COMPOUNDING: the phase ordering (physics.rs:~1182-1189) already resolves gravity-aligned
edges before lateral ones specifically so a falling cell "has nothing left to spread".
The avail_* subtraction then suppresses the same motion a second time.

WHY TESTS MISSED IT: test_liquid_pool_levels_flat_in_closed_box settles with no inflow,
so edge_vel_v decays to 0, avail_a recovers, the pool levels, test passes. The defect
only manifests DURING active flow.

FIX DIRECTION: make the in-transit subtraction conditional on the cell actually being
unsupported. If the cell below is at capacity or blocked (h_below >= cell_capacity - eps,
the same test used elsewhere in this function), the cell is supported and should donate
sideways from its full temp_heights under normal hydrostatic pressure regardless of
inflow from above. Only genuinely free-falling cells should have avail_* reduced.

NEW TEST NEEDED: the existing pool test settles before measuring, which is precisely why
this slipped through. Add one that measures lateral spread WHILE liquid is still being
fed - e.g. pour into a closed box and assert the surface stays level (or spreads to the
walls) during the pour, not only after it stops.
