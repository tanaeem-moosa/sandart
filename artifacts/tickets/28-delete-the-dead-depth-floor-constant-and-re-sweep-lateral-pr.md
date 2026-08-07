# #28 — 2.5 — Delete the dead depth-floor constant and re-sweep LATERAL_PRESSURE_SCALE

**Status:** completed

---

Follow-up to #20 (shipped as fe5b4e9e). Two cleanups left behind by that fix.

1. `LATERAL_PRESSURE_DEPTH_FLOOR` now sits at 0.0 and can never fire — `column_depth` is >= 0 by construction, since `resting_above` is `.max(0.0)`-clamped before being added to the prior row's already-non-negative value. It was kept as a no-op "in case a deadband is needed again", with a paragraph defending it. Delete the constant and simplify the call site; git history holds the deadband if it is ever wanted back.

2. `LATERAL_PRESSURE_SCALE = 5.0` was swept in the OLD regime, when the phantom source depth existed and the floor was 1.5. That sweep read 30060 -> 23526 at scale 2 -> 21938 at scale 5 -> 22085 at scale 10, and appeared to flatten past 5. The flattening may have been the floor clipping the term rather than a real knee. Baseline is now 12106 at scale 5, and the reference ceiling is 6304 (what deleting the in-transit limiter entirely achieves — NOT acceptable, it fans the stream to 59 cells wide).

The binding constraint is `test_liquid_stream_stays_coherent` (max_width <= 8): raising the scale pushes the stream apart. Any sweep must report BOTH metrics at every point, since they trade against each other.

Caveat that outranks the number: this is visible behaviour. Too much lateral pressure makes water look unnaturally flat and eager rather than merely non-towering. The metric is a proxy; the user checks the deployment. Prefer the smallest scale that captures most of the available improvement over the one that minimises the number.
