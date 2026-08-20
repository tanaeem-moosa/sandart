# Overclocking performance profile — 2026-08-20

Profiling/analysis only, per instruction. **Nothing is committed and nothing needs to land.**
The working tree does carry three small, disclosed diagnostic additions (listed in §0) needed to
produce the numbers below; none of them changes simulation behaviour (verified — see §0).

## 0. What changed in the tree, and why

- `Cargo.toml`: added a new `[profile.profiling]` (inherits `release`, `lto = false`,
  `codegen-units = 16`, `debug = true`). **`[profile.release]` itself is untouched** — the real
  shipped/wasm build is bit-for-bit what it was before this session (verified: rebuilt
  `diag_blocks` under plain `--release` after the edit, `[optimized]`, no debuginfo, and
  `cargo check --target wasm32-unknown-unknown` is clean). See §1 for why a separate profile was
  needed at all.
- `sandart-sim/src/lib.rs`: a `thread_local` (`EARLY_TERM_LOG`) plus ~15 lines in `update()`'s
  overclocking rep loop, and a public `early_term_log_take()`. Pure bookkeeping — it reads
  `last_displacements` right after each `settle_tick` call and records `(target sub-steps,
  first rep at which the block's real displacement fell under `MUST_SIMULATE_THRESHOLD`)`. It
  does not write to any buffer `settle_tick`/`update()` reads. Verified non-perturbing: lib suite
  is still **102 passed / 10 failed**, the same ten named failures, and `diag_blocks --overclock 1`
  reproduces OVERCLOCKING.md's numbers (Water 116.10 ms/frame, mass_err 8.95e-10; DrySand 75.46
  ms/frame, mass_err 5.78e-9 — both match the committed baseline within measurement noise).
- `sandart-sim/examples/profile_overclock.rs`: new. Builds the same Hourglass/overclock-on
  scenario as `diag_blocks`, samples it with `pprof` (already a dev-dependency, used by
  `profile_sim.rs`/`bench_sandfall.rs`), and buckets every sample by scanning the whole resolved
  stack for known physics.rs/coarse.rs markers. Prints the bucket breakdown, the top leaf
  symbols, the early-termination histogram, and optionally a flamegraph SVG.

## 1. Method

**`pprof` (in-process sampling), not `perf`.** `perf` was not installed in the `sandart-dev`
container; it was installed (`sudo pacman -S perf`, container has passwordless sudo) and works,
but `sandart-sim` already ships `pprof` as a dev-dependency with two existing examples
(`profile_sim.rs`, `bench_sandfall.rs --flamegraph`) built around it, so that was the lower-risk,
already-proven path and is what `profile_overclock.rs` extends. `flamegraph.svg` /
`flamegraph_gosper.svg` at the repo root are confirmed stale (they profile the Sandbox/Gosper-curve
scenario, not Sand-fall, and predate this feature) — new ones are at `/tmp/flame_water_overclock.svg`
and `/tmp/flame_drysand_overclock.svg` (not committed; regenerate with `--svg` if wanted).

**A real methodology problem, found and fixed.** The shipped `[profile.release]` uses fat LTO +
`codegen-units = 1`. Under that profile, sampling the identical binary twice in a row gave
**wildly different bucket splits** — one run showed the coarse level at 19%, the next run of the
*same binary* showed it at 69%. The bucket percentages were unstable; the underlying *leaf-level*
percentages (a top function at ~50%, another at ~19%, etc.) were stable across runs, but which
named function they were attributed to kept changing (`support_fraction` one run,
`fresh_overburden_must_blocks` the next, `accumulate_edge_totals` the run after — three genuinely
different functions in the source). That is a symbolizer artifact of aggressive whole-program
inlining under DWARF, not a real change in behaviour. Building a `debug=true` profile with LTO
**off** made the same measurement reproducible to within ~1 percentage point across repeated runs,
and — importantly — **did not change the measured ms/frame** (Water 116.3–116.6 ms/frame either
way, DrySand 75.3–75.7 either way), so the non-LTO numbers below are representative of the shipped
build's actual cost distribution, just symbolized reliably. All percentages below are from this
`profiling` build, each figure the average of two repeated runs.

**Classification.** Every pprof sample is a full resolved call stack (all native frames, all
symbols an inlined chain resolves to per frame). Each sample is bucketed by scanning the *whole*
stack, not just the leaf, for markers, in priority order: (1) any `coarse::CoarseState`/
`coarse::` frame → **Coarse** (this catches cost inside the coarse level's own nested
`settle_tick` call, including instances of a fine-level function like `advect_properties` running
*for the coarse grid* — that cost must not leak into the fine-level buckets); (2) LUT markers
(`cached_vertical_lut`, `lookup_equilibrium_lut`, `build_vertical_equilibrium_lut`) → **Lut**,
checked before the solver bucket because the LUT is called *from inside*
`overfill_equilibrium_transfer` and would otherwise be swallowed by it; (3) solver markers
(`overfill_equilibrium_transfer`, `solve_forward`, `overfill_pressure_val`, `cell_potential`,
`coarse_delta_eta_budgeted`, `relative_overfill` — the last four are the nonlinear pressure-excess
arithmetic the wrapper is built from, kept in this bucket regardless of whether the wrapper's own
frame survived inlining) → **EquilSolver**; (4) `advect_properties` → **Advection**; (5) anything
else inside `settle_tick` → **Scaffolding** (the catch-all: classification, arbitration/edge
bookkeeping, activation, the granular jitter/support helpers — everything that is not the solver,
the LUT, or advection). Command: `./target/profiling/examples/profile_overclock --material
{water,drysand} --ticks 400 --hz 997`, same Hourglass/gravity/preset setup as `diag_blocks
--overclock 1`.

## 2. Profile breakdown (percent of an overclocked frame, grid 512)

| bucket | Water | DrySand |
|---|---:|---:|
| **Scaffolding** (classification/arbitration/edge bookkeeping) | **66.9%** | **65.9%** |
| **EquilSolver** (`overfill_equilibrium_transfer`/`solve_forward` + pressure-excess arithmetic) | 16.5% | **29.0%** |
| **Lut** (`cached_vertical_lut`/`lookup_equilibrium_lut`/build) | 7.8% | **0%** |
| **Advection** (`advect_properties`) | 4.9% | 3.7% |
| **Coarse** (the coarse level's own tick: restrict/anchor/advance/export) | 3.6% | 1.0% |
| Other (malloc, `f32::exp`, etc.) | 0.3% | 0.4% |

ms/frame at these settings: Water 116.3–116.6, DrySand 75.3–75.7 (matches OVERCLOCKING.md's
120.10/81.88 within run-to-run noise — the biggest known noise source on this hardware is
Steam Deck thermal/frequency scaling, not this session's tree changes).

**DrySand's Lut is genuinely 0%, not a measurement gap.** `overfill_equilibrium_transfer`'s LUT
fast path is gated on `tau <= 0.0` (`physics.rs:998`) — zero yield stress. DrySand's granular
Mohr-Coulomb path carries `yield_tau > 0`, so it always takes the closed-form `solve_forward`
branch and never touches the LUT; that is also most of why DrySand's EquilSolver share
(29.0%, no LUT to offload any of it to) is almost double Water's (16.5% solver + 7.8% LUT =
24.3% combined, of which the LUT — a cache/table gather — carries about a third).

**Top leaf functions, both materials, roughly stable across repeats:** one function at ~50-53%
(`fresh_overburden_must_blocks`/`support_fraction`/`accumulate_edge_totals`/`settle_tick`
depending on which run's inlining decision survived — this is the Task #47 MUST-tier
classification scan plus the per-edge arbitration bookkeeping, i.e. genuinely Scaffolding, not an
artifact), then `overfill_pressure_val`/`relative_overfill`/`cell_potential` (the solver's
nonlinear term) at ~15-19% combined, `advect_properties` ~4-5%, `in_transit_at` ~2-4%,
`flux_edge_apply` ~1-3%.

**Context: most blocks are not overclocked at all.** `diag_blocks --overclock 1` (4096 blocks
total at grid 512/block_size 8): Water settles at `1/8=3069 1/4=44 1/2=121 1x=309 2x=219 4x=155
8x=179`, mean rate 0.794. The 553 blocks running at 2x/4x/8x are what the 66.9%/65.9%
Scaffolding share is actually paying for, replayed in full for every one of their extra
sub-steps — see §4.

## 3. Early-termination distribution

Job 2 candidate 5's second half: for every block genuinely overclocked this frame (`rate > 1`),
record the first sub-step at which its **real**, physically-computed `last_displacements`
(captured right after `settle_tick`, before `force_overclocked_blocks_active` re-forces it for
the next rep) already read under `MUST_SIMULATE_THRESHOLD` — i.e. it had already reached local
equilibrium and every subsequent forced sub-step this frame did no useful work.

| target sub-steps (n) | block-frames (Water) | settled early | avg settle-rep | block-frames (DrySand) | settled early | avg settle-rep |
|---:|---:|---:|---:|---:|---:|---:|
| 2 | 87,100 | **77.5%** | 0.06 | 72,801 | **85.9%** | 0.04 |
| 4 | 56,095 | **64.6%** | 0.10 | 55,549 | **65.9%** | 0.06 |
| 8 | 67,438 | **53.8%** | 0.02 | 78,785 | **57.1%** | 0.03 |

`avg settle-rep` near 0 means: when a block does settle early, it is almost always already done
by the very first forced repetition — the remaining n-1, n-3, or n-7 repetitions that
`force_overclocked_blocks_active` still runs for it are pure no-ops re-deriving the same
already-converged state.

Weighting by target n (a block at rate 8 that wastes its extra reps wastes 7 of them, one at rate
2 wastes at most 1), the fraction of **all extra-rep executions this frame that are on
already-settled blocks**: **~59% for Water, ~62% for DrySand.**

## 4. The five candidates, costed against this data

### 1. Batch advection once per n sub-steps
Advection is **4.9% (Water) / 3.7% (DrySand)** of the frame. Even hypothetically eliminating it
entirely caps the win at that — nowhere near the 4-5x target. The sharpness argument (fewer,
larger transfers smear less, already measured independently at 118/117/116/114 rows for
sub-steps 1/2/4/8) still stands on its own merits, but it is a **visual-quality change, not a
performance lever** at this scale. **Verdict: not worth doing for performance; revisit only as a
quality change, separately, with its own A/B.**

### 2. Cheap-approximation predictor + exact solver once (Newton step vs. piecewise-quadratic root find)
EquilSolver + Lut together are **24.3% (Water) / 29.0% (DrySand)** of the frame. Even an
unrealistically generous assumption — the cheap path costs ~0 and is used for all but one of
8 sub-steps — caps the win at roughly `7/8 * 24-29%` ≈ 21-25 percentage points, i.e. at best
**~1.3x** on the whole frame. Scaffolding (67%/66%) sets the ceiling, and this candidate does not
touch it. **Verdict: bounded, real but small; not close to 4-5x alone. Could be stacked on top of
candidate 5 for an incremental gain, not pursued first.**

### 3. SIMD 8x8 block arithmetic (wasm32 SIMD128, 4 lanes)
Same ceiling problem as #2 — it can only attack the ≤29% EquilSolver/Lut slice, and that slice is
explicitly the branchy part (regime detection, breakpoint walking) plus a table gather, neither of
which vectorizes cleanly at 4 f32 lanes on wasm32 (gather has no native fast instruction; branch
divergence forces scalar fallback inside a SIMD lane group). A realistic yield on that slice is
1.5-2x, not 4x, giving **at best ~5-10% off the whole frame**, and Scaffolding — the 66-67%
majority — is scalar, branchy, irregular-access bookkeeping that does not vectorize at all.
**Verdict: not worth the complexity here; the ceiling is too low and the target code is the worst
possible fit for it.**

### 4. Gauss-Seidel instead of Jacobi for the 8x8 block
The direct bookkeeping cost of the Jacobi double-buffer (`temp_heights` copy/zero) is small in
this profile (`core::ptr::copy_nonoverlapping` + `alloc_zeroed`/`realloc` together ≈ 1.5-2% of
frame) — removing the buffer alone is not the win. The *real* claimed benefit is different:
faster **per-sub-step convergence**, which would mean a lower clock rate could deliver the same
transport rate, i.e. fewer sub-steps needed in the first place. That's a genuine, different
lever from anything measured directly here (I have no sweep-count/convergence-rate instrument in
this profile — `settle_tick` does not have a distinct internal sweep loop separate from the
sub-step repetitions themselves, so "faster per sweep" here means "faster per overclocking
sub-step", which would compound with candidate 5, not substitute for it). **Flagging the risk as
instructed: it makes results sweep-order dependent, and this project already carries a deliberate
left-right symmetry failure (`test_water_blob_stays_left_right_symmetric_under_gravity`, fails ON
PURPOSE) and a separately-tracked left-drift ticket that came from exactly this class of ordering
change.** **Verdict: plausible but unquantified without a real implementation attempt; given the
project's own history with this exact failure mode, not something to prototype in a
profiling-only session. Worth a dedicated, carefully-verified follow-up if candidate 5 alone
doesn't reach the target.**

### 5. Sub-linear sub-stepping (hoist scaffolding + early-terminate) — clear winner
This is where the money is. Scaffolding is **66.9%/65.9%** of the frame — directly the
"classification, tier selection, arbitration, copy-back" the task named — and §3 shows **~59-62%
of all extra-sub-step executions are on blocks that had already reached local equilibrium**, not
skipped only because `force_overclocked_blocks_active` unconditionally floors their eligibility
for every one of their `rate` repetitions regardless of whether they still have anything to do.
Both halves of this candidate are supported directly by the data, and together they bound on the
dominant cost in the profile (Scaffolding) plus a meaningful share of EquilSolver/Lut (a
skipped block also skips its own solver/LUT calls).

**Estimated factor:** back-of-envelope, eliminating ~60% of wasted extra-rep executions against a
frame where Scaffolding+EquilSolver+Lut (executed once per sub-step) is ~91% (Water) / ~95%
(DrySand) of total time puts a plausible win somewhere in the 2-3x range from early termination
alone, with the hoisting half (running classification once per frame rather than once per
sub-step) adding more on top — genuinely in reach of the stated 4-5x target, unlike any of the
other four candidates individually or combined.

**Why it is not prototyped in this session.** Both halves have a real, specific correctness risk,
not a hypothetical one:
- **Early termination**: `force_overclocked_blocks_active`'s own doc comment explains it
  deliberately over-forces — it also forces every grid-adjacent neighbour of an overclocked
  block, "regardless of the neighbour's own rate", specifically to avoid a half-fast/half-slow
  edge-ownership inconsistency (edges are owned by their lower-index cell — OVERCLOCKING.md's S3).
  Skipping the forced floor for an *already-settled* block, without touching the neighbour-forcing
  rule, is the narrow, low-risk-looking version — but verifying it does not reopen exactly that
  class of edge-ownership bug needs the project's full symmetry/mass-conservation test battery
  run and read carefully, not a five-minute check.
- **Hoisting classification**: `fresh_overburden_must_blocks` reads live heights each sub-step;
  a block that only becomes MUST-eligible during sub-step 3 of 8 would silently miss promotion if
  classification ran once at sub-step 0. This needs a real design decision (re-run classification
  cheaply, or accept the staleness and bound it), not a mechanical hoist.

Per the task's own framing, this is a case where **the highest-leverage lever is clearly
identified and clearly costed, but is not "low-risk" to prototype in the time available** — both
halves touch scheduling correctness this project has been bitten by before (documented symmetry
failure, a separate left-drift ticket). Recommend it as the next dedicated implementation task,
scoped and verified against the full suite (`mass_err`, the symmetry tests, lib suite), not
something to land speculatively here.

## 5. Summary

**Top three cost centres:** (1) Scaffolding — block classification (Task #47's
`fresh_overburden_must_blocks`) + per-edge arbitration bookkeeping, **~66-67% of an overclocked
frame in both materials**; (2) the equilibrium solver's nonlinear pressure-excess arithmetic,
**16.5% (Water) / 29.0% (DrySand)**; (3) for Water only, the vertical-liquid LUT path, **7.8%**
(DrySand takes 0%, structurally — `tau > 0` always skips it).

**Highest-leverage optimisation:** #5, sub-linear sub-stepping (hoist scaffolding out of the
per-sub-step loop; early-terminate a block once it reaches local equilibrium). Estimated factor:
plausibly **2-3x from early termination alone**, with more available from hoisting — the only
candidate of the five with a realistic path to the stated 4-5x target, because it is the only one
that attacks the 66-67% majority rather than the ≤29% solver/LUT minority.

**Prototyped:** no. The lever is real and clearly the right one, but both of its two mechanisms
touch scheduling-correctness invariants (edge-ownership symmetry across clock-rate boundaries,
live-vs-stale classification) that this project has previously broken in exactly this way
(documented left-right symmetry failure, a separate left-drift ticket). Per the task's own
instruction, that combination — real lever, not verifiable as low-risk on this session's clock —
means delivering the profile and the costed recommendation is the correct stopping point, not a
speculative landing.
