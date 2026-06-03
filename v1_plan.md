# Sand Art Simulator: Version 1 Plan

This document captures the research, proposed features, and implementation roadmap for v1 of the Sand Art Simulator. It is intended to serve as a reference when returning to the project.

---

## Goal 1: Interactive 3D Perspective Camera (Orbit / Zoom)

**Feasibility: Medium complexity.**

Replace the flat fullscreen quad with a true 3D vertex-displaced mesh. The vertex shader samples the heightmap and pushes each vertex up by its height value. A user-controlled camera (orbit azimuth + elevation via mouse drag, zoom via scroll wheel) rotates around the table with a perspective projection.

**Key implementation steps:**
- `src/renderer.rs`: Create a static `N×N` vertex grid buffer and index buffer (triangle strip). Vertices store only `(u, v)` — position is derived from UV and displaced height.
- `src/shader.wgsl`: Add a vertex shader stage that reads the heightmap and outputs `ViewProj × (x, y, h × amplitude)`.
- `src/app.rs`: Track `(azimuth, elevation, zoom)` camera state; compute `glam::Mat4::look_at_lh` + `perspective_lh`; upload as a new uniform.
- Enable WGPU depth stencil buffer for correct self-occlusion at oblique angles.

---

## Goal 2: Hyper-Realistic Sand Shading (Journey-Style)

**Feasibility: Low-Medium complexity.**

Inspired by *thatgamecompany*'s *Journey* and Alan Zucconi's shader series:

- **Microfacet Sparkle**: Half-vector `H = normalize(L + V)`. Perturb surface normals with high-frequency hash noise and threshold the specular: `glint = smoothstep(0.96, 1.0, pow(dot(N_perturbed, H), 128))`. Sparkles blink naturally as the camera or light rotates.
- **Half-Lambert Diffuse**: `diffuse = pow(dot(N, L) * 0.5 + 0.5, 2.0)` — softens shadows, prevents flat sand from looking like stone.
- **Fresnel Rim Light**: `rim = pow(1 - dot(N, V), 3) * max(dot(N, L), 0)` — warm scattered backlight along ridge silhouettes.
- **Mipmap Sharpening**: Lock normal-map sampling to mip level 0 to preserve crisp grain detail at distance.

---

## Goal 3: Material Physics Presets

**Feasibility: Medium complexity.**

The current settling algorithm acts like a viscous fluid ("butter-cream"). We will add a **Material** dropdown in the UI that swaps physics constants and the settling mode. All materials share the same heightmap so switching mid-session is valid.

### Material Presets

| Material | Flow Model | Static Repose | Feel |
|---|---|---|---|
| **Butter-Cream** *(current)* | Continuous float CA, linear alpha | Low (`~0.04`) | Smooth, viscous, rolling waves |
| **Dry Sand** | Shear-hysteresis + quantized grain packets | Medium (`~0.08`) | Crisp ridges, sharp slip-faces, crumbly avalanches |
| **Snow** | High cohesion + compaction + slow creep | High (`~0.15`) | Deep vertical-walled trenches, packed flat tracks |
| **Kinetic Sand** | Capillary-bridge cohesion | Medium-High (`~0.12`) | Clumps and holds walls cleanly, crisp cuts |
| **Wet Sand** | Moderate cohesion, slow seep | Medium (`~0.10`) | Wider shallower trenches, holds shape but sags slowly |
| **Fine Powder** | Near-zero repose, ultra-low friction | Very Low (`~0.01`) | Almost liquid, patterns flow and merge immediately |
| **Oobleck** | Non-Newtonian (velocity-dependent) | Fast=high, Slow=low | Stiff under fast marble, flows when marble stops |
| **Moon Dust** | Very low flow rate, steep stable angles | Very High (`~0.20`) | Long-lived sharp craters, minimal settling |
| **Iron Filings** | Magnetically biased ridge deposition | Medium | Ridges form radially outward from marble path |

### Key Physics Techniques (Dry Sand and above)
- **Shear Hysteresis**: Cell transitions to "sliding" state only above a static threshold; locks back below a lower kinetic threshold — prevents continuous micro-creep.
- **Quantized Height Transfers**: Heights move in discrete grain-sized packets instead of continuous fractions — creates terraces and natural grain boundaries.
- **Avalanche Momentum**: Carry a fraction of sliding velocity into neighboring cells so collapses sweep naturally into valleys.
- **Friction Jamming**: Cells with 3+ static neighbors resist flow — models force-chain lockup, allows overhangs near marble path.

---

## Goal 4: Expanded Pattern Generators

**Feasibility: Low complexity.**

Add a richer set of parametric path generators alongside the existing Archimedean spiral:

| Pattern | Description |
|---|---|
| **Lissajous** | `x = sin(a·t + δ)`, `y = sin(b·t)` — classic crossing wave figures |
| **Rose Curve** | `r = cos(k·θ)` in polar — k-petalled flower patterns |
| **Hypotrochoid / Spirograph** | Inner circle rolling inside outer — Spirograph-style loops |
| **Fermat Spiral** | `r = √θ` — evenly spaced arms, sunflower-like |
| **Hilbert Curve** | Recursive space-filling curve — covers the whole bed uniformly |
| **Random Walk / Brownian** | Semi-random drift with configurable bias and step size — organic abstract art |
| **Lemniscate** | Figure-8 / infinity curve `r² = cos(2θ)` |
| **Multi-marble** | Two virtual marbles tracing different patterns simultaneously on the same bed |

All generators should be added to the existing `PatternMode` enum and `PlaybackController` pipeline so they benefit from the existing speed, looping, and pause controls.

---

## Goal 5: GPU Compute Settling

**Feasibility: Medium-High complexity.**

Move the cellular-automata settling loop from CPU (Rust) to a WGPU compute shader. This would:
- Unlock running at full GPU speed with zero CPU-bound stalls.
- Enable much higher-resolution grids (e.g. `2048×2048`) without frame-rate impact.
- Allow more complex per-cell physics (non-Newtonian viscosity, momentum fields) that would be too slow on CPU.

**Key consideration**: The current double-buffered `temp_heights` pattern maps cleanly to a WGPU compute ping-pong between two `StorageTexture` bindings.

---

## Proposed Default Tuning (Apply in v1)

These defaults better match real kinetic sand table behavior and look better out of the box:

| Setting | Current Default | Proposed Default | Range |
|---|---|---|---|
| `marble_size` (radius) | `0.025` | **`0.018`** | `0.006` → `0.054` (⅓× to 3× default) |
| `light_brightness` | `0.8` | **`1.3`** | `0.0` → `3.0` |
| `speed` | `0.15` | **`0.30`** | `0.01` → `2.0` |

---

## v1 Implementation Roadmap

```
Block 1: Default Tuning & Pattern Generators  (quick wins)
  - Update config.rs defaults.
  - Add Lissajous, Rose, Hypotrochoid, Random Walk generators to pattern.rs.

Block 2: Material Physics Presets
  - Add MaterialMode enum to config.rs.
  - Implement shear-hysteresis and quantized settling variants in physics.rs.
  - Add Material dropdown to app.rs UI.

Block 3: 3D Mesh Scaffolding & Camera
  - Build vertex grid buffer and index buffer in renderer.rs.
  - Add vertex shader displacement pass in shader.wgsl.
  - Add orbit camera controller to app.rs.

Block 4: Journey-Style Shading Upgrade
  - Microfacet sparkle, Half-Lambert, Fresnel rim in shader.wgsl.
  - Per-material base color and reflectance uniforms.

Block 5: GPU Compute Settling (stretch goal)
  - Port settle_tick to WGPU compute shader.
  - Ping-pong StorageTexture double-buffer approach.
```
