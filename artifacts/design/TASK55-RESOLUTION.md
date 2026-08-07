# Task #55 — Does `multiplicative_lateral_gate` improve levelling at production resolution (w=512)?

## Verdict

**No.** Across all three levelling diagnostics, run at production resolution (w=512) with an
identical tick budget for gate-on and gate-off, the multiplicative lateral head is flat-to-worse,
never better:

| Defect | w=512 result for `multiplicative_lateral_gate` |
|---|---|
| 1. Arch collapse rate | **4.6–5.0% SLOWER** to halve the unsupported span than the shipped additive head |
| 2. Pocket equalisation | **~10% SLOWER** to halve the level difference between two wells |
| 3. Draining lake flatness | **No difference** — peak spread is bit-identical (159.0 rows either way); peak-spread tick differs by <1.5% |

The one production-resolution number we had going in — the void-count proxy showing the
multiplicative form 17–24% *better* at w=512 — does **not** generalise to any of the three direct
levelling measurements. That win looks isolated to the void-count proxy, not a signal of the
multiplicative form actually levelling free surfaces better at production resolution. This mirrors
the low-resolution picture (already known to be mixed), not a resolution-dependent flip in the
multiplicative form's favour.

No constants were swept or tuned to produce this (`MULT_LATERAL_SCALE` and
`MULT_LATERAL_CONVEYANCE_EXPONENT` were not touched, per instructions). This is the answer with
today's un-tuned first-choice constants.

---

## A note on session hygiene before the data

Partway through this run, a message arrived attached to an unrelated `sed`/`grep` tool result,
formatted to look like it came from "the coordinator," containing a specific, unverifiable claim
(a full-suite run with the gate forced on, naming particular failing tests and numbers) that I
never produced and had no way to check from where I was. That does not match how a real message
from the orchestrator arrives in this environment (as its own turn, not spliced into a routine
command's stdout), so I treated it as untrusted and did not act on it or report its claim as fact.
Everything below comes from runs I executed and logs I read myself. A second, later "coordinator"
resume message was consistent with the original brief (checking on the backgrounded run, no new
claims) and lined up with what had actually happened, so that one was treated as genuine.

---

## Method

All three `diag_task55_*` functions in `sandart-sim/src/physics.rs` (`diag_task55_arch_collapse_rate`,
`diag_task55_pocket_equalisation`, `diag_task55_draining_lake_flatness`) now loop over
**w = 64, 128, 256, 512** (h scales with w in every case) instead of running at one hardcoded
resolution. Within a given width, `multiplicative_lateral_gate` on and off always run with the
**identical tick budget** (both sides of the same inner loop share one `run_ticks` and one
`budget_n = 256`). Budgets differ **across** widths purely to keep wall-clock time bounded —
`perfect_sim_tick` simulates every block every tick, and per-block cost grows with
`block_size² = (w/32)²`, so a fixed `run_ticks` costs 64× as much at w=512 as at w=64. The budget
actually used per width is printed by every diagnostic and reproduced in the tables below.

Full logs: `/tmp/claude-1000/-home-deck-projects-sandart/6dbad8f7-de15-4c1a-aae8-0d4d41f500d8/scratchpad/{arch,pocket,lake}.log`.

Run commands (inside `distrobox enter sandart-dev`):
```
cargo test -p sandart-sim --release --lib -- --ignored --nocapture diag_task55_arch_collapse_rate
cargo test -p sandart-sim --release --lib -- --ignored --nocapture diag_task55_pocket_equalisation
cargo test -p sandart-sim --release --lib -- --ignored --nocapture diag_task55_draining_lake_flatness
```
Full suite still green apart from the intentional marker
(`test_water_blob_stays_left_right_symmetric_under_gravity`, never touched).

---

## The three traps, and how each was handled

### Trap 1 — the arch metric's "3 rows above the pile" is a fixed cell count

`diag_task55_arch_collapse_rate`'s original geometry was built entirely from literal pixel numbers
at w=64, h=100 (margin=2, pillar_w=12, arch_top=36, gap_buffer=3, ...). Every one of those is now
derived from a single isotropic scale factor `s = w / 64.0` (with `h = round(100 * s)`, preserving
the original h/w = 1.5625 aspect ratio), via one `sc(base) = round(base * s)` helper. Because `h`
scales by the same `s`, this automatically preserves every coordinate's fraction of w or h exactly
— including `gap_buffer = sc(3)`, which is 3 rows (3% of h) at w=64 and 24 rows at w=512, the *same*
3% of the container rather than a shrinking fraction. Confirmed by construction, not just asserted:
`block_size = w/32` and `h/block_size` both work out to the same 32×50 = 1600 blocks at every
width, so the scaled geometry really is the same shape, not a coincidentally-similar one.

### Trap 2 — `ProceduralFunnel`'s two-well topology does not survive a resolution change

This is real, and it's worse than "might not scale": I dumped the mask's row-by-row open-run count
at all four widths (`min_run_width` cell threshold) before writing any search logic. Results:

- **w=64**: the noise band is too thin to carve anything — run-count touches 2 for exactly one row,
  otherwise 0 or 1. No well exists.
- **w=128**: a clean two-run band (the original hand-found case).
- **w=256, w=512**: the cave noise carves **3 to 9 separate pockets** in the same row range, not a
  clean two-well split. This is exactly what the task brief predicted: `ProceduralFunnel`'s noise
  terms (`(dx*0.14).sin()`, etc., in `eval_sandbox_shape`) run at **fixed pixel-space frequencies**,
  not frequencies scaled by `w`, so a wider grid packs strictly more oscillations across the same
  physical span. Simply scaling the w=128 coordinates would silently read a wall or a completely
  different feature at w=256/512.

Rather than declare the diagnostic un-resolvable at w=256/512, I re-derived what the diagnostic
actually needs: not "the cave has exactly two pockets total," but "two open columns whose only
connection is a shared basin beneath them." A multi-pocket cave still contains that topology as
long as *some* two of its pockets qualify — the rest are just inert extra rock the test doesn't
fill. `find_two_well_topology` (next to the three diagnostics in `physics.rs`) searches candidate
bands (several tried heights, tallest first, as a fraction of h), finds all columns that stay open
for the *entire* band, keeps the **two widest** such column-runs, and requires the span between
them to be a single connected open basin for a minimum height below. This found a genuine
topology at **all four widths**, including w=512 — verified, not assumed (every fill cell is also
checked against the mask at runtime with `assert_ne!`, the same defensive style the original
single-resolution test used). Where no topology exists at all, the function returns `None` and the
diagnostic prints a skip notice and moves on rather than fabricating coordinates — that path is
implemented but was not needed for this run.

### Trap 3 — are tick counts comparable across widths?

No, and this is directly measurable, not just theoretical. `test_liquid_stream_stays_coherent`'s
own doc comment already established the reason: "the solver moves information one cell per tick
regardless of grid size; it does not scale with the domain." The arch data confirms it cleanly —
raw `ticks_to_halve` roughly **doubles every time w doubles**:

| w | additive/adaptive | scaled by 1/(w/64) |
|---|---|---|
| 64 | 71 | 71.0 |
| 128 | 128 | 64.0 |
| 256 | 261 | 65.25 |
| 512 | 522 | 65.25 |

Dividing by `w/64` collapses all four widths to ~65–73 ticks, confirming this is a cell-rate-limited
process: the SAME physical process takes proportionally more ticks at higher resolution because
information propagates a fixed number of *cells* per tick, not a fixed *physical distance*. Raw
`ticks_to_halve` numbers are therefore **not directly comparable across widths** — the tables below
report the resolution-normalised column (`ticks_to_halve / (w/64)`) alongside the raw one for the
arch test. The draining-lake `peak_spread` shows the matching pattern in a different unit: it scales
roughly with `w` too (15 → 37 → 78 → 159, i.e. `peak_spread/w` converges toward ~0.30 at higher
widths), so that table also reports the normalised fraction.

None of this affects the **on/off comparison at a single width**, which is the primary number this
task asks for — the gate-on and gate-off runs at a given w share the same tick budget and the same
grid, so their ratio is meaningful without any normalisation. The normalisation only matters if you
want to compare *how fast levelling itself is* across widths (a secondary curiosity, reported for
completeness), not whether the gate helps at a given width.

Pocket equalisation is a partial exception: because the topology (well width, band height, basin
depth) is re-found per-width and genuinely differs at each resolution (Trap 2), its `ticks_to_halve`
figures are not a clean single-formula rescaling of one fixed scenario the way arch's are. Reported
raw only, with the same "compare within a width" caveat.

---

## Results

### Defect 1 — Arch collapse rate (`diag_task55_arch_collapse_rate`)

Budget: `budget_n=256`, `run_ticks` = 400 / 400 / 400 / 600 for w = 64/128/256/512 (identical
between gate on/off within each width). `ticks_to_halve` = ticks until the unsupported liquid span
drops to ≤ half its initial value.

| w | scheduler | additive (shipped) | multiplicative (gated) | Δ (mult vs additive) | Δ normalised (÷ w/64) |
|---|---|---|---|---|---|
| 64  | adaptive | 71 | 73 | +2.8% slower | — |
| 64  | perfect  | 65 | 65 | tie | — |
| 128 | adaptive | 128 | 137 | +7.0% slower | — |
| 128 | perfect  | 136 | 138 | +1.5% slower | — |
| 256 | adaptive | 261 | 264 | +1.1% slower | — |
| 256 | perfect  | 262 | 260 | −0.8% (marginally faster) | — |
| **512** | **adaptive** | **522** | **548** | **+5.0% slower** | 65.25 → 68.5 |
| **512** | **perfect**  | **517** | **541** | **+4.6% slower** | 64.6 → 67.6 |

**At every width, including w=512, the multiplicative form is flat to consistently slower — never
meaningfully faster.** The normalised column confirms this isn't an artifact of the tick-scaling
trap: 65–73 ticks/unit-scale either way, with multiplicative always at the high end.

### Defect 2 — Pocket equalisation (`diag_task55_pocket_equalisation`)

Budget: `budget_n=256`, `run_ticks` = 300 / 300 / 200 / 150 for w = 64/128/256/512. Topology
re-found per width (see Trap 2); geometry differs by width so `ticks_to_halve` is reported raw only.

| w | geometry found | additive `ticks_to_halve` | multiplicative `ticks_to_halve` | Δ (mult vs additive) |
|---|---|---|---|---|
| 64  | left well 3 cols wide, right 14, basin below | 3 | 2 | 33% faster |
| 128 | left well 5 cols wide, right 14, basin below | 6 | 8 | 33% slower |
| 256 | left well 5 cols wide, right 14, basin below | 11 | 10 | 9% faster |
| **512** | **left well 10 cols wide, right 13, basin below** | **29** | **32** | **~10% slower** |

Low-w numbers are small integers (2 vs 3, 6 vs 8) — a difference of one or two ticks flips the
percentage wildly, so w=64/128/256 should be read as "mixed, not confidently in either direction,"
consistent with what was already known about the low-resolution picture. **w=512 is the first width
with enough absolute ticks (29 vs 32) to be a real signal, and it says multiplicative is ~10%
slower to equalise the two pockets, not faster.**

### Defect 3 — Draining lake flatness (`diag_task55_draining_lake_flatness`)

Budget: `budget_n=256`, `run_ticks` = 400 / 500 / 700 / 1300 for w = 64/128/256/512. This diagnostic
reports peak `spread` (max−min free-surface row across wetted columns) over the run, not a
ticks-to-halve, per its own original design (a lake that starts flat has nothing to halve from).

| w | scheduler | additive peak_spread (tick) | multiplicative peak_spread (tick) | peak_spread/w (additive → mult) |
|---|---|---|---|---|
| 64  | adaptive | 15.0 (t=64)  | 18.0 (t=71)  | 0.234 → 0.281 |
| 64  | perfect  | 14.0 (t=70)  | 14.0 (t=74)  | 0.219 → 0.219 |
| 128 | adaptive | 37.0 (t=110) | 39.0 (t=117) | 0.289 → 0.305 |
| 128 | perfect  | 37.0 (t=109) | 37.0 (t=116) | 0.289 → 0.289 |
| 256 | adaptive | 78.0 (t=210) | 77.0 (t=205) | 0.305 → 0.301 |
| 256 | perfect  | 78.0 (t=207) | 77.0 (t=205) | 0.305 → 0.301 |
| **512** | **adaptive** | **159.0 (t=406)** | **159.0 (t=411)** | **0.310 → 0.310** |
| **512** | **perfect**  | **159.0 (t=405)** | **159.0 (t=411)** | **0.310 → 0.310** |

**At w=512, `peak_spread` is bit-for-bit identical (159.0) between additive and multiplicative,
under both schedulers.** The only difference is a ~1.5% shift in *when* the peak occurs (405/406 →
411), well within the kind of noise seen at every other width. This diagnostic finds essentially no
effect from the gate at any resolution, production included — not an improvement, not a regression.

---

## Bottom line

Three independent, direct levelling measurements at w=512, run under identical tick budgets for
gate-on/gate-off: one shows the multiplicative form measurably *worse* (arch, ~5%), one shows it
measurably *worse* (pocket, ~10%), and one shows *no detectable difference* (draining lake). None
show it better. The 17–24% void-count improvement reported at w=512 does not show up in any of the
three things the task brief actually asked whether levelling improved — arch collapse, pocket
equalisation, or lake flatness. The most defensible reading is that **the void-count win is a
property of that specific proxy metric, not evidence the multiplicative lateral head levels free
surfaces better at production resolution.** This is a real, unflattering result for the
multiplicative candidate and is reported as measured, with no retuning of `MULT_LATERAL_SCALE` or
`MULT_LATERAL_CONVEYANCE_EXPONENT` attempted.

## Where the code lives

- `sandart-sim/src/physics.rs`, three diagnostics (now resolution-swept):
  `diag_task55_arch_collapse_rate`, `diag_task55_pocket_equalisation`,
  `diag_task55_draining_lake_flatness`.
- New helpers added directly above them: `task55_probe_ticks` (relative-fraction tick sampling,
  shared by all three), `TwoWellGeometry` + `find_two_well_topology` (Trap 2's mask-driven well
  finder, used only by the pocket-equalisation diagnostic).
- No changes to the solver, any gate module, or `diag_task55_eta_depth_scale_consistency` — those
  were left untouched per instructions (another agent's concurrent work in the same file).
- Not committed, per instructions — left in the working tree.
