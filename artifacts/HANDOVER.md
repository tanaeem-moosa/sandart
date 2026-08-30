# Shoal & Swell — handover, updated 2026-08-17

Written at the end of a long session, for whoever picks this up next (human or otherwise). It
assumes you can read code and does not re-explain what the code already says. What it does explain
is the things the code cannot tell you: which experiments already failed, which hypotheses are
already dead, and which passing tests are lying to you.

**Deployed:** `origin/main` = `394eed59`, confirmed serving from `gh-pages`. For any later sha,
confirm the deploy rather than assuming it — read `origin/gh-pages`'s tip message for the sha it was
built from (see §1).
Live at <https://tanaeem-moosa.github.io/sandart/>.

**If you are here to build adaptive overclocking, read §11's brief on it first — the scheduler
already exists, it currently adapts only downward, and there is a live aliasing hazard.**

**Otherwise, if you read only one thing, read §9, then §10.** The oscillation defect that dominated
this project for two days is FIXED, and the fix was structural rather than a constant: every overfill edge used to
compute `flux = c_sq * (potential difference)`, which is a gain times a pressure, and the gain was
three orders of magnitude too large. It is now a solved mass transfer. Settled churn went from
0.22234 to 0.00002 per cell per tick. §9 records the four things that did NOT work first, because
each of them is a natural-looking idea that will occur to you too.

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

**The library suite has TEN failures on `main`, and only ONE of them is sanctioned.** See §10 for
the list and for why sorting them out is the highest-value next task. Do not read "the tests pass"
in any earlier handover entry as covering `--lib`; those statements were about the integration
suites only.

**One test fails on purpose and must keep failing.**
`physics::tests::test_water_blob_stays_left_right_symmetric_under_gravity` is a deliberate marker
for a known, unfixed asymmetry (see ticket #56). A run with **only** that failing is GREEN. Note
#56 now records that this test's `1.4e-5` signal and the *visible* one-sided pile the user
photographs are probably **two different bugs**, and that five previous fixes were all aimed at the
small one. Never
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
cargo test -p sandart-sim --lib --release              # ~70s, the main suite
cargo test -p sandart-sim --release --test overfill_pressure_toggle   # #70 lives here
cargo test -p sandart-sim --release --test perfect_simulation_determinism
cargo test -p sandart-sim --release --test fresh_pressure_field_toggle
cargo test -p sandart-sim --release --test pressure_heatmap_head_field_toggle
cargo test -p sandart-sim --release --test head_field_transport_toggle
cargo test -p sandart-sim --release --test pressure_sensitive_flow_toggle

node scripts/check_js.js                              # REQUIRED before any demo.js push
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
blank canvas, not a build failure. **The former rule "do not change `block_size` or the 32×32
block tiling" is SUPERSEDED as of 2026-08-18** — the user directed that the LOD block become the
same object as the coarse pressure tile, so `block_size` is now `grid_size / 64` (64×64 = 4096
blocks). See `artifacts/design/BLOCK-RESIZE.md` for the measurements and §2 of
`artifacts/design/HIERARCHICAL-PRESSURE.md` for why. What survives of the old rule is the reason
behind it: the block count must stay resolution-invariant, because `sandart-render`'s
`update_block_heat` uploads into a fixed `HEAT_GRID_SIZE²` texture with no bounds check.

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
skips quiet blocks (64×64 = 4096 of them; the tiling was 32×32 until 2026-08-18, see §1). Key
entry point is `physics::settle_tick`. `docs/ARCHITECTURE.md` covers
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

> **STATUS, 2026-08-17: this decision was taken and #70 SHIPPED.** It is the default model, the
> transfer is solved in the pressure domain (§9), and stiffness is a user-facing dial. The section
> below is kept because the *design rationale* is still the best statement of why the model looks
> the way it does — but read it as the argument that was won, not as a choice still open. Three
> claims in it are now testable rather than aspirational: siphons and tension are reachable (the
> signed pressure law exists, `underfill_tension`, default off), free fall is no longer a special
> case, and the Torricelli-vs-Beverloo validation pair still has not been built.

**There was a strategic decision on the table and it changed what "fixing" the tickets below even
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
- **Address the user as a technically proficient manager**, not as a fellow implementer: outcomes,
  status, risk, and what decision is needed — not a walkthrough of how the code got there.
- **Their physical objections are disqualifying controls.** When they push back on an EXPLANATION
  with a one-line physical argument, the explanation is probably wrong; go re-measure rather than
  explain it better. This was right three times out of three on 2026-08-16.
- **`jj` only, not `git`, for anything that writes.** The repo is colocated, so `git` HEAD sits
  detached by design — that is jj's normal state and not a problem to fix.

**A pile of partial fixes for one symptom means the FORMULATION is wrong.** This is the most
expensive lesson on this project and it has now been learned more than once. The oscillation in the
overfill solver accumulated six independent stabilisers — a velocity EMA, an acceleration filter, a
CFL wave-speed compensation, an extra damping constant, a per-cell neighbourhood nudge, and a
fill-domain acceptance rule. Every one was a reasonable idea. Every one measurably helped a little.
None finished the job, because the actual defect was that flux was computed as `gain x pressure`
with a gain three orders of magnitude too large. Recomputing it as a solved mass transfer took the
symptom from 0.22234 to 0.00002 and made all six dead code, deleted in one commit that REMOVED ~300
lines.

The cheap check that would have found it: write down the units of every term being summed in the
governing expression and confirm they are commensurate. `h/cap` spans 0..1 across a cell,
`base_head` was 1.0, and `p` was in the thousands — three terms added together, one of them
1000x the others. And the corollary, which is the useful half: **a correct reformulation is usually
a simplification.** If a proposed fix only adds machinery, suspect it.

**Prefer an irreversible measurement to a reversible one.** The user found the oscillation with
"sand at rest mixes colours slowly" after two sessions of `|dh|` metrics reported the same pools as
still. Colour advection is a ratchet; height amplitude is not. When you need to detect small
persistent motion, look for a quantity that accumulates rather than one that cancels.

**Never pick a constant on one metric.** Stillness is trivially achievable by making the fluid
slow; speed is trivially achievable by making it unstable. Every stiffness value considered on
2026-08-17 was scored on BOTH `diag_task70_rest_color_mixing_and_checkerboard` and
`diag_task70_spread_and_fall`, and the first candidate that looked good on rest alone was wrong.

**When the user proposes a mechanism, build that mechanism.** "We need to compute target in the
pressure domain" was the fix, arrived at while this agent was still sweeping constants. Their
framing ("we saturate pressure, and 512 cells should not be enough to saturate it") had located the
defect more precisely than the measurements had. Their earlier calls were right too: "EMA is a
bandaid" and "a cap that is wrong is not self-correcting, a nudge is".

---

## 8. Session log

Five sessions across two days, in order: 8a, 8b and 8d on 2026-08-16, then 8e and 8f on 2026-08-17.

**Earlier entries are kept VERBATIM with corrections marked inline, not rewritten.** Several of
their claims are now known false, and a reader who finds them deleted will simply re-derive them
from the code and believe them again. The pattern to follow when you add an entry: correct in
place, in bold, saying what was measured instead.

### 8a. Morning — overfill model first cut (commits up to `81cf2452`)

1. **Acoustic / CFL scaling.** Explicit spring stiffness (`K = 3750`) violates the CFL bound.
   Applied a backward-Euler factor `1 / (1 + sqrt(K / base_head))` and damping `0.90` in both
   phases. **CLAIMED to have "mathematically extinguished all alternating-row stripe resonance".
   IT DID NOT.** The `k = pi` checkerboard is alive and is the thing on screen today. Do not rely
   on that claim.
2. **Target hydrostatic compression bound** `h_target(P) = 1.0 + o_max * P/(P+K)` at every
   donor/acceptor interface.
3. **Convective pipe through-flow** — a saturated conduit passes up to 1.0 cell/tick instead of
   absorbing. **Was applied to the LATERAL pass only**, which is why a U-tube basin conducted
   freely while its riser did not (fixed in 8b).
4. **Mass-weighted air gravity** — `weight_A = (h_A/cap_A) * base_head`, so empty air exerts no
   downward push, intended to let liquid rise. **REVERTED in 8b**: it means an empty cell
   contributes zero gravitational head, so ANY cell below holding any material at all out-heads
   the void above it and pushes upward, underfull cells included. A fountain.
5. **Logarithmic heat-map gradation** (superseded in 8b by decile colouring).
6. **Web UI scoping + `scripts/check_js.js`** — a real pre-commit validator for `demo.js`. Run it
   before every push touching that file; it catches the "helper defined inside another function is
   silently unreachable" failure this project has shipped more than once.

### 8b. Afternoon — arbitration bug, unification, stability work (`81cf2452` → `cfcff812`)

**The headline defect: water would not rise, and the cause was not in the physics.**

The vertical edge at the foot of the U-tube riser proposed a candidate flux of exactly `-1.000000`
— a full cell, the solver's maximum — on *every tick*, while the cell above held `0.0000` and the
cell below sat at the `1.900` ceiling. Proposal at maximum with realisation at zero locates the
loss between candidate and apply, i.e. in **arbitration**, and rules out the driving head and the
acceptance rule.

`cell_freecap[i]` — arbitration's per-cell acceptor budget — carries a documented contract that it
be a pure function of cell `i`. The overfill path wrote the per-EDGE limits into it, and those
depend on the far endpoint including via `.min(h_donor)`. Two edges write each cell, so the
surviving value depended on sweep order, and it went wrong in exactly one configuration: **a cell
with empty space above it**. The downward edge from that empty cell contributes `max_accept = 0`
because an empty donor has nothing to give; when that write landed last the cell's acceptor budget
was zero and arbitration scaled its perfectly valid upward flux to nothing. Every tick, at every
rising water front. Laterally the same bad write is nearly always harmless because a lateral
neighbour usually has mass — which is exactly why this presented as *"sideways works, upward does
not"*.

Also landed:

- **The two solver passes were unified.** Phase 0 (gravity-aligned) and phase 1 (lateral) each
  carried their own hand-written overfill acceptance rule ~700 lines apart, drifted on six axes.
  They are meant to differ ONLY in that lateral edges have no hydrostatic step, which is now the
  single parameter `gravity_head` (`+base_head` down, `-base_head` up, `0.0` lateral) passed to
  `overfill_max_accept`. **That duplication is HOW the morning's convective fix reached only one
  pass. Do not reintroduce a second copy.**
- **The acoustic scale was unified.** The passes divided by different denominators, so vertical
  momentum built 4.7x more slowly than lateral for identical physics and the lateral pass ignored
  the gravity slider entirely.
- **Gravity-adapted half-difference limiter.** The lateral pass always had "never donate more than
  half the imbalance"; unifying dropped it and lateral oscillation went up ~190x. Restored and
  generalised: the imbalance is measured from the RESTING configuration, so the term is
  `(donor fill - acceptor fill + gravity_head)`. Two properties fall out — free fall is untouched,
  and an underfull cell cannot lift material.
- **Convective through-flow now requires real pressure** (`p_donor > 0`, i.e. strictly over
  capacity), not `h >= cap` which is true at exactly full where pressure is zero. A settled column
  was granting itself a whole cell per tick of "through-flow" on gravity alone.
- **Per-cell neighbourhood nudge** (`overfill_cell_resistance`). Edges decide the flow WANTED;
  cells decide the flow that HAPPENS. A cell above the gravity-adjusted average of its neighbours
  finds taking on more progressively harder, and vice versa. (The two mistakes made building it:
  measuring the neighbourhood in potential rather than fill units, which saturated; and a hard cap
  that froze the whole simulation because empty cells are trivially at their own neighbourhood
  average and so were given a zero budget and could not receive.)
### 8d. Night — Unified Viscoplastic Constitutive Model, Fast Lateral Spreading, and Hydrostatic Pool Equilibrium (`73b71a81` → `0def46b9`)

1. **Unified Viscoplastic Constitutive Model:**
   - Eliminated all binary `is_liquid` branching across the solver passes in favor of a unified constitutive relation parameterized on edge wetness $w \in [0.0, 1.0]$.
   - Continuous velocity memory EMA:
     $$\alpha(w) = 1.0 - 0.70 \cdot w \in [0.30, 1.00]$$
     $$v_{\text{target}} = c^2(w) \cdot \Delta H_{\text{yielded}}$$
     $$v_{\text{edge}} = ((1.0 - \alpha(w)) \cdot v_{\text{prev}} + \alpha(w) \cdot v_{\text{target}}) \cdot \text{damping}$$
   - **Dry sand ($w = 0$):** $\alpha = 1.0 \implies$ zero velocity memory ($v_{\text{prev}} = 0$). Sand grains freeze rigidly under static Coulomb friction upon stress relaxation, extinguishing Brownian slope shimmering and thermal crawling.
   - **Pure water ($w = 1$):** $\alpha = 0.30 \implies$ full fluid momentum continuity, allowing streams to curl, separate, and splash cleanly without sticky clumping.

2. **Fast Lateral Spreading in Phase 1:**
   - Discovered that Phase 1 (lateral pass) was mistakenly multiplying lateral wave speed $c^2$ by the vertical acoustic scale ($0.0392$), throttling lateral spreading by **$25\times$**.
   - Restored full natural liquid wave speed ($c^2 = 0.24$), allowing poured water on large grids ($512\times 512$) to flatten laterally across the vessel floor immediately upon impact rather than piling into a steep pyramid mound.

3. **Saturation Decile Histogram Equalization:**
   - Updated `refresh_saturation_deciles` with `MIN_BAND_SIZE = 0.05` spacing and distinct threshold redistribution.
   - Eliminated duplicate plateau values in the UI legend (`0.21, 1.07, 1.28, 1.57, 1.83, 1.89, 1.90, 1.90, 1.90`), and added tooltip hover inspection (`chip.title`) showing exact band saturation and percentile rank.

4. **Resolution of the Pool Over-Packing Defect** — **PARTIAL, and the "resolved" claim below is
   wrong.** Gating the levelling term did lower the settled *density*, but the pool was still in a
   permanent limit cycle at 0.222 mass/cell/tick: the metric used here (`|dh|`) measures the net
   and reads a symmetric churn as zero. See 8e. The real cause was the driving term's units, not
   the levelling gate.**
   - **Root Cause:** In `overfill_max_accept`, the gravity-leveling term $\frac{1}{2}(\text{fill}_A - \text{fill}_B + \text{gravity}) \cdot \text{cap}_B$ was continuously donating $+0.50\text{ mass/tick}$ into already-full surface cells ($h_B = 1.0$) during downward pours, artificially packing resting pools all the way to the $1.90$ slider ceiling.
   - **Fix:** Leveling is gated to underfull cells ($h_B < \text{cap}_B$), while transfers into saturated cells ($h_B \ge \text{cap}_B$) require real convective hydrostatic pressure head ($P_{\text{net}} > 0$).
   - **Result:** Resting pool density dropped from $1.87 \to \mathbf{1.01 - 1.03}$ (exact physical nominal water density!). All 7 integration tests in `overfill_pressure_toggle` pass 100%.

### 8e. 2026-08-17 — the oscillation defect, root-caused and fixed (`95ce58e7` -> `e5a722a6`)

The user came back from Antigravity with two observations, and they turned out to be one defect:

1. *"Pressure still has checkerboard patterns, improving but not gone."*
2. *"Sand at rest mixes colors slowly. Which means motion at rest."*

**The second observation is a better instrument than anything in this repo, and it is worth
understanding why.** Colour advection is a RATCHET — `advect_properties` blends, and blending is
irreversible. A flux of `+f` followed next tick by `-f` puts the heightmap back exactly where it
started, so every `|dh|` metric reads zero, while the colour field has been mixed twice. Height
amplitude measures the NET; colour measures the GROSS. `diag_task70_settled_pool_stillness_vs_capacity`
had been reporting settled pools as still while they were churning hard enough to homogenise a
painted stripe pattern in eight seconds of sim time.

That is now `diag_task70_rest_color_mixing_and_checkerboard` (§9's instrument list).

**The control that located it.** Sweeping `overfill_capacity` with the overfill model on and off:

| water pool, 4000 ticks | colour drift | stripe contrast | churn/cell/tick | checkerboard |
|---|---|---|---|---|
| capacity 1.00 (no headroom) | 0.000 | 56.000 -> 56.000 | 0.00000 | 0.001 |
| capacity 1.10 | 111.9 | 56 -> 0.27 | 0.166 | 0.148 |
| capacity 1.90 | 112.0 | 56 -> 0.16 | 0.222 | 0.429 |

Zero headroom, perfect rest to the last byte. Any headroom, full-amplitude limit cycle. Rest was
not being achieved by pressure balancing gravity; it was being achieved by material slamming into
the hard capacity wall. The checkerboard scaled on the same knob (up to 77% of the residual
velocity field in the alternating mode at capacity 1.90), which is what tied the user's two
observations together.

**Also landed on 2026-08-17, before the real fix:**

- **The two solver passes were re-aligned, through `overfill_wave_params`.** Antigravity had
  desynced them again — the lateral pass was handed the raw `wave_params` pair to cure poured water
  piling into a pyramid, leaving it at 5.9x the vertical wave speed with 0.98 damping against 0.90.
  That is the SECOND time this alignment has been broken by a fix written at one of the two sites.
  Re-aligning did NOT reduce the churn (0.207 vs 0.217) — it is a correctness fix, not a cure.
- **The compensation reached the granular blend for the first time.** Dry sand was carrying the
  full stiff pressure term through an UNDAMPED integrator (`c_sq = 1.0, damping = 1.0`). Settled
  dry sand at capacity 1.00 went from 0.0426 of permanent churn to exactly 0.00000.

---

### 8f. 2026-08-17 later — the velocity EMA off, and what it was really doing (`1d02f66b` -> `394eed59`)

The user photographed a multi-neck vessel at 512 and asked about three things at once: a falling
stream that spread sideways as it descended, regular ribs travelling down it with new ones emerging
at the neck, and cone-shaped piles under each neck instead of a level pool. One defect. See §10.

The decisive fact came from the user, not the instruments: *"they travel down with the flow and new
one comes out."* A travelling pattern is advected material; a standing one is resonance. That single
answer ruled out the entire resonance family of hypotheses in one line, and it is not something a
still screenshot can settle — ask.

Also in this entry: the superseded machinery was deleted (§9), and the EMA's arithmetic was pinned
down after this agent claimed it was carrying material up the riser. It cannot. Its steady-state
gain is `(alpha*damping)/(1 - damping*(1-alpha))`, which is 0.9363 at alpha 0.30 against 0.9800 at
1.00 — strictly a lag. The user caught that with "is it not primarily for slowing down things?"

---

---

## 9. FIXED: the oscillation defect — and the four things that did not fix it

**The defect.** Every overfill edge computed its flux as `c_sq * (potential difference)`. A
potential difference is measured in pressure units, and `overfill_head_unit =
(GRAVITY_HEAD_SCALE / depth_scale) * OVERFILL_STIFFNESS_K` was ~3700 at `w = 128` against a gravity
head of 1.0. So the flux overshot equilibrium by three orders of magnitude, hit
`flux_edge_candidate`'s `±1.0` clamp, and stayed pinned there. The result was bang-bang transport
bounded only by the capacity ceiling — a permanent limit cycle that no amount of filtering could
touch.

**The fix (`eb3b7799`).** `overfill_equilibrium_transfer` bisects for the mass `d` that solves

    phi_a(h_a - d) + gravity_head = phi_b(h_b + d)

where `phi = cell_potential` and `gravity_head` is `+base_head` down, `-base_head` up, `0.0`
lateral. The flux becomes O(mass) instead of O(pressure). Two properties fall out that no amount of
tuning had produced:

- **Saturation is unreachable by construction.** The solve never carries an acceptor past the fill
  its own back-pressure supports.
- **Rest is exact, not asymptotic.** An edge already at equilibrium returns `0.0` with no
  iteration, so a settled body transfers nothing and therefore advects no colour.

Settled water pool at capacity 1.90, over 4000 ticks:

|  | churn/cell/tick | stripe contrast | checkerboard |
|---|---|---|---|
| before | 0.22234 | 56 -> 0.16 | 0.4288 |
| after | **0.00002** | 56 -> **56.0** | **0.0030** |

### The four things that did NOT work. Read this before trying a fifth.

Each is a reasonable idea. Each was measured. Each failed for the same underlying reason — the
driving term was ~100x the clamp, so nothing that scaled or filtered it could change the output.

1. **The velocity EMA (`alpha`), and a more aggressive one.** A linear filter between two
   saturating clamps is a no-op while its input is far above the clamp. Measured: alpha 0.30 ->
   0.10 changed settled churn by 2% (0.166 -> 0.162). It only bit at alpha ~0.01, and at alpha
   0.001 the pool was still only because the settled configuration became byte-identical to
   overfill being switched off (6786 cells either way) — it bought stillness by deleting the
   physics.
2. **The linear acceleration filter.** Same shape of idea, same reason it could not work.
3. **Halving the lateral wave speed to re-align the passes.** 0.207 -> 0.217. No effect. Do this
   anyway because the duplication is a real hazard (§6), but do not expect it to help stability.
4. **Lowering `OVERFILL_STIFFNESS_K`.** This one DID work, monotonically, all the way to exact
   rest — which is exactly why it was misleading. `K` was the gain. It bought stillness by making
   the fluid less stiff, i.e. by deleting the physics the setting exists to express.

### Two things that were fighting the fix and had to be removed

- **`overfill_max_accept`'s fill-domain heuristic became a binding constraint rather than a guard.**
  Its `compression` term is `0.5 * (h_target - h_acceptor)` with `h_target` from
  `o_max * p/(p + unit)`, so with `unit` in the thousands a riser foot carrying 30 rows of head was
  granted ~0.007 of a cell per tick of upward acceptance, and the U-tube riser filled ONE row in
  3000 ticks. Both passes now clamp on physical room only; the solve is the acceptance limit. The
  function is still in the file and still `pub` — it has no call sites.
- **`refresh_saturation_deciles`' `MIN_BAND_SIZE = 0.05` spreading.** Added when settled water was
  pinned at the ceiling and the legend read `1.90 1.90 1.90 ...`. Once the fluid stopped
  over-compressing, every pair of consecutive deciles fell inside 0.05, so the redistribution path
  became the ONLY path and destroyed the equalisation it was decorating —
  `spec_task70_saturation_decile_legend` caught it with 58% of occupied cells in one band. True
  nearest-rank deciles are restored.

### `OVERFILL_STIFFNESS_K` is now a physical dial, and it is in the UI

Solving in the pressure domain took stiffness off the stability path. It now means exactly one
thing: how far a column compresses under its own weight. So it was recalibrated from TRANSPORT
rather than from stillness (5.0), and it replaced the "Overfill capacity" slider (`e5a722a6`).

**The ceiling is derived from it and must stay derived.** They are the same physical quantity
stated twice; exposing both let a ceiling be set below what the fluid itself demanded, and the
fluid then packed against it. At stiffness 5.0 with a 1.10 ceiling, 5382 of 6318 occupied cells
pinned to exactly 1.100 and nine of ten decile bands emptied. See `overfill_ceiling_for`.

| stiffness | surface -> floor fill | decile band populations |
|---|---|---|
| 2.0 (springy) | 0.58 -> 2.33 | 401..425 |
| **5.0 (default)** | 1.06 -> 1.68 | 473..533 |
| 15.0 | 1.04 -> 1.27 | 585..679 |
| 40.0 (near-incompressible) | 1.01 -> 1.11 | 585..706 |

A note on the overlay, because it was got wrong once in this session: decile colouring is histogram
equalisation, so it CANNOT look flat. It assigns a tenth of the cells to each band by construction.
The only thing that collapses it is a block of cells sharing one exact saturation — which is what
pinning against the ceiling does, and is now unreachable.

### What the fix let us delete (2026-08-17)

Six mechanisms had accumulated to suppress the oscillation. All six lost their call sites the
moment the driving term was correct, and were removed in one commit with every measurement
bit-identical before and after:

    overfill_max_accept + OVERFILL_COMPRESSION_RELAXATION
                        + OVERFILL_CONVECTIVE_THROUGHPUT    fill-domain acceptance rule
    overfill_cell_resistance + OVERFILL_BAND_SOFTNESS       per-cell neighbourhood nudge
    overfill_cell_budget, potential_slope                   never wired up
    overfill_acoustic_scale + OVERFILL_DAMPING              CFL compensation the potential form needed
    OVERFILL_TRANSFER_RELAXATION                            sat at 1.0, a literal no-op

The last one is the one to understand, because it still looks like a tuning knob. The overfill
driving term is a solved MASS, already in the units the flux is applied in, so there is nothing for
a coefficient to convert; anything below 1.0 means deliberately stopping short of the equilibrium
just solved for. Measured, it bought nothing — free fall 52 rows at 0.5 against 73 at 1.0, rest
exact at either.

**The EMA was then turned OFF (`OVERFILL_MOMENTUM_ALPHA = 1.00`) on 2026-08-17,** because it turned
out to be the direct cause of three visible artifacts rather than a cure for anything. See §10.

### The instruments (all `#[ignore]`d, run with `-- --ignored --nocapture`)

In `sandart-sim/tests/overfill_pressure_toggle.rs`:

- `diag_task70_rest_color_mixing_and_checkerboard` — **the rest instrument, start here.** Settles a
  body, paints stripes, leaves it alone. Reports colour drift, neighbour contrast, `|dh|`, mean
  `|laplacian|` (the numeric read of the on-screen checkerboard) and the signed parity power of the
  vertical edge velocities. Covers water and dry sand, overfill off and on.
- `diag_task70_spread_and_fall` — **the opposing guard, and it is not optional.** Stillness is
  trivially achievable by making the fluid slow, so NO stiffness may be chosen on the rest
  instrument alone. Reports puddle spread, pile peak and free-fall distance.
- `diag_task70_momentum_overshoot` — releases a slab against a wall and counts how many times the
  centre of mass crosses its own final value. Pure relaxation approaches monotonically, so any
  crossing is inertia. (An earlier version measured the dam-break FRONT and was confounded: it hit
  the far wall in every configuration and read zero whatever the physics did.)
- `diag_task70_heatmap_dynamic_range` — decile boundaries, band populations and a depth profile,
  swept over the stiffness dial. This is how "is there anything to see on the overlay" gets
  answered with a number.
- `diag_task70_settled_pool_stillness_vs_capacity` — the older `|dh|`-only stillness metric. Kept,
  but understand its blind spot: it measures the net, so a symmetric churn reads as zero. Prefer
  the colour instrument.
- `diag_task70_riser_foot_realised_profile` — REALISED heights tick by tick at the riser foot. This
  is what showed the riser rising at 0.0003/tick and located `overfill_max_accept` as the binding
  constraint.
- `diag_task70_u_tube_rise_time_series`, `diag_task70_u_tube_mask_profile`,
  `diag_task70_underfill_tension_sweep` — as before.

### Baselines worth keeping (2026-08-17, at stiffness 5.0)

- Settled water pool, capacity 1.90: churn 0.00002/cell/tick, stripe contrast held at 56.
- Spread 59 / pile peak 13 — **identical to the non-overfill baseline**, at every capacity.
- Free fall 73 rows in 100 ticks, against a non-overfill baseline of 122. Still a gap; see §10.

---

## 10. Flow speed, and the three artifacts it causes

The user's screenshot (multi-neck, water, 512x512, overfill on) showed three things at once: a
falling stream that SPREADS SIDEWAYS as it descends, regular ribs travelling down it with new ones
emerging at the neck, and cone-shaped "hats" piling under each neck instead of levelling. They are
one defect.

**A real falling stream narrows** — it accelerates, so mass conservation thins it. Ours widened.
Measured at 512, stream width every 6 rows below the neck:

    velocity EMA on   21  27  33  39  45  51  55
    velocity EMA off  21  21  27  27  33  33  33

That is the whole diagnosis: material was fed in faster than it fell away, so it queued sideways
because there was nowhere else to go. The queue released in slugs (the travelling ribs) and piled
into cones at the floor. The pressure overlay corroborates the last one — the cones read saturation
1.00-1.03, i.e. NOT compressed, just un-levelled, so it is a transport-rate problem and not a
pressure problem.

The cause was the velocity EMA's lag, which cost free fall 122 rows against 73. It is now off.

### The harder half: flow is pinned against a one-cell-per-tick ceiling

    grid   drained per tick   fraction of total per tick
    128    1.27               0.0003
    512    4.99               0.0001

The neck is ~5 cells wide at 512 and passes ~5 mass/tick. That is EXACTLY one cell of mass per
neck-cell per tick — the `±1.0` clamp in `flux_edge_candidate`. Flow is not slow for want of tuning;
it is railed.

And because a cell at 512 is a quarter the physical size, the same scene needs **4x more ticks** to
drain, at 16x the cost per tick. Simulated time per tick is resolution-dependent, in the wrong
direction. Fixing that needs one of:

- **solver sub-steps per frame** — n ticks per rendered frame, scaled with resolution. Simple, and
  the user has an outstanding objection to a related idea (adaptive BLOCK sub-stepping, §11) that
  does not apply here: this is uniform, so there is no aliasing between a sub-step period and a
  per-block schedule.
- **multi-cell transport for free fall** — a parcel moves k cells in one tick. This is the real fix
  and it breaks the one-edge-one-cell assumption the frozen-Jacobi pass is built on, so it is a
  design conversation.

### Proper acceleration is the intended replacement for the EMA

The user's reason for wanting the EMA was to get to real acceleration. The distinction that matters:
the EMA is a filter over a TRANSFER, while acceleration is velocity as physical state integrating
gravity. Both give an edge memory, which is what suppresses the alternating mode — so acceleration
would not need the EMA alongside it for stabilisation, and the EMA never stabilised anything anyway
(a linear filter between two saturating clamps is a no-op).

Note acceleration alone cannot beat the 1 cell/tick clamp: at 512 a falling parcel saturates it
almost immediately. Acceleration and multi-cell transport are the same project.

### What turning the EMA off costs, so it can be watched

Residual colour mixing in a settled body roughly doubles — drift 9.6 against 3.8 over 4000 ticks,
stripe contrast 55.73 against 55.98 out of 56 — and an edge-level alternating mode returns (velocity
parity 0.87 against ~0.05). Both are far below the defect this replaced, where contrast collapsed
from 56 to 0.27, but it is the same failure mode. `diag_task70_rest_color_mixing_and_checkerboard`
is the instrument.

### `spec_task70_u_tube_water_rises_up_the_riser` is PARKED, not weakened

Its `riser_h >= 8` bar at 4000 ticks reads 7 with the EMA off. The requirement it is NAMED for is
met: the long-run rise is identical either way (10 / 13 / 15 / 17 / 19 / 21 / 22 rows at ticks
5000..11000). The bar encodes a rise RATE while the name and failure message claim to test whether
upward transport works at all. The threshold was left at 8 rather than lowered to 7, because a bar
tuned to what the current build happens to do is not a spec. `spec_task70_u_tube_riser_keeps_rising`
now pins the load-bearing requirement — the riser rises and KEEPS rising across four checkpoints —
in a form that does not depend on choosing a tick.

---

## 11. Open backlog and next steps

### The 10 failing library tests

**`cargo test -p sandart-sim --lib --release` has TEN failures on `main`, against a working
agreement (§1) that permits exactly one.**

> **CORRECTED 2026-08-29 — the "pre-existing" claim below is wrong.** `95ce58e7` is dated
> 2026-08-16, which is already two days INTO the overfill work (first overfill commit `c844d68`,
> 2026-08-14), so it establishes only that the failures predate a *later session*. Measured
> directly: at `f43920a`, the commit immediately BEFORE overfill, the suite is **103 passed / 1
> failed**, and that one failure is the sanctioned one. **Nine of these ten are overfill
> regressions, not a baseline.** They have not been bisected. See
> `artifacts/design/SESSION-HANDOVER-2026-08-29.md` §1.

They were failing at `95ce58e7` before any of 2026-08-17's work and are unchanged by it — but a
previous handover stated the suite passed 100%, which was true only of the *integration* suites. Do
not repeat that. The list:

    task55_head_spec::test_task55_dynamic_transport_spec_scoreboard
    tests::test_dry_sand_has_angle_of_repose
    tests::test_head_field_transport_repose_non_regression
    tests::test_liquid_pool_levels_flat_in_closed_box
    tests::test_liquid_stream_stays_coherent
    tests::test_sandbox_wave_decays_to_flat_pool
    tests::test_sandbox_wave_reach_is_budget_independent
    tests::test_sandbox_wave_reflects_off_boundary
    tests::test_sandbox_wave_stays_left_right_symmetric
    tests::test_water_blob_stays_left_right_symmetric_under_gravity   <- the ONE sanctioned failure

Several of them (`liquid_pool_levels_flat_in_closed_box`, `sandbox_wave_decays_to_flat_pool`) name
the exact symptoms the pressure-domain solve was built to cure, and none of them have been re-run
against it with any attention. **That is the highest-value next task**: work out which now pass on
their merits, which encode a requirement that is genuinely unmet, and which are testing something
the model no longer does. Park the genuinely-unmet ones `#[ignore]`d WITH the diagnosis and ticket
number in the doc comment (§1's rule); do not weaken any of them.

### The user's stated priorities, in their order

1. **Free-falling liquid moving sideways (#33).** The pathological version of this — a stream
   widening 21 -> 55 cells as it fell — was a symptom of the EMA lag and is fixed (§10). What
   remains is the real ticket: droplet detachment and splash under high-speed falls.
2. **Faster flow — now the live problem, and it is architectural. See §10.** Flow is railed against
   a one-cell-per-tick clamp, and simulated time per tick gets 4x worse from 128 to 512. Turning the
   velocity EMA off recovered free fall (73 -> 122 rows) and fixed the stream-widening, ribs and
   floor cones it was causing, but the ceiling itself needs solver sub-steps or multi-cell
   transport.
3. **Compressed liquid.** Effectively resolved by §9 and now under user control via the stiffness
   dial. Nothing to do unless the visual feel is wrong.

### ADAPTIVE OVERCLOCKING — read this before starting

The user intends to attempt this next (2026-08-17), possibly in another tool. It was deferred once,
on the user's reasoning that a sub-step period aliasing against an oscillation period would make
things worse. That objection is now **partly** discharged and partly sharper than before. Read all
four points.

**1. A scheduler already exists, and it only adapts DOWNWARD.** `budget_n` (`lib.rs`, bottom of
`update`) is a frame-time governor: it starts at 256, steps down by 4 per frame while the EMA frame
time exceeds `target * 1.05`, and creeps back up by 1. `BUDGET_MIN` is 32. `block_size` is
`grid_size / 32`, so the block grid is **always 32x32 = 1024 blocks** at any resolution — that is
deliberate, so `budget_n` means the same thing everywhere.
**(Superseded 2026-08-18: `block_size` is now `grid_size / 64`, 64x64 = 4096 blocks, and every
constant in this paragraph moved 4x with it — start 1024, `BUDGET_MIN` 128, step down 16, up 4.
The resolution-invariance and the "`budget_n` means the same thing everywhere" reasoning are
unchanged; only the numbers moved. See `artifacts/design/BLOCK-RESIZE.md`.)** Blocks carry a four-level
`BlockActivity` (Inactive / Slow / Medium / Fast). Overclocking is the inverse operation on the same
machinery; build it there, not beside it.

**2. Measure which ceiling is actually binding FIRST.** There are two and they are confusable:

- the `±1.0` clamp in `flux_edge_candidate` — one cell of mass per edge per tick (§10, measured:
  the neck passes ~5 mass/tick through 5 cells);
- `budget_n` starvation — at 512 the user sees 9-11 fps against a ~17 ms target, i.e. ~6x over
  budget, which drives `budget_n` toward `BUDGET_MIN`. If it settles anywhere near 32 of 1024 then
  most active blocks are being SKIPPED most frames, and that dominates everything else.

This has not been measured in the browser, only inferred, and it matters enormously which it is.
Sub-stepping a starved scheduler makes the disparity between served and skipped blocks worse, not
better. **Add a readout of `budget_n` and the active-block count to the UI and look at it before
writing any scheduling code.**

**3. The aliasing hazard is REAL AGAIN, and worse than when it was first raised.** Turning the
velocity EMA off re-excited an edge-level alternating mode: velocity parity power 0.87 against
~0.05 with the filter on (§10). A per-block *variable* tick count can beat against a period-2 mode
in a way a uniform sub-step cannot. So:

- **uniform sub-stepping** (n solver ticks per rendered frame, same n everywhere) carries no
  aliasing risk and is the safe first step;
- **adaptive** (n varying per block) does, and needs
  `diag_task70_rest_color_mixing_and_checkerboard` watched before and after — specifically the
  `vpar` column, which is the direct read of that mode.

**4. The model is on your side here, and §4 says why.** A partially relaxed overfill field is a
VALID state, whereas a partially converged equilibrium solve is garbage. That is the whole #68
argument for why overfill is compatible with a block scheduler and the head field was not. Variable
work per block is legitimate under this model in a way it would not have been under the old one.

Guard rails that must hold throughout: mass conservation (asserted in several specs), and
`cargo test -p sandart-sim --release --test perfect_simulation_determinism`. `last_simulated_ticks`
already exists to let a block know how much simulated time it missed.

### Hierarchical multi-level simulation — lateral movement & adaptive clocking at 512

`artifacts/design/HIERARCHICAL-PRESSURE.md` (2026-08-18). **READ `artifacts/design/HIERARCHICAL-PRESSURE-PROGRESS.md` FIRST** for live status and design evolution.

**Current Build State (as of 2026-08-19):**
- **Step 0 (Coarse Law Falsification):** Unbounded law validated (reaches hydrostatic profile at $o=1.36$ without pinning).
- **Step 1 (Coarse Geometry):** Built on 64x64 grid (`CoarseGeometry`), committed (`e50c0bc`).
- **LOD Block Resize:** Block resized to `grid_size / 64` (4096 blocks) so block and pressure tile are identical, committed (`94d7390`).
- **Step 2 (Restriction & Coarse State):** `CoarseState` built with restriction $A[C]$, persistent coarse mass memory $M[C]$, anchoring $\lambda=0.10$, relaxation ($N=16$), hydraulic head $\eta[C]$, and disagreement $\Delta[C] = M[C] - A[C]$. Tested and instrumented (`diag_coarse_step2.rs`).
- **Step 3 (Fine-Coarse Potential Coupling):** Coarse head $\eta$ coupled into fine liquid solver with unified material continuity across wetness (Mohr-Coulomb yield stress $\tau_{\text{eff}} = \mu \cdot P_{\text{normal}}$ preserves dry sand angle of repose; fluid equalizes U-tubes; LUT thrashing prevented via closed-form $O(1)$ solve). All integration tests in `overfill_pressure_toggle` pass!
- **Step 4:** **DONE** (2026-08-19) -- dynamic priority-based multi-rate block scheduling, from
  `|Delta[b]|` and `tick_count - last_simulated_ticks[b]`. See
  `artifacts/design/OVERCLOCKING.md` for the design, measurements, and the `overclocking_enabled`
  / `coarse_pressure_coupling` toggle split (the latter now defaults OFF, gating only the
  driving-potential coupling; the coarse level's own tick runs unconditionally).

### Smaller things

- The saturation overlay colours FILL. Pressure and depth also have real gradients and might read
  better; the user has not asked for this.

### Verification checklist for incoming agents

    cargo test -p sandart-sim --lib --release                              # expect the 10 above
    cargo test -p sandart-sim --release --test overfill_pressure_toggle    # 7 pass, #70 lives here
    cargo test -p sandart-sim --release --test perfect_simulation_determinism
    cargo test -p sandart-sim --release --test fresh_pressure_field_toggle
    cargo test -p sandart-sim --release --test pressure_heatmap_head_field_toggle
    cargo test -p sandart-sim --release --test head_field_transport_toggle
    cargo test -p sandart-sim --release --test pressure_sensitive_flow_toggle
    cargo check -p sandart-wasm --target wasm32-unknown-unknown --release  # NOT a bare cargo check
    node scripts/check_js.js                                              # before any demo.js push
