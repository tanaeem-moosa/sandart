# Task #55 — read this first (2026-08-05, overnight)

## The headline: the multiplicative form CANNOT be made default

You asked "what happens if we make this the new behavior and push". I tested it
directly rather than guessing: a git worktree pinned to the checkpoint commit
`1ef8507`, with `multiplicative_lateral_gate` forced ON in BOTH places (the
test-mode `thread_local` default and the `cfg(not(test))` twin), full suite.

**Nobody had ever run the suite with this gate on.** Every green run in this
task was green *because* the gate was off.

Result: **91 passed / 4 failed** — three new failures beyond the intentional
symmetry marker:

    test_dry_sand_has_angle_of_repose
    test_color_boundary_does_not_diffuse_under_gravity
    test_liquid_flowing_liquid_does_not_stand_in_walls

### Angle of repose: sand becomes water

    CASE 1 (steep): initial=0.3500 (19.29 deg) final=0.0071 (0.41 deg) total_flow=2061.09
    NON-VACUITY ANCHOR @450 ticks: DrySand=0.0000 (0.00 deg) Water=0.0000 (0.00 deg)

A 19-degree sand slope flattens to 0.4 degrees. The test's non-vacuity anchor —
which exists to confirm sand holds MORE slope than water in an identical rig —
reports both at exactly 0.00.

**This is structural, not tuning.** `flux ~ conveyance * grad(surface)` drives
flow whenever a surface gradient exists. A granular pile at its angle of repose
IS a permanent surface gradient that must produce ZERO flow. The form has no
yield criterion. Janssen saturation bounds how much depth carries flow; it never
says "stop below a critical slope". Sweeping `MULT_LATERAL_SCALE` or the
conveyance exponent cannot fix this.

### The standing-water test fails too — the one it was supposed to fix

    scale=1 w=64 h=64 voids@120=95 voids@160=65 total=29312   (gate ON)
    additive baseline:  voids@120=60 voids@160=6  total=11049

Note the tension this resolves: the "17-24% fewer voids at w=512" result we had
been quoting is from a DIAGNOSTIC variant of this same scenario at a different
resolution. The actual assertion, at its own resolution, fails. Same defect, two
measurements, opposite answers — exactly what the unresolved resolution split
was warning about.

## What this does NOT overturn

- The unit fix is real and correct, and is proven resolution-invariant
  (0.00% deviation in the driving term across w=64/128/256/512). Keep it.
- The `diag_task55_*` levelling instrument is real and is the thing everything
  after this gets measured against. Keep it.
- The finding that these defects are PHYSICS, not scheduling, stands: ticks to
  halve are near-identical under the adaptive scheduler and perfect simulation
  (arch 71 vs 65, pockets 92 vs 92).
- The free-surface DIAGNOSIS may still be right for LIQUID. What is refuted is
  applying one undifferentiated form to both liquid and granular material.

## Repo state

- `origin/main` = `23e48e9` (the slab fix, pushed and deployed).
- Local commit `1ef8507` = the #55 instrument + gated multiplicative head.
  Committed as a checkpoint, deliberately NOT pushed — the gate is off, so it
  is not testable on the deployment, and its only value there is backup.
- Worktree `/home/deck/projects/sandart-mult-test` is the gate-ON rig described
  above. Detached at `1ef8507` with the two gate defaults flipped. Delete with
  `git worktree remove --force /home/deck/projects/sandart-mult-test` when done;
  it is useful for re-running the gate-ON suite.

## SECOND VERDICT (landed later the same night): the form is a null at w=512

`TASK55-RESOLUTION.md` settled the open resolution question. At production
resolution, with identical tick budgets for gate-on and gate-off:

    arch collapse, adaptive:  522 -> 548 ticks  (+5.0% SLOWER)
    arch collapse, perfect:   517 -> 541 ticks  (+4.6% SLOWER)
    pocket equalisation:       29 -> 32 ticks   (~10% SLOWER)
    draining lake peak spread: 159.0 -> 159.0   (bit-identical)

The 17-24% void-count win was isolated to that proxy. It does not appear in any
direct levelling measurement. So the multiplicative form both destroys granular
behaviour AND fails to help liquid levelling where it counts.

### The most valuable number of the night

`ticks_to_halve` scales almost exactly LINEARLY with w (522 at w=512; 65 when
normalised by w/64). Linear scaling in domain width is the signature of a
CELL-RATE-LIMITED process: levelling speed is bounded by how fast information
crosses the domain at one cell per tick, NOT by the magnitude of the driving
potential.

That explains the null cleanly, and it re-ranks the whole task. **Propagation is
the binding constraint; the potential is not.** The elliptic solve is therefore
the load-bearing half of #55, not a complement to the free-surface form. The
sharper acceptance bar for it: does `ticks_to_halve` STOP scaling linearly with
w? That is a stronger test than any single-width speedup, and the elliptic agent
has been asked to measure exactly it.

## THIRD RESULT: the hourglass drains like a liquid (new ticket #59)

`TASK55-SOLIDS.md`. This one is a defect in SHIPPED code on main today, not in
any gated experiment.

Nobody had ever measured discharge rate against fill height, so the agent wrote
the test. Granular material obeys Beverloo: discharge is set by NECK GEOMETRY
ALONE, independent of fill height — that independence is precisely why an
hourglass keeps time. Measured:

    ~14x the drain rate for 6x the fill mass

That is Torricelli's law — hydrostatic head driving faster efflux. Liquid
behaviour in the granular path, and the sharpest confirmation of "current math
is more liquid appropriate" we have.

Localised to the VERTICAL edge: `VERTICAL_PRESSURE_SCALE *
janssen_effective_depth(...)` in phase 0, shipped and tuned by task #54. Not the
lateral path — the gated lateral fix barely moves it (14.00x -> 12.74x), which
is itself the evidence that the vertical term dominates.

Tension to resolve before touching it: #54 tuned that term deliberately so deep
material falls faster, which was requested. Beverloo independence and "deep
water falls faster" are both wanted, for different materials — so the fix is
probably a liquidity split, the same shape as the lateral yield criterion, not a
retune. And since Janssen saturation IS the mechanism that produces Beverloo
independence, and `janssen_effective_depth` is already applied here, the real
question is why it is not saturating in practice. Measure before rewriting.

Also in that report: a real conflation found in the shipped LATERAL yield test —
`tau`, calibrated as a bare geometric slope, is compared against a head
difference that already has the Janssen/earth-pressure depth term folded in.
Fixed behind `granular_yield_gate`, no new constants, only the yield DECISION
changed (an earlier draft that also zeroed `flux_tau` caused a measured
over-drive regression, documented in the code). Honest limitation, stated by the
agent: the fix is bit-identical on both the repose and liquid tests because
`column_depth ~ 0` in the repose rig, so no existing test exercises the path it
corrects. The reasoning is sound; the demonstration is not there yet.

Worktree: `/home/deck/projects/sandart/.claude/worktrees/agent-ad1f5d94ec4ce85bf`
(branched from main, so it does NOT contain the multiplicative gate).

## In flight overnight

Two agents, results in this directory when they land:

- `TASK55-ELLIPTIC.md` — the elliptic head solve (the propagation half). It has
  been told about the repose failure and instructed to either confine itself to
  liquid or carry an explicit yield criterion tied to the existing repose
  machinery, and to report `test_dry_sand_has_angle_of_repose` with its gate on.
  Expect this constraint to bite harder for the solve than for the local form: a
  converged elliptic solve propagates disequilibrium globally in one tick, so by
  a pure free-surface criterion it would flatten every sand pile in the domain
  very fast.
- `TASK55-RESOLUTION.md` — making the three levelling diagnostics
  resolution-parametric and sweeping w=64/128/256/512, to settle whether the
  multiplicative form improves LEVELLING at production resolution or whether the
  w=512 void win was isolated to that proxy.

Neither commits. Nothing gets pushed.

## Suggested reading order tomorrow

1. This file.
2. `TASK55-RESOLUTION.md` — does the liquid case actually hold up at w=512?
3. `TASK55-ELLIPTIC.md` — does propagation help, and at what cost?

The open design question for #55 is now sharper than it was: the free-surface
form needs a material split. Liquid wants `conveyance * grad(surface)`. Granular
wants that only above a critical slope, and the existing repose machinery
already encodes what that slope is.
