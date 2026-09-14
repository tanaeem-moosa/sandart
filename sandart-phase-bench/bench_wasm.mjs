// Wasm half of Step 0 (SESSION-HANDOVER-2026-09-13.md §6). Runs the sandart-phase-bench cdylib
// (target/wasm32-unknown-unknown/release/sandart_phase_bench.wasm, built with
// `RUSTFLAGS="-C target-feature=+simd128 -C link-arg=--import-undefined"` -- the extra linker
// flag is needed only because this module, unlike sandart-kernel-bench's, imports ONE host
// function (`phase_timing_now_ms`, `sandart_sim::phase_timing`'s wasm32 clock -- see that
// module's doc comment); +simd128 matches deploy.yml) under node, with no wasm-bindgen: plain
// `WebAssembly.instantiate` + an `env.phase_timing_now_ms` import backed by `performance.now()`.
//
// Usage:
//   node sandart-phase-bench/bench_wasm.mjs [wasmPath] [warmupTicks] [measuredTicks] [substeps]
//
// Defaults match native_phase_bench.rs's own defaults (WARMUP_TICKS=800, PHASE_TICKS=1000,
// LATERAL_SUBSTEPS=2.5).

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(__dirname, "..");

const wasmPath =
  process.argv[2] ||
  path.join(repoRoot, "target/wasm32-unknown-unknown/release/sandart_phase_bench.wasm");
const warmupTicks = parseInt(process.argv[3] || "800", 10);
const measuredTicks = parseInt(process.argv[4] || "1000", 10);
const substeps = parseFloat(process.argv[5] || "2.5");

// Kept in sync by hand with sandart_sim::phase_timing::SECTION_NAMES -- see that module's doc
// comment. `n_sections()` is read back from the instance too, as a cheap drift check.
const SECTION_NAMES = [
  "fresh_active",
  "classification",
  "temp_heights_copy",
  "phase0_collect",
  "phase0_apply",
  "phase1_traversal",
  "lateral_edge_pass",
  "copy_back",
  "settle_tick_total",
];

async function main() {
  const bytes = readFileSync(wasmPath);
  const importObject = { env: { phase_timing_now_ms: () => performance.now() } };
  const { instance } = await WebAssembly.instantiate(bytes, importObject);
  const ex = instance.exports;

  const nSections = ex.n_sections();
  if (nSections !== SECTION_NAMES.length) {
    console.error(
      `n_sections() = ${nSections} but this script has ${SECTION_NAMES.length} names -- SECTION_NAMES has drifted from phase_timing.rs, update it.`
    );
    process.exit(1);
  }

  console.log(`wasm module: ${wasmPath}`);
  console.log(`init: N=${substeps} warmup=${warmupTicks} ticks=${measuredTicks}`);
  ex.init(substeps, warmupTicks);

  const t0 = performance.now();
  ex.run_measured(measuredTicks);
  const wallMs = performance.now() - t0;
  const wallNs = wallMs * 1e6;

  const blocksPerTick = ex.blocks_per_tick();
  const cellsPerTick = ex.cells_per_tick();

  console.log(
    `wasm_phase_bench: N=${substeps} budget_n=${ex.budget_n()} warmup=${warmupTicks} ticks=${measuredTicks} ` +
      `wall=${(wallNs / 1e6 / measuredTicks).toFixed(4)}ms/tick blocks/tick=${blocksPerTick.toFixed(1)} ` +
      `cells/tick=${cellsPerTick.toFixed(0)}`
  );
  for (let i = 0; i < nSections; i++) {
    const ns = ex.section_ns(i);
    console.log(
      `  ${SECTION_NAMES[i].padEnd(20)} ${(ns / 1e6 / measuredTicks).toFixed(4).padStart(10)} ms/tick  ` +
        `${((100 * ns) / wallNs).toFixed(2).padStart(6)}%  ${(ns / cellsPerTick / measuredTicks).toFixed(2).padStart(8)} ns/cell`
    );
  }
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
