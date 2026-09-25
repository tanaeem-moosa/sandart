# Working on this repo

**The project was declared finished on 2026-09-24.** The live GitHub Pages build is the release.
This file records the final state and the build/test loop, which is not discoverable from the
code. The earlier, much longer running log of this file is in git history (`git log -p CLAUDE.md`);
`artifacts/HANDOVER.md` and `artifacts/design/SESSION-HANDOVER-*` are historical and describe
subsystems that no longer exist. Read them for why things were tried, never for what exists.

## There is no linker on the host

Anything that compiles must run inside the container:

```
distrobox enter sandart-dev -- bash -lc '<command>'
```

The host has `cargo` but no `cc`, so `cargo build`/`cargo test` fail on the host with
`linker 'cc' not found`. **That error means you are outside the container, not that the work
cannot be verified.** The container has `cargo`, `wasm-pack` and `wasm-opt` but not `git` — edit
and commit on the host, compile and test in the container. Use `CARGO_BUILD_JOBS=2` and run cargo
in the foreground; background cargo gets OOM-killed.

## `cargo check -p sandart-wasm` typechecks nothing

The crate is `#![cfg(target_arch = "wasm32")]`-gated, so a host-target check compiles an empty
crate. Always:

```
cargo check -p sandart-wasm --target wasm32-unknown-unknown --release
```

## Tests

Integration tests do not run in the main test command; run them separately. This is the whole list:

```
cargo test -p sandart-sim --lib --release     # ~65s, the main suite
cargo test -p sandart-sim --release --test fresh_pressure_field_toggle
cargo test -p sandart-sim --release --test head_field_transport_toggle
cargo test -p sandart-sim --release --test perfect_simulation_determinism
cargo test -p sandart-sim --release --test pressure_heatmap_head_field_toggle
cargo test -p sandart-sim --release --test pressure_sensitive_flow_toggle
cargo test -p sandart-render --release
node scripts/check_js.js                      # REQUIRED before any web/ push
```

**Final state (2026-09-24): everything passes.** Lib suite 104 passed / 0 failed; all five
integration targets, the render tests, doctests and `check_js.js` pass. Do not report "tests pass"
without saying which target you ran.

`scripts/check_js.js` also validates `index.html` — `<div>` nesting balance, that
`#viewport-container` is still inside `#app-container`, and that every `getElementById(...)` in
`demo.js` resolves. A cleanup once left one unmatched `</div>` and shipped a blank page while every
Rust test passed. **If you edit `index.html`, run this.**

## Accepted defects are guarded, not failing

Four known defects were reviewed and accepted by the user. Each is pinned by a test that fails
only if the defect grows past 1.25x its measured baseline. **If a change REDUCES one, lower its
baseline. Never raise a baseline to silence a failure.** Each test's header comment has the
numbers and the history.

- `test_neck_pulse_does_not_grow` (`task55_head_spec.rs`) — period-2 mass pulse in the column next
  to the hourglass neck (~6.5 cells/tick at w=64, ~38 at w=512).
- `test_water_blob_stays_left_right_symmetric_under_gravity` — residual lean in draining water
  (the former #56 marker).
- `test_sandbox_wave_stays_left_right_symmetric` — residual solver mirror error (~4.7e-7 peak,
  does not decay).
- `test_liquid_flowing_liquid_does_not_stand_in_walls` — draining liquid clings to walls ~3x
  longer than before the red-black lateral pass (2026-09-08). This is the cost side of a trade:
  the same change cut mid-drain mirror asymmetry 27-36%. Original and pre-red-black numbers are in
  the test comment; see `artifacts/design/ASYMMETRY-2026-09-08.md` §8.

## Invariants that have burned people

- **Shape ids are wire format.** `SandboxShape` has explicit discriminants because the UI sends
  shapes as integers. Id 4 (`MultiStageHourglass`, deleted 2026-09-19) is retired; never reuse it.
- **The vessel shape is the physics mask.** Structure is defined once in `eval_sandbox_shape`; the
  rendered outline is the same function at render resolution. If a test has its own copy of the
  shape math, that is the bug.
- **The mirror axis is `(w-1)/2`, a half-integer for even `w`.** A strict `|dx| < 0.5` admits no
  cell; symmetric necks are necessarily even-width. `test_vessel_masks_are_left_right_symmetric`
  pins it. `ChamberNetwork` is deliberately asymmetric and exempted.
- **Chamber network pipes span at most one column** (`|Δcol| <= 1`); the dogleg pipes that
  wider routings needed were removed. Pipe width has a cell floor (`NET_PIPE_HW_MIN_CELLS`) so
  small grids don't seal pipes. Design record: `artifacts/design/network-2026-09-19/`.
- **Edge-velocity filters are a known trap.** Two filters added to the edge velocity on 2026-08-16
  caused nine test regressions that were labelled "pre-existing" for two weeks. See the TOMBSTONE
  comment in `physics.rs` before touching that expression.
- **LOD blocks are a constant 8 cells** (`DEFAULT_BLOCK_SIZE`); `budget_n` and the adaptive
  throttles derive from block count. `budget_n` does not cap simulated blocks (MUST is exempt).

## Verification is the deployed page

`main` auto-deploys to GitHub Pages via `.github/workflows/deploy.yml`. The wasm build is the only
surface the project is tested against, so nothing is verified until it is pushed and loaded there.
There is no working browser driver on this machine — never claim to have screenshotted or visually
confirmed the app.

## Before proposing a design

Search `artifacts/design/` for prior attempts at the same lever before agreeing to a mechanism.
This project rejected designs that measured *well* (`LATERAL-COARSE-CORRECTION.md` Design 1 scored
+41% spread and was killed on visible seams; sub-cell coverage and the overfill model likewise).
Open ideas that were never built are in `artifacts/tickets/INDEX.md`.
