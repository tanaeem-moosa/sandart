# Project Handover: Sandart Simulator Optimizations & Fixes

This document describes the findings from the Senior Software Architect & Critic subagent review, the optimizations implemented, and outstanding tasks for the next steps.

---

## 1. Critic Subagent Findings & Code Review

The critic subagent reviewed the codebase focusing on performance and correctness across three main areas:

### A. CPU Physics Simulation (`src/sim/physics.rs` and `src/sim.rs`)
- **Heap Allocations in Hot Path**: Identified that `add_sand_with_limit` allocated dynamic `Vec`s (`neighbors` and `room_neighbors`) on the heap. Since this function is executed thousands of times per frame inside the pixel-carving sweeping loop, it caused significant allocation overhead and memory fragmentation.
- **Redundant $O(N^2)$ CA Computations**: Noted that the cellular automata settling loop (`settle_tick`) redundantly calculated closest active marble distances, ripple cosine factors, repose thresholds, and normalization vectors for every neighbor of every cell.
- **Viscosity in Multi-Marble Mode**: Noted that Oobleck non-Newtonian flow behavior only evaluated the primary marble's velocity, ignoring other active marbles.

### B. GPU Fragment Shader (`src/shader.wgsl`)
- **Redundant Bilinear Height Fetches**: Evaluated that computing surface normals via finite differences performed 5 bilinear height lookups (20 texture fetches) per pixel.
- **Sparkles Shimmering**: Pointed out that high-frequency sparkles evaluated using `hash(uv * 4000.0)` created aliasing/TV static shimmering when the camera moved.

### C. Pattern Playback & Snap Logic (`src/pattern.rs` and `src/app.rs`)
- **Playback Speed Capped on Dense Paths**: Advancing at most one waypoint per frame capped the marble's actual speed by the waypoint density and frame rate.
- **G-code Parsing Axis Dropping**: Discovered that if X and Y values are split across lines, the interpreter dropped coordinate points until both flags were set.
- **Empty File Crash**: Discovered that loading empty custom patterns led to division/modulo-by-zero panics.

---

## 2. Implemented Changes

We have refactored and implemented the following solutions:

1. **Stack Allocation in Carving Loop**: Replaced `Vec` allocations in `add_sand_with_limit` with stack-allocated arrays `[usize; 4]` and `[(usize, f32); 4]`, completely removing heap allocations.
2. **CA Inner Loop Invariants Extracted**: Refactored `settle_tick` to pre-calculate closest marbles, ripples, thresholds, and pull vectors once per cell, using static direction-indexed dot products instead of normalization in the inner loop.
3. **Multi-Marble Oobleck Physics**: Introduced `ActiveMarbleInfo` carrying position and velocity for all active marbles to compute non-Newtonian flow thresholds correctly.
4. **Analytical Bilinear Derivatives**: Replaced finite differences in the fragment shader with exact mathematical partial derivatives and a single 2x2 texture fetch block, saving 16 texture fetches per pixel.
5. **G-code & Empty File Safety**: Initialized G-code coordinate flags to `true` to ensure single-axis movement is immediately recorded, and added guards against empty custom pattern files to prevent crashes.
6. **Sparkle Grid-Locking**: Adjusted sparkles to use `hash(floor(uv * 4000.0))` to lock glints to discrete sand coordinates, eliminating shimmering.
7. **Multi-Waypoint Playback**: Implemented a `while` loop in `step_playback_all` to consume multiple waypoints per frame based on remaining time-step movement.
8. **Volume Conservation Fixes**: Resolved edge-case sand loss bugs in `add_sand_with_limit` (proper neighbor capacity loop fallback) and `displace_line` (correctly restoring height map on saturation). Verified via continuous single-marble (200 steps) and multi-marble/large-radius (150 steps) spiral simulations.
9. **Premium Material Additions**: Added the requested `Yogurt` (viscous creamy liquid with sluggish wave ripples) and `Coarse Sand` (large quartz grains, high-contrast normal maps, and sparkling glints) material presets.

---

## 3. Current Workspace Status

- **v2 Completion**: The v2 modular refactoring milestones have been 100% completed, verified, and committed.
- **Workspace Architecture**:
  - `sandart`: Binary desktop runner (Egui/Eframe frontend).
  - `sandart-sim`: Standalone simulation library (CA grid, physics presets).
  - `sandart-render`: Standalone WGPU renderer (heightmap rendering, shaders).
  - `sandart-pattern`: Mathematical pattern generators and text parsers.
  - `sandart-wasm`: WASM bindgen bindings for web targets.
- **Build & Tests**: The workspace compiles and runs successfully inside the `sandart-dev` Distrobox container, and all unit tests pass cleanly.
- **WASM Support**: Web bindings can be compiled via `wasm-pack build sandart-wasm --target web`. A self-contained demo page is provided at `sandart-wasm/web/index.html` and `sandart-wasm/web/demo.js` implementing a fully interactive, zero-copy, responsive browser-based sand art simulator.
