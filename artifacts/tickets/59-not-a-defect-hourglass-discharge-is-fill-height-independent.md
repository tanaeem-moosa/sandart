# #59 — 2.36 — NOT A DEFECT: hourglass discharge IS fill-height independent (1.01x); original 14x was a drained-reservoir measurement window

**Status:** completed

---

MEASURED IN SHIPPED CODE, live on main today. `diag_task55_hourglass_discharge_rate_vs_fill_height` (written 2026-08-05, in the solids worktree at /home/deck/projects/sandart/.claude/worktrees/agent-ad1f5d94ec4ce85bf, physics.rs ~L6266) measures drain rate through a fixed neck against fill height.

Granular material obeys Beverloo: discharge rate is set by NECK GEOMETRY ALONE and is independent of fill height. That independence is why an hourglass keeps time. Measured instead: ~14x the drain rate for 6x the fill mass, which is Torricelli's law — hydrostatic head driving faster efflux. That is liquid behaviour in the granular path.

CAUSE (reasoned, then localised by measurement): the VERTICAL edge's always-on overburden bonus, `VERTICAL_PRESSURE_SCALE * janssen_effective_depth(...)` in phase 0, shipped and tuned by task #54. NOT the lateral path — the solids agent's gated lateral yield fix barely moves it (14.00x -> 12.74x), which is itself the evidence that the vertical path dominates.

This is the clearest instance of the user's "current math is more liquid appropriate" for solids, and it is independent of every gated #55 experiment.

Note the tension to resolve before changing anything: #54 tuned VERTICAL_PRESSURE_SCALE deliberately so that deep material falls faster, which was a requested behaviour. Beverloo independence and "deep water falls faster" are both wanted, for different materials. The fix is therefore probably a liquidity split rather than a retune — the same shape as the lateral yield criterion.

Janssen saturation IS the mechanism that produces Beverloo independence (wall friction carries the overburden, so stress at the neck saturates). `janssen_effective_depth` already exists and is already applied here, so the question is why it is not saturating in practice: wrong JANSSEN_DEPTH_SCALE, wrong composition with VERTICAL_PRESSURE_SCALE, or applied to the wrong quantity. Measure before rewriting.
