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
    for (const kernel of ["r", "a", "b"]) {
      const instance = await loadInstance();
      loadSnapshotIntoWasm(instance, file, kernel);
      const { ns, cells, nsPerCell } = timeKernel(instance, kernel);
      console.log(
        `  ${kernel.toUpperCase()}: ${ns.toFixed(0)} ns/pass, ${nsPerCell.toFixed(2)} ns/cell/pass (${cells} cells)`
      );
    }
  }
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
