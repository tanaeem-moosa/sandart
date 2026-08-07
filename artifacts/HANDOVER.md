# Shoal & Swell — handover, 2026-08-07

Written at the end of a long session, for whoever picks this up next (human or otherwise). It
assumes you can read code and does not re-explain what the code already says. What it does explain
is the things the code cannot tell you: which experiments already failed, which hypotheses are
already dead, and which passing tests are lying to you.

**Deployed and current:** `origin/main` = `e4f81163`, published to `gh-pages` as `d2a2271`, live at
<https://tanaeem-moosa.github.io/sandart/>. Build stamp in the panel footer reads `E4FB1163C`.

---

## 1. Read this before you touch anything

These are not style preferences. Each one cost real time to learn.

**There is no linker on the host.** Anything that compiles must run inside
`distrobox enter sandart-dev`. That container has `cargo`, `wasm-pack` and `wasm-opt`; it does
**not** have `git` or `jj`. So the loop is: edit and commit on the host, compile and test in the
container.

**`cargo check -p sandart-wasm` typechecks nothing.** The crate is `#![cfg(target_arch = "wasm32")]`-gated,
so a host-target check compiles an empty crate and passes no matter what you broke. Always
`cargo check -p sandart-wasm --target wasm32-unknown-unknown --release`.

**One test fails on purpose and must keep failing.**
`physics::tests::test_water_blob_stays_left_right_symmetric_under_gravity` is a deliberate marker
for a known, unfixed asymmetry (see ticket #56). A run with **only** that failing is GREEN. Never
fix it, weaken it, `#[ignore]` it, retitle it, or change its scan order or tolerances. Its failure
message contains a long diagnosis that has already been corrected once — read it rather than
re-deriving it.

**Never weaken a test to make a change pass, and never tune a constant until a test goes green.**
If a spec encodes a requirement that turns out to be unmet, park it `#[ignore]`d **with the
diagnosis and the ticket number written into its doc comment** — that is the pattern used by
`spec_task55_dynamic_transport_spec_scoreboard` (parked, later diagnosed, then unparked when the
diagnosis was fixed) and by `spec_task63_deeper_water_discharges_faster` (parked right now).

**Integration tests do not run in the main test command.** Run them separately:

```
cargo test -p sandart-sim --lib --release              # ~60s, the main suite
cargo test -p sandart-sim --release --test perfect_simulation_determinism
cargo test -p sandart-sim --release --test fresh_pressure_field_toggle
cargo test -p sandart-sim --release --test pressure_heatmap_head_field_toggle
cargo test -p sandart-sim --release --test head_field_transport_toggle
cargo test -p sandart-sim --release --test pressure_sensitive_flow_toggle
```

There are ~30 `#[ignore]`d diagnostics across `physics.rs` and `task55_head_spec.rs`. They are the
measuring instruments for most of the open tickets. Run with `-- --ignored --nocapture`.

**The user is the only visual instrument.** There is no working browser driver in this environment
— never claim a screenshot was taken or that anything was visually verified. Verification of
appearance happens when the user looks at the deployed page.

**A push is not a deploy.** `gh` is not installed. Confirm a deploy by fetching and reading
`origin/gh-pages`: its tip commit message names the source sha it was built from
(`Deploying to gh-pages from @ tanaeem-moosa/sandart@<sha>`). During a GitHub Actions incident on
2026-08-06, four consecutive pushes landed their refs and fired zero workflow runs; `deploy.yml`
now also has `workflow_dispatch` so a stalled deploy can be kicked from the Actions tab.

**Load-bearing UI details that have broken before.** Do not change the web colour-scheme `<option>`
*values* (`solid`, `gradient`, `stripes`, `concentric`, `checkerboard`, `rainbow_linear`,
`rainbow_radial`). Do not hand-add `<option>` elements to the material `<select>` — it is populated
from `list_materials()`. `syncSettings()` pushes the whole panel on every control change, so
nothing on that path may reset the sim. `shader.wgsl` compiles at **runtime**, so a WGSL error is a
blank canvas, not a build failure. Do not change `block_size` or the 32×32 block tiling.

**New checkboxes need a listener.** `syncSettings()` reads every checkbox at once but does not
subscribe itself; each control needs its own
`document.getElementById('...').addEventListener('change', syncSettings)`. `check-head-field-transport`
shipped without one and was silently inert for weeks — it only took effect when some *other*
control was touched. That single omission caused a user-reported defect to be misattributed to the
wrong toggle for days (ticket #65 → #68).

---

## 2. What this is

A Rust/WASM sand-and-water simulator deployed to GitHub Pages. Workspace crates:

| Crate | Role |
|---|---|
| `sandart-sim` | All the physics. `physics.rs` is ~16.7k lines and is where nearly all the work happens. |
| `sandart-render` | wgpu renderer; `shader.wgsl` compiles at runtime. |
| `sandart-wasm` | wasm-bindgen bindings + the web front end in `sandart-wasm/web/`. |
| `sandart-pattern` | Pattern generation. |
| `sandart` | Desktop binary. **Invisible to the user's testing** — they test the web build. |

The simulation is a per-edge, mass-conserving flux solver over a heightmap, run in two phases per
tick (phase 0 vertical/gravity-aligned, phase 1 lateral), with an adaptive block scheduler that
skips quiet 32×32 blocks. Key entry point is `physics::settle_tick`. `docs/ARCHITECTURE.md` covers
the broad shape; it predates the head-field work.

**w=512 is the production resolution.** 64/128/256 are diagnostic instruments, not targets. Several
defects only appear at 512 and several only at 64 — always say which resolution a number came from.

---

## 3. The head field (#55) — the centre of gravity of current work

This is the largest in-flight piece and most open tickets hang off it. Read
`sandart-sim/src/task55_head_field.rs`'s module doc comment; it is long and it is accurate.

### The idea

Hydraulic head `head = z + p/(ρg)` is **constant through a connected body at rest** and equal to
that body's free-surface elevation. So pressure is a read-back: `p = head - z`. A single field
gives you Pascal transmission through roofed channels, siphons and U-tubes — things a
column-of-material-directly-overhead measure (`column_depth`, the legacy path) structurally cannot
represent.

### The one rule

```
head[i] = max( own_local_hydrostatic[i], max over connected wet neighbours head[j] )
```

**MAX-propagation, not averaging.** This is the single most important design fact and two previous
attempts died by getting it wrong. Averaging is diffusion: the discrete Laplacian's *settling* time
over an N-cell chain is O(N²) sweeps, not the O(N) wavefront-*arrival* time. Conflating the two is
what made "8 sweeps per tick" look sufficient; at N=512 they differ by 512×. A max update is
Bellman-Ford, so it costs O(graph diameter). Gauss-Seidel max sweeps carry a value arbitrarily far
*along* the sweep direction in one pass (the two-pass chamfer distance-transform mechanism), so the
real cost is the number of direction **reversals** in a connectivity path — about 2 for a U-tube.
Measured: 2 sweeps converges every spec scenario including the U-tube; 1 sweep converges nothing.

Consequences worth stating because each was a bug at some point:

- **No ±1 on vertical neighbours.** Head carries elevation, so "rises going down" and "falls going
  up" are the same statement: `head[below] == head[above]`. The per-row increment reappears only on
  read-back, as pressure.
- **No over-relaxation.** Omega is an averaging-solver accelerator; extrapolating past a max yields
  a value no neighbour holds.
- **Cold seed every tick, never history.** `max` is monotone, so reading the previous tick's value
  ratchets upward and never falls. The field is a pure function of mask + heightmap + material.
- **The field is TOTAL.** Dry cells hold `head = z` so `p = 0`. Leaving them stale at `0.0` made
  every free-fall edge read a large *negative* driving head and go to sleep — a completely frozen
  simulation.
- **Pinned (unsupported) cells are WRITTEN, not maxed.** This is what makes `p == 0.0` an *exact*
  test for "outside the pressure model", which several things now rely on.
- **`z` of air is its elevation, not zero.** Air *pressure* is zero. Forcing `z_air = 0` would make
  every air cell the highest-head thing in the domain and drive water downward everywhere.

### What shipped and what is gated

| Toggle | Default | State |
|---|---|---|
| Pressure heat-map: use new head field | off | Works. Visualisation only, provably cannot perturb the sim. |
| Head-field liquid transport | off | **Broken — see #68.** Regresses the walls test 6 → 157 voids (#64). |
| Pressure-sensitive flow rate | off | **Broken — see #69.** Badly slows a fed falling stream. Depth ordering separately blocked by #67. |
| Fresh pressure field | off | Blocked by #57 — fails the walls test at 66 voids against a bound of 20. |

The refuted "fast liquid levelling" multigrid pass was **deleted** on 2026-08-07 (~1570 lines).
`artifacts/design/TASK55-MULTIGRID.md` and `TASK55-BRIEF.md` describe that design and the averaging
design; **both are superseded** and are kept only as a record of what was tried.

`LIQUID_ELLIPTIC_THRESHOLD` survived that deletion despite the name — it is the head field's own
liquid-only edge gate, not part of the deleted pass.

### The scope restriction, and why it is not negotiable yet

Everything head-field-driven is **liquid only**, gated on
`liquidity(wetness) >= LIQUID_ELLIPTIC_THRESHOLD` at *both* edge endpoints. The field has no yield
criterion, so applying it to granular material flattens a resting pile's angle of repose — a
permanent surface gradient that must produce **zero** flow. `test_dry_sand_has_angle_of_repose` is
re-run with the gate forced on as the non-regression check.

---

## 4. Read #70 before deciding what to work on

**There is a strategic decision on the table and it changes what "fixing" the tickets below even
means.** Ticket #70 proposes replacing the equilibrium head field with per-cell **overfill**: let a
transfer overshoot capacity (target 1.0, tolerate ~1.2), then redistribute the excess on subsequent
ticks, rather than rejecting per-edge against a frozen state.

It is the user's design, decided 2026-08-07, and it dissolves several of the tickets below rather
than solving them:

- The **rigid saturated interior** becomes mobile. Today a full body can only move at its one-cell
  surface skin, because the acceptor clamp cannot see that the acceptor is about to give the same
  mass away. That is why a levelling step produced *identical* flow at 10 and 20 cells deep.
- **Overfill is pressure** — per-cell, local, conserved and accumulating, no propagation sweep.
- **Free fall stops being a special case.** Nothing beneath you pushing back means no overfill means
  zero pressure. #67 and #69 are the same support predicate failing in opposite directions; both
  evaporate if the predicate does not need to exist.
- It is **compatible with the adaptive block scheduler**, and the head field structurally is not — a
  partially relaxed overfill field is a valid state, a partially converged equilibrium solve is
  garbage. That is the #68 argument.

It also covers **solids**, which is what makes it a unification rather than a liquid fix: overfill
is a *stress*, and a stress has a yield criterion where an elevation does not. Mohr-Coulomb becomes
`redistribute only if Δoverfill > μ·overfill + cohesion`, liquid is the `μ = 0` limit of the same
model, and the binary free-fall predicate that broke #67 and #69 becomes the continuous question
"how much load can this cell route to ground". The validation pair to build first is Torricelli
(liquid discharge *must* depend on depth) against Beverloo (granular *must not* — #59 measured 1.01×
today): one model, `μ = 0` versus `μ > 0`, reproducing both.

And it makes **siphons** reachable. They cannot work today even with a perfect head field, because a
full tube is clamped rigid; and a max-propagated field can never represent tension at all
(`p = head − z ≥ h·ds > 0`), which a real crest above the source surface requires. Signed overfill
gives tension, and capping the negative side gives cavitation as a natural limit.

Settled by the user: some slosh is accepted; overfill does **not** render (draw `min(h, cap)`); the
pressure heat-map shows overfill. The head field is kept as an **oracle** — it computes the correct
equilibrium answer, so it becomes the acceptance test for whatever replaces it as the driver, for
the hydrostatic cases at least.

**Cheapest thing that would settle the strategy**, before any of it is built: instrument what
fraction of wet cells are currently clamped to zero flux by the acceptor test. If it is only the
surface skin, that number is the hard ceiling on everything pressure work could ever buy.

## 5. Where to start, in order (if continuing head-field repair)

### #68 first, and it probably unlocks two others

The user photographed the deployed build with head-field transport on: the U-tube's body renders as
**horizontal stripes** in the pressure overlay, where a resting connected body must show a smooth
vertical gradient. The overlay is a pure read-and-convert over the same buffer the solver drives
edges with, so it cannot invent structure — the field itself is striped.

This matters out of proportion to its size because **#64 stops being a mystery if it is true**.
#64 records that head-field transport levels *worse* than the legacy path at w=512 while moving
*more* mass, and its leading hypothesis is an unbounded driving head causing sloshing. If the field
is simply broken under transport, that hypothesis is beside the point. **Do not spend more time on
#64's driving-head magnitude until #68 is resolved.**

Two candidate causes are written up in #68. The second one — that transport moves mass, which
changes the support classification next tick, which changes the field, a feedback loop the
overlay-only path does not have — matches the symptom pattern exactly and is cheap to test.

### #69 and #67 are one problem — fix them together

**#69 first, because it also demonstrates how the test suite can mislead you here.** The user
reported that the pressure-sensitive flow toggle badly slows falling water and makes it spread
sideways. `spec_task63_free_fall_is_bit_identical` **passes** — at both resolutions — asserting the
exact opposite. The spec is not lying about what it measures; it measures the wrong scenario. It
releases a compact slab into empty space with no source above and nothing below, so every cell is
genuinely unsupported, gets pinned to `head = z`, and takes the exemption's exact `1.0` branch. A
real stream is fed from above and lands on standing material, so its cells read as *supported*, get
a real (tiny) head, and are attenuated ~8× — `sqrt(0.3/20) = 0.12` for a typical 0.3-filled stream
cell. Slowed descent backs the stream up, and backed-up material at capacity pushes laterally.

**This is the same defect as #67 with the sign flipped.** #67: a column being extruded through an
orifice is classified as free-falling when it is bearing load. #69: a stream in flight is classified
as supported when it is ballistic. One predicate — "is this material bearing load right now" — wrong
on both sides. Two independent patches will fight each other; find the predicate once.

### #67 in detail

`advance_head_field` pins an **entire column above an orifice** to zero head. Measured directly:
centre column reads `0.00` at the orifice, at the cell above it, and at the column *top*, while an
off-centre column of the same water reads `10.00` / `20.00` correctly at the same instant. Cause is
the transitive-support pass — "a cell resting on falling material is itself falling" — which is
right for a slab falling through air and wrong for a column being **extruded** through a hole. An
extruded column is under pressure; that is the entire content of Torricelli's law.

This blocks the user's stated requirement on #63 ("20 depth should have higher flow than 10") and
probably explains #59 (hourglass discharge measured fill-height *independent*, which is correct for
granular under Beverloo and wrong for a liquid).

### Then re-measure #64, and reconsider #57

Both are transport/pressure-quality tickets whose numbers were taken against a field that may be
broken.

### Independent of all of the above: #66

Advancing the head field costs **+219% ms/tick at w=512** (6.2 → 19.9). Isolated with a three-arm
diagnostic: advancing the field alone costs the same as advancing it *and* applying the rate law,
so it is entirely the advance. Leading suspect is six whole-grid `Vec` allocations per call. This
is paid by anyone using the pressure heat-map's new-field source today. Do **not** respond by
lowering `HEAD_FIELD_SWEEPS_PER_TICK` — it is already an early-exiting cap that measures 2 in
practice; the cost is not in the loop count.

---

## 6. Structural facts that repeatedly ambush people

**Saturation is upstream of pressure.** `flux_edge_candidate` clamps flux to
`(cap_b - h_b).max(0.0)`. In a body full to capacity, every interior acceptor is at capacity, so
those edges are clamped to **zero regardless of how much head their donors carry**. A rigid
container full to the brim of incompressible fluid is genuinely static — that is correct, not a
defect. Two consequences:

- No amount of pressure work can make a full vessel move. Do not respond to "a full vessel does not
  move" by relaxing the donor/acceptor clamp.
- **A levelling scenario cannot measure depth dependence at all.** Measured: 133.60 flow at *both*
  10 and 20 cells deep, toggle on and off. Only the free surface can move, and its head comes from
  its own fill, not from the depth beneath it. Depth dependence only exists where there is somewhere
  to go — an orifice. A first draft of #63's ordering spec used a levelling step and passed on float
  noise.

**Attenuating `c_sq` does not attenuate flow.** Wherever an edge has real room to move into, the
driving head is large enough that velocity reaches the donor-mass/acceptor-room clamp within a tick
or two whatever `c_sq` is — so realised flux is *mass*-limited and `c_sq` drops out. Measured on a
draining vessel: 127.1 at 10 deep vs 124.5 at 20 deep, i.e. nothing. Scale the **flux** (via
`flux_edge_candidate`'s `weight` parameter) if you want to change how much moves.

**Trajectories diverge, so end-state comparisons are not monotonicity tests.** A change that can
only ever *reduce* per-edge flux can still produce a *higher* total displacement after 200 ticks
(measured: 199.5935 with vs 199.5251 without) because it changes which cells hold material later,
which changes which blocks the scheduler wakes. Compare at the **first divergent tick**, the only
window where both runs share an input state.

**`#60` is a dead end for head-field work.** `VERTICAL_PRESSURE_CAP_MULT` and `vertical_bonus` are
computed only inside the *legacy* branch of the driving-head selection. The head-field branch
bypasses both entirely.

**Units.** `depth_scale = REFERENCE_GRID_HEIGHT / w` with `REFERENCE_GRID_HEIGHT = 512`.
`head_scale = GRAVITY_HEAD_SCALE / depth_scale` (25 at w=512, 3.1 at w=64), `GRAVITY_HEAD_SCALE = 25.0`.
Omitting `GRAVITY_HEAD_SCALE` at a driving-head site drives liquid edges 25× too weakly, which does
not read as "a bit slow" — it lands under the solver's own thresholds and reads as a **completely
frozen simulation**. Anything depth-thresholded should be expressed in **reference rows**, not local
cells, or it means a different physical depth at every resolution.

---

## 7. Working agreements

`artifacts/notes/` holds these in full; the load-bearing ones:

- **Report mechanisms, not symptoms — and take the user's wording literally.** When their
  description conflicts with a green test, suspect the test. This has been right every time.
- **Ask what a visual quality word means before building.** "Grainy" and "randomness" cost three
  wrong implementations. Name the measurable quantity first.
- **Get a picture before building metrics.** One photo killed three hypotheses that a careful
  diagnostic had missed.
- **A still photo cannot prove a screen artifact.** "Does it move?" beats anything readable in a crop.
- **Be concise.** Detail belongs in the ticket, not the reply.

---

## 8. Session log — 2026-08-07

Five commits, `2af44fcc` → `e4f81163`:

1. **`0a5875bf`** — surface-valley levelling diagnostic (the instrument for #64).
2. **`acf2da48`** — deleted the refuted "fast liquid levelling" multigrid pass: the pass, its
   thread-local gate, `recompute_column_depth_scoped`, the union-find helpers, seven constants, the
   parameter through `settle_tick`/`DrawingSimulation`/`TestSim`/the wasm binding, the checkbox and
   its help text, and six tests. ~1570 lines out of `physics.rs`.
3. **`73adbf63`** — pressure-sensitive flow rate behind a new default-off toggle.
4. **`36d3cf7b`** — reworked that rate law: `sqrt` (Torricelli) rather than clamped-linear,
   reference rows rather than local cells, and applied to the **flux** rather than `c_sq`.
5. **`e4f81163`** — wired the two missing checkbox change listeners.

Also measured and recorded, without code changes: the fresh-pressure-field promotion the user asked
for is **blocked** (#57 — it fails the walls test at 66 voids against a bound of 20, the same number
recorded two days earlier; nothing since has moved it, and turning on head-field transport makes
that test far worse still at 157).

Tickets filed this session: #66 (head-field advance cost), #67 (draining column pinned to zero),
#68 (transport breaks the pressure field). #65 closed as misattributed. #63 reopened — the rate law
shipped, but the user's depth-ordering requirement is blocked on #67.

---

## 9. Open backlog

Full list with measured numbers and ruled-out hypotheses in
[`tickets/INDEX.md`](tickets/INDEX.md). **#70 is the strategic one — read it first.** The
head-field cluster is #55, #57, #62, #63, #64, #66, #67, #68, #69, and several of them are
superseded rather than solved if #70 goes ahead. Everything else is independent of it and can be picked up in isolation — #27 (water
towers and violent splashing), #33 (sideways movement design), #38 (1024 resolution), #44
(asymmetric drain, with an explicit caution that pressure *masks* it and damping must not be
mistaken for a fix), #49 (falling acceleration), #50 (LOD degradation), #51 (larger grain material),
#52 (vertical striping in the draining funnel), #53 (pressure-projection cost).
