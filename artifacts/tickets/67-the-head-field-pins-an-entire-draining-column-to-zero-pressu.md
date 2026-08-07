# #67 — 2.44 — The head field pins an entire DRAINING column to zero pressure: transitive support treats extrusion as free fall

**Status:** pending

---

MEASURED 2026-08-07 by `diag_task63_orifice_head` (in `task55_head_spec.rs`, `#[ignore]`d, run with `--ignored --nocapture`). A closed vessel at w=512 holding 10 or 20 local cells of water, with an 8-cell orifice in its floor and an open chute beneath. Head reported in reference rows, every tick:

    CENTRE (column standing over the orifice)
      orifice cell      head = 0.00     (both fills, every tick)
      cell above it     head = 0.00
      TOP of the column head = 0.00

    OFF-CENTRE (same water, 15% across, standing on solid floor)
      base              head = 10.00 / 20.00   <- exactly the column depth, correct
      top               head =  1.00           <- correct

So the field is RIGHT in general and wrong specifically above a drain. This is not a plumbing or advance-gating problem: the same field, the same tick, reads correctly 75 cells to the left.

## Cause

`advance_head_field`'s transitive-support pass. It propagates "unsupported" UPWARD through a column with a MIN, on the rule "a cell resting on falling material is itself falling". The orifice cell has empty space below it, so it reads unsupported, and the pin then climbs the entire column. Unsupported cells are WRITTEN to `head = z` (never maxed), so the whole column carries exactly zero pressure.

That rule is correct for the case it was written for -- a slab of water falling through air, where `support_fraction`'s one-cell-down look would otherwise mark only the bottom row. It is wrong for a column being EXTRUDED through a hole. An extruded column is under pressure; that is the entire content of Torricelli's law, and it is the difference between free flight (no contact force, p = 0) and confined flow (contact force, p > 0).

## Why it matters

1. BLOCKS #63's user requirement ("20 depth should have higher flow than 10"). `pressure_rate_factor` takes its free-fall exemption at every cell in the column and returns 1.0 at every depth, so no depth ordering is possible at an orifice -- the one configuration where depth CAN matter. `spec_task63_deeper_water_discharges_faster` is parked `#[ignore]`d against this; unpark it when this is fixed. Measured there: ratio_on = 0.9998 against a required >= 1.10.
2. Independently a #55 correctness issue. With `head_field_transport` on, every liquid edge in a draining column reads a driving head of `z` -- pure elevation, no pressure. Whatever that produces, it is not the Pascal-transmitted head the field exists to supply.
3. Probably explains #59's "hourglass discharge IS fill-height independent (1.01x)". That was recorded as correct-for-granular (Beverloo) and it is, but a LIQUID hourglass should be fill-height DEPENDENT (Torricelli), and it cannot be while the draining column reads zero head.

## What NOT to do

- Do NOT remove or soften `pressure_rate_factor`'s free-fall exemption. The exemption is right; the classification feeding it is not.
- Do NOT lower #63's `MIN_ORDERING` or rewrite its scenario to pass. The scenario is the correct one -- a levelling step provably cannot show depth dependence at all, because in a saturated body every interior acceptor is at capacity and only the free surface can move (measured: 133.60 flow at BOTH 10 and 20 deep, toggle on and off).
- Do NOT simply delete the transitive pass. It exists because `support_fraction` looks exactly one cell down, so a falling slab would otherwise read as supported everywhere except its bottom row.

## First step

Find the predicate that separates ballistic free flight from confined extrusion. Candidates worth measuring, not yet tried: whether the cell has lateral in-mask neighbours holding material (a confined column does, a falling slab's interior does too -- so this alone is not enough); whether the column is CONTINUOUS to a supported cell through any path rather than only straight down (the orifice column is connected to the vessel walls' standing water laterally); or capping how far the pin propagates upward. The head field is already a max-propagation over a connectivity graph, so a lateral-connectivity-aware support test is not foreign to it.

Cross-links: #55 (the field), #63 (blocked by this), #59 (probably explained by this), #58 (`support_fraction`, the one-cell primitive the transitive pass wraps).
