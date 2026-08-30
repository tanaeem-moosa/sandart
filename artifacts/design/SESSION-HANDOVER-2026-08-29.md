# Session handover — the overfill regression, and the credit/debt design's refutation, 2026-08-29

Written at the user's request before clearing context, against `79162f1`. Everything numeric here
was measured this session inside `distrobox enter sandart-dev` unless it cites another document.

The user's standing question, which frames all of this:

> *"I am questioning our whole overfill strategy. I don't think it has been very successful. At
> least outside of 64x64. I am tempted to roll everything back to before our first overfill."*

**§1 is the finding that bears on that decision and it is the most important thing in this
document.** Read it first even if you read nothing else.

---

## 1. THE HEADLINE: overfill broke nine tests, and they were re-labelled "pre-existing"

Measured this session by checking out the commit immediately before the first overfill commit and
running the library suite there:

| commit | date | `cargo test -p sandart-sim --lib --release` |
|---|---|---|
| `f43920a` — **immediately before overfill** | 2026-08-07 | **103 passed / 1 failed** |
| `79162f1` — HEAD, after 114 `#70` commits | 2026-08-29 | **102 passed / 10 failed** |

The single failure at `f43920a` is `test_water_blob_stays_left_right_symmetric_under_gravity` —
which is exactly the ONE failure HANDOVER.md §1's working agreement sanctions. **So the suite was
effectively green before overfill and has nine regressions now.**

The nine, all passing at `f43920a` and failing at HEAD:

    task55_head_spec::test_task55_dynamic_transport_spec_scoreboard
    tests::test_dry_sand_has_angle_of_repose
    tests::test_head_field_transport_repose_non_regression
    tests::test_liquid_pool_levels_flat_in_closed_box
    tests::test_liquid_stream_stays_coherent
    tests::test_sandbox_wave_decays_to_flat_pool
    tests::test_sandbox_wave_reach_is_budget_independent
    tests::test_sandbox_wave_reflects_off_boundary
    tests::test_sandbox_wave_stays_left_right_symmetric

**These are not incidental tests.** They are the core behaviours of the material model: a pool
levels flat, a wave decays to a flat pool, sand holds its angle of repose, a liquid stream stays
coherent. `test_liquid_pool_levels_flat_in_closed_box` failing means **water does not level flat** —
which is the precise problem the last several sessions have been trying to fix with coarse-grid
lateral transport. **We have been building machinery on top of a regression.**

### How the label slipped

HANDOVER.md §11 states the ten failures "are pre-existing — they were failing at `95ce58e7` before
any of 2026-08-17's work". That sentence is *true* and *misleading*. `95ce58e7` is dated
**2026-08-16**, which is two days INTO overfill work (first overfill commit `c844d68` is
2026-08-14). So "pre-existing" meant "pre-existing relative to the session then in progress", not
"pre-existing relative to overfill". Each subsequent session inherited that framing and re-asserted
it, and the CLAUDE.md I wrote earlier today repeats it as "the known-good state".

**Correct the framing wherever it appears.** The ten failures are not a baseline. They are an
unpaid regression with a known first-bad-commit window.

### What this does NOT establish

- It does not localise the break to a specific commit inside the 114. `c844d68..HEAD` is a wide
  window and nobody has bisected it. **That bisect is the single highest-value next action** and it
  is cheap: `git bisect` over ~114 commits is ~7 builds, each a few minutes in the container, with
  `test_liquid_pool_levels_flat_in_closed_box` as the predicate.
- It does not prove overfill is unfixable, only that it was landed without the suite as a guard.
- It does not confirm the user's "at least outside of 64x64" qualifier. That remains an untested
  hypothesis — see §6.

---

## 2. The rollback the user is considering

**Rollback point: `f43920a`** ("Demote the randomness hypothesis in #56", 2026-08-07 17:40). The
first overfill commit is its child-but-one, `c844d68` ("Implement per-cell overfill pressure
simulation (#70)", 2026-08-14 13:28).

Scope of what is between them and HEAD: **114 commits, 66 files, +19,299 / −1,174 lines.** That is
roughly a third of the project's 341 commits.

### What a rollback would DISCARD, and this is the part to weigh

Not just overfill. The `#70` window swallowed several independent subsystems that have nothing to do
with the pressure model and that are, as far as anyone has measured, fine:

- **The whole hierarchical coarse level** (`coarse.rs`, 1430 lines): `CoarseGeometry`, the nested
  64x64 sim, restriction/anchoring, `Delta`, `eta`. Built over STEP0–STEP4 docs.
- **The LOD block scheduler and overclocking** — rank allocation, rate grading, early stop, S3
  neighbour forcing, `rate_gated_reps`. Plus `MASS-ERR-DIAGNOSIS.md`'s real conservation fix.
- **The lateral flow correction** (Design 3, shipped): `LAT_LEDGER`, `compute_lateral_boost`.
- **All of this session's work** (§3).
- **The instruments**: `diag_lateral_corr`, `diag_flow_direction`, `diag_blocks`,
  `diag_block_steps`, `diag_delta_direction`, the coarse overlay, the block heat-map, the saturation
  decile lines, `scripts/check_js.js`.
- **~20 design documents** in `artifacts/design/` recording what was tried and rejected, with
  numbers. Losing these means re-running rejected experiments — the failure mode
  `SEARCH-REJECTED-DESIGNS` exists to prevent.

### The alternative that gets most of the benefit for much less loss

**Bisect first, then revert narrowly.** If the nine regressions trace to a small number of commits
inside the overfill work — plausible, since they are all "does the material settle correctly"
behaviours and likely share one cause — then reverting *those* restores the suite while keeping the
coarse level, the scheduler and the instruments. A full rollback to `f43920a` is the fallback if
the bisect shows the breakage is diffuse and entangled.

**Recommendation: do not roll back before bisecting.** The information is cheap and it changes the
decision. If the bisect implicates one or two commits, a full rollback throws away three weeks of
unrelated, working infrastructure to fix something a revert would fix.

### If the decision is to roll back anyway

- Do it on a branch, not by resetting `main`. `main` auto-deploys.
- Keep `artifacts/` and `docs/` from HEAD regardless — the design record's value is independent of
  the code, and this document plus `LATERAL-COARSE-CORRECTION.md`, `FLOW-DIRECTION.md`,
  `ARBITRATION-AND-N-STEP.md` and `CREDIT-DEBT-TRANSPORT.md` are what stop the next attempt
  repeating the last four.
- Keep `CLAUDE.md` and `scripts/check_js.js`.
- Note that `f43920a` predates the `block_size = grid/64` decision and the coarse level entirely, so
  anything rebuilt on top of it starts from a genuinely different architecture.

---

## 3. What this session did

### 3.1 The credit/debt design (committed, then refuted)

Designed with the user across a long exchange, written up in `CREDIT-DEBT-TRANSPORT.md`
(`14e4ff8`), revised twice (`02c76fa`, and the review pass). Short version of its history:

1. **First form** — persistent per-face debt ledger, priority worklist, sized as
   `0.7 * (coarse_flux - fine_realised)`. Review found the sizing term is
   `LATERAL-COARSE-CORRECTION.md`'s **Design 1 verbatim** — built, measured at +41% spread on
   DrySand, and rejected on visible seams. It also never self-zeroes, because its two terms count
   mass that moved `t` cells against mass that moved 1.
2. **Second form** — the user's three corrections collapsed it: the rate is a request not an
   outcome; `Delta` is recomputed from reality each tick so it self-corrects; anchoring is
   independent and can be raised. **Consequence: the debt ledger deletes entirely — `Delta` already
   is the ledger.** That removed the mass_err rework and all five silent-corruption paths the
   review had found.
3. **Implemented** as `coarse_delta_transport` (§4), then **refuted by measurement** (§3.3).

### 3.2 Three bugs the user found on screen that the tests did not

This is a pattern worth carrying forward. Each was found by looking at the deployed page; each
passed the full suite at the time.

| # | Symptom (user's words) | Cause | Fix |
|---|---|---|---|
| 1 | *"the button does not apply setting"* | Never registered a `change` listener; `syncSettings()` is only called by registered controls | `5eb837e` |
| 2 | *"hourglass is not falling. It is attached to the right"* | Mutated `heights` in place while scanning with frozen `Delta` — scan-order bias, `dx = -2.34` in a symmetric vessel | COLLECT/ARBITRATE/APPLY, `5eb837e` |
| 3 | Checkerboard + row striping (photo) | Moved HALF the difference per face; that is the 1D-stable relaxation, and 2D needs a QUARTER (`1/(2d)`). Shipped default sat 2x over the stability limit | `79162f1` |

**Both regression tests added for these pin physical invariants, not implementation details** —
a symmetric vessel stays symmetric, and high-frequency energy stays bounded — because a checksum, a
mass sum, a per-cell range check and a symmetry check are *all* satisfied by a perfectly
conservative, perfectly symmetric checkerboard.

### 3.3 THE REFUTATION: the fine level cannot undo the coarse level's placement

The user's objection: *"I thought fine simulation would prevent the fattening when it is not
appropriate."* That was the design's own premise — "coarse says how much, fine decides where".

Measured (`diag_delta_transport --sweep`, 256 DrySand hourglass, 300 ticks), stream width in fine
cells on the rows below the neck:

| rate | width | vs baseline | checker | descent |
|---|---|---|---|---|
| off | **5.0** (= neck aperture) | 1.0x | 1.00x | 20.45 |
| 0.10 | 7.4 | 1.5x | 0.92x | 22.83 |
| 0.25 | 17.3 | 3.5x | 0.91x | 25.37 |
| 0.50 | **23.3** | **4.7x** | 0.94x | 27.86 |
| 0.70 | 23.7 | 4.7x | 1.05x | 29.03 |

**The premise is false, and the "+36% descent" I reported as a win is mostly this artifact** —
more mass falls per tick because the hole is 4.7x wider, not because transport improved.

The mechanism, precisely: the deposit is weighted by **headroom**. Beside a falling stream the
neighbouring tiles are empty, so they have the *most* headroom, so they receive the *most*. The
deposit actively prefers the empty space next to the stream. And `settle_tick` cannot undo it —
fine decides where mass goes **next**; it cannot retract where the coarse level **put** it. Sand
sitting in a wide swath simply falls as a wide swath; there is no restoring force. The baseline
stream is narrow only because mass can only ever *enter* through the neck aperture.

**So headroom-weighted deposit IS the coarse level deciding placement** — a softer Design 2, and it
reproduces Design 2's recorded failure ("spread blew up... smeared across the whole vessel"). The
staircases in the user's screenshot are the same thing at tile granularity.

**The general statement, which applies to any future attempt:** the information about *where* mass
should go is not in `Delta`. `Delta` is one scalar per tile. Any within-tile distribution rule is
the coarse level guessing, and `LATERAL-COARSE-CORRECTION.md`'s root-cause line already covers it:
*"both failed designs share one root: they let the coarse level decide where mass goes."*

**The only form that respects the premise** is to supply a potential plus a budget and let the fine
solver move the mass through its own flux edges, gated by its own capacity/repose/aperture logic.
That mechanism already exists here: `coarse_pressure_coupling` feeds `eta` as a driving potential
with `coarse_delta_eta_budgeted` using `|Delta|` as the per-tile flux budget, measured at ~36%
frame time for ~7% spread applied everywhere. So **the "new lever" collapses into the existing
one**, and the remaining design work is making that coupling *targeted* rather than global —
the "selective coupling" item in `SESSION-HANDOVER-2026-08-20-EVENING.md` §5.4.

---

## 4. State of the code

`coarse_delta_transport` is **shipped, default OFF, and known to fatten streams 4.7x at its default
rate.** It is not a candidate for turning on. Options for whoever picks this up: leave it off as a
recorded negative result, or delete it. Deleting costs nothing that is not written down here.

- `physics::apply_coarse_delta_transport` — COLLECT/ARBITRATE/APPLY, conservative, order-independent.
  The quarter factor and its stability derivation are in its doc comment.
- `DrawingSimulation::coarse_delta_transport` / `_rate` (default 0.5) /
  `last_frame_delta_transport`.
- Anchoring rises `0.10 -> 0.50` while the toggle is on (`COARSE_DELTA_TRANSPORT_LAMBDA`). **This
  coupling is sound and worth keeping even if the transport is deleted** — the reasoning is in that
  constant's doc comment.
- UI: checkbox + rate slider + delivered/asked readout, next to the coarse correction controls.
- Tests: `coarse_delta_transport_toggle.rs`, 7 passing.
- Instrument: `examples/diag_delta_transport.rs` — `--sweep` gives checker/descent/width vs rate.
  **This one is worth keeping regardless**; stream width is the metric that caught the refutation
  and no other instrument reports it.

---

## 5. Verification, and how to run anything

`CLAUDE.md` (added this session, `57f5a7e`) is the short version; `artifacts/HANDOVER.md` §1 is the
authority. The essentials:

- **There is no linker on the host.** Everything compiles in
  `distrobox enter sandart-dev -- bash -lc '<cmd>'`. `linker 'cc' not found` means you are outside
  the container, **not** that the work cannot be verified. I got this wrong earlier today and
  committed the claim; the container was documented in README.md and HANDOVER.md the whole time.
- `cargo check -p sandart-wasm` **typechecks nothing** without
  `--target wasm32-unknown-unknown --release` — the crate is `cfg(target_arch = "wasm32")`-gated and
  a host check compiles an empty crate.
- `node scripts/check_js.js` before any `demo.js` push.
- One pre-existing **doctest** failure, `physics::EQUILIBRIUM_LUT_SIZE (line 837)`: a 4-space
  indented formula rustdoc tries to compile as Rust. Present at `f10fc15`. One-line fix, unrelated.

At HEAD: 7/7 delta transport tests, all integration targets green, `--lib` 102/10 (see §1 — that is
a regression, not a baseline), wasm32 release clean, `check_js.js` clean.

---

## 6. Next actions, in the order I would take them

1. **Bisect the nine regressions.** `git bisect start HEAD f43920a` with
   `test_liquid_pool_levels_flat_in_closed_box` as the predicate, ~7 builds. This is cheap and it
   determines whether the rollback needs to be total or narrow. **Do this before deciding
   anything.**
2. **Settle the user's "outside of 64x64" hypothesis**, which is currently untested. The cheap
   version: run the failing settling tests at grid 64, 128, 256 and 512 and see whether the failures
   are resolution-dependent. If overfill is calibrated for one resolution and wrong elsewhere, that
   is a different and much more fixable problem than "overfill is wrong". Note HANDOVER.md §1's
   standing rule that **512 is the production resolution and 64/128/256 are instruments** — several
   defects appear only at 512 and several only at 64, so always state which resolution a number came
   from.
3. **Fix the framing in CLAUDE.md and HANDOVER.md §11** so nobody else inherits "10 failures is the
   known-good state".
4. Only then revisit lateral transport, and if so via targeted `eta` coupling (§3.3), not via
   depositing mass.

## 7. Things that cost time this session, recorded so they do not again

- I concluded "the tests cannot be run here" and committed it. The container was documented in two
  places I did not open. **Read `CLAUDE.md` and `HANDOVER.md` §1 first.**
- I relayed a subagent's verdict ("the design does not survive") further than its evidence reached,
  and the user had to push back to get it corrected. The review had refuted **one term**, not the
  architecture.
- I adopted a reviewer's framing — "does this break O(L^2)?" — that was never the design's claim,
  and let it drive a wrong conclusion into a commit message.
- Three separate defects shipped that the full suite passed. **The suite cannot see visual
  artifacts.** Every mechanism that moves mass needs an invariant test — symmetry, high-frequency
  energy, stream width — not just conservation.
