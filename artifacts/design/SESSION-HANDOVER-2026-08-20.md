# Session handover — two-level simulation, 2026-08-19/20

> **Superseded for next steps by `SESSION-HANDOVER-2026-08-20-EVENING.md`.** §6 below is done or
> overtaken: the clock rate now actually gates participation, block size was swept (8 is optimal),
> and the vpar measurement §7 called for has been taken. §4 (the instrument problem) and §8
> (process notes) still stand and are still worth reading first.

Written to clear context. The design is `artifacts/design/HIERARCHICAL-PRESSURE.md`; read
`HIERARCHICAL-PRESSURE-PROGRESS.md` for the build-order state, then this file for what changed
since and what to do next.

**Deployed:** `origin/main` = `37956de3`, confirmed serving from `gh-pages` (tip `6de6e41` names
that sha). Live at <https://tanaeem-moosa.github.io/sandart/>.

**Unpushed:** one commit, `ba9aff13` (see §5). It is verified and safe to push; it was held only
because it is a ~3% perf change with no behaviour difference and nothing was waiting on it.

---

## 1. The finding that reframed the whole effort

**Transport is clamped at one cell per step, at every resolution** — `flux_edge_candidate` ends in
`.clamp(-1.0, 1.0)`. A 64x64 sim is not better behaved than a 512x512 one; its cells are 8x bigger,
so one cell per tick is 8x the physical distance. Crossing the domain takes 64 ticks at grid 64 and
512 ticks at grid 512.

**Therefore a driving potential can only change WHERE the <=1 cell/tick of movement happens — never
how much happens.** That is why the coarse `eta` coupling measured ~10% on pool levelling for 67% of
frame time, and why 2.2M edges per 400 ticks sit pinned at the clamp. Sub-stepping is the only lever
that raises the transport rate.

This came from the user's own framing ("a 64x64 sim settles fast, we just need our sim to move as
fast but simulating 64x more cells") and it is the single most useful conclusion of the session.

## 2. What works, and what is parked

| mechanism | state |
|---|---|
| **Over/underclocking** | **The win.** 8.3x material movement (Water descent 0.00738 -> 0.06142). Mass-conserving. Default OFF. |
| Coarse pressure coupling | Parked behind `coarse_pressure_coupling`, default OFF. ~10% benefit for ~67% frame cost. |
| Coarse level itself | Runs unconditionally — it produces `|Delta|`, which the scheduler needs regardless of the coupling. |
| Early stop | Its 1.5x was a bug (see §3). Fixed; the speedup went with it. |
| Classification hoist | Correct, ~3%, unpushed (`ba9aff13`). |

**Acceptance criteria** (design §8): criterion 4 (the hourglass) is **MET** — 0 of 9,712 falling
cells carry pressure. Criteria 1, 2 and 3 (pile settles fast enough, U-tube riser rises, no
oscillation) remain **unverified** and need the user's eye; there is no browser driver here.

## 3. Two real defects found and fixed

**The eta over-drive.** The bespoke coarse level used `base_head_coarse = base_head * t` — eight rows
of gravity per coarse row at 512 — while the fine solver already applies one row per edge. Tile-seam
edges were driven at **9x gravity**. Found by the user asking the right control question ("falling
sand works fine in 64x64 today, why would it have issues in coarse?"). Fixed by making the coarse
level literally a 64x64 run of `physics::settle_tick`. **This is what fixed criterion 4**: falling
cells carrying pressure went 16.9% -> 0.0%.

**The S3 violation in early stop.** `still_has_work` gated two different things: whether a block
sweeps its own interior, and whether it keeps acting as a neighbour-FORCER. A fast block at a
clock-domain boundary that settled mid-frame stopped force-waking its slower neighbour, whose owned
edge then went unevaluated. Measured as a redistribution (per-block excess summing to zero within
0.2%) localised to two adjacent block rows with opposite signs and matched magnitudes. Fixed by
separating the two gates — **and that removed essentially all of early stop's speedup**, because the
saving had been coming from the violation. Blocks run 297 -> 489, against 492 with early stop off
entirely.

## 4. Three mistakes of mine, all corrected in-tree — read these before trusting a number

1. **I published a "benign floating-point noise" verdict on the mass discrepancy that was wrong.** I
   used non-monotonicity to rule out the mechanism, named the spatial signature as the evidence that
   would overturn me, and then did not run the instrument that produces it. It was run; it overturned
   me. Retracted in `37956de3`.
2. **I shipped a commit claiming rates were unquantised while the code still stepped by octaves.** I
   verified the doc comments, which had been updated, not the arithmetic, which had not. Corrected in
   `65d267bf`.
3. **I relayed "classification is 54% of the frame" as fact.** It was ~0.6%. The figure came from a
   profile that resolved an unnamed hot chunk by an architectural tie-break argument, and the
   argument was wrong. Corrected in `ba9aff13`.

**The common cause is the instrument.** `physics.rs` is one ~17,000-line function with aggressive
inlining, and the sampling profiler cannot stably name the hot symbol — across ten identical-binary
runs it resolved to three different names. Two of three perf conclusions this session came from it
and both were wrong. **The one that held (§1) came from reading the code, not from sampling.**

**Rule going forward: prefer counters to profiles here.** A count of "how many edges had
`scale == 1.0`" cannot be misattributed. If a profile is used, confirm by a second independent route
— which is how mistake 3 was caught, by hoisting the suspected culprit and watching the chunk not
move.

## 5. The pending commit — `ba9aff13`, verified, unpushed

**"Hoist classification out of the repetition loop, and correct a wrong profile attribution."**

- `fresh_overburden_must_blocks` / `support_fraction` ran unconditionally on **every** `settle_tick`
  call, so overclocking paid it 8x per frame. Now runs once per rendered frame, before the
  repetition loop. Verified engaged by call count (~8:1 cached:live).
- Water 116.8 -> 113.3 ms/frame, DrySand 76.6 -> 74.7 (~3%). Descent bit-identical
  (0.06129 / 0.07449); `mass_err` same order (7.22e-10 / 4.17e-9).
- **Slab divergence bit-for-bit unchanged** — `test_fresh_overburden_predicate_reduces_slab_divergence`
  prints identical numbers before and after. Caveat recorded in the commit: that test uses a
  non-overclocking harness, so it confirms the extraction was behaviour-preserving, **not** that
  overclocking + per-frame caching is jointly safe against slabs.
- Also carries the correction to the 54% attribution.

Verified: lib suite **102 passed / 10 failed** (the same ten named failures), all eight integration
suites, wasm32 and desktop checks, `node scripts/check_js.js`.

**Safe to push.** It changes no behaviour a user would see.

## 6. Next steps, in the order I would take them

1. **The user looks at the deployed build.** Criteria 1, 2, 3 cannot be settled any other way. Turn
   on overclocking alone (leave the coupling off) — expect ~8 fps and 8.3x faster settling. Watch
   for churn at rest, and check the hourglass looks unchanged as a control.
2. **Build explicit phase timers** — COLLECT / ARBITRATE / APPLY / copy-back, per pass — instead of
   trusting the sampling profiler. Deterministic, additive, no symbolizer. This should precede any
   further optimisation; see §4.
3. **Count uncontested edges** (`edge_arbitration_scale == 1.0`). If most edges are uncontested, the
   three-pass structure is being paid for a minority, and COLLECT/APPLY could fuse for the rest. See
   `ARBITRATION-AND-N-STEP.md` §2.
4. **Local time stepping instead of neighbour forcing.** `ARBITRATION-AND-N-STEP.md` §3: the
   Osher–Sanders answer is to accumulate interface flux over the fast side's sub-steps and hand the
   total to the slow cell when it steps — the slow block never runs. Conservation holds by
   construction rather than by scheduling discipline, which retires the S3 hazard, and it makes early
   stop's saving collectable because forcing is what was preventing it. Two catches are written up:
   the frozen-neighbour approximation, and the need to apply the FCT limiter to the **accumulated**
   claim.
5. **Then re-decide the coupling.** It costs ~67% of frame time for ~10%. Once clocking is fast,
   re-run the A/B; my expectation is that it should be deleted, which would also retire the
   saturation, the residual seam bias and the churn regression together. Do not delete it before the
   user has looked — metrics have been the wrong instrument on this project before.

## 7. Open, unmeasured, and worth not forgetting

- **`vpar` and settled churn under unquantised rates** — §7b's concern that arbitrary rates beat
  against the known period-2 checkerboard mode. Three agents have been killed mid-run attempting it.
  **This should gate ever defaulting overclocking ON.**
- **The staleness floor collapses to 0.0 with clocking on** — nothing reaches the 30-tick floor, so a
  backstop that has caught three subtly-wrong activation signals in this project's history is
  currently inactive.
- **Necks are not modelled**: ~three quarters of them fall inside a coarse tile at 512 and vanish
  from the coarse model.
- **Grid 128 is 50-65% slower** since the block resize. Accepted, not fixed.
- **Wake propagation reaches half as many cells per tick** since the block resize, and the test that
  should catch it hardcodes its own `block_size = 16`.

## 8. Process notes that cost real time this session

- **Do not measure a tree an agent is actively editing.** An agent left a
  `let still_has_work = true; // TEMPORARY ISOLATION TEST` line disabling early stop and was killed
  before restoring it; measurements taken against that mutating tree mixed two states and produced a
  misattributed result. A second such leftover **landed on `main`** and made a shipped commit message
  false.
- **Agents repeatedly stall on background Monitors** and are killed while blocked, returning
  half-finished work. Several measurements in this session had to be redone in the main thread.
  Brief them to run measurements in the foreground in bounded chunks.
