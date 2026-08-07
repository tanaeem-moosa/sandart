# #43 — 2.20 — Liquid does not show colour visually

**Status:** completed

---

USER REPORT: liquid doesn't render colour. Consequence beyond cosmetics — the user cannot tell by eye whether liquid drainage behaviour ever worked, so their "it used to be better" report covers SAND ONLY and there is no visual history for water at all.

SCOPE: display-only, as far as is known. The sim-side colour tracer is unaffected — `cell_colors` transport happens in physics.rs (`advect_properties`, mass-weighted blending, conserved to 0.5% by test_color_conservation) and the drain-order diagnostics read sim state directly, not the renderer. So the measurements in 2.19 are valid for liquid despite this bug. Confirm that assumption before relying on it if the cause turns out to be in the sim rather than the shader.

WHERE TO LOOK FIRST: sandart-render/src/shader.wgsl and the material/liquid rendering branch — liquid is likely drawn with a material-derived colour that ignores the per-cell colour buffer, rather than the colour buffer being empty. Check whether cell_colors is even sampled on the liquid path before assuming the data is missing.

WHY IT MATTERS BEYOND LOOKS: the striped-colour test the user devised for sand drainage (see 2.19) cannot be run by eye on water. Fixing this gives them a visual instrument for water behaviour, which is currently only measurable in tests — and water has several open complaints (2.4 towers/thunder, 2.11 tendrils) where visual confirmation has been the limiting factor.
