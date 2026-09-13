# Lateral-pass kernel benchmark: does vectorisation beat a plain restructure? (2026-09-13)

Crate: `sandart-kernel-bench`. Every kernel runs ONE lateral (cross-gravity) edge pass on real grid-512
snapshots taken mid-drain. There are two snapshots: 3-neck water, and a Dry sand -> Water gradient.

- **Timing:** ns per simulated cell per pass, median, quiet machine (load 1.0-1.2).
- **Build:** native release with default features (SSE2); wasm32 + simd128 under node v24, the same
  flags as the deploy.

Re-run:

    cargo test -p sandart-sim --lib --release -- --ignored --nocapture dump_kernel_bench_snapshots
    cargo run -p sandart-kernel-bench --release --bin native_bench
    RUSTFLAGS="-C target-feature=+simd128" cargo build -p sandart-kernel-bench --release --target wasm32-unknown-unknown --lib
    node sandart-kernel-bench/bench_wasm.mjs

## Kernels

- **R:** today's settle_tick structure ported exactly (per-edge collect into touched lists, list-driven
  arbitration, in-place `advect_properties`). Validated bit-exact against a real `TestSim::tick`.
- **A:** array form over row spans, scalar loops. It keeps early-outs on zero candidates and uses
  Jacobi mixing.
- **B:** block-at-a-time `wide::f32x8`, copying data into lane buffers.
- **C:** fully branch-free, with a noise-table RNG instead of hashes; always arbitrates.
- **C8:** C in `chunks_exact(8)`.
- **D:** C's math on padded per-channel scratch, gather-form balancing, per-channel mixing loops.
  Bit-identical to C.
- **E:** D's math in place on the production SoA layout (commit 0253caa) with swapped buffers. It
  departed from its brief: whole-grid temporaries and a fused mixing loop. Kept for the record.
- **E2:** E corrected: D's span-sized temporaries and per-channel loops, reading and writing SoA state
  directly, with no state copies. Bit-identical to D.
- **E2-recip:** E2 with one reciprocal per cell instead of a division per channel.

## Results

    kernel      native water  native gradient   wasm water  wasm gradient
    R               91             94              135          131
    A               71             76               96          108
    B               83             94              103          125
    C              112            119              174          185
    C8             124            130              182          197
    D              122            130              175          189
    E              150            172              210          233
    E2             115            132              177          196
    E2-recip       113            128               -            -

D's per-stage split (native water, same session earlier): copy-in 38, stage2 18, stage3 16,
stage4+5 35, copy-out 14. E2 removed both copies but its stage 4+5 rose to ~52. That lands E2 about
level with D.

v128 op counts (wasm):

| Kernel | v128 ops |
|---|---|
| A | 8 |
| B | 627 |
| C | 306 (stage2 272) |
| D | stage2 156, stage3 45, stage4+5 165 |
| E2 | stage1 198, stage2 156, stage3 45, stage4+5 231 |

The vectorised kernels really do emit SIMD.

## Conclusions

1. **A plain scalar restructure (A) is the fastest kernel measured, natively and in wasm.** It is
   ~1.3x R natively and ~1.25-1.4x R in wasm.
2. **Vectorisation did not pay in any form tried:**
   - the `wide` library (B);
   - auto-vectorised branch-free code (C/C8);
   - branch-free on per-channel scratch (D);
   - branch-free on the production SoA layout with no copies (E2).
   Removing the state copies (D -> E2) gained ~5% natively and nothing in wasm.
3. **CONFIRMED later the same day; see "Why E2 is slow" below. Original hypothesis:** A keeps early-outs on zero candidates, and in these scenes most edges
   are asleep, blocked or pooled. The branch-free kernels do the full arbitration and mixing work on
   every edge and cell. The flux field is sparse, and skipping is worth more than 4-lane SIMD on dense
   work. A per-stage count of nonzero candidates would test it.
4. **The storage refactor (0253caa) is still worth keeping.** It is bit-identical, removes a layout
   conversion, and makes the GPU colour upload a cast. It is not a speed win on its own.
5. **If the lateral pass is rewritten, A's structure is the candidate, not E2's.** Rebench A on the SoA
   layout first; it was measured on the old interleaved layout. A changes behaviour (Jacobi mixing, and
   noise-table RNG if adopted), so it needs the full physics gate list.

## Why E2 is slow (investigated 2026-09-13)

Measured with `cargo run -p sandart-kernel-bench --release --bin census_bench` (re-run independently).
Counts are within simulated spans.

    scene      pass   edges w/ nonzero candidate   edges w/ final flux > MIN_FLUX   cells with any flow
    water        1        5.5%                           4.7%                              5.5%
    water      200       13.4%                          12.7%                             13.0%
    gradient     1       23.2%                          22.6%                             28.1%
    gradient   200       20.5%                          17.2%                             17.8%

1. **Sparsity: CONFIRMED, dominant.** 72-95% of lateral edges and cells do no work on a given pass.
   - A's `if c == 0.0 { continue }` and the no-flow skip in mixing avoid almost all of that work.
   - D, E2 and F do full work on every cell. Colour unpack, mix and repack is the largest single
     sub-stage.
2. **Subnormal floats: REFUTED.** There are zero subnormals in any intermediate array, in both
   scenes, at pass 1 and pass 200. Native FTZ+DAZ timing differences are noise.
3. **Stage 4+5 locality: REFUTED.** Mixing from a small warm buffer instead of the whole-grid SoA
   row makes no consistent difference. E2's larger stage-4+5 figure partly reflects stage boundaries:
   D counts its colour unpack inside copy-in.
4. **Kernel F** (E2 with a chunk-level skip in mixing) is bit-identical to E2.
   - It gains 1-5% natively at chunk 32.
   - It is 1.6-8.4% SLOWER in wasm.
   - Retrofitting skips onto an unconditional vector kernel does not close the gap.

**Implication.** The lateral pass is a SPARSE problem, so the lever is not doing work for inactive
edges at all. SIMD width is not the lever.
- A still computes per-cell stage-1 terms (`in_transit_at`, capacity, head) for every cell in every
  simulated span, including the ~90% that turn out inactive.
- An active-edge set, e.g. edges that moved last pass plus their neighbours, could skip that stage-1
  work too.
- It is untested, and it must not reintroduce the settled-block problems from the LOD history.

## Chunked or packed vectorised kernel? (census, 2026-09-13 afternoon)

Instruments: `sparsity_bench`, `a_soa_bench`, `packed_estimate_bench`. Timings were taken at load
average 1.6-2.5, back-to-back in one process with 3 repeats, so read them as ratios. The counts
are deterministic and were re-run independently.

- **Real inactivity is chunk-skippable. Knowing it cheaply is not.**
  - Truly inactive chunks (no nonzero candidate): water 78-91%, gradient 45-72% at sizes 4-32.
  - Activity is scattered: median active run 2-10 edges, median gap 6-16 cells.
- **A cheap conservative predicate is too imprecise.** It uses no `in_transit_at`, hashes or
  arbitration: donor has mass, acceptor has room, head difference can beat tau minus max
  dispersion, or `edge_vel_h` is nonzero. It has zero false negatives, but it PASSES 47-60% of edges
  while only 5-23% are truly nonzero (precision 11-50%).
  - It fully rejects only 15-41% of chunks.
  - It costs 23-32 ns/cell natively, because it needs most of stage 1 (liquidity, capacity, head,
    tau) anyway.
- **Packed-kernel estimate:** predicate over all cells + scalar gather (6-9 ns per passed edge) +
  E2's stages 2-5 over passed edges lands at 69-76 ns/cell. That is ON the A_soa bar; break-even
  pass fraction 49-62% against real pass fractions of 47-51%. A wash.
- **A_soa** (A's structure on the production SoA layout with a swapped second buffer) is
  bit-identical to A.
  - It is ~5-6% slower than A natively, from writing through every span cell into the second
    buffer. It is indistinguishable from A in wasm.
  - E2 remains 1.6-1.8x A_soa, native and wasm.

**Conclusion: stop pursuing a vectorised lateral pass on this formulation.** Deciding what to skip
costs about as much as the per-cell scalar math it would skip, and A's inline early-outs already
test the real condition for free. A decision layer only pays with a predicate that is BOTH cheap and
precise.

The one untested candidate is temporal: last pass's realised flux, dilated to edges whose inputs
changed since (cell heights, `edge_vel_v` from phase 0, `column_depth`). That is dirty-tracking at
edge granularity. It has the same failure mode as the block LOD's settled-block skipping, stalling
real motion, so it needs its own design and correctness argument before any code.
