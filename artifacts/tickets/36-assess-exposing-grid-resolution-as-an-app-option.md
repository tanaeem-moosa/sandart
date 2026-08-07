# #36 — 2.13 — Assess exposing grid resolution as an app option

**Status:** completed

---

User request 2026-07-30, prompted by #35.

## Why — the user's rationale, which is the real justification
"If we are testing at 64 and watching at 512 we are already assuming it works in that range. By making it a feature we ensure it stays working like that."

Today the test suite runs at 64 and the app runs at 512. That is an UNVERIFIED ASSUMPTION of resolution invariance — precisely the assumption #35 proved false, and it went unnoticed for months. Making resolution a shipped, user-facing feature converts that assumption into a contract that breaks visibly when violated.

Second-order benefit: it upgrades the test suite. Today the 64-cell tests are a PROXY for 512. If 64 is a supported configuration, those tests exercise a real config rather than a scaled-down stand-in, and the 512 harness covers the other. Both become first-class instead of one pretending to speak for the other.

Also a debugging instrument in its own right: the user's original framing was "I could have noticed this if I could have seen simulation on 64 visually." That argues for building it even if the feature value alone would not justify it.

## Scope
Four discrete options: **64 / 128 / 256 / 512**. Not a free slider — that would multiply the testing surface for no benefit. 64 is included deliberately: it is the size the test suite uses, so it must be directly viewable.

This is the FIRST thing the user will test when they regain computer access, so it needs to reach the deployed GitHub Pages build. It must be wired through `sandart-wasm/src/lib.rs` and `web/{index.html,demo.js}` — a desktop-only implementation is invisible to their testing (see ARCHITECTURE.md section 2).

## Known obstacles — assess before implementing
`GRID_SIZE` is currently `pub const GRID_SIZE: usize = 512` in sandart-sim/src/lib.rs.
- The shader hard-codes it: `shader.wgsl` computes `vec2<i32>(i32(uv.x * 512.0), i32(uv.y * 512.0))` and clamps to `vec2<i32>(511)`. Needs to become a uniform.
- Buffers sized from the const and needing reallocation on change: `heightmap.data`, `cell_colors`, `cell_props`, `external_mass_this_tick`, `column_depth`, `edge_vel_h`/`edge_vel_v`, `row_mass`, `last_displacements`, the shape mask.
- The shape mask texture is 512x512 and is uploaded to the GPU (`HeightmapRenderer::update_shape_mask`).
- Block-LOD `block_size` and the budget numbers are tuned against 512 — check whether they need to scale with the grid or stay absolute, and say which.
- Shape geometry is mostly expressed as fractions of `w_f`/`h_f` so it should scale cleanly, but `PEG_SPACING`/`PEG_RADIUS` (GaltonBoard) are absolute cell counts and will change appearance with resolution. Flag anything else absolute.

## Sequencing
Depends on #35 (the resolution-invariance fix) landing first — no point wiring a resolution switch while resolution-dependent physics is still wrong. In fact the switch is the natural ACCEPTANCE TEST for #35: at every resolution the simulation should look and behave equivalently, and if it does not, #35 is incomplete.

Ship and push this SEPARATELY from the rest of the in-flight work, so the user can tell what they are meant to test first from what can wait.

## Must not regress
- Default must remain 512 — changing the shipped default is not part of this.
- All tests pass; do NOT weaken assertions.
- Changing resolution must fully reset/reallocate rather than leaving stale buffers — a partially-resized state would be a memory-safety and correctness hazard.
- Report ms/tick at each resolution; this doubles as a performance control and the numbers matter.
