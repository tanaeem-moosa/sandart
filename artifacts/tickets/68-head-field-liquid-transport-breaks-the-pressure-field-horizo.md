# #68 — 2.45 — Head-field liquid transport BREAKS the pressure field: horizontal stripes in the U-tube. This is what #65 misattributed to the block heat map

**Status:** pending

---

USER-REPORTED 2026-08-07 with two photographs of the deployed build `E4FB1163C` (origin/main e4f81163), U-tube flow-through, 512x512, Water, pressure field heat-map on with "use new head field" selected: "head field liquid transport breaks pressure. (that's what I blamed on block heat map before)"

## What the photos show

Both frames show the U-tube's upper-left body rendered as HORIZONTAL BANDS alternating magenta and violet, sharply layered row by row, with the right arm a flat dark violet. On the fixed log ramp the overlay uses (violet = low/void, magenta = mid, pale yellow = high) a resting connected body should read as a SMOOTH vertical gradient -- head is uniform through a connected body at rest, so `p = head - z` must increase monotonically and smoothly with depth. Stripes are not a gradient. The second photo shows the banding more clearly than the first.

## Why this is a real defect and not an overlay artifact

The overlay is a pure read-and-convert (`head_field_to_pressure`) over the same `head_field` buffer the transport path reads. It cannot invent structure. So striping in the overlay means striping in the field the solver is driving liquid edges with.

## THIS SUPERSEDES #65

#65 recorded, tentatively and with the user's own caveat ("or maybe I imagined it"), that the BLOCK heat-map overlay was perturbing the simulation. The user has now re-attributed it: the toggle actually responsible is `head_field_transport`. #65 should be closed as misattributed rather than investigated on its own terms. Note the two were easy to confuse for a mechanical reason worth remembering: until e4f81163 the "Head-field liquid transport" checkbox had NO change listener, so clicking it did nothing until some OTHER control was touched -- which means a user toggling transport and then toggling the block heat map would have seen transport's effect appear at the moment they clicked the heat map.

## Where to start

Almost certainly the same root cause as #67, or a close relative. #67 established that `advance_head_field` pins an entire column above an orifice to `head = z` via the transitive-support pass, while an adjacent column on solid floor reads correctly. A row-banded field is what a support classification that flips per row would produce. Check FIRST whether the striped rows correspond to rows where `effective_support_transitive` is 0 or fractional (`0 < s < 1`, where the head is blended `s * best + (1-s) * pin_target`).

SECOND candidate, independent of #67: `head_field_transport` moves mass, which changes the heightmap, which changes the support classification on the NEXT tick, which changes the field -- a feedback loop the overlay-only path (`pressure_heatmap_head_field` alone) does not have. That would explain why the field looks fine when only the overlay is on and breaks when transport is on, which is exactly the user's report. If so, the striping is an oscillation and consecutive-tick sampling of one column will show it alternating.

REPRODUCE FIRST, in a test, before theorising further: the U-tube geometry already exists (`SandboxShape::UTubeFlowThrough`, #61) and `build_u_tube_siphon_primed` in `task55_head_spec.rs` builds a primed one. Run it with `head_field_transport` on for a few hundred ticks and dump `rows_of_head_at` down a column each tick.

## Cross-links

#67 (draining column pinned to zero -- same subsystem, probably same cause), #65 (superseded by this), #64 (head-field transport levels WORSE than legacy at w=512 and regresses the walls test 6 -> 157 voids; a broken field is a strong candidate explanation for BOTH), #55 (the field itself).

Note how much this ties together: if the field is genuinely striped/broken under transport, #64's transport regression stops being a mystery about driving-head magnitude and becomes a straightforward consequence. Do NOT continue investigating #64's "unbounded driving head" hypothesis until this is resolved -- fix the field first, then re-measure #64.
