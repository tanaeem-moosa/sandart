# Vertical Mode: Hourglass Simulation — Design

**Status**: Proposal — awaiting review.
**Scope**: Add directional gravity + hourglass boundary shape to the existing simulation.
No new crate; no new rendering pipeline. Same UI with additional controls.

---

## Motivation

The current simulation has no concept of directional gravity. Sand flows symmetrically
toward any lower neighbor — it is a horizontal table viewed from above. This makes it
impossible to simulate an hourglass, where sand must fall *downward* through a narrow
constriction under the pull of gravity.

The goal is to add a **gravity bias vector** to the existing CA settling, plus a new
`SandboxShape::Hourglass` boundary, enabling hourglass-style simulations while sharing
>95% of the existing code.

---

## Conceptual Model

The grid represents a **vertical cross-section** of the hourglass when gravity is enabled:

```
  ┌──────────────────────┐
  │   x →                │
  │ y                    │   Grid coordinates:
  │ ↓    ╔══════════╗    │   - x: horizontal position
  │      ║ upper    ║    │   - y: vertical position (y=0 is top)
  │      ║ chamber  ║    │   - height value: depth of sand "into the screen"
  │      ╚════╗╔════╝    │
  │           ║║  neck   │   Gravity pulls sand toward +y (downward in grid)
  │      ╔════╝╚════╗    │
  │      ║ lower    ║    │
  │      ║ chamber  ║    │
  │      ╚══════════╝    │
  └──────────────────────┘
```

The height value at each cell still represents the *amount* of sand at that (x, y) position.
From the camera's perspective (top-down on the grid), you see the sand distribution as a
depth map. The marble can still carve grooves and push sand around within this cross-section.

---

## Two Independent Features

The design decomposes into two orthogonal features that compose cleanly:

1. **Directional gravity** — a bias vector that makes sand preferentially flow in one
   direction. Works with *any* sandbox shape (Circle, Square, Oval, Hourglass).
2. **Hourglass shape** — a new `SandboxShape` variant with two chambers and a neck.
   Works with *or without* gravity enabled.

Either feature is useful alone. Together they produce an hourglass simulation.

---

## 1. Directional Gravity

### New field on `DrawingSimulation`

```rust
/// Gravity direction in grid space. (0, 0) = no directional gravity (horizontal table).
/// (0, 1) = sand falls toward +y (downward on screen). Magnitude controls strength.
pub gravity_dir: Vec2,
```

Default: `Vec2::ZERO` (preserves current horizontal-table behavior).

### Physics change: biased CA flow

In `settle_tick`, the CA flow loop (physics.rs ~L965–L1004) currently checks whether
`h_center - h_neighbor > threshold` and then flows proportionally. The change adds a
gravity-driven component:

```rust
// Current neighbor offsets (grid-space unit vectors)
const NEIGHBOR_DIRS: [(f32, f32); 4] = [
    (-1.0, 0.0),  // Left
    ( 1.0, 0.0),  // Right
    ( 0.0,-1.0),  // Top    (toward y=0)
    ( 0.0, 1.0),  // Bottom (toward y=max)
];

for (i, &neighbor_idx) in neighbors.iter().enumerate() {
    let h_neighbor = heightmap.data[neighbor_idx];
    let geom_slope = h_center - h_neighbor;

    // --- NEW: gravity bias ---
    // dot(neighbor_direction, gravity_dir) is positive when this neighbor is
    // "downhill" in the gravity sense. This effectively lowers the threshold
    // for downward flow and raises it for upward flow.
    let (ndx, ndy) = NEIGHBOR_DIRS[i];
    let gravity_dot = ndx * gravity_dir.x + ndy * gravity_dir.y;
    let gravity_push = gravity_dot * GRAVITY_STRENGTH;

    let effective_slope = geom_slope + gravity_push;
    // --- END NEW ---

    if effective_slope <= threshold {
        continue;
    }

    // ... rest of flow calculation uses effective_slope instead of geom_slope
}
```

**`GRAVITY_STRENGTH`**: Tunable constant. A value of ~0.03–0.05 produces a gentle but
visible downward drift. Higher values make sand pour rapidly.

### Key properties

- When `gravity_dir == Vec2::ZERO`, `gravity_push == 0` for all neighbors and the
  simulation is **identical** to today. Zero behavioral change for horizontal mode.
- Gravity bias is **per-tick additive** — it does not replace the slope-based flow.
  Sand on flat ground slowly migrates downward; sand on slopes still flows by angle-of-repose.
- The gravity push also applies to the **avalanche safety check** (physics.rs ~L888–L923),
  using the same `effective_slope` instead of `geom_slope`.
- **Wave propagation** (liquid cells, wetness ≥ 0.75) does not need gravity bias — the
  wave equation naturally redistributes liquid under its own dynamics.

### Gravity in `displace_line`

`displace_line` does not use slope thresholds — it carves a groove and deposits sand at
fixed ridge offsets. No changes needed. Gravity will naturally settle the deposited ridges
downward via the CA step.

### `update()` signature change

```rust
pub fn update(
    &mut self, dt: f32,
    targets: &[Option<Vec2>; 5],
    marble_radius: f32,
    material: MaterialMode,
    shape: SandboxShape,
    last_frame_time_ms: f32,
    target_frame_time_ms: f32,
    gravity_dir: Vec2,        // ← NEW
)
```

Store `gravity_dir` on the struct and pass it through to `settle_tick`.

### `settle_tick` signature change

```rust
pub fn settle_tick(
    // ... existing params ...
    gravity_dir: Vec2,        // ← NEW
) -> f32
```

---

## 2. Hourglass Shape

### New `SandboxShape` variant

```rust
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum SandboxShape {
    Circle,
    Square,
    Oval,
    Hourglass,  // ← NEW
}
```

### Geometry

Two circles connected by a narrow rectangular neck:

```
           r_chamber
         ┌───────────┐
    ╭────╯           ╰────╮   ← upper chamber center at (center_x, center_y - chamber_offset)
    │                     │
    ╰──╮               ╭──╯
       │   neck_width  │      ← neck region, height = neck_height
    ╭──╯               ╰──╮
    │                     │
    ╰────╮           ╭────╯   ← lower chamber center at (center_x, center_y + chamber_offset)
         └───────────┘
```

Parameters (all as fractions of grid size):
- `chamber_radius`: 0.28 × grid_size — radius of each circular chamber
- `chamber_offset`: 0.32 × grid_size — vertical offset of each chamber center from grid center
- `neck_half_width`: 0.04 × grid_size — half-width of the connecting passage

### Boundary function (`is_inside`)

```rust
crate::SandboxShape::Hourglass => {
    let chamber_r = 0.28 * w_f;
    let chamber_offset = 0.32 * h_f;
    let neck_hw = 0.04 * w_f;

    let chamber_r_sq = chamber_r * chamber_r;

    // Upper chamber
    let dy_upper = dy + chamber_offset;
    let in_upper = dx * dx + dy_upper * dy_upper < chamber_r_sq;

    // Lower chamber
    let dy_lower = dy - chamber_offset;
    let in_lower = dx * dx + dy_lower * dy_lower < chamber_r_sq;

    // Neck: a rectangle bridging the two chambers
    let in_neck = dx.abs() < neck_hw
        && dy.abs() < chamber_offset;

    in_upper || in_lower || in_neck
}
```

### `clamp_to_sandbox` for Hourglass

The marble needs to be constrained to the hourglass interior. The clamping logic projects
the marble position onto the nearest valid point:

```rust
SandboxShape::Hourglass => {
    let chamber_r = 0.92 - marble_radius;  // normalized coords
    let chamber_offset = 0.58;             // normalized vertical offset
    let neck_hw = 0.07 - marble_radius;    // normalized neck half-width

    // Check if in upper chamber, lower chamber, or neck
    let in_upper = Vec2::new(pos.x, pos.y - chamber_offset).length() < chamber_r;
    let in_lower = Vec2::new(pos.x, pos.y + chamber_offset).length() < chamber_r;
    let in_neck = pos.x.abs() < neck_hw && pos.y.abs() < chamber_offset;

    if in_upper || in_lower || in_neck {
        pos  // already inside
    } else {
        // Clamp to nearest boundary (upper or lower chamber)
        let to_upper = Vec2::new(pos.x, pos.y - chamber_offset);
        let to_lower = Vec2::new(pos.x, pos.y + chamber_offset);
        if to_upper.length() < to_lower.length() {
            let dir = to_upper.normalize_or_zero();
            Vec2::new(0.0, chamber_offset) + dir * chamber_r
        } else {
            let dir = to_lower.normalize_or_zero();
            Vec2::new(0.0, -chamber_offset) + dir * chamber_r
        }
    }
}
```

### Hourglass initialization

On reset with `SandboxShape::Hourglass`, only the upper chamber is filled with sand:

```rust
fn initialize_hourglass(heightmap: &mut Heightmap) {
    let w = heightmap.width;
    let h = heightmap.height;
    let center_x = w as f32 / 2.0;
    let center_y = h as f32 / 2.0;
    let chamber_r = 0.28 * w as f32;
    let chamber_offset = 0.32 * h as f32;
    let neck_hw = 0.04 * w as f32;
    let chamber_r_sq = chamber_r * chamber_r;

    for y in 0..h {
        for x in 0..w {
            let dx = x as f32 - center_x;
            let dy = y as f32 - center_y;

            // Upper chamber
            let dy_upper = dy + chamber_offset;
            let in_upper = dx * dx + dy_upper * dy_upper < chamber_r_sq;

            // Lower chamber
            let dy_lower = dy - chamber_offset;
            let in_lower = dx * dx + dy_lower * dy_lower < chamber_r_sq;

            // Neck
            let in_neck = dx.abs() < neck_hw && dy.abs() < chamber_offset;

            let idx = y * w + x;
            if in_upper {
                heightmap.data[idx] = DEFAULT_SAND_HEIGHT;
            } else if in_neck || in_lower {
                heightmap.data[idx] = 0.02; // near-empty
            } else {
                heightmap.data[idx] = 0.0;  // outside boundary
            }
        }
    }
}
```

---

## 3. Shader Changes

### Hourglass casing outline

Add a new branch in the fragment shader (shader.wgsl ~L169–L201) alongside the existing
Circle/Square/Oval casing logic:

```wgsl
} else if (uniforms.sandbox_shape == 3u) { // Hourglass
    let u = uv.x - 0.5;
    let v = uv.y - 0.5;
    let chamber_r = 0.28;
    let chamber_offset = 0.32;
    let neck_hw = 0.04;

    // Distance from each chamber center
    let d_upper = sqrt(u * u + (v + chamber_offset) * (v + chamber_offset)) / chamber_r;
    let d_lower = sqrt(u * u + (v - chamber_offset) * (v - chamber_offset)) / chamber_r;

    // Neck region
    let in_neck_region = abs(u) < neck_hw && abs(v) < chamber_offset;

    let d_hourglass = min(d_upper, d_lower);
    let near_edge = (d_hourglass >= 0.95 && d_hourglass < 1.0) && !in_neck_region;

    // Neck walls
    let near_neck_wall = abs(v) < chamber_offset
        && abs(abs(u) - neck_hw) < 0.015;

    in_casing = (d_upper >= 1.0 && d_lower >= 1.0 && !in_neck_region)
        || near_neck_wall;
    in_led = near_edge || near_neck_wall;
}
```

### Camera defaults

No shader changes for camera — the camera angle is controlled by `camera_azimuth` and
`camera_elevation` in the WASM state. When switching to Hourglass shape, set:
- `camera_elevation` → 1.2 (more top-down to see both chambers)
- No forced side-view required; the heightmap representation works from above

---

## 4. Hourglass Flip

The signature UX of an hourglass is **flipping it**. In this simulation, a flip is:

1. **Invert `gravity_dir`**: `(0, 1) → (0, -1)` or vice versa
2. **Swap chamber contents**: Mirror the heightmap vertically around the grid center
3. **Swap cell_colors and cell_props**: Same vertical mirror

```rust
pub fn flip_hourglass(&mut self) {
    // 1. Invert gravity
    self.gravity_dir.y = -self.gravity_dir.y;

    // 2. Mirror heightmap vertically
    let w = self.heightmap.width;
    let h = self.heightmap.height;
    for y in 0..h / 2 {
        let y2 = h - 1 - y;
        for x in 0..w {
            let i1 = y * w + x;
            let i2 = y2 * w + x;
            self.heightmap.data.swap(i1, i2);
            self.temp_heights.swap(i1, i2);
            self.wave_vel.swap(i1, i2);
            self.sliding.swap(i1, i2);

            // Swap colors (4 bytes per cell)
            for ch in 0..4 {
                self.cell_colors.swap(i1 * 4 + ch, i2 * 4 + ch);
            }
            // Swap props (4 floats per cell)
            for ch in 0..4 {
                self.cell_props.swap(i1 * 4 + ch, i2 * 4 + ch);
            }
        }
    }

    // 3. Reset block activity to force full re-evaluation
    self.active_blocks.fill(crate::BlockActivity::Inactive);
    self.last_displacements.fill(0.5); // Force all blocks to be re-simulated
    self.tick_count = 0;
}
```

---

## 5. WASM API Changes

| New export | Purpose |
|---|---|
| `set_gravity(x: f32, y: f32)` | Set the gravity direction vector |
| `flip_hourglass()` | Flip the hourglass (invert gravity + mirror state) |
| `get_gravity_x() → f32` | Read back gravity X component |
| `get_gravity_y() → f32` | Read back gravity Y component |

The existing `set_sandbox_shape(u32)` gains a new case:
```rust
3 => SandboxShape::Hourglass,
```

---

## 6. UI Changes

### Shape selector (index.html)

Add "Hourglass" to the shape dropdown. When selected:
- Auto-enable gravity: `set_gravity(0.0, 1.0)` — sand falls down
- Reset the simulation with `initialize_hourglass()` fill pattern (upper chamber full)

### New controls (visible when Hourglass is selected)

| Control | Type | Range | Default | Purpose |
|---|---|---|---|---|
| Gravity Strength | Slider | 0.0–0.10 | 0.04 | Magnitude of gravity pull |
| Flip | Button | — | — | Calls `flip_hourglass()` |
| Neck Width | Slider | 0.02–0.12 | 0.04 | Adjusts neck constriction (requires reset) |

### Gravity controls (visible for all shapes)

Optionally expose gravity as a general control (not just hourglass-specific):
- "Gravity Direction" dropdown: None / Down / Up / Left / Right
- "Gravity Strength" slider: 0.0–0.10

This lets users tilt any sandbox shape and watch sand flow to one side — a fun mode
independent of the hourglass feature.

---

## 7. Code Sharing Summary

| Module | Shared with horizontal mode | Vertical-mode additions |
|---|---|---|
| `grid.rs` (Heightmap) | 100% | None |
| `advect_properties()` | 100% | None |
| `displace_line()` | 100% | None |
| `add_sand_with_limit_properties()` | 100% | None |
| `get_ca_params()` | 100% | None |
| `wave_params()` | 100% | None |
| `settle_tick()` CA loop | ~95% | ~15 lines: gravity bias on slope calculation |
| `settle_tick()` avalanche check | ~95% | ~5 lines: same gravity bias |
| `SandboxShape` enum | Extended | +1 variant (`Hourglass`) |
| `is_inside()` closure | Extended | +1 match arm (~12 lines) |
| `clamp_to_sandbox()` | Extended | +1 match arm (~15 lines) |
| `DrawingSimulation` struct | Extended | +1 field (`gravity_dir: Vec2`) |
| `update()` | Extended | Pass `gravity_dir` through |
| Shader casing outline | Extended | +1 branch (~15 lines) |
| WASM API | Extended | +4 exports |
| UI (HTML/JS) | Extended | +3 controls |

**Estimated new code**: ~120 lines Rust, ~20 lines WGSL, ~50 lines JS/HTML.
**Estimated modified code**: ~30 lines across existing functions.

---

## 8. File Change Summary

| File | Change |
|---|---|
| `sandart-sim/src/lib.rs` | Add `gravity_dir: Vec2` field to `DrawingSimulation`; add `Hourglass` to `SandboxShape`; add `flip_hourglass()` method; add `initialize_hourglass()` helper; extend `clamp_to_sandbox()` with Hourglass arm; thread `gravity_dir` through `update()` |
| `sandart-sim/src/physics.rs` | Add `gravity_dir: Vec2` param to `settle_tick()`; add `NEIGHBOR_DIRS` constant; compute `gravity_push` and `effective_slope` in CA loop and avalanche check; add Hourglass arm to `is_inside()` closure |
| `sandart-render/src/shader.wgsl` | Add `sandbox_shape == 3u` branch for hourglass casing/LED outline |
| `sandart-wasm/src/lib.rs` | Add `gravity_dir` field; add `set_gravity()`, `get_gravity_x()`, `get_gravity_y()`, `flip_hourglass()` exports; extend `set_sandbox_shape()` with case 3; pass `gravity_dir` to `sim.update()` |
| `sandart-wasm/web/index.html` | Add "Hourglass" option to shape dropdown; add Gravity Strength slider, Flip button, Neck Width slider (conditionally visible) |
| `sandart-wasm/web/demo.js` | Wire new controls to WASM exports; auto-enable gravity when Hourglass shape is selected |

---

## 9. Testing

### Unit tests

**`test_gravity_zero_is_noop`**: Run `settle_tick` with `gravity_dir = Vec2::ZERO` on a
known heightmap. Assert results are bit-identical to the current (non-gravity) code path.

**`test_gravity_downward_flow`**: Create a flat heightmap at 0.35 with `gravity_dir = (0, 1)`.
Run 100 settle ticks. Assert that cells near the bottom have gained height and cells near
the top have lost height.

**`test_gravity_conserves_mass`**: Run 200 ticks with gravity enabled. Assert
`sum(heightmap) ≈ initial_sum` within f32 tolerance.

**`test_hourglass_boundary`**: Verify `is_inside()` returns true for points inside both
chambers and the neck, and false for points outside.

**`test_hourglass_flip`**: Fill upper chamber, run `flip_hourglass()`, verify upper chamber
is now empty and lower chamber is full.

**`test_hourglass_neck_flow`**: Set up hourglass with upper chamber full, gravity down.
Run 500 ticks. Assert sand has appeared in the lower chamber and decreased in the upper.

### Manual verification

- Select Hourglass shape. Verify sand fills the upper chamber and slowly pours through
  the neck into the lower chamber.
- Click Flip. Verify the hourglass inverts and sand begins pouring the other direction.
- Draw with the marble inside a chamber. Verify the groove and ridge behave normally.
- Switch back to Circle shape. Verify the simulation behaves identically to before
  (gravity auto-resets to zero).
- Enable gravity on a Circle shape. Verify sand drifts toward the gravity direction
  while the marble still carves normally.

---

## 10. Future Extensions

- **Timer mode**: Count elapsed seconds from flip to "all sand in lower chamber" and
  display as a clock overlay.
- **Adjustable neck shape**: Funnel/cone neck instead of rectangular for more realistic
  flow throttling.
- **Multi-material hourglass**: Upper chamber with CoarseSand, lower with FinePowder —
  watch different flow rates through the neck.
- **Tilt mode**: Map device accelerometer to `gravity_dir` for phone/tablet users.
- **Glass walls**: Render translucent hourglass glass overlay in the shader for visual
  realism.
