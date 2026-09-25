# Ticket index

Exported from the session task store on 2026-08-07, one file per ticket, session's own numbering
(gaps at 1-9, 17, 21 predate the export). The `2.x` prefixes in titles are the user's own scheme
and do not correspond to the ticket numbers.

Re-triaged 2026-09-24, at end of project, against `main` and `git log` rather than each ticket's
own status line (several were stale: shipped features later deleted with the subsystem that housed
them, and one whole design direction — #70's overfill — was built in full and then deleted).
Status:

- **DONE** — shipped and still present in the code (or the finding was itself the resolution, e.g.
  "not a defect").
- **OBSOLETE** — shipped then deleted with the subsystem, or superseded by later work, or won't-do.
- **OPEN** — still a real idea or a real unfixed defect. File kept.

DONE and OBSOLETE ticket files were removed with `git rm` after this triage; their outcome is
recorded here and their text is in git history. OPEN ticket files are kept in this directory.

## Summary

37 DONE, 4 OBSOLETE, 18 OPEN.

## Table

| # | Title | Status | Outcome |
|---|---|---|---|
| 10 | Add mass-distribution quantile lines overlay | DONE | Shipped; `sandart-sim/src/quantiles.rs`, row-sum quantile overlay live. |
| 11 | Add edge sleeping to flux solver and measure perf | DONE | Edge sleeping + dirty-region colour sync shipped (`fe2701a8`); f32 vs u16 colour question resolved by #16's dithering instead. |
| 12 | 2.1 — Visual polish: verify colours against real lit sand | DONE | Palette/casing/panel values shipped (`d07fc3f6`); later UI/shader passes (settings-panel redesign, chamber-network work) superseded the specific numbers this ticket was checking. |
| 13 | Fix water walls: lateral flow suppressed in fed columns | DONE | Root cause (in-transit flux wrongly subtracted from supported-column donation) fixed. |
| 14 | Fix sandbox wave instability from flux rewrite | DONE | Fixed post edge-flux rewrite (bisected to `cce3b571`). |
| 15 | Add depth-integrated lateral pressure for liquid | DONE | Shipped; superseded in detail by later Stage C / Janssen / resolution-invariance work but the capability landed. |
| 16 | Dither f32 colour to u8 for sand grain texture | DONE | Stochastic rounding at the f32→u8 display conversion, shipped. |
| 18 | Fix sandbox waves freezing before reaching the edge | DONE | Two-part fix to edge-sleeping thresholds, cost-neutral. |
| 19 | Stage B: move sand onto the edge-flux solver | DONE | Shipped `73cebd33`; DrySand 2.55x faster, referenced by later Stage C ticket. |
| 20 | 2.2 — Remove the lateral-pressure depth floor | DONE | Shipped (`fe5b4e9e`/`b6246ae`); floor later fully deleted by #28. |
| 22 | Make Flip invert the container structure, not just the sand | DONE | `flipped` flag on the sim, mask regenerated on flip; still relevant (StaircaseCascade/ProceduralFunnel/MultiNeckHourglass are all still live shapes). |
| 23 | Fix Galton board peg lattice so no clear vertical channels form | DONE | Stagger bug and peg size both fixed. |
| 24 | Rebuild multi-stage as an 8-4-2-1 cascade | OBSOLETE | Shipped (MultiNeck + Staircase + Serpentine cascade), but the whole `MultiStageHourglass` shape was deleted 2026-09-19, superseded by `SandboxShape::ChamberNetwork`. |
| 25 | Redesign the settings panel — it has grown messy | DONE | Panel redesigned with the `frontend-design` skill; current `index.html` reflects it. |
| 26 | 2.3 — Stage C: move the rest of sand onto the flux solver | DONE | Shipped as `edda8630` (confirmed via #42's own text, which cites it as landed). Ticket's own status line ("pending") was stale. |
| 27 | 2.4 — Water looks wrong: towers with a wide neck, and splashes violently | OPEN | Symptom B (asymmetric/synchronised tendrils) fixed and user-confirmed via #34. Symptom A (towers with a wide neck) was never explicitly closed out, even though later lateral-pressure/Janssen work (#26/#42/#45) plausibly improved it — worth re-checking against the current deployment before closing. |
| 28 | 2.5 — Delete the dead depth-floor constant and re-sweep LATERAL_PRESSURE_SCALE | DONE | Constant deleted, scale re-baselined against `test_liquid_stream_stays_coherent`. |
| 29 | 2.6 — Show a build version in the UI so a refresh is verifiable | DONE | `SANDART_GIT_SHA` build-time stamp in `sandart-wasm/build.rs`, shown in the panel. |
| 30 | 2.7 — Decile line still pinned: row_mass counts out-of-mask phantom cells | DONE | `refresh_row_mass_*` mask-filtered; out-of-mask cleanup added to `reset()`/`set_sandbox_shape`. |
| 31 | 2.8 — Neck width down to 1 cell at any resolution | DONE | Shipped `7387250a`; neck floor 0.5 (1-cell opening), resolution-aware slider step. |
| 32 | 2.9 — Slider for MultiStage bottom-chamber count (5..16, default 8) | OBSOLETE | Shipped `7387250a` alongside #31, but the whole `MultiStageHourglass` shape (and its slider) was deleted 2026-09-19 with the cascade-to-network replacement. |
| 33 | 2.10 — Design: sideways movement for water and sand under gravity | OPEN | No standalone design doc was ever produced. The question was instead answered piecemeal across many later tickets (Stage C, Janssen overburden, the hydraulic head field), and the one attempt at a unifying answer (#70's overfill model) was built in full and then deleted 2026-08-30 for measuring no benefit. The underlying design question is still not settled. |
| 34 | 2.11 — Tendrils: detector BUILT and green; fix work now unblocked | DONE | Fixed by the frozen-state Jacobi conversion; user-confirmed on the Pages deployment 2026-08-01. |
| 35 | 2.12 — Lateral pressure resolution-invariance | DONE | Fixed via `REFERENCE_GRID_HEIGHT = 512`; production (512x512) is an exact no-op, confirmed bit-identical. |
| 36 | 2.13 — Assess exposing grid resolution as an app option | DONE | Shipped; 64/128/256/512 (and later 1024, #38) selectable in `index.html`. |
| 37 | 2.14 — wave_params relaxation rate is per-tick, so it scales with resolution in TIME | OPEN | Diagnosed as a real, non-tunable defect (sweeping `LATERAL_PRESSURE_SCALE` to 100x does not fix it). No structural fix found in git history. |
| 38 | 2.15 — Add 1024 resolution for high-DPI displays | DONE | Shipped; `index.html` grid `<select>` offers `1024` (currently the shipped default option). |
| 39 | 2.16 — Build a 512-scale voids metric and A/B it across the Jacobi fix | OPEN | Never built; no production-scale (512) enclosed-void metric exists in the test suite today. |
| 40 | 2.17 — Rainbow preset swatch does not look like a rainbow | DONE | Swatch given a genuine multi-hue gradient in place of the generic two-stop split. |
| 41 | 2.18 — Cascade necks drain onto walls at 10 of the 12 chamber counts | OBSOLETE | Fix shipped `3c661eb6` (merge-tree by width balance) and worked while the shape existed, but `MultiStageHourglass`/cascade was deleted 2026-09-19. |
| 42 | 2.19 — Sand drains as a thin central channel fed from the top, not from the sides at depth | DONE | Fixed by Stage C (`edda8630`) + lateral overburden pressure for granular material (`21ca3843`). |
| 43 | 2.20 — Liquid does not show colour visually | DONE | Liquid colour rendering fixed. |
| 44 | 2.21 — Water drains asymmetrically; left drift | DONE | Resolved by the 2026-09-08 mirror-axis fix (`eval_sandbox_shape` centre `w/2` → `(w-1)/2`); `artifacts/design/ASYMMETRY-2026-09-08.md` explicitly names the eliminated "persistent settled lean" as "the historical tendrils usually on the left complaint." |
| 45 | 2.22 — Pressure projection: SHIPPED in ddd9658, awaiting visual verification | DONE | Shipped and visually confirmed by the user (root cause: 45-degree numerical-domain-of-dependence artifact). |
| 46 | 2.23 — Graininess: randomized-weighted property/colour transfer | DONE | Shipped; `GRAIN_JITTER_SCALE`/`edge_share_jitter`/`grain_jitter_strength` present in `physics.rs`. |
| 47 | 2.24 — Slabs: SHIPPED in 23e48e9 | DONE | Support-based MUST-simulate predicate (`support_fraction`), -95.7% cumulative divergence from perfect simulation. |
| 48 | 2.25 — Rename "colour" to "color" throughout the codebase | DONE | No remaining `colour` identifiers; residual `colour` spelling is limited to comments (mostly "red-black colouring", an unrelated graph-colouring term). |
| 49 | 2.26 — Give falling material acceleration when the space below is empty | OPEN | Never implemented as a distinct feature. Worth re-examining against the 2026-08-30 TOMBSTONE finding that the edge-velocity update is already an accumulating integrator (`v = (v_prev + c_sq*yielded) * damping`), which may already provide this. |
| 50 | 2.27 — Make LOD block-dropping degrade quality, not correctness | DONE | Superseded: solved via the `support_fraction` MUST-simulate predicate (`23e48e9`, the same commit that fixed #47) rather than the literal "promote all potentially active blocks to Fast" design this ticket proposed. |
| 51 | 2.28 — Add a material with larger grain size, possibly individually visible | OPEN | Never added; no material coarser than CoarseSand exists (`PROP_GRAIN_SIZE` still tops out there). Ticket itself flags that "visible" needs to be pinned down with the user before starting. |
| 52 | 2.29 — Material forms straight VERTICAL lines: no lateral mixing in the draining funnel | OPEN | Its lead hypothesis (sand can't move sideways at depth) was refuted, and its mirrored-pair measurement technique was reused to close #44 — but the vertical-line report itself was never explicitly closed. |
| 53 | 2.30 — Pressure projection costs +33% to +53% ms/tick | OPEN | Documented, accepted cost at the time; no optimisation pass found in git history since. |
| 54 | 2.31 — Make pressure drive EVERY flow | OPEN | Vertical half ("deep material falls faster") shipped in `bb0633e`. Lateral half ("empty space next to material spreads sideways under pressure") was never delivered. |
| 55 | 2.32 — REWRITE: unified hydraulic-head field for liquid AND solids | DONE | The field (`task55_head_field::advance_head_field`, max-propagation) is shipped and deployed; transport is wired and correct on every spec, default OFF. Its known follow-on defects are tracked as their own tickets below (#62, #64, #66, #67, #68, #69), all still open. |
| 56 | 2.33 — ASYMMETRY HUB | DONE | The macroscopic MultiNeckHourglass pile asymmetry this ticket chased is resolved by the same 2026-09-08 mirror-axis fix as #44. The ticket's own randomness/edge-ownership hypotheses were correctly demoted before that fix landed. |
| 57 | 2.34 — BLOCKS making the fresh pressure field default | OPEN | `fresh_pressure_field` is still `false` by default in `sandart-sim/src/lib.rs` and the toggle was never deleted; the walls-test regression this ticket diagnosed (66/20 at grid 64) still blocks making it default. |
| 58 | 2.35 — "is not supported" primitive: support_fraction | DONE | Shipped `23e48e9`, reused by #47's predicate exactly as this ticket intended. |
| 59 | 2.36 — NOT A DEFECT: hourglass discharge IS fill-height independent | DONE | Confirmed not a defect; original 14x reading was a drained-reservoir measurement-window artifact. |
| 60 | 2.37 — VERTICAL_PRESSURE_CAP_MULT clamps the vertical head at <1 cell of depth | OPEN | Still `const VERTICAL_PRESSURE_CAP_MULT: f32 = 1.0;` in `physics.rs` — #54's "deep material falls faster" is still capped inert beyond roughly one cell of depth, exactly as this ticket found. |
| 61 | 2.38 — Add a U-shaped flow-through vessel | DONE | Shipped `53516eb`; `SandboxShape::UTubeFlowThrough` (id 9) still present. |
| 62 | 2.39 — Warm-start the head field with a decay term | OPEN | Explicitly gated on a motivating measurement ("do not start without one") that was never taken; not started. |
| 63 | 2.40 — Make material flow rate sensitive to pressure | OPEN | Base feature shipped behind a toggle (`73adbf6`), but the user's nonlinear 10-vs-20-depth requirement is blocked by #67, which is also still open. |
| 64 | 2.41 — Surface levelling does not complete at w=512 | OPEN | No fix found in git history since the 2026-08-07 measurement; head-field transport is still recorded as slower than legacy at w=512. |
| 65 | 2.42 — MISATTRIBUTED, closed | DONE | Closed correctly as misattribution (missing change-listener, not the block heat-map); the real defect was re-filed as #68. |
| 66 | 2.43 — Advancing the head field costs +219% ms/tick at w=512 | OPEN | Allocation hypothesis (`advance_head_field` allocating six whole-grid Vecs/tick) was never confirmed or fixed in git history. |
| 67 | 2.44 — The head field pins an entire DRAINING column to zero pressure | OPEN | No fix found; transitive-support-as-free-fall bug over orifices appears unaddressed. |
| 68 | 2.45 — Head-field liquid transport BREAKS the pressure field | OPEN | No fix found; this is what blocks #55's transport path from ever being enabled by default. |
| 69 | 2.46 — Pressure-sensitive flow badly slows a FED falling stream | OPEN | No fix found; the free-fall exemption still only covers compact unsupported slabs, not a fed stream landing on standing material. |
| 70 | 2.47 — DESIGN DIRECTION: replace the equilibrium head field with per-cell OVERFILL | OBSOLETE | Implemented in full (`c844d68` through the multiplicative/pressure-domain/stiffness-dial iterations) and then entirely deleted 2026-08-30 — "its own instruments recorded no benefit" (`CLAUDE.md`, `artifacts/design/SESSION-HANDOVER-2026-08-30.md`). Superseded, not pursued further. |

## Open tickets (18), kept as files

#27, #33, #37, #39, #49, #51, #52, #53, #54, #57, #60, #62, #63, #64, #66, #67, #68, #69.

All of #62–#69 (bar #65, closed) are defects/extensions on the hydraulic-head-field subsystem
(#55), which is shipped but default-off and, as far as this triage could establish, has not been
touched since 2026-08-07 — `VERTICAL_PRESSURE_CAP_MULT` and `JANSSEN_DEPTH_SCALE` are still at
their original values. Anyone picking this area back up should read #55 through #69 as one
connected story, in ticket order.
