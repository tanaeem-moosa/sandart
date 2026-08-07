# #49 — 2.26 — Give falling material acceleration when the space below is empty

**Status:** pending

---

User request 2026-08-01, sequenced explicitly AFTER pressure projection (#45).

Material currently falls at a fixed rate regardless of how long it has been falling. Real free fall accelerates. User's framing: "we need acceleration if the space below is empty. that would allow us to naturally fix this sort of issues."

## The argument, in the user's terms — and why it is right

"if nothing below it, it moves faster. so it allows the block above to catch up. it will probably create sand empty sand empty but that is better than ssseeessss"

The point is about the SCALE of the artifact, not its presence. Gaps between falling layers do not disappear under acceleration — but they become BOUNDED instead of growing:

- Today a stalled upper layer contributes a gap that grows for as long as it stays unscheduled (previously up to `MAX_STALENESS` = 30 ticks). Unbounded in practice, hence 20-cell slabs.
- With acceleration plus prompt activation, the layer behind is only ONE tick late in release, so it is only one tick behind in accumulated velocity. Spacing settles at roughly a cell.

Alternating sand/empty at 1-cell scale reads as grains dispersing in free fall, which is what falling sand looks like. Coarse slabs read as a bug. Same artifact class; the scale is the entire difference.

An earlier note here framed this as "acceleration re-opens the gaps #47 closed". That was the wrong framing and is superseded — what matters is whether spacing is bounded, and acceleration bounds it.

## The real constraint: displacement per tick vs block_size

`block_size = (grid_size / 32).max(1)` gives only 2 cells at 64. If accelerated displacement per tick exceeds `block_size`, material jumps OVER an entire block and the activation chain breaks — a skipped link, not a wider gap. That is a different and worse failure than the one fixed in #47, because the upstream wake added there propagates one block at a time.

So this needs a CFL-style bound: maximum displacement per tick tied to `block_size`, which at 64 means a cap around 2 cells. Establish the current fall speed properly first — it was measured at ~2 cells/tick during #47, i.e. already AT that bound on the 64 grid.

## Other design notes
- `edge_vel_h` / `edge_vel_v` already exist (allocated in `new_with_size`, lib.rs). Check whether they are live and whether acceleration belongs there rather than in a new buffer.
- Multi-cell displacement per tick also collides with frozen-state Jacobi's one-cell-per-tick information limit. That is a different transport scheme, not a parameter change.

## Why it belongs after pressure (#45)
Pressure projection changes what drives motion. Adding velocity state before that would be tuned against driving forces about to be replaced.
