# #60 — 2.37 — VERTICAL_PRESSURE_CAP_MULT clamps the vertical head at &lt;1 cell of depth, so #54's "deep material falls faster" is inert

**Status:** pending

---

MEASURED IN SHIPPED CODE. Found while disproving #59 (see TASK55-SOLIDS.md, "Task #59 update" section, and the worktree at /home/deck/projects/sandart/.claude/worktrees/agent-ad1f5d94ec4ce85bf).

The vertical driving head saturates almost immediately — at raw depth ~0.2-0.45, i.e. under ONE cell — for BOTH liquid and granular material. The cause is the hard `VERTICAL_PRESSURE_CAP_MULT` clamp, not Janssen's own saturating shape: `janssen_effective_depth` does not reach half its plateau until raw depth ~24, roughly two orders of magnitude beyond where the cap has already bound.

Two consequences, both of which mean shipped code is not doing what its own doc comments describe:

1. Task #54 shipped `VERTICAL_PRESSURE_SCALE` specifically so deep material falls faster — a behaviour the user asked for by name ("deep water falls faster"). The cap cancels it beyond the first cell of depth. The feature is effectively not operating.

2. Janssen saturation on the vertical path is inert. Whatever `JANSSEN_DEPTH_SCALE` is set to makes no difference while the cap binds first, so any reasoning that depends on Janssen shaping the vertical head is currently reasoning about dead code.

Note this is NOT the same as #59, which was disproved: granular discharge IS correctly fill-height independent (~1.01x rate over a 1.7x mass range). Fill-height independence is presently being produced by the CAP rather than by Janssen wall friction. That happens to give the right answer for granular material and the wrong one for liquid, where depth SHOULD drive faster fall.

Before changing anything, decide what the cap is actually for — it was presumably added for stability (an uncapped depth term is an obvious CFL hazard on the vertical edge). Removing it to let #54's feature work risks whatever instability it was guarding. The likely shape of the fix is a liquidity split, as with everything else in this area: granular keeps saturation (whether by cap or by a correctly-scaled Janssen), liquid gets a depth-dependent term that is bounded by a CFL-derived limit rather than by a constant that binds under one cell.

Cross-check `VERTICAL_PRESSURE_CAP_MULT = 1.0` and `JANSSEN_DEPTH_SCALE = 24.0` against each other before touching either.
