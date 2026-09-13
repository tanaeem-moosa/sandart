// Step 4 (wasm half) + Step 4's wasm timing table. Runs the sandart-kernel-bench cdylib
// (target/wasm32-unknown-unknown/release/sandart_kernel_bench.wasm, built with
// `RUSTFLAGS="-C target-feature=+simd128"`, same as .github/workflows/deploy.yml) under node,
// with no wasm-bindgen: plain `WebAssembly.instantiate` + `performance.now()`.
//
// Usage:
//   node sandart-kernel-bench/bench_wasm.mjs [snapshotDir] [wasmPath]
//
// Defaults: snapshotDir = this session's scratchpad (same default
// `dump_kernel_bench_snapshots` and `native_bench` use); wasmPath =
// target/wasm32-unknown-unknown/release/sandart_kernel_bench.wasm relative to the repo root.

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(__dirname, "..");

const snapshotDir =
  process.argv[2] ||
  "/tmp/claude-1000/-home-deck-projects-sandart/f1b6526a-1df3-459e-85d7-c652aafd17ae/scratchpad";
const wasmPath =
  process.argv[3] ||
  path.join(repoRoot, "target/wasm32-unknown-unknown/release/sandart_kernel_bench.wasm");

function median(samples) {
  const s = [...samples].sort((a, b) => a - b);
  return s[Math.floor(s.length / 2)];
}

async function loadInstance() {
  const bytes = readFileSync(wasmPath);
  // The cdylib has no imports (pure computation + its own bump-less allocator via Vec) --
  // confirmed by wasm-tools below; instantiate with an empty import object.
  const { instance } = await WebAssembly.instantiate(bytes, {});
  return instance;
}

function loadSnapshotIntoWasm(instance, filePath, kernel) {
  const data = readFileSync(filePath);
  const ptr = instance.exports.alloc(data.length);
  const mem = new Uint8Array(instance.exports.memory.buffer);
  mem.set(data, ptr);
  instance.exports[`init_${kernel}`](ptr, data.length);
}

function timeKernel(instance, kernel, { warmup = 10, iters = 60 } = {}) {
  const run = instance.exports[`run_${kernel}`];
  for (let i = 0; i < warmup; i++) run();
  const samples = [];
  for (let i = 0; i < iters; i++) {
    const t0 = performance.now();
    run();
    samples.push((performance.now() - t0) * 1e6); // ms -> ns
  }
  const cells = instance.exports[`cell_count_${kernel}`]();
  const ns = median(samples);
  return { ns, cells, nsPerCell: ns / cells };
}

// `kernel_c::precompute_head_static` alone, isolated exactly like `run_${kernel}` above but with
// no `cell_count_*` denominator of its own -- reported per-grid-cell using w*h (262144 at grid
// 512), separate from ns/cell/pass (see kernel_c.rs's module doc comment point 2).
function timePrecomputeC(instance, gridCells, { warmup = 10, iters = 60 } = {}) {
  const run = instance.exports.run_precompute_c;
  for (let i = 0; i < warmup; i++) run();
  const samples = [];
  for (let i = 0; i < iters; i++) {
    const t0 = performance.now();
    run();
    samples.push((performance.now() - t0) * 1e6);
  }
  const ns = median(samples);
  return { ns, nsPerCell: ns / gridCells };
}

// D's precompute analogue: same shape as timePrecomputeC, but the denominator is the cells the
// precompute actually visits (span cells), read back via `precompute_cell_count_d`, not w*h.
function timePrecomputeD(instance, { warmup = 10, iters = 60 } = {}) {
  const run = instance.exports.run_precompute_d;
  for (let i = 0; i < warmup; i++) run();
  const samples = [];
  for (let i = 0; i < iters; i++) {
    const t0 = performance.now();
    run();
    samples.push((performance.now() - t0) * 1e6);
  }
  const ns = median(samples);
  const cells = instance.exports.precompute_cell_count_d();
  return { ns, nsPerCell: ns / cells, cells };
}

// D's per-stage split: bracket each of the 5 stage exports with performance.now(), calling them
// in sequence each iteration (copy-in, stage2, stage3, stage4+5, copy-out) so the loaded state
// keeps evolving exactly like a real run_d() call -- see kernel_d.rs's module doc comment.
function timeDStages(instance, { warmup = 10, iters = 60 } = {}) {
  const stages = ["copy_in", "stage2", "stage3", "stage45", "copy_out"];
  const exportsByStage = stages.map((s) => instance.exports[`run_d_${s}`]);
  for (let i = 0; i < warmup; i++) for (const fn of exportsByStage) fn();
  const samples = stages.map(() => []);
  for (let i = 0; i < iters; i++) {
    for (let s = 0; s < stages.length; s++) {
      const t0 = performance.now();
      exportsByStage[s]();
      samples[s].push((performance.now() - t0) * 1e6);
    }
  }
  const cells = instance.exports.cell_count_d();
  const medians = samples.map(median);
  const total = medians.reduce((a, b) => a + b, 0);
  return stages.map((name, i) => ({
    name,
    ns: medians[i],
    share: medians[i] / total,
    nsPerCell: medians[i] / cells,
  }));
}

// E's per-stage split: same shape as timeDStages, but the 5 stages are precompute/stage2/stage3/
// stage4+5/swap (see kernel_e.rs's module doc comment) -- `run_e_swap` is the whole point of E's
// design (O(1) mem::swap instead of D's O(n) copy-out), so its own median should be near-zero.
function timeEStages(instance, { warmup = 10, iters = 60 } = {}) {
  const stages = ["precompute", "stage2", "stage3", "stage45", "swap"];
  const exportsByStage = stages.map((s) => instance.exports[`run_e_${s}`]);
  for (let i = 0; i < warmup; i++) for (const fn of exportsByStage) fn();
  const samples = stages.map(() => []);
  for (let i = 0; i < iters; i++) {
    for (let s = 0; s < stages.length; s++) {
      const t0 = performance.now();
      exportsByStage[s]();
      samples[s].push((performance.now() - t0) * 1e6);
    }
  }
  const cells = instance.exports.cell_count_e();
  const medians = samples.map(median);
  const total = medians.reduce((a, b) => a + b, 0);
  return stages.map((name, i) => ({
    name,
    ns: medians[i],
    share: medians[i] / total,
    nsPerCell: medians[i] / cells,
  }));
}

async function main() {
  console.log(`wasm module: ${wasmPath}`);
  const snapshots = [
    ["water", path.join(snapshotDir, "water_snapshot.bin")],
    ["gradient", path.join(snapshotDir, "gradient_snapshot.bin")],
  ];

  for (const [label, file] of snapshots) {
    console.log(`\n=== Step 4: wasm timing (node, +simd128) -- ${label} ===`);
    // Fresh instance per snapshot per kernel: run_* mutates state in place across the timed
    // iterations (same choice as native_bench's timing loop -- see its module doc comment), and
    // a fresh instance avoids any cross-kernel/cross-snapshot memory-growth interaction.
    for (const kernel of ["r", "a", "a_soa", "b", "c", "c8", "d", "e", "e_recip", "e2"]) {
      const instance = await loadInstance();
      loadSnapshotIntoWasm(instance, file, kernel);
      const { ns, cells, nsPerCell } = timeKernel(instance, kernel);
      console.log(
        `  ${kernel.toUpperCase()}: ${ns.toFixed(0)} ns/pass, ${nsPerCell.toFixed(2)} ns/cell/pass (${cells} cells)`
      );
    }

    // Precompute cost, isolated (see kernel_c.rs's module doc comment point 2). Grid cell count
    // (w*h) is read back from the snapshot header via a dedicated `alloc`+parse-free peek: reuse
    // `init_c`'s cell_count_c (owned/simulated cells) is the WRONG denominator here -- the
    // precompute runs over every grid cell, not just simulated ones -- so read w/h directly out
    // of the raw snapshot bytes (bytes 4..8 and 8..12, little-endian u32, per snapshot.rs's
    // documented format) instead of adding another wasm export just for this.
    {
      const instance = await loadInstance();
      const data = readFileSync(file);
      const w = data.readUInt32LE(4);
      const h = data.readUInt32LE(8);
      const ptr = instance.exports.alloc(data.length);
      new Uint8Array(instance.exports.memory.buffer).set(data, ptr);
      instance.exports.init_precompute_c(ptr, data.length);
      const { ns, nsPerCell } = timePrecomputeC(instance, w * h);
      console.log(`  C precompute: ${ns.toFixed(0)} ns/call, ${nsPerCell.toFixed(3)} ns/cell (${w * h} cells)`);
    }

    // D precompute, isolated the same way but over span cells only (see kernel_d.rs).
    {
      const instance = await loadInstance();
      const data = readFileSync(file);
      const ptr = instance.exports.alloc(data.length);
      new Uint8Array(instance.exports.memory.buffer).set(data, ptr);
      instance.exports.init_precompute_d(ptr, data.length);
      const { ns, nsPerCell, cells } = timePrecomputeD(instance);
      console.log(`  D precompute: ${ns.toFixed(0)} ns/call, ${nsPerCell.toFixed(3)} ns/cell (${cells} simulated cells)`);
    }

    // D's per-stage split.
    {
      const instance = await loadInstance();
      loadSnapshotIntoWasm(instance, file, "d");
      const stages = timeDStages(instance);
      console.log(`  D per-stage (median ns/pass, share of D's own stage total, ns/cell/pass):`);
      for (const s of stages) {
        console.log(`    ${s.name.padEnd(10)}: ${s.ns.toFixed(1).padStart(8)} ns  (${(100 * s.share).toFixed(1).padStart(5)}%)  ${s.nsPerCell.toFixed(3)} ns/cell/pass`);
      }
    }

    // E's per-stage split.
    {
      const instance = await loadInstance();
      loadSnapshotIntoWasm(instance, file, "e");
      const stages = timeEStages(instance);
      console.log(`  E per-stage (median ns/pass, share of E's own stage total, ns/cell/pass):`);
      for (const s of stages) {
        console.log(`    ${s.name.padEnd(10)}: ${s.ns.toFixed(1).padStart(8)} ns  (${(100 * s.share).toFixed(1).padStart(5)}%)  ${s.nsPerCell.toFixed(3)} ns/cell/pass`);
      }
    }
  }
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
