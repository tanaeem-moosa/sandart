# Working on this repo

This file exists because the build/test loop is not discoverable from the code, and getting it
wrong produces confident, wrong conclusions. It was written after a session concluded "the tests
cannot be run here" and committed that claim — the loop was documented in `README.md` and
`artifacts/HANDOVER.md` §1 the whole time.

`artifacts/HANDOVER.md` is the authority for all of this. Read its §1 before touching anything.
What follows is the short version, not a replacement.

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

Integration tests do **not** run in the main test command; run them separately. The full list is in
`HANDOVER.md` §2.

```
cargo test -p sandart-sim --lib --release     # ~70s, the main suite
cargo test -p sandart-sim --release --test <name>
node scripts/check_js.js                      # REQUIRED before any demo.js push
```

The library suite is **110 passed / 2 failed on `main`**, and that is the current expected state.
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
pass were about the integration suites, not `--lib`. The integration suites all pass (10 targets).

There is also one pre-existing **doctest** failure, `physics::EQUILIBRIUM_LUT_SIZE (line 837)`: a
4-space-indented formula in a doc comment that rustdoc tries to compile as Rust. Present since at
least `f10fc15`. Unrelated to whatever you are working on.

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
