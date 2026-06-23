# Per-Cell Color & Material Advection — Design

**Status**: Ready to implement. Multicolor UI already ships. This adds the physics layer that makes colors move with sand.

---

## Motivation

Currently the `colormap_tex` is a static CPU-generated texture sampled at fixed world-space UV. Colors are painted on the ground and never move.

The desired behavior is that **color is a conserved property of sand grains**, just like mass. When sand flows (via gravity settling or marble displacement), the color flows with it and mixes proportionally. A rainbow stripe pattern carved by a marble should shear, smear, and blend as gravity settles the displaced grains outward.

---

## Conservation Principle

Color is treated identically to mass: **total color is conserved**. When height Δh moves from cell A → cell B:

    color'_B = (color_B × h_B  +  color_A × Δh) / (h_B + Δh)

This is a **weighted average mix** — the receiving cell blends in the incoming color proportional to how much mass arrived. The source cell's color is unchanged (its concentration stays the same; it just has less mass).

This formula applies in **all** height-displacement contexts:
1. **CA settling** — gravity-driven grain flow between neighbors
2. **`displace_line`** — marble physically pushing sand

Both are displacement events and both must advect color to maintain conservation. Treating them differently would create color artifacts (e.g. marble carves a dark trough into a colorful bed but the pushed pile has the wrong color).

---

## Data Model

### CPU side (`sandart-sim`)

Add to `DrawingSimulation` in `lib.rs`:

```rust
/// Per-cell RGBA color. Advected with height during all displacement events.
/// Initialized from the UI-selected color pattern. Updated in-place each tick.
pub cell_colors: Vec<u8>,  // GRID_SIZE * GRID_SIZE * 4  (≈ 4 MB)
```

Initialize in `new()` with a flat desert-tan default. Expose `set_cell_colors(&[u8])` to bulk-load a pattern from JS.

No wetness or material-property buffer yet — that comes in the multi-material phase (see §Future).

### GPU side (`sandart-render`)

`colormap_tex` stays `Rgba8Unorm` 1024×1024. **No format changes, no new bindings.**

Add `update_colormap_partial(queue, data, bounds: ActiveBounds)` alongside the existing full-upload `update_colormap`. Mirrors the existing `update_heightmap_partial` — only the dirty bounding rect is transferred each frame.

### Shader (`shader.wgsl`)

**Zero changes.** The shader already samples `colormap_tex` at world UV and uses it as `mat_base_color` when `color_mode > 0`. The texture is just now dynamic instead of static.

---

## Physics Changes

### `settle_tick` in `physics.rs`

**Signature addition**: `cell_colors: &mut Vec<u8>`

Wherever the existing code does:
```rust
temp_heights[center_idx] -= clamped_flow;
temp_heights[neighbor_idx] += clamped_flow;
```

Immediately after, mix colors using heights *before* the flow:
```rust
let h_dst = temp_heights[neighbor_idx];  // height at dest before flow arrives
let total = h_dst + clamped_flow;
if total > 1e-6 {
    let w_keep  = h_dst        / total;
    let w_arrive = clamped_flow / total;
    let src = center_idx * 4;
    let dst = neighbor_idx  * 4;
    for ch in 0..3 {
        cell_colors[dst + ch] = (
            cell_colors[dst + ch] as f32 * w_keep
            + cell_colors[src + ch] as f32 * w_arrive
        ).round() as u8;
    }
    cell_colors[dst + 3] = 255; // alpha always opaque
}
```

Applies to **both** the dynamic CA path (stochastic DrySand/CoarseSand) and the static CA path (KineticSand, WetSand, FinePowder, Snow, etc.).

Does **not** apply to the wave path (liquid materials — those have no CA flow steps to hook into).

### `displace_line` in `physics.rs`

**Signature addition**: `cell_colors: &mut Vec<u8>`

`displace_line` calls `add_sand_with_limit` to push displaced mass into neighbor cells. Each call site where sand is moved applies the same mixing formula — the raised cells receive a color blend of their existing color and the color from the lowered cells, weighted by mass.

The marble displaces sand by lowering cells under its path and raising surrounding cells. The raised cells receive a color mix of: their existing color + the color from the lowered cells. This is physically correct — the marble pushes grains outward, and the pile is a mixture of whatever was there plus what was pushed.

---

## Upload Strategy

After each `sim.update()` call in `sandart-wasm`:

```rust
if sim.active_bounds.active {
    renderer.update_colormap_partial(&queue, &sim.cell_colors, sim.active_bounds);
} else if full_upload_needed {
    renderer.update_colormap(&queue, &sim.cell_colors);
}
```

`full_upload_needed` is set on reset/pattern-change. Otherwise only the dirty rect is uploaded — same bandwidth budget as the existing heightmap partial upload.

---

## Initialization Flow

1. User selects a color pattern + preset in the UI (existing controls — no changes).
2. JS calls `generateColormap(pattern, color1, color2)` → `Uint8Array` (4 MB).
3. JS calls **`state.set_cell_colors(data)`** — stores into `sim.cell_colors` on CPU.
   - Replaces old `state.update_colormap(data)` which uploaded directly to GPU, bypassing advection.
4. `full_upload_needed = true` triggers a GPU upload on the next render frame.
5. From that point, colors evolve via physics and get partially re-uploaded each tick.

---

## WASM API Changes (summary)

| Old call              | New call                       | Reason                                       |
|-----------------------|--------------------------------|----------------------------------------------|
| `state.update_colormap(data)` | `state.set_cell_colors(data)` | Must go through CPU buffer for advection |
| *(new)*               | `state.set_cell_colors(data: &[u8])` | Stores pattern into sim, marks full upload |

---

## What Does NOT Change

| Component                           | Unchanged |
|-------------------------------------|-----------|
| `shader.wgsl`                       | ✅        |
| `heightmap_tex` format (`R32Float`) | ✅        |
| `colormap_tex` format / binding     | ✅        |
| `color_mode` uniform                | ✅        |
| All UI controls (pattern, presets, pickers) | ✅ |
| `generateColormap` in `demo.js`     | ✅        |

---

## File Change Summary

| File | Change |
|------|--------|
| `sandart-sim/src/lib.rs` | Add `cell_colors: Vec<u8>` field; add `set_cell_colors` method |
| `sandart-sim/src/physics.rs` | Add `cell_colors` param to `settle_tick` and `displace_line`; insert mix formula at each flow site |
| `sandart-render/src/lib.rs` | Add `update_colormap_partial` method |
| `sandart-wasm/src/lib.rs` | Add `set_cell_colors` WASM export; wire partial/full color upload in render path |
| `sandart-wasm/web/demo.js` | Change `state.update_colormap(...)` → `state.set_cell_colors(...)` |

---

## Testing

### Unit tests to add

**`test_color_advection_ca`** (`physics.rs`):
Set up two cells, A (color red, height 0.5) and B (color blue, height 0.5). Run one settle step where A flows Δh=0.1 into B. Assert B's color is the correct weighted average and total color intensity is conserved.

**`test_color_advection_displace_line`** (`physics.rs`):
Place marble over a red cell, call `displace_line`. Assert that the pushed-out neighbors receive a blend of red and their original color proportional to the displaced mass.

**`test_color_conservation`** (`physics.rs`):
Run 200 settle ticks on a full grid. Assert that the sum of each color channel across all cells is conserved within floating-point tolerance.

---

## Future: Multi-Material Wetness Extension (Phase 2)

This design is Phase 1. Phase 2 adds a `wetness: Vec<f32>` field alongside `cell_colors` — the **exact same advection formula applies** (`wetness` is another per-cell scalar conserved like mass).

Phase 2 changes:
- `Rg32Float` heightmap texture (height + wetness in one sample).
- Colormap V-axis maps wetness (V=0 → dry colors, V=0.5 → wet sand colors, V=1.0 → water blue).
- Wave equation activates per-cell when `wetness >= 0.75`.
- `displace_line` gains `paint_wetness: Option<f32>` to let the marble paint material type.

Phase 1 composes cleanly with Phase 2 — the advection formula and upload strategy are identical.
