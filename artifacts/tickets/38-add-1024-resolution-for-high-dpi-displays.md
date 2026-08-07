# #38 — 2.15 — Add 1024 resolution for high-DPI displays

**Status:** pending

---

User request 2026-07-31, after testing the new resolution switch: "I think 512 is the visually stunning resolution but maybe we should add 1024 for high dpi device."

Add 1024 as a fifth option alongside 64/128/256/512. Default stays 512.

The plumbing is already generic — `DrawingSimulation::new_with_size`, `HeightmapRenderer::new(grid_size)`, and the `grid_size` shader uniform all take a runtime value, so this should be close to adding one `<option>` plus whatever validation guards the list. Verify rather than assume.

## Check before shipping
- **Perf.** Cost scales roughly with cell count, so 1024 is ~4x the work of 512. Measured at 512: Water 1.470 ms/tick, DrySand 5.080 ms/tick at budget 256. DrySand at 1024 could land near 20 ms/tick, which would not hold 60fps. Measure both materials and report. If it cannot hold frame rate, that is worth knowing before it ships rather than after — but per the user's standing position on the neck width ("no point not allowing me to pick"), a slow option is still probably worth offering, clearly labelled.
- **GPU texture limits.** `device.limits().max_texture_dimension_2d` is already consulted for the surface; confirm 1024x1024 R8Uint (shape mask), the heightmap texture and the colormap texture are all within limits on the WebGL2 fallback path as well as WebGPU. The downlevel WebGL2 defaults are the binding constraint.
- **Memory.** Every per-cell buffer quadruples versus 512: heightmap, cell_colors (4 bytes/cell), cell_props (4 f32/cell), external_mass_this_tick, column_depth, edge_vel_h/v, sliding, shape_mask, last_displacements. Report the total allocation at 1024.
- **Block-LOD.** `block_size` is `(grid_size / 32).max(1)`, giving 32 at 1024 and keeping the 1024-block count invariant. Confirm that still behaves, since budget constants are absolute block counts tuned around that invariant.

## Note
`REFERENCE_GRID_HEIGHT = 512` means `depth_scale = 0.5` at 1024, so lateral pressure will be half what it is per-cell at 512 — which is correct and is the whole point of the resolution-invariance fix. Confirm the simulation looks equivalent at 1024, since that is the first resolution ABOVE the reference and the fix has only ever been exercised below it.
