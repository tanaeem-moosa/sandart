// Kernel F (hypothesis-1 hybrid) wasm timing -- A/E2/F, back to back, 3 repeats, same protocol
// as bench_wasm.mjs (see that file's header). Not wired into bench_wasm.mjs itself so the
// documented Step 4 command stays exactly reproducible; this is an ad hoc addition for the
// vectorisation-slowdown investigation (see the report).
//
// Usage: node sandart-kernel-bench/bench_wasm_f.mjs [snapshotDir] [wasmPath]

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
    samples.push((performance.now() - t0) * 1e6);
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
    console.log(`\n=== Kernel F wasm timing -- ${label} (3 repeats) ===`);
    for (let rep = 0; rep < 3; rep++) {
      const row = {};
      for (const kernel of ["a", "e2", "f"]) {
        const instance = await loadInstance();
        loadSnapshotIntoWasm(instance, file, kernel);
        row[kernel] = timeKernel(instance, kernel);
      }
      console.log(
        `  [rep ${rep}] A=${row.a.nsPerCell.toFixed(2)} E2=${row.e2.nsPerCell.toFixed(2)} F=${row.f.nsPerCell.toFixed(2)} ns/cell/pass` +
          `  (F vs E2: ${(100 * (row.f.nsPerCell - row.e2.nsPerCell) / row.e2.nsPerCell).toFixed(2)}%)`
      );
    }
  }
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
