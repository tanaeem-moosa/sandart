# #25 — Redesign the settings panel — it has grown messy

**Status:** completed

---

User: "have the skill evaluate our settings panel. it is getting messy." Use the `frontend-design` skill.

Scope: sandart-wasm/web/index.html (and demo.js for any wiring changes). The panel has accreted controls over many sessions — shape select with optgroups that show/hide by mode (`#shape-group-sandfall`, `#sandfall-controls`), material select, neck width / chamber curvature sliders, LED mode, quantile overlay select, flip button, and more.

Fold in #12 (visual polish) where it overlaps — that task covers: quantile lines should be recycled-glass green (~#7FA98B-#9CBFA6, currently drawn in shader.wgsl at the end of fs_main), the background/casing is too dark (`vec3(0.07, 0.07, 0.08)` in shader.wgsl), and general prettiness.

Constraint: this is a WASM canvas app; the panel is plain HTML/CSS/JS with no framework and no build step for the web layer. Keep it dependency-free.
