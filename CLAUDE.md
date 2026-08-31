# Working on this repo

This file exists because the build/test loop is not discoverable from the code, and getting it
wrong produces confident, wrong conclusions. It was written after a session concluded "the tests
cannot be run here" and committed that claim — the loop was documented in `README.md` and
`artifacts/HANDOVER.md` §1 the whole time.

**This file is the authority.** `artifacts/HANDOVER.md` was written on 2026-08-17 and is now a
HISTORICAL document: most of what it describes as live — the overfill model, the hierarchical
coarse level, the block-clock scheduler and their overlays — was deleted on 2026-08-30. Its build
and test instructions are still correct; its account of what the code does is not. Read it for why
things were tried, never for what exists.

## There is no linker on the host

Anything that compiles must run inside the container:

```
distrobox enter sandart-dev -- bash -lc '<command>'
```

The host has `cargo` but no `cc`, `gcc` or `libgcc`, so `cargo build` and `cargo test` fail on the
host with `linker 'cc' not found`. **That error means you are outside the container, not that the
work cannot be verified.**

The container has `cargo`, `wasm-pack` and `wasm-opt`. It does **not** have `git` or `jj`. So the
loop is: **edit and commit on the host, compile and test in the container.**

## `cargo check -p sandart-wasm` typechecks nothing

The crate is `#![cfg(target_arch = "wasm32")]`-gated, so a host-target check compiles an empty
crate and passes no matter what you broke. Always:

```
cargo check -p sandart-wasm --target wasm32-unknown-unknown --release
```

## Tests

Integration tests do **not** run in the main test command; run them separately. There are five,
and this is the whole list (`HANDOVER.md` §2's list is stale — it names five more that were deleted
with the subsystems they tested):

```
cargo test -p sandart-sim --lib --release     # ~35s, the main suite
cargo test -p sandart-sim --release --test fresh_pressure_field_toggle
cargo test -p sandart-sim --release --test head_field_transport_toggle
cargo test -p sandart-sim --release --test perfect_simulation_determinism
cargo test -p sandart-sim --release --test pressure_heatmap_head_field_toggle
cargo test -p sandart-sim --release --test pressure_sensitive_flow_toggle
node scripts/check_js.js                      # REQUIRED before any web/ push -- see below
```

`scripts/check_js.js` is not only a JS syntax check. It also validates `index.html`: `<div>`
nesting balance, that `#viewport-container` is still inside `#app-container`, and that every
`getElementById(...)` in `demo.js` resolves to an id that exists. Those HTML checks were added on
2026-08-31 after a cleanup left one unmatched `</div>`, which re-parented the canvas out of the
container that sizes it and shipped a blank page to Pages. The Rust suite and the old `check_js`
both passed on that commit, because nothing anywhere looked at the HTML. **If you edit
`index.html`, run this.**

The library suite is **98 passed / 2 failed on `main`**, and that is the current expected state.
The two are:

- `test_water_blob_stays_left_right_symmetric_under_gravity` — the deliberate #56 marker that must
  keep failing. See HANDOVER.md §1.
- `test_sandbox_wave_reach_is_budget_independent` — a real, open regression. The wave now reaches
  the far wall (245/245 at both budgets) but its far-peak amplitude still depends on the budget
  (0.006726 at 32 vs 0.007285 at 64). This is a SCHEDULER symptom, not a material-model one; it was
  not part of the edge-velocity regression and points at the overclocking work. Not yet bisected.

**History, because the framing here was wrong twice.** From 2026-08-16 to 2026-08-30 the suite was
102 passed / 10 failed, and successive handovers called that "pre-existing" or "the known-good
state". It was neither: at `f43920a`, immediately before the first overfill commit, the suite was
103 passed / 1 failed. Nine were regressions. They were **bisected on 2026-08-30** and traced to two
commits inside a single 45-minute window on 2026-08-16, both adding a filter to the edge velocity in
two different functions — `33b3059` in `flux_edge_apply` and `73b71a8` in `flux_edge_candidate`.
Reverting both fixed eight of the nine. See the TOMBSTONE comment in `physics.rs` before touching
that expression, and `artifacts/design/SESSION-HANDOVER-2026-08-29.md` §1 for how the label slipped.

Do not report "tests pass" without saying which target you ran — earlier entries claiming the tests
pass were about the integration suites, not `--lib`. The integration suites all pass (5 targets).

**On 2026-08-30 the overfill model, the hierarchical coarse level and the block-clock scheduler were
deleted.** ~13k lines: `coarse.rs`, the overfill law and its equilibrium solver, the overclocking
scheduler and early-stop machinery, the lateral-correction and delta-transport experiments, five
toggle test suites, 25 diagnostic examples, and the debug overlays those fed. The library suite lost
13 tests with them (12 `coarse::tests::*` plus one overlay test) — that is the whole 110 -> 98 drop;
no physics test was lost. The reason is in the git history and in `artifacts/design/`, which was
kept in full: overfill's own instruments recorded no benefit, and the coarse level and scheduler
were reachable only through it. See `artifacts/design/SESSION-HANDOVER-2026-08-30.md` for the
bisect that preceded it and the full account.

**Doctests pass** (`cargo test -p sandart-sim --doc --release`, 0 tests). Earlier revisions of this
file recorded a permanent `physics::EQUILIBRIUM_LUT_SIZE` doctest failure and called it unrelated
pre-existing noise. It was neither: the 4-space-indented formula rustdoc kept trying to compile was
part of the overfill equilibrium solver's doc comment, and it went when the solver did.

## Verification is the deployed page

`main` auto-deploys to GitHub Pages via `.github/workflows/deploy.yml`. The wasm build is the only
surface the project is actually tested against, so nothing is really verified until it is pushed
and loaded there. There is no working browser driver on this machine — never claim to have
screenshotted or visually confirmed the app.

## Before proposing a design

Search `artifacts/design/` for prior attempts at the same lever before agreeing to a mechanism, not
at review time. This project rejects designs that measured *well* — `LATERAL-COARSE-CORRECTION.md`
Design 1 scored +41% spread and was killed on visible seams — so "would this help?" will not
surface the prior attempt. Only the archive will.
