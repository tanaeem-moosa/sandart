# Block resize: `block_size` from `grid_size/32` to `grid_size/64`

Status: **implemented, verified, not committed.** Written 2026-08-18. Makes the LOD scheduler's
block the same object as `coarse::CoarseGeometry`'s pressure tile
(HIERARCHICAL-PRESSURE.md §2), in preparation for wiring the coarse pressure level into
`settle_tick` (§5) — that wiring is NOT part of this change; `coarse.rs` still has no reference to
`block_size` and nothing reads its output.

This overrides the standing rule in `artifacts/HANDOVER.md` and `artifacts/design/HANDOFF.md`
("do not change `block_size` or the 32x32 block tiling"), on the user's explicit instruction. The
rule's reasons are recorded below where they turned out to still bind (grid 128's cost, the render
crate's fixed-texture assumption) and where they didn't (the wave-reach test, which turned out not
to depend on the production constant at all).

## The change

`sandart-sim/src/lib.rs`, `DrawingSimulation::new_with_size`:

```rust
let block_size = (grid_size / 64).max(1);   // was (grid_size / 32).max(1)
```

Floor stays `.max(1)`, unchanged from before. This was a deliberate reversal of an initial draft
that used `.max(2)` — see "Floor decision" below for why `.max(1)` won.

## 1. Budget constants (block counts) rescaled 4x

| constant | before | after | site |
|---|---|---|---|
| `budget_n` (construction) | 256 | 1024 | `lib.rs` `new_with_size` |
| `budget_n` (`reset()`) | 256 | 1024 | `lib.rs` `reset` |
| `BUDGET_MIN` | 32 | 128 | `lib.rs` `update` |
| `BUDGET_STEP_DOWN` | 4 | 16 | `lib.rs` `update` |
| `BUDGET_STEP_UP` | 1 | 4 | `lib.rs` `update` |

`budget_max = cols * rows` is already computed at runtime in `update`, so it needed no change —
it automatically tracks the new block count (4096 at grid >= 128, 4096 at grid 64 too, since the
floor stays `.max(1)` — see below).

Grepped for every other `budget_n`/`BUDGET_MIN`/`BUDGET_STEP_*`/`32x32`/`/ 32`/`* 32` site in
`sandart-sim`, `sandart-render`, `sandart-wasm` to confirm nothing else needed rescaling. The
remaining hits are either the render/shader/web sites below, or self-contained test harnesses in
`physics.rs` that hardcode their own `block_size`/`budget_n` literals directly (never derived from
`lib.rs`'s formula) and are therefore unaffected — e.g. `wave_pool`'s test helper uses a literal
`bs = 16`, several `diag_task47_*`/`diag_task55_*` `#[ignore]`d diagnostics use a literal
`(grid / 32).max(1)` for their own scenario setup rather than calling `DrawingSimulation`. These
were deliberately left alone: they are `#[ignore]`d instruments, not part of the default test run,
and updating their internal formula to match production would be a separate, larger diff with no
correctness stake in this task.

## 2. Wake reach (`activate_neighbor_upstream`/`_side`)

`test_sandbox_wave_reach_is_budget_independent` (one of the ten pre-existing library failures —
still failing, not a new regression) constructs its own `TestSim` directly with a **hardcoded**
`block_size = 16`, never calling `DrawingSimulation::new_with_size`. It is therefore **not**
affected by the production constant this task changes.

Measured before and after:

    before: reach/far-peak = 70/0.00000 at budget 32, 70/0.00000 at 64
    after:  reach/far-peak = 70/0.00000 at budget 32, 70/0.00000 at 64

Identical. The design brief anticipated this test might show a real propagation-speed regression,
but because it hardcodes its own block size the change doesn't reach it. The real-world
consequence — wake propagates one `block_size` cells per tick, so it now travels half the physical
distance per tick at any resolution >= 128 (8 cells/tick instead of 16 at grid 512) — is real and
un-instrumented by any existing test; it would only show up in a test that builds its scenario
through `DrawingSimulation` at production resolution. None of the ten existing failures or six
integration suites regressed, so nothing currently catches this, but it should be treated as a live
consequence, not a false alarm.

## 3. Block heat-map overlay (three files outside `sandart-sim`)

**`sandart-render/src/lib.rs`**: `HEAT_GRID_SIZE` 32 -> 64, plus its own and three other doc
comments describing "32x32" (struct field docs on `block_heat_texture`/`pressure_heat_texture`,
and the two texture-creation comments in `HeightmapRenderer::new`).

**`sandart-render/src/shader.wgsl`**: two runtime-compiled sites —
`let heat_block_coord = vec2<i32>(vec2<f32>(uv.x, uv.y) * 32.0)` -> `* 64.0`, and its clamp
`vec2<i32>(31)` -> `vec2<i32>(63)`; plus two doc comments (`block_heat_tex` binding comment,
`pressure_heat_tex` binding comment) that named "32x32". `cargo test -p sandart-render` is the
guard for this file (WGSL compiles at runtime, not build time) — see verification below.

**`sandart-wasm/web/index.html`**: two doc-comment sentences describing "32x32 LOD block", and the
`<b id="stat-blocks">1024</b>` placeholder text (immediately overwritten at runtime by
`demo.js`'s `statBlocks.innerText = fast+medium+slow`, so this was cosmetic, not load-bearing —
updated anyway for accuracy). `sandart-wasm/src/lib.rs`'s `get_active_block_counts` iterates
`self.sim.active_blocks` with no hardcoded length, so it needed no change. `node scripts/check_js.js`
passed after the `index.html` edit.

**Why this mattered more than a cosmetic-comment sweep:** `sandart-render::update_block_heat`
uploads `sim.block_heat_texels()` (length = current block count) into a texture hard-sized to
`HEAT_GRID_SIZE x HEAT_GRID_SIZE`, with **no bounds check** — the non-aligned-row branch does a
raw `data[src_start..src_start + size]` slice copy assuming the source is exactly `size x size`
bytes. If the block count had NOT stayed resolution-invariant (see the floor decision below), this
would have been a real out-of-bounds panic or silently wrong texture at whichever resolution
produced a mismatched block count — not merely a documentation staleness issue. This is the reason
the floor decision (§4) went the way it did.

## 4. Floor decision for grids 64 and 128

**Grid 64: floor stays `.max(1)`, giving `block_size = 1` (one cell per block, 4096 blocks).**
An earlier draft of this change used `.max(2)` (floor at the same `t >= 2` boundary `coarse.rs`
uses for its own `available` flag), which would have made grid 64 use 32x32 = 1024 blocks while
every other shipped resolution uses 64x64 = 4096. That was rejected specifically because of §3's
finding: the render crate's heat-map texture upload has no path for a source buffer smaller than
`HEAT_GRID_SIZE^2`, so a resolution-dependent block count would have reintroduced exactly the kind
of "silently misreads" bug this task was warned to avoid, in a place a `.max(2)` floor would have
created rather than fixed. `.max(1)` keeps the block count at the same 4096 for every shipped
resolution, matching the invariant the render/wasm/web layers already depend on.

This is a *different* decision from `coarse.rs`'s for the (still unwired) pressure module, which
disables itself (`available = false`) below `t = 2` rather than floor, because that module has a
real correctness constraint the LOD scheduler doesn't share: a coarse pressure cell's own overfill
pressure would double-count against itself at `t = 1`. The LOD scheduler has no such constraint —
a 1-cell block is just the smallest possible scheduling unit — so accepting it, rather than
disabling or flooring, was the option that cost nothing else.

Measured at grid 64, Water, `diag_blocks --ticks 300`:

| | block_size | blocks | budget | ms/frame |
|---|---|---|---|---|
| before | 2 | 1024 | 256 (25%) | 0.52 |
| after | 1 | 4096 | 1024 (25%) | 0.66 |

+27% at a budget fraction matched to before (25% of the block grid either way). Absolute cost is
negligible at this resolution (well under a millisecond either way).

**Grid 128: no floor, `block_size = 2` (2x2 cells, 4096 blocks) — measured, not assumed.**
This is exactly the case `physics.rs`'s `VERTICAL_PRESSURE_CAP_MULT` doc comment names as the known
slab-artifact geometry ("material moving multiple cells per tick while `block_size` is 2 cells").
Measured at grid 128, `diag_blocks --ticks 300`, budget fraction matched to before (25%):

| material | | block_size | blocks | budget | ms/frame | must/blocks | must fraction |
|---|---|---|---|---|---|---|---|
| Water | before | 4 | 1024 | 256 | 1.23 | 225.8 | 22% |
| Water | after | 2 | 4096 | 1024 | 2.03 | 689.5 | 67% |
| DrySand | before | 4 | 1024 | 256 | 0.82 | 139.3 | 14% |
| DrySand | after | 2 | 4096 | 1024 | 1.24 | 421.3 | 41% |

Grid 128 got measurably (50-65%) SLOWER, unlike grid 512 (next section), and the MUST tier's share
of all blocks roughly tripled. Reading: with 2x2 blocks, far fewer cells' worth of "quiet" pad each
block, so far more individual blocks register real per-tick displacement and land in the
budget-exempt MUST tier — less averaging/coalescing benefit than the old 4x4 block gave. This is
consistent with (though does not by itself confirm) the slab-artifact mechanism the existing
comment warns about: smaller blocks make simultaneous full-clamp movement across many blocks more
likely, not less.

**Decision: shipped without a floor at 128 anyway**, because:
- The absolute cost is still small (2.0 ms/frame worst case, far under any frame budget).
- No test in the suite (including the ten pre-existing failures) changed character at grid 128 —
  the standard test suite runs at 64 and 256, not 128, so this consequence is real but currently
  untested by anything in the repo.
- Flooring grid 128 to `block_size = 4` (matching the old tiling) would break the same
  resolution-invariant-block-count property that decided the grid-64 floor, at a resolution where
  it matters more (128 is likely to be used, unlike 64 which the design already treats as a
  degenerate edge case for the pressure module).
- This is flagged here as a **known, measured, accepted cost**, not a silent one, per the task's
  instruction not to ship a degenerate scheduler without a deliberate decision. If the slab
  artifact becomes visible at grid 128 in practice, this measurement is the first thing to revisit.

## 5. Performance at grid 512 (design's main prediction)

Design's claim: classification-loop cost was 0.01 ms at 1024 blocks; 4x that (4096 blocks) is
still negligible. Measured, `diag_blocks --ticks 300`, budget fraction matched to before (25% of
the block grid, so `--budget 256` before / `--budget 1024` after):

| material | | block_size | blocks | ms/frame | must | budgeted | stale | run |
|---|---|---|---|---|---|---|---|---|
| Water | before | 16 | 1024 | 20.3 | 76.0 | 173.8 | 23.8 | 273.7 |
| Water | after | 8 | 4096 | 21.1 | 224.9 | 751.7 | 102.6 | 1079.2 |
| DrySand | before | 16 | 1024 | 13.2 | 88.5 | 156.6 | 25.7 | 270.8 |
| DrySand | after | 8 | 4096 | 12.4 | 235.6 | 503.4 | 111.0 | 850.0 |

**Confirmed: negligible at 512.** Water +4%, DrySand -6% — both within run-to-run noise on this
machine (the runs above are each the median of 3). The 4x larger classification loop and 4x more
`run` (block-ticks actually simulated) does not show up as a proportional cost increase, because
each block is 4x fewer cells (8x8 vs 16x16), so total *cell*-work moved in the opposite direction.
Mass error stayed at the same order of magnitude (2-3e-8 to 2-3e-9) before and after, i.e. no
conservation regression.

(An earlier, unmatched comparison — `--budget 256` after the change, i.e. 256 of 4096 = 6.25% of
the domain rather than 25% — showed ms/frame roughly HALVE, 20.3 -> 8.3. That number is not
included in the table above because it compares different fractions of the domain and is
misleading on its own; it is mentioned here only so a reader who reproduces it with an unmatched
budget isn't confused by the discrepancy.)

## 6. Staleness-floor cost in cells (design's second prediction)

Design (§7b "Resolution note"): staleness should force ~137 blocks/tick instead of ~34 at grid 512,
but cost roughly the same number of *cells* (~8,700) either way, because each block is a quarter
the area. Measured (`stale` column above, Water, matched 25% budget fraction):

| | stale blocks/tick | cells/block | stale cells/tick |
|---|---|---|---|
| before | 23.8 | 256 (16x16) | 6,093 |
| after | 102.6 | 64 (8x8) | 6,566 |

Stale block count roughly **4.3x** (23.8 -> 102.6), consistent with the design's ~4x prediction
(exact figures differ from the design's own 34/137 because that measurement was taken on a
different scenario — the 512 hourglass in `physics.rs`'s own diagnostics — not this tool's
Hourglass drain scenario at `--ticks 300`). Stale cost in cells: **6,093 -> 6,566, roughly flat**
(+8%), confirming the design's "resolution-invariant in the unit that matters" claim.

## 7. Test results

`cargo test -p sandart-sim --lib --release`:

    before (baseline, per HANDOVER §11 and this task's own baseline run): 98 passed / 10 failed / 46 ignored
    after:  98 passed / 10 failed / 46 ignored

Identical failure set, both runs:

    physics::task55_head_spec::test_task55_dynamic_transport_spec_scoreboard
    physics::tests::test_dry_sand_has_angle_of_repose
    physics::tests::test_head_field_transport_repose_non_regression
    physics::tests::test_liquid_pool_levels_flat_in_closed_box
    physics::tests::test_liquid_stream_stays_coherent
    physics::tests::test_sandbox_wave_decays_to_flat_pool
    physics::tests::test_sandbox_wave_reach_is_budget_independent
    physics::tests::test_sandbox_wave_reflects_off_boundary
    physics::tests::test_sandbox_wave_stays_left_right_symmetric
    physics::tests::test_water_blob_stays_left_right_symmetric_under_gravity   <- the one sanctioned failure

Zero new failures, zero changed-character failures (assertion values in the printed diagnostics for
each of the ten are unchanged from what HANDOVER §11 and this task's brief record).

Six integration suites, all pass, no change from baseline:

    overfill_pressure_toggle:              7 passed, 0 failed, 13 ignored
    perfect_simulation_determinism:        2 passed, 0 failed
    fresh_pressure_field_toggle:           2 passed, 0 failed
    pressure_heatmap_head_field_toggle:    2 passed, 0 failed
    head_field_transport_toggle:           2 passed, 0 failed
    pressure_sensitive_flow_toggle:        3 passed, 0 failed, 1 ignored

`cargo test -p sandart-render`: 2 passed (`test_pipeline_creation_validation`,
`test_headless_render_capture`) + `shader_wgsl_parses_and_validates` and
`validator_rejects_broken_shaders` both pass — the WGSL edits (§3) parse and validate correctly.

`cargo check -p sandart-wasm --target wasm32-unknown-unknown --release`: clean.

`node scripts/check_js.js`: all checks pass (`index.html` edit didn't break `demo.js`).

## 8. Other files touched, and why

- `sandart-sim/src/coarse.rs`: module doc comment updated. It explicitly said the
  don't-touch-`block_size` constraint "is still in force for this step" — that became false the
  moment this change landed, so the comment was rewritten to say the change happened, what the two
  modules' floor decisions have in common and where they differ (grid 64: `coarse.rs` disables,
  the LOD scheduler doesn't), and that the wiring/de-duplication between the two `t`/`block_size`
  computations is still deferred (nothing in `coarse.rs` reads `block_size`; nothing in the LOD
  scheduler reads `COARSE_GRID`). No code in `coarse.rs` changed — it was already built
  independently of `block_size` by design.
- `sandart-sim/examples/diag_blocks.rs`: added a `--grid <N>` flag (default 512, matching its
  previous hardcoded behaviour) so this tool could produce the grid-128/64 measurements above.
  This is the measurement instrument itself, not a change to any tested behaviour.

## 9. Not touched, deliberately

- `sandart-sim/examples/diag_coarse_step0.rs` — another agent's uncommitted file, per instruction.
- The `#[ignore]`d `diag_task47_*`/`diag_task55_*` diagnostics in `physics.rs` that hardcode their
  own `(grid / 32).max(1)` — see §1.
- `artifacts/HANDOVER.md` / `artifacts/design/HANDOFF.md`'s standing-rule text ("do not change
  `block_size` or the 32x32 tiling") — still says the old rule. Left alone because updating
  historical handover documents wasn't asked for, but flagged here: whoever next reads those files
  will see a rule this task was explicitly told to override, with no note there that it happened.
  Worth a follow-up edit.

## 10. What §2 did not predict

Nothing broke that the design section didn't anticipate. The one genuine surprise was empirical
rather than architectural: grid 512's cost stayed flat/improved rather than merely "not
regressing" (§5), while grid 128's cost rose 50-65% rather than the "negligible, 0.01 ms" framing
that was written for the classification loop specifically (not for the whole scheduler at small
grids) — the design didn't distinguish these, and the measurement shows they behave oppositely at
the two ends of the resolution range.
