# #12 — 2.1 — Visual polish: verify colours against real lit sand

**Status:** completed

---

NEEDS HUMAN EYES. Cannot be settled from a static render — that is exactly the gap this task exists to close.

## Already done (commit d07fc3f6)
- Quantile lines: saturated cyan @ 0.85 alpha -> recycled-glass green `vec3(0.561, 0.722, 0.631)` @ 0.55 alpha. In `sandart-render/src/shader.wgsl`, end of `fs_main`, inside `if (uniforms.quantile_count > 0u)`.
- Outer casing: `vec3(0.07, 0.07, 0.08)` -> `vec3(0.125, 0.13, 0.145)`, shader.wgsl ~line 213 (the `else` branch after the LED ring cases). Addresses "the background is too dark".
- Viewport gradient lifted to match: `radial-gradient(circle at 50% 42%, #232833, #12151b 68%, #0d1015)` in sandart-wasm/web/index.html, `#viewport-container`.
- Panel accent unified to the same green (`--accent: #8fb8a1`) so chrome and overlay agree.

## What still needs checking, and why it could not be checked here
Every one of those values was chosen against a headless-Chrome render with an EMPTY canvas — no sand, no lighting. They have never been seen against the actual thing they sit next to.

1. Does the lifted casing read right against lit sand, in each of the five LED modes? `led_mode` 0=Studio, 1=LED ring, 2=Ambient glow, 3=Moonlight, 4=Night. Modes 3 and 4 dim everything (`m_brightness` multipliers 0.22 and 0.05 in shader.wgsl ~line 169), so a casing tuned for Studio may glare in Night or vanish in Moonlight.
2. Are the quantile lines legible over DESERT-palette sand specifically? That is the default preset (`#ebd9bb` / `#8b5a2b`) and the closest in hue to the line green — the worst case. If they disappear, RAISE ALPHA FIRST; only change hue if alpha alone cannot do it, since the green is now shared with the panel accent and the two should stay in agreement.
3. User said the default lines were "too intense". Alpha dropped 0.85 -> 0.55, but the default line COUNT was never revisited — `#quantile-select` in index.html offers Off / Quartiles / Deciles. Deciles may simply be too many lines regardless of alpha.

## How to check
Load the deploy (GitHub Actions publishes to gh-pages on push to main), switch to Sand-fall, set Weight lines to Quartiles, and cycle LED modes with sand actually falling.
