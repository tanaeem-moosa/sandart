# Ticket index

Exported from the session task store on 2026-08-07. One file per ticket, named `NN-slug.md`.
Numbering is the session's own; the `2.x` prefixes in titles are the user's own scheme.

Sorted: in-progress first, then open, then closed.

## In progress

- [#55 — 2.32 — REWRITE: unified hydraulic-head field for liquid AND solids. Shipped multigrid pass is visually refuted (see scratchpad/TASK55-BRIEF.md)](55-rewrite-unified-hydraulic-head-field-for-liquid-and-solids-s.md)
- [#63 — 2.40 — Make material flow rate sensitive to pressure: slow down at LOW pressure rather than speed up at high](63-make-material-flow-rate-sensitive-to-pressure-slow-down-at-l.md)

## Open

- [#26 — 2.3 — Stage C: move the rest of sand onto the flux solver](26-stage-c-move-the-rest-of-sand-onto-the-flux-solver.md)
- [#27 — 2.4 — Water looks wrong: towers with a wide neck, and splashes violently](27-water-looks-wrong-towers-with-a-wide-neck-and-splashes-viole.md)
- [#33 — 2.10 — Design: sideways movement for water and sand under gravity](33-design-sideways-movement-for-water-and-sand-under-gravity.md)
- [#37 — 2.14 — wave_params relaxation rate is per-tick, so it scales with resolution in TIME](37-wave-params-relaxation-rate-is-per-tick-so-it-scales-with-re.md)
- [#38 — 2.15 — Add 1024 resolution for high-DPI displays](38-add-1024-resolution-for-high-dpi-displays.md)
- [#39 — 2.16 — Build a 512-scale voids metric and A/B it across the Jacobi fix](39-build-a-512-scale-voids-metric-and-a-b-it-across-the-jacobi.md)
- [#44 — 2.21 — Water drains asymmetrically; left drift. CAUTION: pressure MASKS this, do not mistake damping for a fix](44-water-drains-asymmetrically-left-drift-caution-pressure-mask.md)
- [#49 — 2.26 — Give falling material acceleration when the space below is empty](49-give-falling-material-acceleration-when-the-space-below-is-e.md)
- [#50 — 2.27 — Make LOD block-dropping degrade quality, not correctness; then reduce how often it drops](50-make-lod-block-dropping-degrade-quality-not-correctness-then.md)
- [#51 — 2.28 — Add a material with larger grain size, possibly with individually visible grains](51-add-a-material-with-larger-grain-size-possibly-with-individu.md)
- [#52 — 2.29 — Material forms straight VERTICAL lines: no lateral mixing in the draining funnel](52-material-forms-straight-vertical-lines-no-lateral-mixing-in.md)
- [#53 — 2.30 — Pressure projection costs +33% to +53% ms/tick; cost is fixed per-phase, not the Jacobi loop](53-pressure-projection-costs-33-to-53-ms-tick-cost-is-fixed-per.md)
- [#54 — 2.31 — Make pressure drive EVERY flow: deep material falls faster, and material next to empty space spreads into it](54-make-pressure-drive-every-flow-deep-material-falls-faster-an.md)
- [#56 — 2.33 — ASYMMETRY HUB. Randomness is probably NOT the cause (math recorded). Salt experiment first, then edge OWNERSHIP and arbitration, which never alternate](56-asymmetry-hub-randomness-is-probably-not-the-cause-math-reco.md)
- [#57 — 2.34 — BLOCKS making the fresh pressure field default: arches do not COLLAPSE fast enough. It makes an existing failure worse, not a new one](57-blocks-making-the-fresh-pressure-field-default-arches-do-not.md)
- [#60 — 2.37 — VERTICAL_PRESSURE_CAP_MULT clamps the vertical head at &lt;1 cell of depth, so #54's "deep material falls faster" is inert](60-vertical-pressure-cap-mult-clamps-the-vertical-head-at-lt-1.md)
- [#62 — 2.39 — Warm-start the head field with a decay term so switchback geometry does not re-derive from cold every tick](62-warm-start-the-head-field-with-a-decay-term-so-switchback-ge.md)
- [#64 — 2.41 — Surface levelling does not complete at w=512, and head-field transport levels SLOWER than legacy there](64-surface-levelling-does-not-complete-at-w-512-and-head-field.md)
- [#66 — 2.43 — Advancing the head field costs +219% ms/tick at w=512; it allocates six whole-grid Vecs every tick](66-advancing-the-head-field-costs-219-ms-tick-at-w-512-it-alloc.md)
- [#67 — 2.44 — The head field pins an entire DRAINING column to zero pressure: transitive support treats extrusion as free fall](67-the-head-field-pins-an-entire-draining-column-to-zero-pressu.md)
- [#68 — 2.45 — Head-field liquid transport BREAKS the pressure field: horizontal stripes in the U-tube. This is what #65 misattributed to the block heat map](68-head-field-liquid-transport-breaks-the-pressure-field-horizo.md)
- [#69 — 2.46 — Pressure-sensitive flow badly slows a FED falling stream and makes it spread sideways: the free-fall exemption only covers compact unsupported slabs](69-pressure-sensitive-flow-badly-slows-a-fed-falling-stream-and.md)
- [#70 — 2.47 — DESIGN DIRECTION: replace the equilibrium head field with per-cell OVERFILL. Unifies liquid and granular, because overfill is a STRESS and head is only an elevation](70-design-direction-replace-the-equilibrium-head-field-with-per.md)

## Closed

- [#10 — Add mass-distribution quantile lines overlay](10-add-mass-distribution-quantile-lines-overlay.md)
- [#11 — Add edge sleeping to flux solver and measure perf](11-add-edge-sleeping-to-flux-solver-and-measure-perf.md)
- [#12 — 2.1 — Visual polish: verify colours against real lit sand](12-visual-polish-verify-colours-against-real-lit-sand.md)
- [#13 — Fix water walls: lateral flow suppressed in fed columns](13-fix-water-walls-lateral-flow-suppressed-in-fed-columns.md)
- [#14 — Fix sandbox wave instability from flux rewrite](14-fix-sandbox-wave-instability-from-flux-rewrite.md)
- [#15 — Add depth-integrated lateral pressure for liquid](15-add-depth-integrated-lateral-pressure-for-liquid.md)
- [#16 — Dither f32 colour to u8 for sand grain texture](16-dither-f32-colour-to-u8-for-sand-grain-texture.md)
- [#18 — Fix sandbox waves freezing before reaching the edge](18-fix-sandbox-waves-freezing-before-reaching-the-edge.md)
- [#19 — Stage B: move sand onto the edge-flux solver](19-stage-b-move-sand-onto-the-edge-flux-solver.md)
- [#20 — 2.2 — Remove the lateral-pressure depth floor](20-remove-the-lateral-pressure-depth-floor.md)
- [#22 — Make Flip invert the container structure, not just the sand](22-make-flip-invert-the-container-structure-not-just-the-sand.md)
- [#23 — Fix Galton board peg lattice so no clear vertical channels form](23-fix-galton-board-peg-lattice-so-no-clear-vertical-channels-f.md)
- [#24 — Rebuild multi-stage as an 8-4-2-1 cascade](24-rebuild-multi-stage-as-an-8-4-2-1-cascade.md)
- [#25 — Redesign the settings panel — it has grown messy](25-redesign-the-settings-panel-it-has-grown-messy.md)
- [#28 — 2.5 — Delete the dead depth-floor constant and re-sweep LATERAL_PRESSURE_SCALE](28-delete-the-dead-depth-floor-constant-and-re-sweep-lateral-pr.md)
- [#29 — 2.6 — Show a build version in the UI so a refresh is verifiable](29-show-a-build-version-in-the-ui-so-a-refresh-is-verifiable.md)
- [#30 — 2.7 — Decile line still pinned: row_mass counts out-of-mask phantom cells](30-decile-line-still-pinned-row-mass-counts-out-of-mask-phantom.md)
- [#31 — 2.8 — Neck width down to 1 cell at any resolution (colour half DONE)](31-neck-width-down-to-1-cell-at-any-resolution-colour-half-done.md)
- [#32 — 2.9 — Slider for MultiStage bottom-chamber count (5..16, default 8)](32-slider-for-multistage-bottom-chamber-count-5-16-default-8.md)
- [#34 — 2.11 — Tendrils: detector BUILT and green; fix work now unblocked](34-tendrils-detector-built-and-green-fix-work-now-unblocked.md)
- [#35 — 2.12 — Lateral pressure resolution-invariance (FIXED, pending push)](35-lateral-pressure-resolution-invariance-fixed-pending-push.md)
- [#36 — 2.13 — Assess exposing grid resolution as an app option](36-assess-exposing-grid-resolution-as-an-app-option.md)
- [#40 — 2.17 — Rainbow preset swatch does not look like a rainbow (picker half DONE)](40-rainbow-preset-swatch-does-not-look-like-a-rainbow-picker-ha.md)
- [#41 — 2.18 — Cascade necks drain onto walls at 10 of the 12 chamber counts](41-cascade-necks-drain-onto-walls-at-10-of-the-12-chamber-count.md)
- [#42 — 2.19 — Sand drains as a thin central channel fed from the top, not from the sides at depth](42-sand-drains-as-a-thin-central-channel-fed-from-the-top-not-f.md)
- [#43 — 2.20 — Liquid does not show colour visually](43-liquid-does-not-show-colour-visually.md)
- [#45 — 2.22 — Pressure projection: SHIPPED in ddd9658, awaiting visual verification](45-pressure-projection-shipped-in-ddd9658-awaiting-visual-verif.md)
- [#46 — 2.23 — Graininess: randomized-weighted property/colour transfer (NOT surface shape)](46-graininess-randomized-weighted-property-colour-transfer-not.md)
- [#47 — 2.24 — Slabs: SHIPPED in 23e48e9 (support-predicate, -95.7% divergence)](47-slabs-shipped-in-23e48e9-support-predicate-95-7-divergence.md)
- [#48 — 2.25 — Rename "colour" to "color" throughout the codebase](48-rename-colour-to-color-throughout-the-codebase.md)
- [#58 — 2.35 — "is not supported" primitive: support_fraction SHIPPED in 23e48e9; pressure reuse still open](58-is-not-supported-primitive-support-fraction-shipped-in-23e48.md)
- [#59 — 2.36 — NOT A DEFECT: hourglass discharge IS fill-height independent (1.01x); original 14x was a drained-reservoir measurement window](59-not-a-defect-hourglass-discharge-is-fill-height-independent.md)
- [#61 — 2.38 — Add a U-shaped flow-through vessel: inlet one side, drain the other](61-add-a-u-shaped-flow-through-vessel-inlet-one-side-drain-the.md)
- [#65 — 2.42 — MISATTRIBUTED, closed: the block heat map was not the cause. User re-attributed it to head-field liquid transport (see #68)](65-misattributed-closed-the-block-heat-map-was-not-the-cause-us.md)

