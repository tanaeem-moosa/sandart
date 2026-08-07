# #51 — 2.28 — Add a material with larger grain size, possibly with individually visible grains

**Status:** pending

---

User request 2026-08-02, explicitly sequenced LAST: "material with even larger grain size, maybe visible one. after the rest."

Current `PROP_GRAIN_SIZE` range runs FinePowder 0.05 .. CoarseSand 0.80. This would add something coarser — gravel or similar.

## PIN DOWN "VISIBLE" BEFORE BUILDING ANYTHING
Do not infer what this means. Two readings with completely different costs:

1. **Rendering only** — a coarser grain TEXTURE, so the surface reads as chunky. The shader already scales with grain size: `grain_scale = mix(3500.0, 300.0, grain_size)` and `grain_strength = mix(0.0, 0.55, grain_size)`. This is a parameter extension and is cheap.
2. **Individually resolvable grains** — a grain occupying one or more cells, discrete rather than continuum. The simulation is a continuum height field, so this is NOT a parameter change; it would need discrete particles or a sub-cell representation, and is a different simulation.

Ask which. See the memory note on visual quality words costing three wrong implementations.

## Known interactions

- **Grain jitter (#46) is already saturated.** `GRAIN_JITTER_SCALE = 1.25` with `GRAIN_JITTER_MAX = 0.95` means any grain_size above ~0.76 clamps. CoarseSand at 0.80 is already at the ceiling, so a coarser material gets NO additional randomised-arbitration effect unless that mapping is revisited. If the new material should feel grainier in motion as well as in texture, `GRAIN_JITTER_SCALE`/`MAX` need re-deriving over the widened range — and note FinePowder already behaves anomalously under that mapping (measured contrast moved the WRONG way, unexplained, recorded in 5aa34448).
- Materials come from `list_materials()`. Do NOT hand-add `<option>` elements to the material select in the web UI.
- Grain size also drives `roughness`, `sparkles_power` and `rim_mult` in the shader — check those still look sane at the extended range rather than only checking the grain texture.
- Repose angle is currently ~5 degrees against real sand's 32-35 (knob: `GRANULAR_TAU_SCALE`, 8.0 gives 18.75). Coarser material should have a HIGHER repose angle than fine sand; if repose is still broken when this is picked up, a gravel that flows like water will look wrong regardless of its texture.

## Sequence
Last. After #45 pressure, #47 slabs, #49 acceleration, #50 scheduler robustness, #44 symmetry.
