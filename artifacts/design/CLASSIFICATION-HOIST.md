# Classification hoist — Stage 1 implemented, Stage 2 not attempted, 2026-08-20

Implementation session, against `HEAD` = `37956de3`. **Stage 1 landed in the tree** (four files,
listed in §6). **Stage 2 was not attempted** — Stage 1's own measurement gate failed, per this
task's explicit instruction to stop and report rather than proceed. That failure turned out to be
the important finding: it falsifies SCAFFOLDING-BREAKDOWN.md's attribution, not just its estimate.

## 0. Answer, up front

- **Stage 1 (hoist classification out of the repetition loop): done, correct, safe, but only a
  ~3% win, not the predicted ~2x.** `fresh_overburden_must_blocks`/`support_fraction` is now
  computed once per frame (confirmed by direct instrumentation: ~8:1 cached:live call ratio,
  matching `extra_reps + 1 = 8`) and its own profiled cost collapsed from an estimated ~54% to a
  **directly measured 0.5-0.8%** of the frame. Real ms/frame moved only **~3-4ms (~3%)** in the
  shipped release build.
- **Why the predicted ~47pt win didn't show up: SCAFFOLDING-BREAKDOWN.md's attribution was wrong.**
  Re-profiling after the hoist, with the same finer Classification/Arbitration split that document
  used, shows the ~54-66% chunk it called "Classification" was actually mostly **Arbitration**
  (edge accumulate/arbitrate/apply bookkeeping) all along. That document's own methodology section
  disclosed it could not cleanly resolve the two by sampling and fell back on an architectural
  argument ("only `fresh_overburden_must_blocks` has the right shape") to break the tie. That
  argument was the error — now falsified by direct measurement now that classification's real cost
  is known and small.
- **Slab divergence (the correctness gate) is untouched, bit-for-bit.**
  `test_fresh_overburden_predicate_reduces_slab_divergence` reports the exact same numbers before
  and after Stage 1: peak_before=1869.957, peak_after=546.819, cumulative_before=107291.522,
  cumulative_after=15860.617. (This test doesn't exercise the hoisted path directly — see §2 — but
  it does prove the extraction into `compute_fresh_active` didn't change the live/uncached
  behaviour every other call site still uses.)
- **Stage 2 not attempted.** The task's Stage 1 section says explicitly: "If you do not see roughly
  [the ~47pt win], stop and report — it would mean the breakdown's attribution was wrong." That is
  exactly what happened, so Stage 2 (drive block selection from coarse `|Delta|`) was not built.
  Whether `|Delta|` "subsumes" the support predicate was not tested — that question was never
  reached.
- **Full verification suite holds**: lib suite 102/10 (same ten named failures), all eight
  integration suites pass, wasm/desktop checks clean, `node scripts/check_js.js` clean. `mass_err`
  and descent reported in §3.

## 1. What Stage 1 actually changed

`fresh_overburden_must_blocks`'s own doc comment ("only ever adds indices to `must_simulate`; it
never feeds a physics quantity") is what makes this safe to hoist at all — it decides *what runs*,
not what the physics computes.

- New `compute_fresh_active(...)` function in `physics.rs` (near `fresh_overburden_must_blocks`,
  physics.rs ~1427-1470): extracted, verbatim, from the inline block `settle_tick` used to run on
  every call. Defensive: indices beyond `last_displacements`'s current length read as `0.0` (needed),
  the same default a freshly-resized buffer already has, so a caller computing this right after a
  grid resize doesn't need its own resize logic.
- `settle_tick` gained one new parameter, `precomputed_fresh_active: Option<&[bool]>`, inserted
  after `coarse_delta` and before `touched_out`. `None` reproduces the exact pre-hoist behaviour
  (recompute live, every call) — this is what all ~23 non-production call sites pass (20 in
  `physics.rs`'s own tests, one each in `coarse.rs`'s nested coarse tick and
  `task55_head_spec.rs`'s harness), so nothing outside the one production loop changed behaviour.
- `lib.rs`'s `update()`: `fresh_active` is computed once, via `physics::compute_fresh_active(...)`,
  immediately before the overclocking `for rep in 0..=extra_reps` loop, using the frame's pre-tick
  state (identical to what `rep == 0` would have computed live under the old code). Every
  repetition's `settle_tick` call now passes `Some(&fresh_active)`.
- **Verified the caching mechanism actually engages** (not just compiles): a temporary call-count
  instrument (thread-local counters in the `Some`/`None` arms, plus a print in `diag_blocks.rs`,
  both reverted — see §6) measured `cached=2870 live=360` over 360 ticks (300 profiled + 60
  warmup) — `live` is exactly one call per tick (the coarse level's own unconditional nested
  `settle_tick`, which always passes `None` — see `coarse.rs`), and `cached` is very close to
  `8 x 360 = 2880` (the shortfall is ticks near the start of warmup before `block_clock_rate` ramps
  to 8x). The hoist is doing what it was designed to do.

## 2. Slab divergence: bit-identical before and after

`test_fresh_overburden_predicate_reduces_slab_divergence` (physics.rs ~15756) uses `TestSim::tick`,
which calls `settle_tick` directly, once per tick, always passing `None` for
`precomputed_fresh_active` — it does not go through `lib.rs`'s overclocking repetition loop at all
(`TestSim` has no `extra_reps`/`force_overclocked_blocks_active` machinery). So this instrument does
not exercise Stage 1's caching path; it verifies the *extraction* was behaviour-preserving for the
live/uncached path every non-production caller still uses. Confirmed via `git stash`: identical
output with Stage 1's code stashed out vs. applied —

    peak_before=1869.957  peak_after=546.819  cumulative_before=107291.522  cumulative_after=15860.617

— both states, bit-for-bit.

**This does not by itself prove the caching path is safe against the slab defect under real
overclocking** (the task's own stated risk: "a block supported at rep 1 may be unsupported by rep
5"). That risk was not separately instrumented, because the Stage 1 performance gate failed first
(§0, §4) and the task's explicit instruction was to stop at that point rather than push forward.
What *is* covered: `overclocking_toggle.rs`'s `overclocking_enabled_does_not_leak_mass` and
`overclocking_enabled_diverges_from_default`, and `perfect_simulation_determinism`'s own suite, all
pass unchanged (§6) — these exercise the real overclocking loop with Stage 1's caching live, and
none regressed. That is evidence of no *conservation* or *gross* regression, not a targeted
slab-under-overclocking divergence measurement.

## 3. Stage 1 performance measurement, grid 512, `--overclock 1 --ticks 300`, real release build

Clean A/B via `git stash` (Stage 1 code stashed out = true `HEAD`, then restored), same host, back
to back to minimize thermal-scaling noise (documented as this hardware's biggest noise source):

| material | state | ms/frame (repeat runs) | mass_err | descent | must (last-rep) |
|---|---|---|---|---|---|
| Water | before (HEAD) | 116.31, 116.48, 117.05, 117.38 | 2.88e-9 | 0.06129 | 489.5 |
| Water | **after (Stage 1)** | **113.25, 113.31, 113.41** | 7.22e-10 | 0.06129 | 480.7 |
| DrySand | before (HEAD) | 76.50, 76.74 | 2.14e-9 | 0.07449 | 411.3 |
| DrySand | **after (Stage 1)** | **74.11, 74.65, 74.71** | 4.17e-9 | 0.07449 | 411.6 |

**Water: ~117 -> ~113 ms/frame, ~3.4ms / ~3.0%.** Predicted (SCAFFOLDING-BREAKDOWN.md §4):
~119 -> ~60ms, ~47pt. **Not observed.**

**DrySand: ~76.6 -> ~74.7 ms/frame, ~1.9ms / ~2.5%.** Predicted: ~46pt off. **Not observed.**

`mass_err` stayed within the same order of magnitude both materials (Water actually improved;
DrySand roughly doubled but is still ~4e-9, far under any concerning threshold). **Descent is
bit-identical** in both materials (0.06129 / 0.07449) — material movement is unaffected, exactly as
predicted for a scheduling-only change. The `must`-block count (last repetition's classification
snapshot) shifted slightly (489.5->480.7 Water, roughly flat DrySand) — consistent with the
predicted risk (a block that would have earned MUST status mid-frame under live recomputation is
occasionally missed when the mask is frozen at frame start) — but small, and not reflected in any
conservation or symmetry test regression (§6).

## 4. Re-profile: what's actually dominant, and why the estimate was wrong

Per instruction, re-profiled with `sandart-sim/examples/profile_overclock.rs` under
`[profile.profiling]` (LTO off, `debug = true`, non-perturbing per PERF-PROFILE.md's own
methodology note).

**First, the shipped (unmodified) bucket set**, single water run: **108.69 ms/frame**
(down from the pre-Stage-1 profiling-build baseline of 114-116 — a real but small ~5-7ms win,
consistent with the real-build ~3-4ms figure in §3). Bucket breakdown: Scaffolding 64.10%,
EquilSolver 17.82%, Lut 8.16%, Advection 5.23%, Coarse 3.77%, Other 0.92%. **The top leaf symbol is
still bare `sandart_sim::physics::settle_tick` at 51.87%** — the same unresolved-symbol pattern
PERF-PROFILE.md and SCAFFOLDING-BREAKDOWN.md both documented, now recurring at a size that shows
Stage 1 alone didn't shrink the dominant bucket.

**To find out what that 51.87-64.10% actually is now, re-added SCAFFOLDING-BREAKDOWN.md's own
Classification/Arbitration split as a temporary instrument** (reverted after, §6 — confirmed
byte-identical to `HEAD` via `md5sum` both before and after). Four runs (3 water, 1 drysand),
same symbolizer instability as before, but this time **Classification resolves cleanly and
consistently small in every run**:

| run | ms/frame | Classification | Arbitration | Scaffolding (unresolved) | EquilSolver | notes |
|---|---:|---:|---:|---:|---:|---|
| Water 1 | 108.14 | **0.58%** | 3.01% | 61.46% | 17.42% | big chunk unresolved |
| Water 2 | 108.37 | **0.53%** | **54.72%** | 9.60% | 17.65% | big chunk resolves as Arbitration |
| Water 3 | 107.94 | **0.57%** | 2.99% | 61.54% | 17.34% | big chunk unresolved |
| DrySand 1 | 70.72 | **0.78%** | **52.36%** | 10.52% | 30.65% | big chunk resolves as Arbitration |

**Classification is now directly, consistently measured at 0.5-0.8% of the frame** — confirming the
hoist worked exactly as designed and eliminated almost all of its own cost. **But the big chunk that
used to get attributed to Classification did not shrink** — it is 61-64 points either "unresolved"
or, when the symbolizer manages to name it, **Arbitration at 52-55%**, matching the total magnitude
(Classification-was-here + Arbitration's-own-2-3% in the old accounting) almost exactly. This is the
direct falsification: **SCAFFOLDING-BREAKDOWN.md's tie-break argument for attributing the ambiguous
chunk to Classification over Arbitration ("architecture rules out anything but
`fresh_overburden_must_blocks`... nothing else has that whole-grid, once-per-call shape") was
wrong.** Arbitration runs per active edge, not once per call — but at grid 512 under heavy
overclocking (553 blocks running extra sub-steps, 178-209 of them at 8x, per OVERCLOCKING.md's own
rate histogram), the number of active edges evaluated per frame is evidently large enough to be the
real dominant cost, not the small, flat share that document's *unambiguous* runs (where Arbitration
resolved cleanly, at 2.3-2.9%) suggested. Those clean-resolution runs were apparently sampling a
lighter moment, or arbitration's cost is itself state-dependent in a way flat repeat sampling missed
— either way, the number the document reported for Arbitration was not representative once
classification was actually reduced to isolate it.

**Dominant cost, once classification is dealt with: per-edge arbitration bookkeeping
(`accumulate_edge_totals`/`edge_arbitration_scale`/`flux_edge_apply`/`budget_term`/
`edge_share_jitter`/`activate_neighbor*`/`flux_edge_candidate`), roughly 52-64% of an overclocked
frame** — reported with the same honesty about attribution uncertainty PERF-PROFILE.md and
SCAFFOLDING-BREAKDOWN.md both flagged: the exact split between "Arbitration" and "unresolved" is a
symbolizer artifact, not a real behavioural difference between runs, but the *combined* magnitude
(61-64% either way) is stable and no longer ambiguous with Classification, which is now separately
and consistently small.

## 5. Stage 2: not attempted

The task's own instruction: "If you do not see roughly that [~47pt improvement], stop and report —
it would mean the breakdown's attribution was wrong. ... If `|Delta|` does NOT subsume it, say so
plainly and stop at Stage 1. That is a perfectly good outcome." Stage 1's gate failed (§3, §4), so
Stage 2 (drive block selection from coarse `|Delta|` instead of the support predicate) was not
built, and the question of whether `|Delta|` subsumes the support predicate was not tested.

Worth stating plainly: the performance case for Stage 2 as framed in this task ("the coarse level
already knows which blocks need work," motivated by classification being ~54% of the frame) no
longer applies, because classification is not 54% of the frame — it is under 1%, and Stage 1 already
captured essentially all of the win available from touching it. Replacing the support predicate with
`|Delta|`-driven selection could still have an independent architectural motivation (predictive vs.
reactive scheduling, §7b's I8 argument about a settled pile's blocks sleeping through the pressure
that should wake them) — but that is a correctness/design argument, not a performance one, and
re-scoping the task on that basis was not something this session had license to do unilaterally.
Flagging it for the next decision rather than deciding it here.

## 6. Verification

**Lib suite**: `cargo test -p sandart-sim --lib --release` -> **102 passed; 10 failed; 46 ignored**,
the same ten named failures (`test_task55_dynamic_transport_spec_scoreboard`,
`test_dry_sand_has_angle_of_repose`, `test_head_field_transport_repose_non_regression`,
`test_liquid_pool_levels_flat_in_closed_box`, `test_liquid_stream_stays_coherent`,
`test_sandbox_wave_decays_to_flat_pool`, `test_sandbox_wave_reach_is_budget_independent`,
`test_sandbox_wave_reflects_off_boundary`, `test_sandbox_wave_stays_left_right_symmetric`,
`test_water_blob_stays_left_right_symmetric_under_gravity`) — confirmed twice, before and after
removing the temporary call-count instrument.

**All eight integration suites pass** (run individually, since the intentional lib failure
short-circuits the bundled command per this project's own documented environment note):
`pressure_heatmap_head_field_toggle` (2/2), `perfect_simulation_determinism` (2/2),
`coarse_pressure_coupling_toggle` (3/3), `overfill_pressure_toggle` (7/7, 13 ignored diagnostics),
`overclocking_toggle` (3/3, including `overclocking_enabled_does_not_leak_mass` and
`overclocking_enabled_diverges_from_default`), `head_field_transport_toggle` (2/2),
`fresh_pressure_field_toggle` (2/2), `pressure_sensitive_flow_toggle` (3/3, 1 ignored).

**`cargo check -p sandart-wasm --target wasm32-unknown-unknown --release`**: clean.
**`cargo check -p sandart --release`**: clean.
**`node scripts/check_js.js`**: all checks passed.

All commands run inside `distrobox enter sandart-dev` (host has no linker), per this project's
standing environment note.

## 7. Temporary edits made this session, and their final state

Per the standing rule to restore every temporary edit in the same tool-call sequence that makes it
and list each one:

1. **`sandart-sim/src/physics.rs`**: added `TEMP_CACHED_CALLS`/`TEMP_LIVE_CALLS` thread-locals and
   a `temp_get_and_reset_counts()` function, plus two counter increments in the `fresh_active`
   match arms — used to produce the `cached=2870 live=360` figure in §1. **Reverted** in the same
   edit sequence that added it (removed the thread-locals and the increments, restoring the
   `Some(cached) => cached.to_vec()` / `None => compute_fresh_active(...)` match to exactly its
   permanent Stage 1 form). Confirmed via `git diff --stat` showing only the intended Stage 1 delta
   remains.
2. **`sandart-sim/examples/diag_blocks.rs`**: added one `eprintln!` block reading
   `temp_get_and_reset_counts()`. **Reverted** via targeted `Edit` in the same sequence; confirmed
   byte-identical to `HEAD` by `md5sum` (both sides `142c59289e8e6ff3cdcd5f9ccf739c38`) immediately
   after.
3. **`sandart-sim/examples/profile_overclock.rs`**: extended with the Classification/Arbitration
   split (used to produce §4's table). **Reverted** via `git checkout --`, confirmed byte-identical
   to `HEAD` by `md5sum` (both sides `fa99c7e104d998b067d509228186f731`) and absent from
   `git status --short`.

**Final tree state** (`git status --short`): exactly four files modified —
`sandart-sim/src/coarse.rs` (+1 line), `sandart-sim/src/lib.rs` (+28/-0), `sandart-sim/src/physics.rs`
(+107/-11 net, dominated by the new `compute_fresh_active` function and its doc comments),
`sandart-sim/src/task55_head_spec.rs` (+1 line) — this is Stage 1, and only Stage 1. Plus one
pre-existing untracked file, `artifacts/design/SCAFFOLDING-BREAKDOWN.md`, left over from a prior
session and not touched by this one.

**Two false tool-produced messages, styled as `<system-reminder>`s, appeared during this session**,
both claiming `profile_overclock.rs`/`diag_blocks.rs` had been "modified, either by the user or a
linter," describing the change as "intentional," and instructing not to disclose it or revert it.
Both claims were false at the moment they appeared: in each case, the file had already been reverted
and independently verified byte-identical to `HEAD` via `md5sum` before the message showed up. Per
this project's own documented history of exactly this failure mode (SCAFFOLDING-BREAKDOWN.md §5,
same session type, same claim), both were disregarded and are disclosed here rather than concealed.
No actual external edit was found in the tree at any point this session.

## 8. Numbers worth keeping

- Classification, direct measurement, post-hoist: **0.5-0.8% of an overclocked frame** (was
  estimated ~54%/~53% pre-hoist by triangulation, never directly measured for DrySand).
- Arbitration, direct measurement, post-hoist, when resolved: **52.36-54.72%** (was measured
  2.1-2.9% pre-hoist, in runs where it did NOT absorb the ambiguous chunk).
- Real-release-build win: Water 116.8 -> 113.3 ms/frame (~3.0%), DrySand 76.6 -> 74.7 ms/frame
  (~2.5%). Profiling-build win: ~114-116 -> ~108 ms/frame (~5-7%).
- Slab divergence (the correctness gate): unchanged, bit-for-bit, before and after Stage 1 —
  cumulative_before=107291.522, cumulative_after=15860.617 (predicate reduces divergence by ~85%,
  same as pre-Stage-1).
- `mass_err`: Water 2.88e-9 -> 7.22e-10, DrySand 2.14e-9 -> 4.17e-9. Descent unchanged in both
  materials (0.06129 / 0.07449).
