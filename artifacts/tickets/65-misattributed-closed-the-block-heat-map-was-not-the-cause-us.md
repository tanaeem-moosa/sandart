# #65 — 2.42 — MISATTRIBUTED, closed: the block heat map was not the cause. User re-attributed it to head-field liquid transport (see #68)

**Status:** completed

---

CLOSED 2026-08-07 as misattributed. Do not investigate the block heat-map overlay on the strength of this ticket.

ORIGINAL REPORT 2026-08-07, with the user's own caveat attached: "I think turning on block heat map overlay somehow messes with the simulation that only refresh fixed. I am not worrying about it right now but something to consider. or maybe I imagined it."

USER RE-ATTRIBUTION, later the same day, with photographs: "head field liquid transport breaks pressure. (that's what I blamed on block heat map before)". Filed as #68, which has the evidence and the investigation plan.

## Why the two were easy to confuse, which is worth remembering

Until commit e4f81163 the "Head-field liquid transport" checkbox had NO `change` listener in `demo.js`. `syncSettings()` reads every checkbox at once, but nothing invoked it when that box was clicked. So transport only took effect when some OTHER control was touched -- and "Block heat-map overlay" sits directly above it in the Debug group. A user who ticked transport, saw nothing, then ticked the block heat map would watch transport's effect appear at the exact moment they clicked the heat map, and would reasonably conclude the heat map caused it.

That also explains the "only refresh fixed" part: a page refresh rebuilds the sim and re-runs `syncSettings()` from the restored checkbox states, so the confusing intermediate state disappears.

Both listeners (transport and the new pressure-sensitive flow rate) are wired as of e4f81163.

## What was verified before closing

`set_heatmap_overlay` in `sandart-wasm/src/lib.rs` is a plain field write to `heatmap_enabled` with no simulation path -- it cannot perturb `settle_tick`. That was checked when this ticket was first filed and holds.

Superseded by #68.
