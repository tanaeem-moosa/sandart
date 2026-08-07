# #22 — Make Flip invert the container structure, not just the sand

**Status:** completed

---

`DrawingSimulation::flip_hourglass` (sandart-sim/src/lib.rs:502) mirrors only the CONTENTS — heightmap, temp_heights, sliding, cell_colors, cell_props — around center_y, then clears edge momentum and column_depth. `shape_mask` is never touched.

For vertically symmetric shapes (Hourglass, GaltonBoard, MultiNeckHourglass) that is invisible. For the asymmetric ones it is wrong: StaircaseCascade (alternating shelves), MultiStageHourglass/Serpentine (three offset stages), ProceduralFunnel (noise) keep their original orientation while the sand mirrors into it.

Design: do NOT mirror the mask array in place — it is regenerated whenever shape params change (neck width, curvature, shape select), which would silently drop the flip. Add a persistent `flipped: bool` on the sim that the mask generator consults, negating `dy` when set. Flip then toggles the flag, regenerates the mask, and mirrors contents as it does today.

Rendering is free: sandart-render/src/shader.wgsl reads the casing from `shape_mask_texture`, an R8Uint texture uploaded from the CPU via `update_shape_mask` (sandart-render/src/lib.rs:402). Regenerating and re-uploading the mask flips the drawn casing automatically — no shader change needed.

Watch: the existing post-flip cleanup loop zeroes any mass left outside MASK_OUTSIDE. With the mask changing at the same time, ordering matters — regenerate the mask BEFORE that cleanup, or sand ends up culled against the old geometry.
