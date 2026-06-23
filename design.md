# Sandart: Per-Cell Property Advection — Design

**Status**: Ready to implement. Multicolor UI already ships.
**Scope**: Replace global `MaterialMode` enum with per-cell conserved scalar properties.
Magnetism (`IronFilings`, `Ferrofluid`) is removed outright.

---

## Motivation

Currently every physics parameter is controlled by a single global `MaterialMode` enum.
All cells in the simulation share one angle-of-repose, one flow rate, one wave speed.
When the marble carves through a sand bed you cannot have wet sand on one side and dry
on the other — it is all-or-nothing.

The goal is to make **every material property a per-cell conserved scalar**, advected with
the sand grains just like mass. When sand flows from cell A → cell B, all of its properties
mix proportionally:

    property'_B = (property_B × h_B  +  property_A × Δh) / (h_B + Δh)

This applies in **all** displacement contexts — CA settling AND `displace_line`.
Materials become *initialization presets* that stamp per-cell values onto the grid, not
a global mode that overrides everything.

---

## Per-Cell Properties

Five scalar channels, all stored as `f32` per cell, all advected identically:

| Channel     | Range | Meaning |
|-------------|-------|---------|
| `color`     | RGBA u8 | Visual color (already in plan) |
| `wetness`   | 0.0–1.0 | 0=dry CA sand, 1=pure liquid wave |
| `threshold` | 0.0–1.0 | Angle-of-repose; steeper slopes allowed before sliding |
| `flow_rate` | 0.0–1.0 | Fraction of excess height that flows per CA step |
| `grain_size`| 0.0–1.0 | Visual grain texture scale + sparkle character |

`threshold` and `flow_rate` together replace the per-material match arms in physics.rs.
`grain_size` replaces the per-material render branches in the shader.
`wetness` controls whether a cell runs CA or wave propagation.

### Why not more channels?

Stochastic vs. deterministic settling (`is_dynamic`) is fully determined by `threshold` + `flow_rate`
values — no separate flag needed. Wave parameters (`c_sq`, `damping`) are interpolated from
`wetness` via a lookup function. Oobleck shear-thickening is a function of local velocity
(already computed) — it needs no new property. Every existing branch in physics.rs collapses.

---

## Material Presets → Per-Cell Values

`MaterialMode` becomes a pure initialization preset. It fills the per-cell buffers on
reset/load and is then discarded — it has no runtime role.

| Old mode       | wetness | threshold | flow_rate | grain_size | fate |
|----------------|---------|-----------|-----------|------------|------|
| DrySand        | 0.00    | 0.08      | 0.25      | 0.45       | keep |
| CoarseSand     | 0.00    | 0.11      | 0.22      | 0.80       | keep |
| KineticSand    | 0.20    | 0.10      | 0.15      | 0.35       | keep |
| WetSand        | 0.45    | 0.14      | 0.08      | 0.40       | keep |
| FinePowder     | 0.00    | 0.05      | 0.30      | 0.05       | keep |
| Snow           | 0.05    | 0.15      | 0.20      | 0.20       | keep |
| MoonDust       | 0.00    | 0.20      | 0.20      | 0.10       | keep |
| Oobleck        | 0.55    | 0.04      | 0.12      | 0.15       | keep (shear-thickening via velocity) |
| ButterCream    | 0.70    | 0.04      | 0.15      | 0.08       | keep |
| Water          | 1.00    | 0.00      | 0.00      | 0.00       | keep (wave only) |
| CalmWater      | 0.90    | 0.00      | 0.00      | 0.00       | keep |
| Milk           | 0.95    | 0.00      | 0.00      | 0.00       | keep |
| VegetableOil   | 0.85    | 0.00      | 0.00      | 0.00       | keep |
| Yogurt         | 0.75    | 0.00      | 0.00      | 0.08       | keep |
| IronFilings    | —       | —         | —         | —          | **REMOVE** |
| Ferrofluid     | —       | —         | —         | —          | **REMOVE** |

Wave `c_sq` and `damping` are interpolated from `wetness` by a small lookup function rather
than per-mode match arms — no new per-cell channel required.

---

## CPU Data Model

### `DrawingSimulation` additions (sandart-sim/src/lib.rs)

```rust
/// Per-cell RGBA color. Advected with height during all displacement.
pub cell_colors:    Vec<u8>,   // GRID_SIZE * GRID_SIZE * 4  (4 MB)

/// Per-cell physics & render properties. Advected with height.
/// Layout: [wetness, threshold, flow_rate, grain_size] interleaved.
pub cell_props:     Vec<f32>,  // GRID_SIZE * GRID_SIZE * 4  (16 MB)
```

Helper to access channels:
```rust
const PROP_WETNESS:    usize = 0;
const PROP_THRESHOLD:  usize = 1;
const PROP_FLOW_RATE:  usize = 2;
const PROP_GRAIN_SIZE: usize = 3;

fn cell_prop(props: &[f32], idx: usize, ch: usize) -> f32 {
    props[idx * 4 + ch]
}
```

Initialize both buffers from the selected preset on `new()` and `reset()`.

---

## Physics Changes (sandart-sim/src/physics.rs)

### Advection helper (call at every flow site)

```rust
fn advect_properties(
    colors: &mut [u8], props: &mut [f32],
    src: usize, dst: usize,
    flow: f32, h_dst: f32,
) {
    let total = h_dst + flow;
    if total < 1e-6 { return; }
    let w_keep   = h_dst / total;
    let w_arrive = flow   / total;

    // Color (u8 RGBA)
    for ch in 0..3 {
        colors[dst * 4 + ch] = (
            colors[dst * 4 + ch] as f32 * w_keep
            + colors[src * 4 + ch] as f32 * w_arrive
        ).round() as u8;
    }
    colors[dst * 4 + 3] = 255;

    // Float properties (wetness, threshold, flow_rate, grain_size)
    for ch in 0..4 {
        props[dst * 4 + ch] =
            props[dst * 4 + ch] * w_keep
            + props[src * 4 + ch] * w_arrive;
    }
}
```

One helper, called identically everywhere height moves.

### `settle_tick` signature

```rust
pub fn settle_tick(
    heights:    &mut Heightmap,
    cell_colors: &mut Vec<u8>,
    cell_props:  &mut Vec<f32>,
    // ... existing params (active_blocks, wave_vel, marbles, etc.)
)
```

Remove `material: MaterialMode` parameter entirely. All per-cell branches read
`cell_props` instead of matching on `material`. For example:

```rust
// OLD
let threshold = match material {
    MaterialMode::DrySand => 0.08,
    MaterialMode::WetSand => 0.14,
    ...
};

// NEW — read directly from the cell being processed
let threshold = cell_props[center_idx * 4 + PROP_THRESHOLD];
let flow_rate = cell_props[center_idx * 4 + PROP_FLOW_RATE];
let wetness   = cell_props[center_idx * 4 + PROP_WETNESS];
```

Wave equation path activates when `wetness >= 0.75`. `c_sq`/`damping` come from
`wave_params(wetness)` interpolation, unchanged from current design.

Call `advect_properties(...)` immediately after every height move.

### `displace_line` signature

```rust
pub fn displace_line(
    heights:     &mut Heightmap,
    cell_colors:  &mut Vec<u8>,
    cell_props:   &mut Vec<f32>,
    // ... existing params
    paint_color:  Option<[u8; 4]>,     // None = no repaint, Some = stamp color
    paint_props:  Option<[f32; 4]>,    // None = no repaint, Some = stamp properties
)
```

Wherever displaced sand is pushed from one cell to another, call `advect_properties`.
`paint_color` / `paint_props` are optional overrides — if `Some`, the marble stamps new
values onto carved cells *after* advection (marble painting future feature).

### Removal of magnetism

Delete:
- All `IronFilings` and `Ferrofluid` match arms throughout physics.rs
- Magnetic force calculation blocks (the `to_magnet_norm` / `iron_filings_threshold` paths)
- Ferrofluid spike deformation in shader.wgsl
- All shader branches gated on `material_mode == 9u` (IronFilings) or `== 12u` (Ferrofluid)
- The two enum variants from `MaterialMode` in lib.rs

---

## GPU Data Model

### Heightmap texture: `R32Float` → `Rgba16Float`

Pack all per-cell data the shader needs into one texture:

| Channel | Content |
|---------|---------|
| R | Height (existing) |
| G | Wetness |
| B | Grain size |
| A | (reserved / future) |

`Rgba16Float` is 8 bytes/texel (vs 4 for `R32Float`). 1024² × 8 = 8 MB GPU texture.
Half-precision (f16) is sufficient for all three — height needs the most precision and
is well within f16 range.

`threshold` and `flow_rate` are CPU-only physics parameters; the shader does not need them.

### Colormap texture: `Rgba8Unorm` (unchanged format, now dynamic)

Still 1024×1024. Now driven by `cell_colors` CPU buffer, uploaded partially each frame
using existing `ActiveBounds` dirty region (same as heightmap partial upload).

### Upload each frame

```rust
// After sim tick:
renderer.update_heightmap_partial(&queue, &interleaved_h_w_g, bounds);
renderer.update_colormap_partial(&queue, &sim.cell_colors, bounds);
```

`interleaved_h_w_g` is assembled from heights + `cell_props` channels 0, 2 (wetness, grain_size).

---

## Shader Changes (shader.wgsl)

```wgsl
// Sample all per-cell properties in one call
let hm = textureSample(heightmap_tex, heightmap_sampler, uv);
let height    = hm.r;
let wetness   = hm.g;
let grain_size = hm.b;   // replaces per-material grain_scale

// Colormap lookup (unchanged call, now driven by dynamic CPU buffer)
let base_color = textureSample(colormap_tex, heightmap_sampler, uv).rgb;

// Derive grain / sparkle / roughness from grain_size scalar
// instead of per-material-mode if/else chain
let grain_scale = mix(50.0, 800.0, grain_size);
let roughness   = mix(0.55, 0.92, grain_size);
let sparkle_thr = mix(0.980, 0.998, grain_size);
```

Remove:
- All `if (uniforms.material_mode == Xu)` branches that set grain/sparkle/roughness/rim params
  (replaced by the scalar mix above)
- IronFilings magnetic spike deformation pass
- Ferrofluid branch
- `color_mode` uniform check (always use colormap when `cell_colors` are active)

---

## WASM API Changes (sandart-wasm/src/lib.rs)

| New export | Purpose |
|---|---|
| `set_cell_colors(data: &[u8])` | Store initial color pattern (replaces direct GPU upload) |
| `set_cell_props(data: &[f32])` | Store initial per-cell properties from preset |
| `set_material_preset(preset: &str)` | Fill both `cell_colors` and `cell_props` from named preset |

Remove:
- `set_material_mode(u32)` — no longer meaningful at runtime
- `update_colormap(data)` — replaced by `set_cell_colors` + internal partial upload

---

## UI Changes

### Material selector becomes "Preset" (index.html + demo.js)

- Rename "Material" dropdown to "Initial Preset".
- Remove IronFilings and Ferrofluid options.
- Selecting a preset calls `state.set_material_preset(name)` and resets the simulation.
- Add a future "Paint Mode" dropdown (material + color painting — Phase 2).

### Color pattern is decoupled from preset

The color pattern (solid/gradient/stripes/rainbow) and the material preset are independent.
`generateColormap` still generates a 4 MB RGBA pattern; it is passed to `set_cell_colors`.
`set_material_preset` fills `cell_props` based on the chosen physics preset without touching
`cell_colors`.

---

## File Change Summary

| File | Change |
|---|---|
| `sandart-sim/src/lib.rs` | Add `cell_colors`, `cell_props` fields; add `set_cell_colors`, `set_cell_props` methods; remove IronFilings/Ferrofluid from `MaterialMode` |
| `sandart-sim/src/physics.rs` | Remove `material` param from `settle_tick`/`displace_line`; add `cell_colors`/`cell_props`; replace all material match arms with per-cell reads; add `advect_properties` helper; delete all magnetism code |
| `sandart-render/src/lib.rs` | Change heightmap texture to `Rgba16Float`; add `update_colormap_partial`; update upload methods to interleave h+w+g |
| `sandart-render/src/shader.wgsl` | Read grain_size + wetness from heightmap channels; replace per-material branches with scalar `mix()`; delete IronFilings/Ferrofluid passes |
| `sandart-wasm/src/lib.rs` | Add `set_cell_colors`, `set_cell_props`, `set_material_preset`; remove `set_material_mode`, `update_colormap`; wire partial uploads in render path |
| `sandart-wasm/web/index.html` | Remove IronFilings/Ferrofluid; rename "Material" → "Initial Preset" |
| `sandart-wasm/web/demo.js` | Replace `state.update_colormap` → `state.set_cell_colors`; wire `set_material_preset` |

---

## Memory Budget

| Buffer | Size |
|---|---|
| `cell_colors` (CPU) | 1024² × 4 B = 4 MB |
| `cell_props` (CPU) | 1024² × 16 B = 16 MB |
| Heightmap GPU (`Rgba16Float`) | 1024² × 8 B = 8 MB |
| Colormap GPU (`Rgba8Unorm`) | 1024² × 4 B = 4 MB |
| **Total new** | **~32 MB** |

Previous total: ~8 MB (height R32Float + colormap). Increase of ~24 MB, well within budget.

---

## Testing

### Unit tests

**`test_advect_properties_weighted`**: Two cells, red/fine-powder source, blue/coarse-sand dest.
Flow half the source height into dest. Assert all channels (color, wetness, threshold,
flow_rate, grain_size) are correctly weighted-averaged.

**`test_property_conservation`**: 200 settle ticks. Assert sum of each property × height is
conserved within f32 precision (properties are concentration; total = Σ prop_i × h_i).

**`test_displace_line_advects`**: Call displace_line on a colored/propertied cell. Assert
pushed-out neighbors received blended properties.

**`test_magnetism_removed`**: Verify `MaterialMode::IronFilings` and `Ferrofluid` are not
in the enum. Verify `settle_tick` compiles without any `IronFilings` match arm.

### Manual verification

- Load **Stripes** pattern (DrySand + WetSand alternating bands). Run Gosper spiral.
  Verify stripes shear, blend, and that the wet band visibly resists settling more than dry.
- Paint a river (Water preset, future Phase 2) through a dry sand bed.
  Verify wave propagation on the liquid side, CA settling on the dry side, mixed behavior at boundary.
- Check that all previously-working material presets still produce distinct visual behavior.

---

## Future: Phase 2 — Material Painting

`displace_line` gains `paint_props: Option<[f32; 4]>` so the marble can deposit a
specific material type as it moves. This enables:
- Drawing a river of water through dry sand (wetness=1.0).
- Stamping wet sand onto a dry bed.
- Mixing materials mid-simulation with a UI "paint brush" mode.

This composes cleanly: painting is just `advect_properties` with a forced source value.
