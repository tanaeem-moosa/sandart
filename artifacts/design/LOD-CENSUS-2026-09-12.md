# Block-LOD census — is the scheduler simulating settled blocks? (2026-09-12)

This is a measurement only. The question: frame time at grid 512 is ~34 ms with `lateral_substeps`
= 2.5. Would skipping settled blocks more aggressively buy it back?

Instrument: `diag_lod_flux_budget_survey`.

    cargo test -p sandart-sim --lib --release -- --ignored --nocapture diag_lod_flux_budget_survey

- **Scene:** grid 512, MultiNeckHourglass (3 necks), Water, upper half filled to 0.5, N = 2.5.
- **Entry point:** `DrawingSimulation::update`, the production path.
- **Sampling:** flux is recorded only on the sample tick and the tick before it. Timing comes from a
  third tick with flux recording off, so it doesn't measure the instrument.
- **Displacement source:** attributed against the PREVIOUS tick's flux, since that is the flux that
  wrote `last_displacements`.

## Answer: no. The blocks being simulated are genuinely moving.

Budget 128, the adaptive controller's floor at 512 and where a 34 ms app sits:

    tick   MUST   MUST with max edge flux >= 1e-2   STALE   BUDGETED
    200    1194   1185 (99.2%)                      51      0
    1000    491    489 (99.6%)                      62      0
    3000    360    359 (99.7%)                      82      0

- **What makes a block MUST:** every MUST-by-displacement block got there from real flux, either on
  its own edges (~65%) or on an edge shared with a neighbour block (~35%). None came from a wake hint.
  `fresh_active` accounts for at most 3 blocks.
- **Why the draining body counts:** a draining chamber's whole body drops by more than 0.01 per tick,
  so it really is moving.

Counterfactual MUST threshold (budget 128): blocks dropped / share of tick flux they carried.

    threshold   t=200          t=1000         t=3000
    0.02        1.5% / 0.01%   5.7% / 0.3%    3.1% / 0.1%
    0.05        2.5% / 0.02%   14%  / 1.8%    11%  / 0.8%
    0.10        4.1% / 0.08%   23%  / 5.0%    19%  / 2.7%

A more aggressive threshold removes at most ~20% of blocks, and it does so by dropping several
percent of real flow, i.e. stalls. **Not worth pursuing.**

- **STALE blocks** (50-80) carry essentially zero flux: a small, bounded waste.
- **The BUDGETED tier** is empty at the floor. At full budget it holds 360-500 blocks, ~65% of them
  with no flux at all.

## CORRECTION: the census timings are inflated; use the release-build numbers

The census ran in a `cfg(test)` build, which does per-edge thread_local instrumentation checks, so its
absolute times are ~2x the shipped build's. Measured without a profiler in the release profile, same
scene, ticks 800-1800 (`SUBSTEPS_SWEEP=1 cargo run -p sandart-sim --release --example
profile_sandfall_water`):

    N   ms/tick   simulated cells/tick   ns per simulated cell per tick
    1   10.6      33.8k                  314
    2   17.2      36.8k                  467
    3   24.7      39.4k                  625

- **Marginal cost of one extra lateral pass:** ~155 ns per simulated cell in situ.
- **Standalone comparison** (sandart-kernel-bench, same snapshot): today's pass ported as-is is
  104-115 ns, and the array-form kernel A is 72-81 ns.
- **So:** the in-situ overhead beyond the flux math is ~1.4x, not the 2.5-3x an early reading of the
  census suggested.

The percentages below are still a reasonable guide to relative cost.

## Cost split (native `Instant` proxy, not wasm)

    phase 0 (gravity-aligned)                        ~18%
    phase 1 (traversal + first lateral pass)         ~28%
    extra lateral passes (2b + arbitrate/apply)      ~49%
    everything else                                  2-7%

- **Cost per simulated block:** ~65-70 us per tick at N = 2.5 (75 ms / 1194, 25 ms / 360).
- **What drives cost:** the number of genuinely moving blocks, and the extra lateral passes are the
  largest single cost.
- **Caveat:** whether the timing tick rolled 1 or 2 extra passes is not recorded.

## Option 1: how many simulated blocks do the extra lateral passes do nothing in?

Simulated blocks (MUST + STALE) bucketed by the lateral flux realised in the extra passes, summed
per block over the tick. Budget 128, from a second run:

    tick   simulated   extra-pass flux: none   < 1e-2   >= 1e-2
    200    1245        292 (23%)               42       911
    1000    553        153 (28%)               19       381
    3000    442        160 (36%)               10       272

- **The upper bound for "rerun only blocks still flowing laterally":** ~25-36% of extra-pass block
  visits, which is ~12-18% of tick time given the ~49% cost share.
- **Caveats:**
  - "none" includes the STALE blocks, which carry nothing anyway.
  - A real rule must PREDICT from earlier passes. This counts blocks that turned out to realise
    nothing over the whole tick, so the saving it measures is an upper bound.

## Implication

The remaining levers are per-block cost, not block selection:
- run extra lateral passes only on blocks still flowing laterally (the "option 1" count is in
  progress);
- make each lateral pass cheaper, since several per-edge terms are constant within a tick.

Flux shares above sum to ~1.12 of the tick total because a cross-block edge is credited to both
blocks. They are comparable to each other, not absolute.
