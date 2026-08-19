# Step 3 fixes — LUT thrashing, deadband, flux budget

Written against agy's uncommitted step 2/3 build (`artifacts/design/HIERARCHICAL-PRESSURE-PROGRESS.md`,
`git diff sandart-sim/src/physics.rs sandart-sim/src/coarse.rs`), fixing the three defects called out
for this session: LUT thrashing (blocking), the missing hourglass deadband (I5), and the missing
per-tile flux budget (I4). All work left uncommitted per instructions.

---

## Defect 1 — LUT thrashing (blocking)

**Root cause.** agy's coupling folded `delta_eta` directly into `gravity_head` before calling
`overfill_equilibrium_transfer`. `cached_vertical_lut` is a single-entry cache keyed on
`(overfill_ratio, unit, tension, gravity_head)`; a `gravity_head` that varies per edge (every
inter-tile vertical edge has a distinct `delta_eta`) rebuilds the whole 4096-entry table (each entry
its own 64-step bisection) on every such edge. The same fold also let a large `delta_eta` push the
LATERAL pass's head (`gravity_dir.x * GRAVITY_HEAD_SCALE + dispersion + delta_eta`) past the LUT
gate's `gravity_head >= 0.5`, newly routing lateral edges through the VERTICAL LUT.

**Fix.** `overfill_equilibrium_transfer` gained a separate `coarse_head: f32` parameter. The LUT
fast-path gate now requires `coarse_head == 0.0`; whenever it is nonzero the function falls straight
to the existing exact closed-form `solve_forward` (already O(1), no bisection), with `gravity_head +
coarse_head` substituted everywhere the stress expression uses `gravity_head`. Both production call
sites (vertical pass, lateral pass) now pass `base_head`/`gravity_dir.x*SCALE+dispersion` as
`gravity_head` unchanged, and `delta_eta` as the new, separate `coarse_head` argument — never folded
together. Intra-tile edges (7 of 8 vertical edges at grid 512) get `coarse_head == 0.0` from
`coarse_delta_eta`'s own same-tile short-circuit and keep hitting the LUT; only the inter-tile
minority take the exact path.

Also removed the hardcoded `4096`/`64`/`w / 64` from `physics.rs`: a new `coarse_delta_eta` /
`coarse_tile_indices` pair of helper functions use `crate::coarse::COARSE_GRID` throughout, and the
"is the coarse level available this tick" test is no longer `coarse_eta.len() == 4096` (which stayed
spuriously true even when `CoarseGeometry::available` was false, since `CoarseState`'s buffers are
always sized `COARSE_GRID * COARSE_GRID`). `lib.rs`'s call site now passes an EMPTY `&[]` for
`coarse_eta` (and the new `coarse_delta`, see Defect 3) whenever `self.coarse.available` is false,
and `physics.rs` trusts that emptiness rather than inferring availability from length.

**Measured.**

| | `overfill_pressure_toggle` wall clock | mechanism |
|---|---|---|
| Committed tree (design's own baseline) | 7-8 s | no coupling at all |
| agy's uncommitted diff, unfixed | not finished after 12 min (per task brief) | LUT rebuilt per inter-tile edge |
| After the LUT-thrashing fix alone | **95.15 s** | LUT fixed; coarse relax itself now dominant |
| Same, with `CoarseState::tick`'s relax disabled entirely (isolation test) | **8.69 s** | confirms the LUT fix is complete |

The isolation run (relax disabled, everything else identical) lands within a few percent of the
7-8s baseline, proving the LUT-thrashing fix removed exactly the hazard it targeted. The 95s figure
that remains with relax enabled is **not** the LUT — it is a separate, pre-existing cost: see
"Found beyond the three defects" below.

---

## Defect 2 — deadband (I5), the hourglass

**Derivation used (written into `coarse.rs`'s doc comments in full).** A falling fine cell can only
carry `h > cap` (local overfill pressure) if something stops its descent — §3's "compression is
support," structurally exact: 0 of 9,647 falling cells ever have `o > 0`. `support_mass[C]`
aggregates this EXACTLY per coarse tile during `restrict`: `sum((h_i - cap_i).max(0))` over the
tile's fine cells, using the same nominal per-cell capacity (`cell_capacity_for(wetness)`) the fine
solver's own `o = (h-cap)/cap` uses. `grounded[C] = support_mass[C] > 0` is the deadband gate — no
epsilon constant anywhere in the decision.

**What is gated, and what is not.** `eta[C]`'s below-capacity linear term (`phi = x = M/cap` for
`x <= 1`) and the elevation term (`- cy * base_head_coarse`) are NEVER gated — the linear term is
the harmless "water table" signal U-tube levelling needs even between two arms neither of which is
near capacity, and the elevation term must stay live on BOTH sides of every edge or `eta` stops
being elevation-consistent (see the "what didn't work" list below). Only the PRESSURE EXCESS
(`unit*(x-1)` for `x > 1`) is withheld when `!grounded[C]`, capping `phi` at `1.0` instead.

**What was tried and measured worse (all documented in `coarse.rs`, not just this file).** Four
transitivity/floor-contact extensions were tried, each measured on `diag_support --grid 512 --ticks
400` (hourglass, "free-falling cells carrying nonzero pressure"):

| variant | result |
|---|---|
| Local-only gate, excess term only (**shipped**) | **8.4%** |
| Propagate `grounded` through any geometrically open tile below | 73.5% |
| Propagate only through tiles "packed" (`a_mass/capacity >= 0.98`) | 63.5% |
| Same, bounded to one hop above a directly-compressed tile | 63.9% |
| Floor/casing contact treated as automatically grounded | 21.0% |
| Zero the WHOLE `eta` (both terms) for an ungrounded tile | 55.2% |

The common failure in the first four: the tile immediately above (or beside) a genuinely compressed
tile is very often the SAME tile a falling stream is still landing in — I5's own "mixed tile... is
the case that decides it" — and any rule granting it grounding via a NEIGHBOUR's state reproduces
the forbidden "upward excursion at impact." The last (zeroing the whole `eta`) is a distinct bug:
stripping the elevation term from only the ungrounded side of a grounded/ungrounded pair
manufactures a spurious `delta_eta` from elevation alone, exactly the sawtooth §0.2 already solved
once — worse than not gating at all.

**Measured, before/after, coupling ON throughout (`diag_support --grid 512 --ticks 400`, hourglass):**

| | free-falling cells carrying nonzero pressure |
|---|---|
| No deadband (excess unconditionally added) | 8.3% (839 / 10,065) |
| **Shipped deadband** (excess gated on `support_mass[C] > 0`) | **8.4% (849 / 10,104)** |

**Honest residual: the deadband, as scoped, barely moves this number.** The dominant leak mechanism
is the never-gated linear fill-fraction term, not the excess term this deadband withholds — gating
the linear term is what reproduces the catastrophic 55.2%/63-73% failures above. This is a REAL,
unresolved limitation, consistent with the design's own §10 ("the single largest unaddressed risk")
and I5's admission that the mixed tile "is the case that decides it." The deadband is still correct
and worth keeping — it removes the one component that CAN be safely withheld, and it is what a
correct I4/I5 implementation is built on — but acceptance criterion 4 ("zero pressurised tiles") is
NOT met by this deadband alone.

**`diag_coarse`'s coarser metric (tile-average fill, not per-fine-cell), before/after, same run:**

| | falling-dominated tiles over capacity | worst fill in a falling tile |
|---|---|---|
| No deadband | 0 | 0.9875 |
| Shipped deadband | 0 | 0.9874 |

This metric was already 0 in both cases — it is much less sensitive than `diag_support`'s per-cell
check and does not by itself demonstrate the hourglass is protected. `diag_support` is acceptance
criterion 4's real instrument, per the design's own §8b table, and that is the number to track.

---

## Defect 3 — per-tile flux budget (I4)

**Implementation.** `coarse_delta_eta_budgeted` (physics.rs) wraps `overfill_equilibrium_transfer`
at both coupling sites. For each inter-tile edge it solves the transfer TWICE: once with
`coarse_head = 0` (`d_uncoupled`, what gravity alone would move) and once with the real `delta_eta`
(`d_coupled`). `excess = d_coupled - d_uncoupled` is the mass the coarse term alone is responsible
for on this edge. A per-tile remaining-budget buffer (`coarse_budget`, local to `settle_tick`,
initialised each tick from `|Delta[C]|` = `coarse_state.delta.abs()`) is checked against the tile
LOSING mass to that excess; if the excess would exceed what remains, `delta_eta` is scaled down and
the transfer is RE-SOLVED (not clamped post-hoc — the solver is nonlinear) so the excess exactly
exhausts the remaining budget, and the tile's budget is zeroed for the rest of the tick.

This bounds the CANDIDATE flux, before arbitration. `flux_edge_apply`'s arbitration only ever scales
a candidate DOWN (never up) to resolve competing donors, so bounding the candidate is a sound upper
bound on the realised, post-arbitration mass I4 actually cares about: `realised <= candidate <=
budget`, every tile, every tick. Three solver calls in the worst case, all exact O(1) closed-form —
consistent with the Defect 1 fix this depends on.

New parameter `coarse_delta: &[f32]` on `settle_tick`, same emptiness contract as `coarse_eta`
(`lib.rs` passes `&self.coarse_state.delta` iff `self.coarse.available`, else `&[]`).

**Measured, coupling on (`diag_coarse --grid 512 --ticks 400`, hourglass and U-tube):**

| scenario | edges where `delta_eta` was scaled down by the budget |
|---|---|
| Hourglass | 19,773 / 400 ticks (~49/tick) |
| U-tube flow-through | 173,206 / 400 ticks (~433/tick) |

Nonzero in both scenarios — the budget is doing real, frequent work, not a no-op. The U-tube's much
higher count matches the design's own step-2 instrumentation (`diag_coarse_step2`: "93.0% of active
tiles have `|Delta| > 2.0`" on U-tube flow-through vs "66.6%" on the hourglass) — more sustained
coarse-fine disagreement means the budget binds more often.

**§8 "No bang-bang transport" — checked, not fixed (out of scope per the brief's own wording:
"Report whether it fires... with a count").** `overfill_equilibrium_transfer`'s `solve_forward`
still returns its full mass limit when `st(limit) >= tau`, which then meets
`flux_edge_candidate`'s `.clamp(-1.0, 1.0)`. Instrumented with a thread-local counter
(`bang_bang_count`/`reset_bang_bang_count`, physics.rs), gated to only count edges where
`coarse_head != 0.0` (i.e. the coarse coupling is what pushed the edge to its limit):

| scenario | fires (400 ticks) |
|---|---|
| Hourglass | **298,055** (~745/tick) |
| U-tube flow-through | 87,341 (~218/tick) |

**This DOES fire, frequently, with the coupling on.** It is the pre-#70 saturation defect I1 does
not rule out (I1 rules out overshoot in the *potential*; this is saturation of the *mass limit*).
Flagged here per the brief's instruction to report, not fix — a real, unaddressed follow-up.

---

## Verification, all three defects together

- `cargo test -p sandart-sim --lib --release`: **101 passed / 10 failed**, same ten names, unchanged
  in character, at every checkpoint (after Defect 1 alone, after Defect 2, after Defect 3). One
  additional failure appeared transiently after Defect 2's first draft
  (`coarse::tests::coarse_relaxation_propagates_hydrostatic_head_downward`) because that test drives
  `relax`/`update_head_and_disagreement` directly, bypassing `restrict` — it never populated
  `support_mass`, so the new deadband correctly (if inconveniently for the test) suppressed the
  pressure it asserted. Fixed by having the test register `support_mass` for its synthetic bottom
  tile, exactly as `restrict` would from real fine data resting on a container floor. Confirmed
  `test_water_blob_stays_left_right_symmetric_under_gravity` untouched throughout (still fails, on
  purpose, unchanged).
- Six integration suites: `overfill_pressure_toggle` **95.12 s** (unchanged from Defect 1's number —
  Defects 2/3 add negligible cost, consistent with their O(1) extra solver calls);
  `perfect_simulation_determinism`, `fresh_pressure_field_toggle`, `pressure_heatmap_head_field_toggle`,
  `head_field_transport_toggle`, `pressure_sensitive_flow_toggle` all pass, combined **36.0 s**.
- `cargo test -p sandart-render --release`: pass.
- `cargo check -p sandart-wasm --target wasm32-unknown-unknown --release`: pass.
- `cargo run -p sandart-sim --release --example diag_blocks -- --ticks 200 --grid 512 --material water`:
  **37.9 ms/tick** (stable across repeats), against the committed tree's ~21 ms/frame Water baseline
  and the design's own "under 15% of the tick" budget (~24.15 ms). **Budget NOT met** — roughly 80%
  overhead, not 15%. See "found beyond the three defects" for why.

---

## Found beyond the three defects

**The coarse relax's own per-tick cost is the real remaining performance problem, and it is NOT the
LUT.** Isolated by disabling `CoarseState::tick`'s relax step only (env-var-gated, temporary, removed
after measurement): at grid 512, `diag_blocks` breaks down as

| stage | ms/tick |
|---|---|
| Baseline (committed tree, no coupling) | ~21 |
| + `restrict`/`anchor`/readback, `sweeps = 0` | 28.84 (+7.8 for the full-grid `restrict` scan) |
| + `N = 8` relax sweeps (design's own budgeted value) | 38.92 (+10.1, ~1.26 ms/sweep-pair) |
| + `N = 16` relax sweeps (agy's shipped default, `COARSE_DEFAULT_SWEEPS`) | 46.03 (+17.2, ~1.07 ms/sweep-pair) |

`COARSE_DEFAULT_SWEEPS = 16` is DOUBLE the `N = 8` the design's own §8 budget arithmetic
("`8 * 4096 / 262144 = 12.5%` of one fine sweep") is computed against, with no re-derivation
recorded anywhere in `HIERARCHICAL-PRESSURE-PROGRESS.md` for why it was doubled. Even at the
design's own `N = 8`, the measured overhead (38.92 ms, ~85% over baseline) is still far over the
15% budget — the `restrict` pass and even zero-sweep bookkeeping alone already cost 7.8ms (37% of
baseline). **This was left at `N = 16` (agy's shipped value) rather than changed, since retuning a
convergence-affecting constant was out of scope for the three assigned defects** — reported here as
a real, measured budget violation for the next pass to address, not fixed.

This same cost is why `overfill_pressure_toggle` still takes 95s despite the LUT fix: the fixed
64x64 coarse grid does not shrink with the fine grid (§2's deliberate design choice), so at the
test suite's `w=128` scenarios the coarse grid is much LARGER relative to the fine grid than at 512,
making the relax cost dominate even more badly in the small-grid regime the test suite runs in.

**The `k_lat`/liquidity gate (I7) is not applied to the coarse coupling.** Design §6 I7 requires the
coupling to gate on `liquidity(wetness) >= LIQUID_ELLIPTIC_THRESHOLD` at both endpoints "until the
granular case is designed deliberately," on pain of growing the driving term without growing the
Mohr-Coulomb yield stress and collapsing dry sand's angle of repose at depth. agy's
`HIERARCHICAL-PRESSURE-PROGRESS.md` records this as a deliberate, later user ruling superseding I7
("Wait no, I want liquid and solid to be unified... anywhere in between I want it to move smoothly"),
implemented via `k_lat_a`/`k_lat_b` scaling the coupling continuously by liquidity rather than a hard
gate. `overfill_pressure_granular_preserves_angle_of_repose` still passes (part of the unchanged
101/10), so this has not visibly broken in the tests that exist — flagged here only because it is a
documented deviation from the written design, not because it demonstrably fails.
