# Session handover — the overfill verdict, the bisect, and the deletion

Covers 2026-08-30 to 2026-08-31, against `2ef840c`. Supersedes
`SESSION-HANDOVER-2026-08-29.md`, which recommended the bisect this session ran.

The user's framing, which drove everything here:

> *"I am trying to decide if to throw away all the work with overfill. I was very excited about it
> being technically elegant. But I don't think it worked. And not just tests. It degraded the app
> experience more than it helped."*

Outcome: they were right, the reason is now measured rather than felt, and overfill plus everything
that depended on it is deleted. The suite is green apart from two known failures.

---

## 1. The nine regressions were two commits, in one 45-minute window

`SESSION-HANDOVER-2026-08-29.md` §1 established that `f43920a` (immediately pre-overfill) was
103 passed / 1 failed while `main` was 102 / 10, so nine were regressions rather than a baseline.
It could not say where they came from. Now we can.

**They are not the overfill feature.** `overfill_pressure` defaults to `false` and `TestSim::new`
leaves it off, so the nine failed with every overfill toggle off. The damage was in unconditional
edits to the shared solver made during the `#70` window. Flipping the toggle could never have
recovered them — which is why three weeks of work on top of them never did.

Bisected with `test_liquid_pool_levels_flat_in_closed_box` as the predicate:

    135e5a5  16:11  ok        baseline
    15da8fe  16:28  FAILED    accel filter, `0.7*v_raw + 0.3*v_prev`
    56b9b91  16:45  ok        accel filter removed -- a 17-minute transient
    33b3059  16:55  FAILED    liquid-only temporal EMA in `flux_edge_apply`
    73b71a8  17:10  FAILED    "unified viscoplastic" blend in `flux_edge_candidate`
    ...             FAILED    never recovers, through 2026-08-29

**The signal is non-monotonic, and that matters methodologically.** A plain `git bisect` reports
`15da8fe`, the transient, because bisect assumes exactly one transition. The durable causes are
`33b3059` and `73b71a8`. If you bisect anything in this repo's `#70` window, sample the timeline
around the answer before believing it — `git bisect` gave the wrong commit here on the first run.

**Why it survived three weeks:** the two that stuck live in DIFFERENT FUNCTIONS — the candidate
half and the apply half of the same edge computation. Each read as a single isolated knob, so every
later session's alpha-tuning only ever moved half the problem.

**What the change actually did**, which is worse than "adds lag": it converted the edge velocity
update from an INTEGRATOR to a FILTER.

    before:  v = (v_prev + c_sq*yielded) * damping      velocity ACCUMULATES
    after:   v = ((1-a)*v_prev + a*v_target) * damping  velocity BLENDS toward a target

No blend rate reproduces the original — at `a = 1.0` the `v_prev` term is DELETED, not
"unfiltered". So the solver silently went from second-order to first-order and stayed there while
everything else was built on top of it.

Measured on the library suite:

    102 passed / 10 failed   both filters live
    106 passed /  6 failed   candidate half reverted only
    110 passed /  2 failed   both reverted (one is the sanctioned #56 marker)

Both are reverted in `7856840`. **See the TOMBSTONE comment in `physics.rs` before touching that
expression.** If inertia is wanted again the answer is acceleration — velocity as physical state
integrating gravity — which is what the integrator already is, not a filter over the transfer.

### The contaminated A/B

The filters were neutralised on the overfill path alone (rate pinned to 1.0 there). So for three
weeks **every overfill-on/overfill-off comparison was really measuring "filters off vs filters
on"**. That is why overfill appeared to help in Sand-fall and appeared to make Sandbox worse:
Sandbox runs the default path, which was the degraded one. Treat every overfill measurement taken
between 2026-08-16 and 2026-08-30 as untrustworthy for this reason.

---

## 2. Why overfill was deleted rather than kept behind its flag

Its own instruments, from `HANDOVER.md` §9's baselines at the model's best-tuned state:

- Spread 59 / pile peak 13 — **identical to the non-overfill baseline, at every capacity.**
- Free fall 73 rows in 100 ticks against a non-overfill baseline of **122**.

Null on the metric it existed to move, worse on free fall, and the capacity dial changed neither
across its whole range.

**It was aimed at a constraint that was not binding.** `HANDOVER.md` §10 diagnoses the cones as
reading saturation 1.00–1.03 — not compressed, just un-levelled — i.e. a transport-rate problem.
The rate is railed by the `±1.0` clamp in `flux_edge_candidate`: one cell of mass per cell per
tick. Overfill is a pressure model. **That clamp is still there and is still the real limit on
flow.** It is the most promising thing in the archive to attack next.

Nothing in the test suite ever asserted overfill made anything better. `overfill_pressure_toggle.rs`
asserted it *diverged* from off, *conserved mass*, and held the angle of repose — all true of a
change that helps nothing. That gap is how three weeks of null results went unnoticed.

---

## 3. What was deleted, and why the cut was larger than overfill

Following the dependencies forced the scope:

- **`coarse.rs` ran the overfill law unconditionally** (`overfill_pressure: true` at its
  `settle_tick` call). It could not outlive it.
- **The block-clock scheduler derived its rates from coarse-fine disagreement** and gated on
  `coarse.available`. With coarse gone it would still compile and run and never assign a rate —
  dead code wearing a feature's clothes.

So all three went together: the overfill law and its equilibrium solver/LUT, `coarse.rs` (1430
lines), the overclocking scheduler, early stop, rate-gated reps, the lateral flow correction, the
delta transport, the flow ledger, the binding-constraint census, four debug overlays, five toggle
test suites, and 25 diagnostic examples. ~13k lines across `43b08a2` and `2ef840c`.

**Behaviour is unchanged.** Every `if overfill_active { .. } else { .. }` collapsed to its else
branch, which is exactly the configuration shipping since `set_overfill_pressure(false)`.

The suite went 110 → 98 passing. All 13 absences are 12 `coarse::tests::*` plus one overlay test,
deleted with their subsystem — verified by diffing the full `--list` output before and after. **No
physics test was lost.**

`artifacts/design/` was kept in full, deliberately. The code is gone; the record of what was
measured and rejected is what stops it being rebuilt.

---

## 4. Two mistakes this session made, and the guards added

**The revert was too broad at first.** Reverting both paths to the integrator broke
`spec_task70_u_tube_riser_keeps_rising` (riser stalled at 25/27/26/26). The integration suites
caught it. The cause was a real distinction: on an overfill edge `yielded` was already a SOLVED
mass transfer, so accumulating a previous velocity double-counted it. Only the default path needed
the integrator. (Moot now that overfill is deleted, but the reasoning is why the revert is shaped
the way it is.)

**A stray `</div>` shipped a blank page.** The cleanup removed the opening tag of
`overfill-stiffness-row` and left its closing tag. That closed `#app-container` early, so
`#viewport-container` and the canvas became direct children of `<body>` instead of flex children of
the element that sizes them:

    before   body > div#app-container > div#viewport-container > canvas#sand-canvas
    broken   body >                     div#viewport-container > canvas#sand-canvas

The page rendered nothing and the sidebar spilled over the render area. **The whole Rust suite and
`scripts/check_js.js` both passed on that commit, because nothing in the project looked at the
HTML.** `check_js.js` now checks `<div>` nesting balance, that `#viewport-container` is still
inside `#app-container`, and that every `getElementById(...)` in `demo.js` resolves to a real id
(76 today). All three fail on `43b08a2` and pass on `2ef840c`. The id check also caught this
session deleting the pause/step buttons and the fall-jitter slider mid-cleanup.

---

## 5. State at `2ef840c`, and what is open

- Library suite **98 passed / 2 failed**. Doctests pass (0 tests) — the permanent
  `EQUILIBRIUM_LUT_SIZE` doctest failure that several handovers called unrelated pre-existing noise
  was overfill debris and went with the solver.
- All five remaining integration targets pass. wasm32 check clean. `cargo build --release` clean.
- The two failures:
  - `test_water_blob_stays_left_right_symmetric_under_gravity` — the sanctioned #56 marker. Must
    keep failing.
  - `test_sandbox_wave_reach_is_budget_independent` — **the only open regression.** Much weaker
    than it was: the wave now reaches the far wall at both budgets (245/245); only the far-peak
    amplitude still varies (0.006726 at 32 vs 0.007285 at 64). It was never part of the
    edge-velocity regression and pointed at the scheduler — **which no longer exists**, so re-read
    this failure before bisecting it. It may have changed character or become fixable trivially.

**Not done, deliberately:** the render-side overlay plumbing (four textures, their bind groups and
the `shader.wgsl` branches) is unreachable but still present. Removing it means editing bind-group
indices and the shader, which cannot be verified without loading the page — there is no browser
driver on this machine. It is a separate change, and it needs a human to look at the result.

**The next lever, if you want one:** the `±1.0` clamp in `flux_edge_candidate` (§2 above, and
`HANDOVER.md` §10). Flow is railed against it at every resolution, and it gets worse with grid size
because a cell at 512 is a quarter the physical size. Sub-stepping the whole solver per frame is
the option `HANDOVER.md` §10 names and nobody has tried. Search `artifacts/design/` before
proposing anything else — this project rejects designs that measured well.
