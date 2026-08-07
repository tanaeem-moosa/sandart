# Handoff — sandart, 2026-08-05

## Repo state

`origin/main` = **ba991a2** ("Make the pressure field visible..."). Everything through that is pushed and deployed to gh-pages.

**UNCOMMITTED, UNVERIFIED**: `sandart-sim/src/physics.rs`, ~1010 insertions. Left by a slab-fix agent that was killed mid-edit by the session limit. It had already implemented the work; its last action was updating a call-site comment.

New symbols in that diff:
- `support_fraction(..)` — the #58 "is not supported" primitive
- `fresh_overburden_must_blocks(..)` — the block-activation predicate
- `fresh_overburden_gate` module with `FreshOverburdenVariant` (7 variants, default `UnsupportedAndRoom`)
- consts `FRESH_OVERBURDEN_MATERIAL_EPSILON` 1e-5, `FRESH_OVERBURDEN_SKIN_CELLS` 1.5, `FRESH_OVERBURDEN_ROOM_EPSILON` 1e-3, `SUPPORT_FRACTION_EPSILON` 0.02
- test helpers `perfect_sim_tick`, `settled_then_flipped`, `ALL_FRESH_OVERBURDEN_VARIANTS`, `report_fresh_overburden_fraction`

**VERIFIED 2026-08-05 after the interruption: it COMPILES AND PASSES.** `cargo test -p sandart-sim --release` = **93 passed / 1 failed / 23 ignored** against a 92/1/20 baseline, i.e. +1 regression test and +3 ignored diagnostics, with only the intentional symmetry failure. So the implementation is complete; what was lost is a call-site comment update and the measurement report.

**FIRST ACTION ON RESUME**: the code is fine, but it has NOT been measured. Before pushing, get from it (re-run its own ignored diagnostics):
- block fraction at 64 and 512 for resting pile / mid-drain hourglass / fresh flip, against perfect-sim's 20-33% reference
- cumulative and peak divergence vs perfect-sim at budget 256, before and after
- bench_sandfall ms/tick Water and DrySand
- the falling-column dump: how many rows deep accumulated overburden stays under the 1.5 threshold (this is the evidence that justified the support-fraction rewrite and belongs in the commit message)
- whether any of the 7 `FreshOverburdenVariant`s beats the shipped default, and whether any of them approaches perfect-sim's block fraction (if the only way to close the divergence is to promote what perfect-sim promotes, SAY SO — that means the predicate cannot beat just running everything)

Also check the call-site comment near the `must_simulate.push` for staleness; it may still describe the superseded overburden predicate rather than `support_fraction`.

If anything is wrong, `git checkout -- sandart-sim/src/physics.rs` and re-run from the brief in #58 — nothing here is precious.

    distrobox enter sandart-dev -- bash -lc "cd /home/deck/projects/sandart && cargo test -p sandart-sim --release 2>&1 | tail -20"

Expected baseline BEFORE this diff: 92 passed / 1 failed / 20 ignored. The 1 failure is `test_water_blob_stays_left_right_symmetric_under_gravity`, INTENTIONAL and FORBIDDEN to touch.

## Environment (easy to lose, expensive to rediscover)

- Host has cargo but **NO LINKER**. Anything that compiles must run in `distrobox enter sandart-dev`. That container has **no git and no jj**.
- `cargo check -p sandart-wasm` **typechecks nothing** — the crate is `#![cfg(target_arch = "wasm32")]`-gated and compiles to nothing on the host. Always `--target wasm32-unknown-unknown`.
- Integration tests do NOT run in the main test command; the intentional lib failure short-circuits it. Run `--test perfect_simulation_determinism` and `--test fresh_pressure_field_toggle` separately.
- VCS is jj colocated with git: `jj commit -m "..." <paths>`, `jj bookmark set main -r @-`, `jj git push --bookmark main`.
- No browser driver. The user is the only visual instrument, via the gh-pages deployment.
- `shader.wgsl` compiles at RUNTIME — a WGSL error fails no build and shows as a blank canvas. `cargo test -p sandart-render` is the guard.

## The four debug toggles now shipped (all in the UI "Debug" group, all default off)

1. **Perfect simulation** — every in-mask block holding material, every tick, ignoring `budget_n`. Correct BY CONSTRUCTION, so it is a ground truth.
2. **Block heat-map overlay** — per-block, how often each block was simulated (10 buckets x 30 ticks).
3. **Fresh pressure field (experimental)** — standalone unconditional `column_depth` pass instead of inline.
4. **Pressure heat-map overlay** — per-cell `column_depth`, `ln(1+x)/ln(1+512)`, violet->magenta->yellow.

## What the user established visually (these are settled, do not re-litigate)

- **Slabs are 100% a scheduling artifact.** Perfect simulation has no slabs. Physics not implicated.
- **The arch is REAL**, not a metric artifact. Confirmed on the deployment.
- **Arching happens WITHOUT the fresh field too.** The fresh field makes an existing failure worse; it does not introduce a new one. The defect is that arches do not COLLAPSE fast enough.
- **The pressure-field difference between inline and fresh is tiny** — "just a little bit thin layer of moving water on the top with lower pressure."

## The key diagnosis (this is the thread to pull)

The fresh field is CORRECT: it zeroes overburden in the thin moving surface layer. But lateral driving is `fill + k*column_depth`, so where overburden correctly goes to zero the driving collapses to the bare fill difference. **The moving surface layer is exactly where levelling must happen, so a correct pressure field removes the lateral driving from the one place that needs it.** The stale inline field was accidentally supplying it.

**The fix (user's framing, "flow toward pressure gradient but respect gravity"): depth should MULTIPLY the driving force, not ADD to it.**

    flux ~ conveyance(depth) * grad(free-surface elevation)

Three consequences, all matching observation: a flat lake cannot flow at any depth (today's additive term CAN drive flow on a flat surface when depths differ — **cheap unverified prediction worth testing headlessly**); the thin moving layer still levels because a cusp has a huge surface gradient even at near-zero depth; an arch has the largest surface gradient in the domain and collapses fast, and keeps doing so while draining.

This is the OTHER HALF of #55's elliptic solve. The solve supplies propagation, this supplies the right potential to propagate. Neither alone is enough.

## Agreed plan

1. **#47 slabs** via the #58 support predicate — IN FLIGHT, uncommitted (above).
2. **#55 elliptic head solve + multiplicative free-surface form** — the next major piece.
3. Then investigate the arch further if it has not dissolved.

## Numbers worth keeping

- Slab predicate v1 (overburden-based): divergence vs perfect-sim at budget 256 improved 106221->97251 cumulative (-8.4%), peak 1890->1629 (-14%). Block fraction 2-10% vs perfect-sim's 20-33%. Cost +10-15% ms/tick.
- **At `budget_n = usize::MAX` the fixed and unfixed runs track perfect-sim BIT-FOR-BIT.** The slab defect exists only under budget pressure.
- Why v1 under-detects: `in_transit_at` under-reports free fall (a column draining 0.77-1.00 -> 0-0.19 in ONE tick reported in_transit 0.0-0.55), leaving `resting_above` ~0.4-0.5/row, so accumulated overburden crosses the 1.5 threshold after ~3 rows. Hence the support-fraction rewrite.
- `test_liquid_flowing_liquid_does_not_stand_in_walls` baseline is **6** voids (the "19" in a physics.rs comment ~line 2930 is STALE, from 11cb1775, before ~10 physics commits — worth correcting in place).
- Arch under fresh field: ONE contiguous 55-cell region, several cells present 20/20 sampled ticks. Baseline's 6 voids are single-cell, max persistence 5/20.
- Perf paid so far: pressure projection +33-53%; vertical overburden Water +8% / DrySand -4%.

## Standing rules

- Never weaken, #[ignore], re-tune or retitle `test_water_blob_stays_left_right_symmetric_under_gravity`.
- Do not change `block_size` or the 32x32 tiling (lib.rs:444-451).
- Do not alter web colour-scheme `<option>` VALUES; do not hand-add `<option>` to the material select.
- `syncSettings()` pushes the whole panel on every control change — nothing on that path may reset the sim. (A bug of exactly this kind was fixed in b28ff5a.)
- Delegate implementation to Sonnet subagents; keep verification in the main thread.
- Be concise in replies; detail goes in tickets.

## Live tickets

#47 slabs (in flight), #58 support primitive, #55 elliptic solve + multiplicative form (next major), #57 the arch (cause unexplained, 5 candidates ruled out), #54 pressure drives every flow (half shipped), #56 damping vs real symmetry fix, #44 left drift, #53 pressure perf, #52 vertical striping (its lead hypothesis was refuted — sand CAN move sideways at depth, 675x-13050x depth/tau, equalises in one tick).
