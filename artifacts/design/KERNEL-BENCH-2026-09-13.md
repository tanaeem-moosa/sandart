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
3. **Hypothesis, not measured:** A keeps early-outs on zero candidates, and in these scenes most edges
   are asleep, blocked or pooled. The branch-free kernels do the full arbitration and mixing work on
   every edge and cell. The flux field is sparse, and skipping is worth more than 4-lane SIMD on dense
   work. A per-stage count of nonzero candidates would test it.
4. **The storage refactor (0253caa) is still worth keeping.** It is bit-identical, removes a layout
   conversion, and makes the GPU colour upload a cast. It is not a speed win on its own.
5. **If the lateral pass is rewritten, A's structure is the candidate, not E2's.** Rebench A on the SoA
   layout first; it was measured on the old interleaved layout. A changes behaviour (Jacobi mixing, and
   noise-table RNG if adopted), so it needs the full physics gate list.
