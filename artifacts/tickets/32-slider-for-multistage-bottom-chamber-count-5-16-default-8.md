# #32 — 2.9 — Slider for MultiStage bottom-chamber count (5..16, default 8)

**Status:** completed

---

SHIPPED in 7387250a (pushed to main, live on Pages).

"Widest tier" slider, 5..16, default 8, shown only for shape 4 (renamed "Cascade, 8 to 1" -> "Merging cascade"). Tier chain derived by physics::multistage_tier_chambers(n) = repeated ceil(n/2) to 1. Five tiers for n>=9, four for 5..8 — accepted, not designed around. tier_h = 2*total_half / tiers.len(); nothing assumes four tiers, including initialize_hourglass's fill threshold (was hard-coded -0.21*h).

n=8 BIT-IDENTICAL, proven by rasterisation: test_multistage_n8_is_bit_identical_to_shipped_geometry diffs a verbatim copy of the pre-feature branch cell-by-cell against live code. I re-verified independently with the 3-cell floor restored on both sides so the neck change could not mask it: 0 mismatches across grid {64,128,256,512} x full 0.005..0.12 neck range x 5 curvatures. Non-vacuity confirmed — perturbing the cap 0.30 -> 0.31 fails at grid 64 within 8 cells.

Real bug found by generalising: at grid 64 with widest tier >= 10, the 3-cell neck floor exceeded half a chamber's width and adjacent necks merged. Fixed with anti_merge_ceiling = (chamber_w/2 - 0.5).max(0.5); confirmed load-bearing by disabling it (fails at w=64 n=10). The bit-identity test doubles as proof it never engages at n=8.

Readout shows "8 - 64.0 cells" (chamber width at current grid). Renderer/shader needed nothing — neck_width/hourglass_curve/sandbox_shape are declared in shader.wgsl Uniforms and never read.

NOT VERIFIED IN A BROWSER. No working driver was available; this rests on Rust tests, node --check, and code tracing.
