# Session handover — lateral substeps, Oobleck removal, SoA storage, kernel benchmark, array-form lateral pass

Covers 2026-09-11 evening to 2026-09-13 afternoon. `origin/main` = `3769c43`, and the working tree
was clean at handover except for the in-flight item in §6.

## 0. State
- Lib suite: **100 passed / 4 failed**. The four are the known ones in CLAUDE.md. The 100 includes
  the new guard `test_neck_pulse_does_not_grow`.
- All 5 integration targets pass.
- wasm32 check, workspace build and check_js are clean.
- Everything below is pushed to `main`.
- The user tests on the deployed page. There is still no browser driver here.

## 1. Shipped (in order)
- `8968667`, `8dcbe9b`: **wetness-weighted lateral sub-passes**.
  - Extra lateral passes per tick: `lateral_substeps` N, UI slider "Lateral substeps", page default
    **2.5** chosen by the user; the library default stays 1.0.
  - The fractional part of N is realised as ONE hashed roll per tick, global to the grid:
    M = floor(N) or floor(N)+1.
  - Extra pass k moves `clamp(1 + (M-1)*liquidity(DONOR) - k, 0, 1)` of its flux. The weight scales
    MASS; the stored edge velocity stays unweighted (`cand_h_unweighted`).
  - Extra passes read heads from `temp_heights`.
  - Fixes the 45-degree water facet: levelling range at t=3000 goes 168 -> 77 at N=2.5, and the
    stream stays ≤ 12 wide. See ASYMMETRY-2026-09-08.md §10-11.
- `f5bf430`: **Oobleck removed**, along with its wetness-band carve-out.
  - That carve-out made a one-way lateral wall at the band edge: the flux edge gate checked only the
    left owner cell, and the CA only pushes out.
  - This was the cause of the vertical cliffs in Dry sand -> Water gradient scenes.
  - The instrument is `diag_gradient_cliffs`.
- `fc3f4b1`: **LOD census** (LOD-CENSUS-2026-09-12.md). More than 99% of MUST blocks really move ≥ 0.01,
  so a more aggressive threshold would stall real motion. Not worth pursuing.
  - Its absolute timings came from a cfg(test) build, which is ~2x inflated. Corrected in the doc.
- `0253caa`: **SoA storage**.
  - `CellProps { wetness, threshold, flow_rate, grain_size }` as four Vecs; colour is one
    `u32` per cell (r | g<<8 | b<<16 | a<<24), uploaded via `bytemuck::cast_slice`.
  - Bit-identical, proven by `diag_state_checksum` (5 scenarios) plus identical diagnostic outputs.
  - The JS API keeps its interleaved formats.
- `eeefce7`: **array-form lateral pass in production** (`run_lateral_edge_pass`, kernel A's
  structure).
  - Row spans, per-cell frozen arrays, candidate -> arbitrate -> apply, **Jacobi mixing**, scalar
    early-outs. Red-black removed.
  - Release ms/tick: 9.13 -> 6.75 (N=1), 13.95 -> 9.46 (N=2), 19.61 -> 12.52 (N=3). All physics gates
    held, and mirror asymmetry improved.
- `157f76f`: `spec_draining_vessel_surface_dips` now averages over the last 50 ticks.
  - It had read a single tick, sampling a pulse.
  - New guard `test_neck_pulse_does_not_grow`: fails above 1.25x the baseline of 6.4508 cells/tick
    (w=64) and 37.8867 (w=512).
  - The near-neck **period-2 pulse is pre-existing and ACCEPTED by the user**, identical before and
    after `eeefce7`. The baseline may only be lowered.
- `3769c43`: the stats footer always shows **"step · render ms"**.
  - "render" is the `state.render()` CALL, mostly CPU-side texture prep; not GPU execution.
- The benchmark commits (`cf5ee9b`, `092ad75`, `008db70`, `349f34b`, `468b122`) are measurement only.

## 2. Vectorisation: measured and closed
Full record in KERNEL-BENCH-2026-09-13.md. The crate is `sandart-kernel-bench`.
- Quiet-machine ns/cell/pass, water:
  - native: R (old structure) 91, **A 71**, B (`wide`) 83, C (branch-free) 112, D 122, E2 (SoA, no
    copies) 115;
  - wasm: R 135, **A 96**, E2 177.
- **Why SIMD loses: the lateral pass is SPARSE.** Only 5-23% of edges carry flux, and A's scalar
  early-outs skip the rest.
- Refuted: subnormal floats, stage-4+5 locality, copy cost (D -> E2 gained ~5% natively and nothing
  in wasm).
- Kernel F (chunk skip) is a wash.
- A cheap conservative predicate passes 47-60% of edges while only 5-23% are real, so a packed kernel
  lands on A's cost.
- **Do not reopen SIMD for this formulation.** The only untested idea is temporal dirty-tracking of
  active edges, which carries the same stall risk as LOD settled-block skipping.
- The earlier block-size sweep (BLOCK-SIZE-SWEEP.md) found 8 optimal. Bigger LOD blocks add quiet
  cells, which is worse for any kernel.

## 3. User decisions this session (binding)
- No liquid-only / material-conditional paths, not even as optimisations. Behaviour must be
  continuous in wetness.
- Lateral substeps: donor wetness weighting; fractional N realised stochastically and globally;
  2.5 on the page.
- Oobleck is gone ("we never got it working").
- An accepted pre-existing defect gets a "must not grow" guard, not a failing marker.
- The user would rather NOT fork the simulation into a GPU compute path (option 3), and finds
  multithreading on gh-pages too complex for now (option 2).
- **Interested in: simulate at 256, render at 512/1024 without the staircase (option 1), after
  option 4.**
  - The shader already bilinear-filters heights (`sample_height_bilinear`, sandart-render/src/shader.wgsl).
  - The per-cell staircase at 256 likely comes from the shape mask (`textureLoad` with integer cell
    coords, fragment ~line 225) and from colours.
  - UNCONFIRMED: ask the user which staircase they see (vessel outline, colour boundaries, or the
    surface).

## 4. Open items
- **Option 4 IN PROGRESS: restructure phase 0 (gravity-aligned pass) and the phase-1 traversal like A.**
  See §6.
- User has seen ~30 ms "CPU" in the browser, possibly before `eeefce7`. With the new footer, check
  whether step() is still ~30 ms. If so, most of it is outside `settle_tick`: instrument `update()`.
- Render-call prep (~5 ms) rebuilds an interleaved heights/wetness/grain float buffer every frame for
  the active bounds (sandart-wasm/src/lib.rs ~1009-1090). A cheap win candidate.
- The period-2 neck pulse is accepted and guarded. It may be the edge-velocity alternating mode
  (HANDOVER.md §10), and may be the "drainage lines". Unconfirmed; not a priority.
- A small incompressibility leak exists in mixed materials: max(h - cap) = 2.68e-2 on the gradient
  snapshot. When wetter material mixes into a sand cell, capacity drops 1.5 -> 1.0 while its height
  stays.
- Still open from before: `anti_merge_ceiling` (the cascade failure); the dead overlay plumbing in
  the renderer; an unidentified RNG path reaching the liquid solver.

## 5. Environment lessons (cost real time this session)
- **Run cargo in the FOREGROUND with `CARGO_BUILD_JOBS=2`.** Background cargo tasks were killed
  twice for "low memory" even with 7 GB free.
- **`pgrep -f <pattern>` in a wait loop matches the loop's own command line.** Use `pgrep -x`, or
  check the args.
- Other projects (pixelsim `ca_explorer.py`, `studio.py`) can load the CPU. Check `uptime`/`ps` before
  any timing, and compare old/new back-to-back in alternating rounds.
- cfg(test) builds carry per-edge thread_local instrumentation, so their timings are ~2x inflated.
  Time release builds, e.g. `SUBSTEPS_SWEEP=1 cargo run -p sandart-sim --release --example
  profile_sandfall_water`.
- pprof under the `profiling` profile still misattributes inlined `settle_tick` time; a helper once
  showed 78% "self". Use section timers instead.
- Sonnet agents hit session limits mid-task. Resume them with SendMessage after the reset; they keep
  context.
- Agents deviated from briefs in ways that decided results:
  - skipped Step-0 baselines;
  - used whole-grid temporaries and fused loops (kernel E);
  - wrote scatter `+=` in "branch-free" code (C).
  Read the key loops before trusting a timing.

## 6. In-flight at handover: option 4 agent (stopped by rate limit)
- The agent was restructuring phase 0 and the traversal. It stopped at the start of Step 1, while
  still reading the phase loop.
- Left behind:
  - `sandart-sim/Cargo.toml` has a new `phase-timing` feature stanza (uncommitted). The feature's
    code sites may not exist yet; grep `phase_timing`.
  - A pre-change copy of the tree: `scratchpad/pre_phase0_repo/`.
  - A `scratchpad/phase0/` directory, possibly empty or partial, for the before_* baselines.
- To resume: SendMessage the agent (its brief is in the transcript), or re-brief.
- The Step-0 breakdown comes first: which of classification, temp_heights copy, phase 0 collect /
  apply, phase-1 traversal, lateral passes, copy-back and the rest of update() dominates the ~140
  ns/cell outside the lateral pass.
- Gates:
  - lib 100/4 with the pulse guard passing;
  - the 5 integration targets;
  - stream width ≤ 12 at N=2.5, levelling, repose, mass, cliffs, mirror;
  - `diag_task70_rest_color_mixing_and_checkerboard` (colour mixing, velocity parity);
  - the wasm check; the `phase-timing` feature must stay off for wasm (`Instant` panics there).
- The main thread does the authoritative before/after timing against `pre_phase0_repo`.
