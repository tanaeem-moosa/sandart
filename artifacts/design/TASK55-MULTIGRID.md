# Task #55 — multigrid replacement for the flat elliptic smoother

Read this cold. Starting tree: detached HEAD `9f38d31` ("Add the elliptic propagation solve and
resolution-parametric levelling diagnostics") — the exact checkpoint the brief describes, with
`elliptic_liquid_level_pass` already scoped (Job 1/2 from `TASK55-PERF.md`) and gated OFF by
default. Only file touched: `sandart-sim/src/physics.rs`. Nothing committed. `elliptic_head_gate`
reads `false` in the non-test build, verified by diff against `git show HEAD:...` (see "Constraints
checklist" at the end).

## Criterion 1 — does normalised `ticks_to_halve` FALL as w grows? Yes, clearly.

`diag_task55_elliptic_resolution_scaling`, `--ignored --nocapture`, unmodified:

| w | mode | gate | ticks_to_halve (raw) | normalised (÷ w/64) |
|---|------|------|----------------------:|---------------------:|
| 128 | adaptive | OFF | 128 | 64.0 |
| 128 | adaptive | ON (multigrid) | **43** | **21.5** |
| 128 | perfect_sim | OFF | 136 | 68.0 |
| 128 | perfect_sim | ON (multigrid) | **42** | **21.0** |
| 512 | adaptive | OFF | 522 | 65.25 |
| 512 | adaptive | ON (multigrid) | **79** | **9.875** |

For reference, the OLD flat-smoother-only figures this replaces (from `TASK55-ELLIPTIC.md`, same
diagnostic, same scenario): w=128 ON=115 (normalised 57.5), w=512 ON=466 (normalised 58.25) — flat
across both widths, the signature of a fixed-reach-per-tick process.

**With the multigrid coarse step, the normalised figure drops from 21.5 at w=128 to 9.875 at
w=512 — falling, not flat, as w grows by 4x.** That is the qualitative signature the task asked
for: the coarse step's reach is not bounded by a fixed cell count, so a wider domain (more of it
inside the SAME connected component) gets proportionally MORE benefit from a single per-tick
correction, not less. Raw ticks-to-halve also improved far beyond the old flat-smoother numbers:
128→43 (66% cut) and 522→79 (85% cut), vs. the old smoother's 128→115 and 522→466 (~10-18% cuts).

## Criterion 2 — cost. 16.4x, down from 45x. Still above the "sluggish but usable" (~10x) line.

Command (exact, per brief): `cargo run --release --example bench_sandfall -- --ticks 60
--materials water --budgets 1024 --reps 3 --warmup 5`, w=512, gate reached via the temporary
`#[cfg(not(test))]` twin edit (reverted after, diff-verified — see checklist).

| config | ms/tick | multiplier vs. gate-off |
|---|---:|---:|
| gate OFF (baseline) | 3.4367 | 1.0x |
| gate ON, flat 48-iteration smoother (prior agent's number) | 153.3–154.5 | ~45x |
| gate ON, **this multigrid build** | **56.2719** | **16.4x** |

A ~2.7x reduction in the gate's own overhead (45x → 16.4x), from replacing a fixed 48-iteration
Gauss-Seidel sweep with (per component) an O(n log n) exact coarse solve plus an 8-iteration
post-smoother. **This is a real improvement but does not cross the "~10x, shippable as a
default-off debug toggle" line the brief set** — it lands roughly 1.6x past it, closer to shippable
than before but not there. See "What's unresolved" for where the remaining cost most likely is.

## Criterion 3 — pocket equalisation at w=512 must not regress from 8. It improved to 6.

`diag_task55_composition_pockets_w512`, `fresh_pressure=OFF / elliptic=ON`: **`ticks_to_halve = 6`**
(down from the prior flat-smoother figure of 8, and from shipped baseline's 29). Composition with
`fresh_pressure=ON` also improved: 33 (old flat-smoother figure) → **13**.

## Criterion 4 — `test_dry_sand_has_angle_of_repose`, gate forced ON

Run via the existing `diag_task55_elliptic_propagation` (`std::panic::catch_unwind` around the
unmodified test, gate forced on for the duration): **PASS.**

```
CASE 1 (steep):   initial=0.3500 (19.29 deg) -> final=0.0886 (5.07 deg), total_flow=412.70
CASE 2 (shallow): initial=0.0532 -> final=0.0534 (3.06 deg)
CASE 3 (at angle):initial=0.0886 -> final=0.0760 (4.34 deg)
CASE 4 (deposit): flank_slope=0.0972 (5.55 deg), total_flow=74.00
NON-VACUITY ANCHOR @450 ticks: DrySand=0.0652 (3.73 deg), Water=0.0000 (0.00 deg)
```

These numbers are **bit-identical** to the pre-multigrid flat-smoother run reported in
`TASK55-ELLIPTIC.md`. That is expected, not a coincidence worth worrying about: the domain
restriction (`liquidity(wetness) >= LIQUID_ELLIPTIC_THRESHOLD`) is unchanged, so DrySand cells never
enter either the coarse or the fine part of this solve regardless of which one runs — the repose
mechanism is structurally untouched by this rewrite, only the liquid-only propagation path is.

## Criterion 5 — residual monotonicity and mass conservation

`test_elliptic_residual_falls_monotonically` — **PASS.** Trace (coarse-step residual, then 8
post-smoother iterations, then final):

```
[0.121082, 0.121082, 0.121078, 0.121071, 0.121063, 0.121059, 0.121052, 0.121044, 0.121037, 0.121029]
```

Non-increasing throughout (asserted); mass conserved to within the test's 1e-3 tolerance (asserted,
and true by construction — see "Mass conservation" below). `test_elliptic_eta_is_row_independent`
also passes unmodified (the `eta` formula itself was not touched).

Full suite: `cargo test -p sandart-sim --release` → **96 passed / 1 failed / 36 ignored**, the one
failure being `test_water_blob_stays_left_right_symmetric_under_gravity`, the pre-existing
intentional marker, untouched, same scan order — bit-identical to the pre-multigrid baseline.
`--test perfect_simulation_determinism` (2/2) and `--test fresh_pressure_field_toggle` (2/2) both
green. `cargo check -p sandart-wasm --target wasm32-unknown-unknown --release` typechecks clean.

---

## What was built: a two-level scheme, not a full V-cycle hierarchy

Per the brief's own suggestion, started with the two-level scheme and measured before going
further — the numbers above say it already delivers the qualitative fix (criterion 1), so a deeper
hierarchy was not attempted.

**The insight exploited.** For `div(conveyance * grad(eta)) = 0` (no source term) on a connected
domain, the equilibrium is provably `eta = constant` everywhere in that component, REGARDLESS of
how conveyance varies spatially — conveyance only affects the RATE of approach to equilibrium, not
the equilibrium value itself. So "the coarse grid" here is not a spatially-downsampled copy of the
fine grid (which is where 2x2-style aggregation would have to reason about mask boundaries and risk
straddling one, per the brief's warning) — it is algebraically **one node per connected component**,
and the equilibrium at that one node is computable directly, with no iteration.

**Connectivity — reused, not rebuilt.** Components are found via union-find (`uf_find`/`uf_union`,
new small helpers, path-compression, no rank heuristic) applied to the SAME edge list the fine
Gauss-Seidel smoother already builds (every adjacent in-domain pair). This sidesteps the
"two-wells-through-a-basin" trap entirely: the basin's own chain of cell-to-cell edges transitively
unions both wells into one component by construction, the same way the fine solver already treats
them as one connected graph — no separate geometric reasoning about basins, masks, or component
shape was needed. `diag_task55_composition_pockets_w512` (the scenario built exactly to catch a
naive 2x2-aggregation failure here) improved, not regressed (criterion 3), which is the direct
confirmation this connectivity handling is correct on that case.

**The coarse solve — exact, O(n log n), no bisection.** Every node's fill-vs-target-eta relation
has the SAME slope (`1/depth_scale`), so "how much total mass does this component hold at a given
shared eta level" is a monotone piecewise-linear function of that level with exactly 2 breakpoints
per node (where it turns on at `eta = cd[i] - row(i)*depth_scale`, where it saturates at
`+ cap[i]*depth_scale` above that). Sorting all `2n` breakpoints once and sweeping them (tracking
how many nodes are "active" between consecutive breakpoints) finds the exact `eta*` that matches the
component's current total mass by one division inside the correct interval — `O(n log n)` per
component, no bisection, no fixed iteration budget standing in for convergence.

**Applying the correction — damped, mass-exact, capacity-exact, no `clamp_edge_feasible` needed
here.** `MULTIGRID_COARSE_OMEGA = 0.5` (new constant, same conservative-starting-point reasoning as
`ELLIPTIC_EDGE_OMEGA`, not swept against a target): each node moves from its current fill toward its
individually-clamped target fill by that fraction — a plain per-node linear interpolation, not an
edge transfer, so `clamp_edge_feasible` does not apply to it (that requirement is honored by leaving
every FINE-level transfer, the post-smoother's edge sweeps, exactly as it was, still individually
clamped through `clamp_edge_feasible`). Both invariants hold algebraically, not by a defensive
fixup pass:
- **Capacity.** `[0, cap[i]]` is convex; both the current value and the target value already lie in
  it (the target is clamped at construction), so any convex combination of the two does too.
- **Mass.** Summing the per-node delta over a component telescopes to
  `omega * (sum(target_i) - sum(current_i))`, and `sum(target_i) == sum(current_i)` by construction
  of `eta*` (that equality is exactly the equation the breakpoint sweep solves) — so the
  component's total mass moves by (float-precision-only) zero, with no separate reconciliation step
  needed.

**The post-smoother — repurposed, not removed.** `ELLIPTIC_ITERATIONS` dropped from 48 to **8**.
Its job changed: before, it carried the entire correction by itself (bounded reach = the propagation
defect this task exists to fix); now the coarse step already carries the global part, so the
smoother's remaining job is polishing local structure the coarse step's per-node-independent target
doesn't model (conveyance-driven rate differences between neighbours). 8 was chosen as a modest,
round clean-up budget (a full forward+backward alternating pair runs 4 times over), not tuned to
land inside any test's pass window — the resolution-scaling and cost numbers above are what actually
show its effect, not a target this number was reverse-engineered from.

## What's unresolved

- **Cost is improved (45x → 16.4x) but still past the "~10x, shippable as default-off debug
  toggle" bar the brief set.** The likely remaining cost driver, not investigated further given
  time: the coarse-step's component grouping currently uses a `HashMap<usize, Vec<usize>>` (one
  more full scoped pass over domain nodes, plus hashing overhead) rather than a flat counting-sort
  into a pre-sized array — a cache-friendlier grouping pass is a plausible further win that wasn't
  attempted here. The `events.sort_by` per component is also unexplored for further optimisation
  (e.g. avoiding the sort for small components, or batching the breakpoint arrays without
  reallocating per component).
- **`diag_task55_composition_arch_w512`** (the `fresh_pressure_field` × `elliptic_head_gate` arch
  matrix from `TASK55-PERF.md`) was NOT re-run — it takes ~570s wall-clock even at the OLD 45x cost
  and wasn't required by this task's acceptance criteria list, but a prior agent's finding that
  ON/ON there beat shipped baseline on every metric measured is presumably now even more favourable
  given `diag_task55_elliptic_propagation`'s own ARCH numbers here (71→22 ticks adaptive, 65→23
  perfect_sim, both markedly better than the old flat-smoother's 71→44/65→47) — but this is
  inference, not a re-measurement, and is flagged as such rather than reported as fact.
  `diag_task55_composition_pockets_w512` (criterion 3, required) WAS re-run and reported above.
- **`MULTIGRID_COARSE_OMEGA` and the reduced `ELLIPTIC_ITERATIONS = 8`** are both first-choice,
  documented-reasoning picks, not swept. The brief explicitly warns against picking a constant
  because it makes a test pass; neither was chosen that way, but neither has been explored for
  whether a different value moves the cost/propagation tradeoff further in either direction.
- **Did not build a deeper (3+ level) multigrid hierarchy.** The two-level scheme already delivers
  the qualitative fix (criterion 1's falling normalised figure); a real geometric multi-level
  hierarchy was not attempted, per the brief's own permission to stop at two levels once measured.

## Constraints checklist

- `elliptic_head_gate`'s `#[cfg(not(test))]` twin: temporarily edited to `true` to reach the gate
  from `bench_sandfall` (criterion 2's measurement), then reverted to `false`. Verified: the
  current block is byte-for-byte identical to `git show HEAD:sandart-sim/src/physics.rs`'s
  corresponding block (both read `false`).
- All gates OFF by default in the tree left behind (`elliptic_head_gate`,
  `multiplicative_lateral_gate`, `fresh_pressure_field`, `pressure_gate`, `upstream_wake_gate`,
  `fresh_overburden_gate` — none of these were touched except `elliptic_head_gate`'s temporary
  revert-verified edit above).
- Full suite bit-identical to pre-change baseline with gates off: 96 passed / 1 known-failing / 36
  ignored, both before and after this session's changes.
- `pressure_project`, `clamp_edge_feasible`, `support_fraction`, `fresh_overburden_must_blocks`,
  `recompute_column_depth` — none modified. `clamp_edge_feasible` is still the ONLY mechanism the
  fine-level (post-smoother) edge transfers use; the coarse-level correction is node-local (not an
  edge transfer) and does not call it, for the reason explained above.
- `block_size` / 32x32 tiling: untouched.
- Nothing committed. `git status --porcelain` shows only `sandart-sim/src/physics.rs` modified.

## Files touched

`sandart-sim/src/physics.rs` only:
- New: `uf_find`, `uf_union` (union-find helpers).
- New: step 4a inside `elliptic_liquid_level_pass` (the coarse-grid per-component solve).
- New constant: `MULTIGRID_COARSE_OMEGA`.
- Changed: `ELLIPTIC_ITERATIONS` (48 → 8, doc comment rewritten to describe its new post-smoother
  role in the multigrid design).
- Changed: `elliptic_liquid_level_pass`'s own top doc comment (added a "MULTIGRID ADDENDUM" section
  summarising this revision; the rest of the original doc comment, describing the still-unchanged
  fine-level mechanics, was left in place).
- No changes to any diagnostic (`diag_task55_*`), any other gate, or any test — all re-run
  unmodified against the new code.
