# TASK 55 OPTIMISE — result

**Worktree:** `/home/deck/projects/sandart/.claude/worktrees/agent-aa6012d312194e921`
**File changed:** `sandart-sim/src/physics.rs` (only file touched)
**Gate:** `elliptic_head_gate` confirmed OFF in both places (`#[cfg(test)]` default `Cell::new(false)`
and the `#[cfg(not(test))]` production twin `is_enabled() -> false`). Suite is bit-identical to
baseline with the gate off.

## Headline

| | ms/tick (w=512, Water, full budget) | multiplier over gate-off |
|---|---|---|
| Baseline (gate off) | 3.34–3.36 | 1x |
| **Before this task's changes** (gate on, multigrid as handed off) | 55.8–57.7 (mean ≈ 56.2) | **≈16.8x** |
| **After this task's changes** (gate on) | 48.6–50.7 (mean ≈ 49.2) | **≈14.6x** |

Net: **~13% reduction in absolute ms/tick**, multiplier **16.8x → ~14.6x**. This does not reach the
~10x target stated in the brief. See "Honest negative result" below for why, and where the
remaining cost actually is.

**Exactness: bit-identical.** All three changes are provably order/value-preserving (argued per-change
below, not just tested), and this is corroborated empirically:
- Full `cargo test -p sandart-sim --release`: **96 passed, 1 failed, 36 ignored** before and after —
  identical to the measured baseline. The 1 failure is
  `test_water_blob_stays_left_right_symmetric_under_gravity`, the pre-existing intentional marker
  (untouched, unweakened).
- `--test perfect_simulation_determinism` and `--test fresh_pressure_field_toggle`: both pass, both
  tests, before and after.
- Three `#[ignore]`d TASK #55 diagnostics run with the gate forced on
  (`diag_task55_elliptic_resolution_scaling`, `diag_task55_elliptic_propagation`,
  `diag_task55_pocket_equalisation`) reproduce the **exact reference numbers cited in this task's own
  brief**: `ticks_to_halve` normalised = **21.5** at w=128, **9.875** at w=512 (brief cited 21.5 /
  9.875 verbatim). These are integer-tick-derived figures, sensitive to any behavioural drift, so an
  exact match is strong evidence nothing changed numerically.
- `cargo check -p sandart-wasm --target wasm32-unknown-unknown --release` typechecks clean.

One process note on the brief's revert-check instruction: "diff against `git show HEAD:...physics.rs`"
doesn't apply literally here — this worktree's `HEAD` predates the entire elliptic-pass/multigrid
work (it's from before that work was even started, since it was handed to me uncommitted). I verified
the revert directly instead: grepped `mod elliptic_head_gate` and confirmed both the test-default
`Cell` and the `#[cfg(not(test))]` twin read `false`, and grepped the whole file for any stray
diagnostic/instrumentation leftovers (none).

## What was profiled, and the honest negative result

The brief's hypothesis was that `HashMap<usize, Vec<usize>>`-based component grouping (step 4a of
`elliptic_liquid_level_pass`) dominates. **This was profiled directly (pprof flamegraph +
manual step-by-step `Instant`-based timing instrumentation, both removed before finishing) and found
to be wrong as a *cost* hypothesis, though right as a *code-smell* one:**

- A flamegraph of the pre-fix code showed `hashbrown::raw::RawTable::reserve_rehash` at **0.02%** of
  total `settle_tick` time (1 sample out of 4326). The HashMap's hashing/probing itself was never the
  problem — in this workload (one big connected water body per tick) there are only a handful of
  distinct components, so the hash table stays tiny and cache-resident.
- Direct step-by-step timing (temporary `Instant` instrumentation, later fully removed) attributed
  the elliptic pass's own cost roughly as: **smoother (step 5, 8 Gauss-Seidel iterations) ≈ 63%**,
  **coarse step (step 4a, contains the HashMap) ≈ 20%**, conveyance/capacity + edge-build loops
  ≈ 13%, domain scan + column-depth ≈ 4%.
- A rigorous back-to-back A/B (same machine session, reconstructed pre-fix code immediately
  before/after the post-fix code, 4 reps each) of *only* the HashMap→counting-sort replacement plus
  `edges: Vec::with_capacity` showed **no measurable improvement**: before 55.19–56.73ms (mean
  55.84), after 56.39–56.92ms (mean 56.60) — statistically indistinguishable, if anything slightly
  worse within noise. The flamegraph's other allocator-churn evidence (`RawVec::grow_one` →
  `realloc` → occasional `brk`, ~4.5% of total time) was real and is what `with_capacity` targets,
  but it wasn't enough to clear the noise floor on its own.

**Where the time actually goes:** `eta_of`, the closure evaluated to compute `eta` for both endpoints
of an edge, is called **~4 times per edge per smoother iteration** (twice building the residual,
twice inside `apply`) × `ELLIPTIC_ITERATIONS` (8) — millions of calls for a large connected body. Its
original form was:
```rust
temp_heights[idx] * depth_scale + cd[idx] - (idx / w) as f32 * depth_scale
```
`idx / w` is a **runtime integer division** (w is not a compile-time constant), repeated on every
single call for a divisor that is invariant for the entire function call. This — not the HashMap —
was the dominant, fixable cost.

## What was changed (three changes, all in `elliptic_liquid_level_pass`)

1. **`row_term` precompute** (the actual win). Added `row_term: Vec<f32>` filled inside step 3's
   existing per-domain-node loop (zero extra passes — folded into a loop that already visits every
   domain node once, same trick the function already uses for `parent`'s self-loop init). `eta_of`
   and `b_of` (step 4a) now read `row_term[idx]` instead of recomputing `(idx / w) as f32 *
   depth_scale`. **Exactness**: the cached value is produced by the identical formula, and neither
   closure's surrounding expression changes operation grouping (`... + cd[idx] - row_term[idx]`
   matches `... + cd[idx] - (idx/w) as f32 * depth_scale` term for term) — so this cannot perturb the
   result by even one float ULP, unlike a reassociation such as `(A+B)-C` → `A+(B-C)`, which floating
   point's non-associativity *would* make observable. This is the change that produced essentially
   all of the measured win (isolated before/after: ~56.6ms → ~49.2ms, ≈-13%).

2. **HashMap → counting sort** for step 4a's component grouping (the requested fix, kept despite the
   negative perf result above because it's strictly better practice and is not itself a source of any
   regression): `root_count`/`offsets`/`cursor`/`flat` replace `HashMap<usize, Vec<usize>>`. Two
   linear passes over the same row-major domain scan order steps 3/4 already use, one allocation of
   the exact final size instead of N hashmap probes + unbounded per-root `Vec::push` growth.
   **Exactness**: both passes visit domain nodes in the identical row-major order the HashMap version
   built its per-root Vecs in, so for any component `flat[start..start+count]` is element-for-element,
   order-for-order identical to the old `groups[&root]` Vec — meaning `total_mass`'s accumulation
   order, `events`' pre-sort order (so `sort_by`'s *stability* breaks ties identically), and the final
   delta-application order are all unchanged. Iterating components in root-index order rather than the
   HashMap's hasher-seed-dependent order is also safe: components are disjoint by construction, so one
   component's computation never reads or writes another's `temp_heights`/`net_activity` entries.

3. **`edges: Vec::with_capacity(domain_cell_count * 2)`** instead of `Vec::new()` grown via `push`
   (every domain node contributes at most 2 edges — right, down — so this is an exact upper bound).
   Purely a capacity hint; changes nothing about which elements end up in the Vec or their order, only
   how many times the backing buffer is reallocated while filling it.

## DrySand cost with the gate on (task's other ask)

The pass is liquid-only and exits at `domain_cell_count < 2` before doing anything else for a
non-liquid scenario, so this was expected to cost close to nothing beyond the one full-grid domain
scan (`is_node`/`participating_columns` construction).

| | ms/tick (w=512, DrySand, full budget) |
|---|---|
| Gate off | 21.6 |
| Gate on | 22.5–22.6 |

**≈4–4.6% overhead**, unchanged by this task's fixes (DrySand never reaches step 3 onward, so none of
the three changes touch its cost path). This matches the "cheap domain-membership check, nothing
more" expectation — it is not scanning anything it shouldn't.

## What's left if someone wants to push past ~14.6x

Not attempted here (out of this task's scope / risk budget), but worth naming since the direct timing
breakdown makes it clear where the remaining ~60%+ of the pass's own cost sits:

- The **smoother's 8 Gauss-Seidel iterations** over the full edge list are real, necessary,
  non-leaky arithmetic per the physics design (`ELLIPTIC_ITERATIONS` is deliberately fixed, not
  convergence-gated, so runs must be dropped, not skipped, to go faster — and dropping iterations
  changes the result, which this task's exactness constraint forbids). Any further win here would
  need either a genuinely cheaper per-edge kernel (e.g. removing the still-separate residual-check
  pass by fusing it into the previous iteration's `apply` sweep) or accepting a *different*, not
  merely faster, algorithm — both out of scope for "optimise, don't redesign."
- The O(n log n) sort inside step 4a's breakpoint sweep was **measured, not assumed**, at ~3.6% of
  total `settle_tick` time (flamegraph `driftsort_main`) — small enough that per the brief's own
  instruction ("measure before touching it") it was left alone.
