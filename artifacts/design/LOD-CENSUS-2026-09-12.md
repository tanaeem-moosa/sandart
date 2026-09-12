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

## Cost split (native `Instant` proxy, not wasm)

    phase 0 (gravity-aligned)                        ~18%
    phase 1 (traversal + first lateral pass)         ~28%
    extra lateral passes (2b + arbitrate/apply)      ~49%
    everything else                                  2-7%

- **Cost per simulated block:** ~65-70 us per tick at N = 2.5 (75 ms / 1194, 25 ms / 360).
- **What drives cost:** the number of genuinely moving blocks, and the extra lateral passes are the
  largest single cost.
- **Caveat:** whether the timing tick rolled 1 or 2 extra passes is not recorded.

## Implication

The remaining levers are per-block cost, not block selection:
- run extra lateral passes only on blocks still flowing laterally (the "option 1" count is in
  progress);
- make each lateral pass cheaper, since several per-edge terms are constant within a tick.

Flux shares above sum to ~1.12 of the tick total because a cross-block edge is credited to both
blocks. They are comparable to each other, not absolute.
