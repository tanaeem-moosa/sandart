# Credit/debt transport: the design, and the review that shrank it — 2026-08-26, rev. 2026-08-27

> # THE LIVE DESIGN IS §2.3. EVERYTHING IN §3 IS SUPERSEDED.
>
> **Read §2.3 first.** The document records three passes over one idea, and only the third is a
> proposal. §§0-2.2 are the history and the review; §§3-6 are the superseded mechanism, kept because
> what killed it is the reason §2.3 looks the way it does.
>
> **§2.3, in one paragraph.** Correct the fine level by a coarse-derived *mass* signal —
> `Delta[C] = M[C] - A[C]`, already maintained, in fine mass units, zero at agreement — routed
> between neighbouring tiles as half the difference of their `Delta`s, placed inside the receiving
> block by that block's own interior solve, and capped by headroom, static geometry and donor tile
> mass. Anchoring rises from `lambda = 0.1` to `0.5`. **There is no debt ledger and nothing is
> stored**: `Delta` is recomputed from the true fine state every tick, so a tile that was not paid
> simply still shows a `Delta`.
>
> **What the review killed, and it was one term, not the design.**
> `0.7 * (coarse_flux - fine_realised)` (§3.2) is Design 1's defect verbatim — built, measured at
> +41% spread on DrySand for +10% frame time, rejected on visible seams (0.80 → 3.60 DrySand lateral,
> 0.94 → 4.74 Water). It also never self-zeroes (§3.9), because its two terms count mass that moved
> `t` cells against mass that moved 1. `Delta[C]` has neither defect.
>
> **What the review got wrong, and what this document's first revision repeated.** §2.1 framed "does
> a coarse budget break O(L^2)?" as the bar. That was never the claim. The claim is that the fine
> level should track the coarse level, and the prize is the coarse level's own levelling rate — a
> `t*t = 64` factor at grid 512, which on a fixed-size real-time grid is the whole prize.
>
> **What the earlier form cost that §2.3 does not pay.** Persistent debt state, the `mass_err`
> rework, and all five bypass paths the review found (`restrict_incremental` staleness,
> `activate_neighbor`, edge momentum, the granular CA, structurally unpayable debt). Nothing is
> stored, so none of them exist. That deletion is the largest single change between revisions.
>
> **Still open, and honest about it:** `Delta` is per-tile while the credit is per-face, so
> half-the-difference is a one-tile-lookahead approximation to `div F = Delta` (§7(ii), multigrid,
> remains strictly stronger and unbuilt); and the U-tube fixture is deferred by user decision until
> after a first lateral-flow test, which means the first measurement can show whether material moves
> sideways but not whether the loop converges or oscillates.

**Design only — nothing built, nothing measured.** Written against `f10fc15`. Every number quoted
comes from an earlier measured document and is cited to it; nothing here has been measured.

---

## 0. What is old here, and what is not

The design below was reached from first principles in discussion, without reference to
LATERAL-COARSE-CORRECTION.md. **Its sizing term turned out to be Design 1's, verbatim. Its
architecture turned out to be new.** Both halves of that sentence matter, and the first revision of
this document stated only the first.

**What has been built before (§0.1, §0.2):** the sizing term, and the two placement strategies.
Designs 1, 2 and 3 were all *one-shot per-tick corrections inside the existing global-pass
architecture* — Design 1 applied its defect "as a limited flux across the single line of cells at
the block boundary", that tick, with nothing carried forward.

**What has NOT been built** — none of these exists anywhere in the repo's history:

- Correcting the fine level by a coarse-derived **mass** signal at all. Design 3, the shipped one,
  sets a conveyance *coefficient*; Designs 1 and 2 moved mass but within the tick that computed it.
- Persistent per-face debt, deferred payment, or block-at-a-time settling.
- A worklist scheduling blocks by disagreement, replacing the global repetition loop.

A reader coming here to check "have we tried this before" should answer: **the sizing term yes, the
mechanism no.**

Note that §2.3 keeps only the first of those bullets. Persistent debt and deferred payment were
proposed, reviewed, and then **deleted as unnecessary** once it was noticed that `Delta` recomputed
each tick already carries the memory they were introduced to provide. They are recorded here as
genuinely novel, and as genuinely not needed.

### 0.1 Design 1 — this design's §3.2, already built and rejected

LATERAL-COARSE-CORRECTION.md §2, *"Design 1 — move the defect across the block face. **Wrong:
visible seams.**"*

```
defect[face] = strength * ( coarse_flux[face] * t*t  -  fine_flux[face] )
```

That is §3.2's term with `strength = 0.7`. It was applied as a limited flux across the single line
of cells at the block boundary.

**It measured well and was still rejected**: +41% spread on DrySand for +10% frame time, and
visibly broken on screen — the user at the time: *"look here I can see block edges"*, then *"the
block boundaries are very visible"* with the pressure overlay, *"where the block grid is an
unmistakable lattice."*

The seam metric (`diag_lateral_corr`: mean height step across block boundaries ÷ across interior
pairs, so 1.0 = invisible), carried verbatim:

| | off | lateral | vertical | both |
|---|---|---|---|---|
| DrySand lateral seam | 0.80 | **3.60** | 1.45 | **3.47** |
| DrySand vertical seam | 1.05 | 1.49 | **4.39** | **4.20** |
| Water lateral seam | 0.94 | **4.74** | 1.56 | **4.21** |
| Water vertical seam | 0.67 | 2.42 | **5.87** | **5.43** |

Note the axis correlation is exact — a lateral correction wrecks the lateral seam and leaves the
vertical one nearly alone, and vice versa. That is what makes the attribution unambiguous.

### 0.2 Design 2 — uniform spread, and why our §3.6 is genuinely different

LATERAL-COARSE-CORRECTION.md §2, *"Design 2 — spread the defect uniformly through the block.
**Wrong, and worse.**"* Seams improved (3.60 → 1.76). Everything else got worse: **spread blew up
to 40–64 cells**, which at grid 512 is material smeared across the whole vessel rather than a pile
spreading.

The user named the cause before the measurement finished: *"what if a block is partially full. that
is why we simulate the block. to get the flow. when we are using coarse flow to have more material
flow, we can't skip the fine simulation."*

And the root-cause line that this design was, in effect, an attempt to answer:

> **Both failed designs share one root: they let the coarse level decide where mass goes.**

**Our §3.6 placement rule is not Design 2.** Distributing by the block's own interior solve —
headroom and head field, the block deciding where the mass lands — is precisely the fix for that
root cause, and it is a genuine advance over both failed designs. That much of the discussion holds
up.

**But the reviewer's counter is sound and decisive.** Design 2 *fixed the seams* (3.60 → 1.76) and
still failed, because it blew the **magnitude** up to 40–64 cells of spread. Seams and magnitude are
two different defects. Design 1 had the wrong placement; Design 2 had the wrong magnitude; **our
design fixes placement and inherits Design 1's magnitude term unchanged.** Better placement does not
shrink an oversized flux — it only distributes it more plausibly.

So on the sizing term, this design sits between two measured failures with no stated position on
magnitude. **That is an argument for replacing the term (§3.9.1), not against the architecture** —
and it is worth noting that the architecture attacks Design 1's seam cause from a second direction
the review did not credit: Design 1 concentrated an entire tick's defect onto one cell line, whereas
a persistent debt spreads payment across ticks *as well as* placing it by the interior solve. Two
independent mechanisms against the defect that actually killed Design 1.

---

## 1. The problem, and everything already ruled out

Water does not spread sideways fast enough. FLOW-DIRECTION.md, lateral/down across the same physical
block boundaries:

| | fine | coarse | factor |
|---|---|---|---|
| DrySand | 0.056 | 0.121 | **2.2** |
| Water | 0.079 | 0.118 | **1.5** |

**Note the water row.** Most discussion in this project quotes 2.2x; that is DrySand. Water's gap is
**1.5x**. The material we are trying to fix has the smaller disagreement of the two.

The reconciling transport — the minimum-energy flux `F` with `div F = delta` — is lateral/down
**0.327** for DrySand, ~6x the fine level's realised 0.056 (FLOW-DIRECTION.md, converged SOR at
omega 1.9, ~1,220 sweeps/tick to residual 9.98e-5).

### 1.1 Ruled out, with numbers — do not re-run these

**Conveyance boosting is dead as a lever for water** (`f10fc15`). With a correct, targeted,
scale-free signal: lateral-only boost on Water gives **+0.6% and +0.4%** spread at strengths 0.25
and 1.0; DrySand **+8.7% to +14.4%**; block-steps **+75% to +118%** with spread flat — churn, not
transport.

**Both axes on water is actively harmful**: **-5.4% to -7.7%** spread, because the vertical boost
speeds drainage and water that drains faster has less time to level.

**Headroom and the clamp are ruled out** by the `ee2c5e1` census. Conveyance **binds 93% of water's
lateral edges** — raised, and no more lateral transport came out.

**Design 1 (defect across the face) is ruled out on seams** — §0.1.

**Design 2 (uniform spread) is ruled out on magnitude** — §0.2.

**The structural reason.** An explicit local relaxation cannot be made to carry the long-wavelength
mode by raising its coefficient. Past the stability bound the extra coefficient becomes ringing, not
transport. `f10fc15` fixed the signal and the answer was still +0.6%.

---

## 2. The core idea, and what the prize actually is

**As stated in discussion:** coarse supplies the flux, fine supplies the distribution. Rather than
telling the fine level to try harder, move the mass the coarse level already moved and leave the
fine level only the job of placing it — a local problem, well inside CFL.

The appeal is real, and the second half of it survives: **§3.6's placement rule is a genuine answer
to the root cause of both prior failures** (§0.2).

### 2.1 The coarse level is a simulation, not a solve — so the prize is a 64x constant

**This section is revised.** Its first version asked "does a coarse-supplied budget break O(L^2)?",
answered no, and treated that as disqualifying. That was the wrong bar: **breaking O(L^2) was never
the design's claim.** The claim is that the fine level should track the coarse level. What follows
is what that is worth.

`coarse.rs:329` — the coarse level is *"literally a persistent `coarse_n` x `coarse_n`
`DrawingSimulation`-shaped sandbox, run one `physics::settle_tick` per coarse tick"*, per the user's
own directive (STEP4-COARSE-IS-A-SIM.md): *"let's just make coarse sim same is 64x64."* One step per
tick, unconditionally. And it is the bottom of the hierarchy: `coarse_eta` / `coarse_delta` are
passed as `&[]` because *"the coarse level is not itself coupled to a third, still-coarser level."*

So the coarse level is **a single local explicit relaxation at one fixed 64x64 level**, and
levelling a span of `L` remains O(L^2) at both levels. But its cells are `t = 8` across, so it
covers that span in O((L/t)^2) — a **`t*t = 64` factor**. If the fine level tracks the coarse level,
that factor is what the fine level gains.

**64x on a fixed-size real-time grid is the whole prize, not a consolation.** The asymptotic framing
is an academic bar that this project does not need to clear: grid size is bounded and known, and no
previous attempt has delivered anything like 64x on water. Whether the constant is reachable is an
open empirical question (§7); whether it would be worth having is not.

**What remains true and worth stating**, because the repo contains the correct diagnosis and the
built system does not implement it — LATERAL-COARSE-CORRECTION.md:19-21:

> The coarse level is where the smooth mode is solvable. **That is the entire reason multigrid
> exists.**

A multigrid V-cycle would carry the mode outright rather than buying a constant on it, it is scoped
in TASK55-MULTIGRID.md, and it is **not built**. It remains §7(ii) — now as the strictly stronger
option rather than as the only legitimate one.

### 2.2 The argument that most defends this design: persistence decouples the two bounds

Neither the first draft nor the review credited this, and it is the design's strongest claim.

Design 3 (the shipped conveyance boost) failed on water for a reason LATERAL-COARSE-CORRECTION.md
§3 states explicitly: *"Its lateral conveyance is already ~0.235 per tick against a ±1.0 clamp, so
there is little headroom to buy."* That is a **per-tick coefficient-headroom bound**, and
`flux_edge_candidate`'s `v.clamp(-1.0, 1.0)` (physics.rs:2211) is what enforces it.

**A persistent debt is not bounded by it.** A debt of `5x` face capacity, paid down over five ticks,
moves five times the mass while never once exceeding the per-tick cap. Total transport and per-tick
transport become separate quantities, and *only* the second is CFL-bounded. Nothing in Designs 1–3
had this property, because all three applied their whole correction within the tick that computed
it.

This also answers the review's sharpest structural objection (that capping per-edge debt at face
capacity reinstates the CFL face bound): **the cap is per-tick, the ledger is cumulative.** Capping
what crosses in one tick does not cap what crosses over ten.

The open question this raises — and it is the right one to take to a measurement rather than an
argument — is whether a debt that persists across ticks levels a U-tube, or merely oscillates. See
§7(i).

`coarse.rs:329` — the coarse level is *"literally a persistent `coarse_n` x `coarse_n`
`DrawingSimulation`-shaped sandbox, run one `physics::settle_tick` per coarse tick"*, per the user's
own directive (STEP4-COARSE-IS-A-SIM.md): *"let's just make coarse sim same is 64x64."* One step per
tick, unconditionally. And it is the bottom of the hierarchy: `coarse_eta` / `coarse_delta` are
passed as `&[]` because *"the coarse level is not itself coupled to a third, still-coarser level."*

So the coarse level is **a single local explicit relaxation at one fixed 64x64 level**. It is faster
per tick only because its cells are 8x bigger — the same `t*t` restatement FLOW-DIRECTION.md already
made. A local smoother handing its budget to another local smoother is still a local smoother: it
buys a **`t*t = 64` constant**, not a change of convergence class. Levelling a span of `L` blocks
remains O(L^2); the constant improves.

**The irony is worth stating plainly**, because the repo already contains the correct diagnosis and
the built system does not implement it. LATERAL-COARSE-CORRECTION.md:19-21:

> The coarse level is where the smooth mode is solvable. **That is the entire reason multigrid
> exists.**

That sentence is right. But the coarse level *as built* is a simulation, not a solve, and it stops
at 64x64 with nothing above it. The thing that would make the sentence true — a multigrid V-cycle —
is scoped in TASK55-MULTIGRID.md and **is not built**. See §7.

### 2.3 SUPERSEDING REVISION — the design as it now stands (2026-08-27)

Three corrections from the user collapse the mechanism. **This section replaces §3.** Everything
below §2 is retained as the record of what was proposed first and why it did not hold.

**Correction 1 — `0.7` is a request, not an outcome.** Headroom, aperture and donor-mass caps all
bind, so realised transport is `<= 0.7 * signal` and frequently well under. Any argument that
reasons from the nominal rate is reasoning from a ceiling.

**Correction 2 — `Delta[C]` is recomputed from reality every tick, so it self-corrects.** `A[C]` is
re-restricted from the true fine grid at the top of every `CoarseState::tick` (coarse.rs:810), so it
is a fresh measurement, not an accumulator. If the credit moved mass, `Delta` shrinks honestly. **If
the credit was capped, blocked, or simply not paid, `Delta` is still there next tick.**

**Correction 3 — anchoring is independent and can be aggressive: `lambda = 0.1 -> 0.5`.**

#### What these three changes buy

**The persistent debt ledger is deleted. `Delta` already is the ledger.** A tile that was owed mass
and did not get it shows the same `Delta` next tick. That is persistence, recomputed rather than
stored — and it removes the single most expensive and most dangerous part of the earlier design:
§3.1's debt state, the `mass_err` rework in §5.4, and all five bypass paths the review identified
(`restrict_incremental` staleness, `activate_neighbor`, `edge_vel_h/v` momentum, the granular CA,
structurally unpayable debt). None of those hazards exist if nothing is stored.

**`lambda = 0.5` restores the grounding, which `0.7` against `0.1` had inverted.** Anchoring closes
`Delta` by moving `M` (coarse forgets); the credit closes it by moving `A` (fine acts). At 0.7
against 0.1 roughly 7/8 of every gap closed by fine accommodating coarse, which makes coarse the de
facto authority and contradicts the architecture's own principle that fine is the source of truth.
At 0.7 against 0.5 — less, given Correction 1 — the two are comparable.

**And it changes what `Delta` means, for the better.** At `lambda = 0.1` the carried residual can
accumulate to ~10 coarse steps of disagreement. At 0.5 it halves every tick, so `Delta` is
approximately **one coarse step of transport measured from a freshly-tethered state**. That is
exactly the amount worth borrowing, and it bounds a wrong coarse opinion to one step before
anchoring erases it. Aggressive anchoring makes `Delta` safe to act on rather than too small to use.

#### The mechanism, stated fresh

1. **Signal.** `Delta[C] = M[C] - A[C]` (coarse.rs:758), per tile, in fine mass units, recomputed
   every tick. Zero at agreement by construction — the self-zeroing property §3.2's term lacked.
2. **Routing.** Between neighbouring tiles, move **half the difference** of their `Delta`s:
   `transfer = 0.5 * (Delta[neighbour] - Delta[self])`, symmetric so it cannot double-count from
   both sides, and the same relaxation the fine solver already applies to heights.
3. **Rate.** Scale by the request factor (0.7 as a starting value), then cap.
4. **Caps, unchanged from §3.3/§3.4.** Receiver headroom (exact); face aperture from `shape_mask`,
   which is a hard no and readable on both sides since it is static geometry; donor coarse-tile
   mass. Fine face *occupancy* is still NOT a cap — empty-now means cannot-pay-yet, and `Delta`
   persisting next tick is what handles it.
5. **Placement, unchanged from §3.6 and still the part that answers both prior failures.** The
   receiving block distributes by its own interior solve — headroom and head field — never uniformly
   across the face.
6. **Direction disagreement, unchanged from §3.8.** Treat the far side as full when donating, empty
   when borrowing, so zero transfer falls out of the caps rather than being a special case.
7. **Anchoring.** `lambda = 0.5`. Fixed, not a UI slider, until a first test says otherwise.

#### What is still open, honestly

- **`Delta` is per-tile; the credit is per-face.** Half-the-difference is a local answer to a routing
  question whose exact answer is `div F = Delta`. It looks one tile ahead, so it is a Gauss-Seidel
  style approximation, and whether that carries material across a long span is the open empirical
  question. §7(ii) — multigrid — remains the strictly stronger answer and remains unbuilt.
- **§2.2's persistence argument needs re-stating and softening.** It was written against a stored
  debt. With `Delta` recomputed, the credit still escapes Design 3's *coefficient* bound (it does not
  run through `flux_edge_candidate`'s conveyance, so water's ~0.235/tick ceiling does not apply), but
  it is still subject to a per-tick *transport* cap of broadly similar magnitude. The honest claim is
  therefore narrower than §2.2 states: the gain is that transport is driven by the coarse level's
  global opinion rather than by a local height difference — **not** that the per-tick bound is
  escaped. This should be measured, not argued.
- **The U-tube fixture is deferred** by user decision until after a first lateral-flow test. The
  consequence to hold in mind: the existing spread metric is an aggregate, so it will show whether
  material moves sideways but **not** whether the loop converges or oscillates. If spread improves
  while block-steps or churn rise, that is the missing discriminator making itself felt, and the
  fixture becomes the next thing to build.

---

## 3. The mechanism as first proposed — SUPERSEDED by §2.3

**Retained as the record of what was proposed first and what the review found. §2.3 is the live
design.** §3.9 and §3.5 are where this form fails; §3.4, §3.6 and §3.8 carry forward into §2.3
unchanged.

### 3.1 Signed per-face credit/debt, and it is real state

One signed scalar per block face. Block A borrows `x`: A gains `+x`, the ledger records B owes `-x`.

**The ledger must be persistent state accounted for in `mass_err`.** If A takes `+x` and the frame
ends before B pays, total mass is wrong unless the debt is real state the conservation check counts.
`mass_err` is the tightest invariant this project holds (`< 0.0001`, lib.rs:3618/3781) and its shape
changes here: from *feasible at every step* to *feasible once the ledger is included*.

### 3.2 Sizing: `0.7 * (coarse_flux - fine_realised)`, floored at zero

**This is Design 1 (§0.1). It is rejected. The algebra below is retained because the contraction
argument is sound in itself and would carry over to any correctly-dimensioned replacement.**

With `e = coarse_flux - fine_realised`, the agreed form `credit = 0.7 * e`:

```
realised' = realised + 0.7 * (flux - realised)
e'        = flux - realised'  =  0.3 * (flux - realised)  =  0.3 * e
```

A geometric contraction, factor 0.3 per visit (~3 visits to within 3%), fixed point
`realised = flux`. Total transport is `0.7*flux + 0.3*realised <= flux`, equality only in the limit
— it never exceeds the coarse level, and it converges to it rather than to a fraction of it.

The rejected alternative `credit = 0.7*flux - realised` hard-sets the total to `0.7*flux`, so
`e' = 0.3*flux` **constant**, independent of starting error — not a contraction, a permanent 30%
haircut — plus a discontinuous dead zone above 69% that would make the priority signal oscillate.

`0.7` is an under-relaxation factor, not a target; `0.5` changes only the rate, not the fixed point.

### 3.3 Caps

1. **Receiver headroom** — exact; the borrowing block is awake.
2. **Face aperture from `shape_mask`** — exact, readable on *both* sides, since static geometry is
   not dynamic state.
3. **Donor coarse tile mass** — estimated; "does B have the mass anywhere in its tile"
   (`restrict_incremental`, coarse.rs:538).
4. **Per-edge ceiling at the face's total cell capacity** — principled, not tuned: above that the
   debt provably cannot be paid in one visit.

### 3.4 Geometry is a hard no; occupancy is not a cap at all

- **`shape_mask` is a hard no.** A wall does not move.
- **Fine face occupancy is NOT a cap.** "B's face cells are empty now" means *cannot pay yet*. B's
  face may fill from B's own interior once B runs; the tile's top half can be empty purely because B
  has not been simulated. Gating on occupancy refuses the borrow exactly when the mechanism is
  needed. **The debt is what schedules B.**

(See §5.4 for why the donor tile-mass cap does not actually guarantee payability.)

### 3.5 Lateral and vertical faces treated identically — DISQUALIFIED, see §3.9

**As argued in discussion:** conveyance needed axis gating because it had no self-limiting property.
A credit against the *shortfall* self-zeroes wherever fine already keeps up, so most vertical faces
in a draining pool get nothing and there is no drainage speedup to reproduce.

**The U-tube objection that forced this is still correct and still valuable.** Water in one arm
raises the far arm — mass moving *up*, against gravity, on hydrostatic head the coarse level sees
and local fine relaxation cannot carry. A lateral-only rule serves it not at all, and the earlier
lateral-only recommendation was drawn from an aggregate spread metric over a pile-and-pool scene
that **cannot exhibit a U-tube**. That reasoning stands and motivates §7(i).

**But the safety argument fails on the numbers.** Uniform axis treatment is safe *only if* the
sizing term genuinely self-zeroes. §3.9 shows it does not. **§3.5 is not unproven — it is
disqualified.**

### 3.6 Placement is the block's own — THE PART THAT SURVIVES

A borrows, adds the mass to its own boundary cells **distributed by its own interior solve**
(headroom, head field), and records the debt. B is not read, not written, not woken. When B runs it
reads the debt and chooses which of *its* cells pay.

**Never distribute uniformly across the face** — that is Design 2, and splitting `x` into
`x/block_size` per cell imposes a distribution the interior did not choose, manufacturing exactly
the seams `last_frame_stalled_boundaries` exists to count.

This directly answers LATERAL-COARSE-CORRECTION.md's root-cause line: it is the coarse level
supplying a **budget** while the fine level retains authority over **placement**. Carry it forward
into any future design.

### 3.7 Scheduling

```
priority(b) = 0.5 * shortfall(b) + |debt(b)|
```

Ties by block index, for determinism.

**Dimensionally inconsistent as written** — see §6(f). The existing `shortfall` (physics.rs:1413) is
**dimensionless**, clamped to `[0,1]`; `debt` is a **mass**. The code already keeps them distinct:
`deficit = shortfall * want.abs()` (physics.rs:1414) is the mass form. Which of the two is meant
must be stated.

The per-edge debt cap (§3.3.4) and the `|debt|` term are **one design decision**: the cap is the
only thing stopping the second term from running away.

### 3.8 Direction disagreement is a boundary condition

`f10fc15`'s direction check (physics.rs:1406) skips faces where fine pushes opposite to coarse.
Handle it by treating the far side as **full when we would donate, empty when we would borrow** —
zero credit change then *falls out of the existing caps* rather than being a branch. The debt
**freezes rather than resets**. The ordinary fine solver still evaluates the edge; only the credit
mechanism goes inert.

### 3.9 FATAL: the difference is not an error signal

**This is the finding that kills the sizing term — not the architecture — and it is sharper than a
units complaint.** Both replacements in §3.9.1 self-zero, so this objection does not survive the
term it is about.

There is a coherent reading in which `coarse_flux * t*t` is exactly the mass the coarse level moved
across that plane, in fine mass units; the design asserts the coarse level as the authority, so
there is no dimensional crime, and Design 1 measuring +41% rather than diverging is consistent with
that reading. **The objection is not that the subtraction is ill-typed.**

The objection is that the two terms **count different physical events**. `coarse_flux * t*t` is mass
that moved **`t` cells** across the plane in one coarse step. `fine_realised` is mass that moved
**1 cell** across the same plane. Their difference is therefore not an error signal at all — it is a
**restatement of the resolution ratio**, and it is nearly constant regardless of how well the fine
level is performing.

**It therefore never self-zeroes.** Which kills §3.5's self-limiting property. Which was the entire
safety argument for treating vertical faces uniformly.

**The water numbers, computed from FLOW-DIRECTION.md:24-25** (grid 512, `t = 8`, `t*t = 64`, 200
ticks, coarse vs fine-across-block-boundaries — the same physical boundaries):

| axis | coarse × 64 | fine cross-block | ratio |
|---|---|---|---|
| Water lateral | 8.95 × 64 = **572.8** | **25.30** | **22.6x** |
| Water vertical | 75.66 × 64 = **4842** | **321.56** | **15.1x** |

(These replace the ~13x DrySand-era figure quoted in `compute_lateral_boost`'s comment; the water
ratios are larger.)

**The consequence is disqualifying.** Under §3.5's uniform-axis rule, the vertical credit would be
`0.7 × (4842 − 321.56) ≈ 3164` — roughly **10x the fine level's entire realised drainage**, deposited
every tick, with no CFL or FCT clamp to absorb it. And `f10fc15` already measured that a mere
**coefficient** boost on both axes cost water **-5.4% to -7.7%** by speeding drainage. A direct mass
credit at 10x realised drainage is that same failure mode with the brakes removed.

The shortfall is also saturated in the same way `f10fc15` documented for the pre-fix signal: pinned
near `1 - 1/22.6` laterally and `1 - 1/15.1` vertically on essentially every face — measured mean
shortfall **1.07 and 1.09** at two strengths under the old absolute comparison, i.e. saturated. A
saturated shortfall is a flat global multiplier in a targeted costume, and as a mass credit it is a
flat global transport multiplier.

### 3.9.1 Candidate replacements for the sizing term

**(a) The reconciling flux — the only option that changes the convergence class.** The correct
quantity is not how much the coarse level *moved* but how much mass must move to reconcile the two
levels: the minimum-energy `F` with `div F = delta`, whose lateral component per face
FLOW-DIRECTION.md §2 already computes. Scale-correct by construction, and genuinely self-zeroing at
agreement.

Its cost blocker — ~1,220 SOR sweeps/tick — **has a named, scoped, previously-successful fix in this
repo**: a multigrid V-cycle on the 64x64 grid, scoped in TASK55-MULTIGRID.md, and FLOW-DIRECTION.md
already names it as the fix. This is not "physically correct but expensive"; it is the one path with
a real destination. See §7(ii).

**(b) Share × fine magnitude — DROPPED.** `0.7 * (coarse_share - fine_share) * fine_total_transport`
is unit-safe and self-zeroing, but it is **a bounded rescale of an existing local flux — i.e. a
coefficient in a third costume**. `f10fc15` already priced coefficients on water at **+0.6% / +0.4%**.
Do not build it.

**(c) `Delta[C]` — commensurable and self-zeroing, but does NOT change the convergence class.**
`Delta[C] = M[C] - A[C]` (coarse.rs:758) is a per-tile mass discrepancy in fine mass units, already
maintained, and **already used as a per-tile flux budget** — `coarse_delta_eta_budgeted`
(physics.rs:4707), described at lib.rs:3054 as *"the total real mass the coarse term causes to leave
any one tile."* Dimensionally correct, self-zeroing at agreement, and free.

Three reasons it is not the answer:

- **It is a divergence, not a flux.** Turning per-tile `Delta` into per-face credits *is* the
  `div F = delta` problem. Doing it greedily face-by-face is Gauss-Seidel on that Poisson problem —
  a better constant, still **O(L^2)**.
- **`anchor` decays it.** `M += lambda * (A - M)` at `lambda = 0.10`/tick (coarse.rs:590-599,
  `COARSE_DEFAULT_LAMBDA`) pulls `M` toward `A` regardless of what the fine level does, so `Delta`
  decays ~10%/tick on its own — a **~10-tick lag** inside a high-gain feedback loop.
- **Double-counting.** The coarse level has *already moved* this mass in `M`. Only anchoring
  prevents the credit from compounding on top of it.

So `Delta` buys commensurability and self-zeroing — genuinely more than (a) offers today — but
**not** the convergence class. It is the right term for a *bounded correction*, not for the
long-wavelength transport this design set out to deliver.

---

## 4. What this does NOT obsolete — the §4 claim was wrong

**The original draft claimed that a priority worklist makes S3 forcing and stalled seams vanish by
construction. That is false, and it matters, because it was one of the design's stated payoffs.**

**Forcing does not exist because of the coarse correction.** It exists because the ordinary flux
solver's edges span block boundaries and belong to their lower-index cell (lib.rs:1866-1871):

> Edges are owned by their lower-index cell, so on a repetition where a fast block runs but its
> slower neighbour does not, a boundary edge whose owner happens to be that slower (non-running)
> neighbour would silently never be evaluated this repetition — chosen by grid geometry, not physics.

**A priority worklist IS differential block scheduling.** Some blocks run, others do not, and edge
ownership is unchanged. Every seam and every S3 obligation survives intact.

**Worse, the credit mechanism ADDS a reach into the neighbour.** §3.6 claims B is "not touched", but
writing `-x` into B's ledger *is* writing shared state. And "the debt schedules B" is **neighbour
forcing under a new name** — with a wake set that is unbounded rather than the current fixed
4-neighbourhood, since a debt chain can propagate arbitrarily far in one frame.

**And the deeper problem: this is a different solver, not a different scheduler.** The fine solver is
frozen-Jacobi with **global per-cell arbitration** — COLLECT over every cell, then a single
ARBITRATE+APPLY pass (physics.rs:5364 phase loop, physics.rs:7000). Arbitration resolves competition
for a cell's limited availability and headroom across **all** edges touching it, which is what makes
the FCT limiter's bound exact in one pass (ARBITRATION-AND-N-STEP.md §1). A block-at-a-time worklist
breaks that: two blocks pulling from a shared boundary cell at different times see different
availability, so the per-cell oversubscription arbitration **and its determinism contract would have
to be re-derived from scratch.** That is a solver rewrite, and it is not costed anywhere in this
document.

**Forcing's current cost, still on record** (ARBITRATION-AND-N-STEP.md §3): early stop's 1.5x
evaporated once forcing was honoured — **297 blocks run** without forcing, **489 with it**, against
**492 with early stop off entirely**. The prize for retiring forcing is real; this design does not
collect it.

### 4.1 "Edge ownership fallback" — dropped, and its open item is now RESOLVED

A smaller fix was designed and dropped during the same discussion: since edges belong to their
lower-index cell, make ownership *fall back* — evaluate the edge if its owner runs, **or** if the
owner is asleep and the far side runs.

It was dropped as subsumed by credit/debt. **Given §4, it is no longer subsumed** — it remains the
cheap, targeted fix for the seam problem, which credit/debt does not solve. It does nothing for
lateral flow, so it is not a substitute for §7; it is an independent, separable cleanup.

**Its one open item is now closed.** The concern was that the per-edge jitter might be keyed on
which block evaluates the edge, so the result would change with the participation mask and break
determinism. **It is edge-keyed.** `edge_share_jitter` (physics.rs:2560-2589) hashes
`(time_seed, edge_key, salt)` with `EDGE_SALT_H` / `EDGE_SALT_V` plus phase; nothing in it depends
on the evaluating block. **The determinism concern for ownership fallback is resolved.**

---

## 5. What already exists — reuse, do not rebuild

Verified by reading the code at `f10fc15`.

### 5.1 `LAT_LEDGER` is already a per-face, two-level flow ledger

physics.rs:994-1180. Four thread-local buffers: `COARSE_H`, `COARSE_V`, `FINE_H`, `FINE_V`.

**It is per-FACE**, though the name does not say so. `lat_ledger_record` files each crossing under
the **lower-index** block (`a_b`), and each face has exactly one lower-index owner — so `FINE_H[b]`
is the signed flux across the face between `b` and `b+1`, and `FINE_V[b]` between `b` and `b+cols`.
Both axes recorded; sign means `a -> b`.

**It already handles the subtlety that would have made it wrong.** Sand's angle of repose lives
entirely in the granular CA, not the flux solver, so a ledger watching only `flux_edge_apply` would
read DrySand's realised lateral transport as near zero. `lat_ledger_record_ca` (physics.rs:1136)
counts the CA path too, decomposing diagonal moves as a staircase. **Do not build a replacement that
watches only the flux solver.**

### 5.2 The coarse-vs-fine pairing is already wired

lib.rs:2746-2843: `lat_ledger_ensure` arms it before the coarse tick; the coarse tick writes the
coarse half; `lat_ledger_set_coarse(false)` switches attribution; `lat_ledger_snapshot()` feeds
`compute_lateral_boost`; `lat_ledger_clear_fine()` resets the fine half.

### 5.3 Geometry identities — confirmed, but they are RUNTIME properties

`COARSE_GRID = 64` (coarse.rs:55). At the shipped geometry `block_size == t == grid/64`, so block
index and coarse tile index coincide.

**This is a runtime property, not a static guarantee, and it has been false before.**
`compute_lateral_boost` **degrades gracefully** on it — `t != block_size` returns empty buffers and
the correction silently does nothing (physics.rs:1328). That is correct for a *coefficient*.

**A mass-moving mechanism cannot degrade gracefully.** Silently skipping a credit leaves outstanding
debt unpayable and mass mis-accounted. Any implementation needs a **hard assertion or an explicit
off-path**, because `block_size` has been deliberately decoupled from the coarse tile before —
commit `67dcb08`, *"Decouple block size from the coarse tile, and sweep it"*.

### 5.4 NOT ready — and the silent-corruption paths

The ledger is a **diagnostic**: `thread_local!` `Cell`/`RefCell`, default **off**, armed only when
`correction_active` (`coarse_flow_correction && coarse.available && coarse_correction_damping > 0.0`).
`mass_err` is a **test-side computation** (lib.rs:3618, 3781), not a runtime field.

Beyond that, a credit that deposits mass outside the normal flux path bypasses five mechanisms that
each assume they see every mass movement:

1. **`restrict_incremental` skips untouched tiles** (coarse.rs:558-561: `if !touched[ci] { continue; }`).
   Credit deposited without marking `blocks_touched` leaves `A[C]` **stale forever** — which corrupts
   `Delta`, `eta`, the clock scheduler's signal, *and the credit's own donor cap*, since that cap is
   tile mass. A silent, compounding, self-referential corruption.
2. **`activate_neighbor` bypassed** (physics.rs:4173). It is what marks a block modified and bumps
   its next-frame displacement. Skip it and the receiving block never re-classifies into
   `will_simulate` — **it never sweeps the mass it was given.**
3. **`edge_vel_h`/`edge_vel_v` momentum** (physics.rs:2210) left inconsistent with realised flux —
   persistent per-edge state that carries tick to tick.
4. **The granular CA `try_move` is a second mass-moving path** with no notion of a per-face budget.
   Sand's lateral transport runs through it entirely. A per-face credit that only the flux solver
   honours is half a mechanism.
5. **Debt can be structurally unpayable.** The donor cap is tile `a_mass`, but that mass may sit
   behind a `MASK_OUTSIDE` wall *inside* the tile, so it can never reach the face. And `shape_mask`
   is **user-mutable between ticks** (lib.rs:1315/1332 rebuild it from shape parameters), so a face
   can close while debt is outstanding.

**(5) forces the answer to §6(c):** "project the debt away at frame end" is the only end-of-frame
contract that provably terminates. Carrying debt indefinitely admits permanently unpayable entries.

---

## 6. Open risks

**(a) The k-visits problem.** Coarse says `x` crosses a face, but B's mass may sit in the wrong
corner of its tile, so the debt propagates *inward* before it can propagate laterally, taking `k`
block-visits to clear. §3.2's contraction covers convergence per visit and says nothing about `k`.
Lowering `0.7` to `0.5` does not help — it changes the rate, not where the mass is.

**(b) Lagged feedback — and a sub-step mismatch that is a real gap, not just a lag.**

`fine_realised` is the *previous* tick's ledger; lib.rs defends this pairing explicitly (*"the boost
has to be in place BEFORE `settle_tick`"*). Fine.

**The gap is worse than a one-tick lag.** `lat_ledger_clear_coarse` runs once per tick and the
coarse level takes **exactly one `settle_tick` step** (coarse.rs:329). The fine half is cleared once
per **frame** (lib.rs:2831) and accumulates across **all `extra_reps + 1` repetitions** (the `for rep
in 0..=extra_reps` loop, lib.rs:2975). So the subtraction pits a **per-frame sum of up to 8 fine
sub-steps** against a **single coarse step**. The two sides of `e` are not measured over the same
interval, and the ratio between them varies with the clock rate — which is itself driven by the
signal `e` feeds. This alone would make the term unstable independent of §3.9.

**(c) What the credit does to `LAT_LEDGER` is never specified, and both answers are bad.**

- **Not recorded**: `fine_realised` never rises to reflect the credited mass, so `e` stays wide open
  and the credit becomes a **permanent constant forcing at 70% of coarse throughput** — precisely
  the flat global multiplier §3.9 describes.
- **Recorded**: the ledger drives its own input. `fine_realised` rises, `e` shrinks, the credit
  shrinks, `fine_realised` falls — a **limit cycle**, with the §6(b) sub-step mismatch setting its
  period.

This must be answered before any implementation, and neither branch is currently acceptable.

**(d) End-of-frame contract.** Forced to "project the debt away" by §5.4(5).

**(e) The size of the prize is unmeasured.** "Conveyance binds 93% of water's lateral edges" is a
**coefficient** statement, not a **magnitude** one. And water's coarse/fine lateral gap is 1.5x, the
smaller of the two materials.

**(f) §3.7's priority function is dimensionally inconsistent** — `shortfall` dimensionless
(physics.rs:1413), `debt` a mass. State which is meant; `deficit` (physics.rs:1414) is the existing
mass form.

---

## 7. Recommendation

This is now the only part of this document that is a recommendation rather than a record.

> **REVISED 2026-08-27.** The user has deferred the U-tube fixture until after a first lateral-flow
> test: *"we will get to it after we fix lateral flow."* The ordering below is therefore (ii) first
> in practice — build §2.3 and measure it on the existing spread metric. §(i) is retained unchanged
> because the reasoning for it has not changed, only its position in the queue, and §2.3 records
> what deferring it costs: the spread metric cannot distinguish converging from oscillating.

### (i) Build the U-tube fixture — regardless of what happens to this design

It does not exist in the current test set. It is cheap. It is the correct discriminator for the
long-wavelength mode: a mechanism that levels a U-tube is carrying that mode, and one that does not,
is not. **It would have falsified all three previous attempts** — and it would have falsified them
early, on a picture, rather than after a build.

Every metric this project has used to judge lateral flow is an aggregate over a pile-and-pool scene,
and §3.5 records that such a scene **cannot exhibit** the configuration that matters. Building the
fixture is the cheapest available correction to that blind spot. **Do this first, unconditionally.**

### (ii) Build the multigrid V-cycle on the 64x64 coarse grid — as its own deliverable

**This is the only thing named anywhere in this document that changes the convergence class.**

Everything else on offer — conveyance coefficients, `Delta` budgets, credit/debt, greedy face-wise
distribution — buys a constant on an O(L^2) local smoother. §2.1 is the reason: the coarse level as
built is a simulation at one fixed level, and LATERAL-COARSE-CORRECTION.md:19-21 already states the
correct diagnosis that the built system does not implement.

It is **already scoped in TASK55-MULTIGRID.md**, FLOW-DIRECTION.md already names it as the fix for
the reconciling-flux cost blocker (~1,220 SOR sweeps/tick), and this repo has a track record of
landing exactly this kind of numerical work with an explicit residual test — FLOW-DIRECTION.md's own
SOR replacement caught an unconverged solver that had produced a wrong published number (0.534 →
0.345). **Ship it with a residual test so an unconverged run announces itself.**

Scope it as its own deliverable with its own acceptance criteria. Do not make it a sub-task of a
transport design.

### (iii) Only then revisit credit/debt

With (ii) in hand, sized against the **reconciling flux `div F = delta`** — repair (a), §3.9.1 —
which is the only sizing term that is both dimensionally commensurable and genuinely self-zeroing.

Carry §3.6's placement rule forward: **the coarse level supplies the budget, the fine level retains
authority over placement.** That is the one part of this design that answers
LATERAL-COARSE-CORRECTION.md's root-cause line, and it is worth keeping.

Put **Design 1's and Design 2's measured tables at the top of the page** when that work starts, so
the next attempt is measured against them from the first line rather than rediscovering them at
review.

And before building anything: **read the ledger that already exists.** Snapshot `coarse_h`/`fine_h`
on a water scene and report *magnitudes* per face, not ratios. That sizes §6(e) with no new code.

---

## 8. Summary

**THE LIVE DESIGN IS §2.3.** What follows classifies the earlier form.

**Deleted by §2.3, and this is the largest simplification in the document:**

- §3.1's persistent debt state — `Delta[C]`, recomputed each tick, already is the ledger.
- §5.4's `mass_err` rework and all five bypass paths. Nothing is stored, so nothing can go stale.
- Deferred payment as a separate mechanism — a tile that was not paid simply shows the same
  `Delta` next tick.

**Novel, and NOT refuted — unvalidated is a different verdict:**

- Correcting the fine level by a coarse-derived mass signal rather than by a coefficient. No prior
  design does this; Designs 1-3 were one-shot per-tick corrections, and Design 3 acts on
  conveyance.
- §3.7 — block scheduling by disagreement. Real work (see §4), not a refutation.
- §2.2 — **softened, see §2.3.** The credit escapes Design 3's coefficient bound but not a per-tick
  transport cap of similar size. The gain is that transport follows the coarse level's global
  opinion rather than a local height difference. Measure it; do not argue it.

**Survives:**

- §3.6 — placement by the block's own interior solve. Answers the root cause of both prior failures,
  and combines with persistence to attack Design 1's seam cause from two directions (§0.2).
- §3.2's contraction algebra — sound in itself, portable to any correctly-dimensioned term.
- §3.5's U-tube reasoning — motivates §7(i) even though the axis-uniformity conclusion is dead.
- §4.1 — the jitter determinism question, now **resolved**: edge-keyed, concern closed.

**Refuted:**

- §3.2's sizing term — **is Design 1**, built, measured, rejected on seams (§0.1). Replace it;
  §3.9.1(a) or (c). This is the one blocking item.
- §3.9 — `coarse_flux*t*t` and `fine_realised` count different physical events; the difference
  restates the resolution ratio and never self-zeroes. Water: 22.6x lateral, 15.1x vertical. Dies
  with the term — both replacements self-zero.
- §3.5's uniform axis treatment — **disqualified as sized**, for the same reason. A self-zeroing
  term may restore it; do not assume either way without measuring.
- §4 — S3 forcing and seams do **not** vanish; the credit adds an unbounded wake set, and a
  block-at-a-time worklist is a different **solver**, requiring per-cell arbitration and its
  determinism contract to be re-derived. A cost, not a refutation.
- Repair (b) — **dropped**: a coefficient in a third costume, already priced at +0.6%/+0.4%.

**Corrected in this revision:**

- §2.1's O(L^2) framing. Breaking O(L^2) was never the claim; the prize is a 64x constant, which on
  a fixed real-time grid is the whole prize. The first revision over-read the review here.
- §0's "sits between two measured failures" verdict, which applied to the sizing term and was
  wrongly generalised to the architecture.

**Unanswered, and blocking any future attempt:**

- §6(c) — what the credit does to `LAT_LEDGER`. Both branches are currently unacceptable.
- §6(b) — the per-frame-sum vs single-coarse-step mismatch in `e`.
- §5.4 — five bypassed mechanisms, of which (1) `restrict_incremental` staleness is a silent,
  compounding, self-referential corruption.
