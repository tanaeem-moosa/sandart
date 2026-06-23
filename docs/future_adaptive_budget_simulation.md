# Adaptive Budget Simulation

## Status: Approved Design / Not Started

## Summary

Replace the current 3-tier block LOD system with a **priority-based scheduler** that:

1. **Always simulates** blocks with displacement ≥ 0.1 (critical flow)
2. **Budget-simulates** the next highest-priority blocks up to a dynamic limit N
3. **Force-simulates** any block that hasn't been touched in 30 ticks (stale limit)
4. **Auto-tunes** N using an exponential moving average of frame time to hold 30 FPS

## Design Decisions

- **Approach A (single-threaded adaptive budget)** chosen over background-thread decoupling
- WASM threading (`SharedArrayBuffer`) adds deployment friction for no real gain — we're vsync-locked at 30 FPS on Steam Deck anyway, so both approaches process the same amount of work per second
- The adaptive budget fills the entire 33ms frame budget naturally

## Per-Tick Loop

```
┌─────────────────────────────────────────────────┐
│  1. Score all active blocks                      │
│     priority = f(displacement, staleness)        │
│                                                  │
│  2. Partition into three sets:                   │
│     MUST  = { displacement ≥ 0.1 }              │
│     STALE = { staleness ≥ 30 }                  │
│     REST  = everything else                      │
│                                                  │
│  3. Simulate all MUST blocks (uncapped)          │
│     Simulate all STALE blocks (uncapped)         │
│                                                  │
│  4. remaining = budget_N - |MUST| - |STALE|      │
│     if remaining > 0:                            │
│       top-N select from REST by priority         │
│       simulate those                             │
│                                                  │
│  5. Update EMA frame time, adjust budget_N       │
└─────────────────────────────────────────────────┘
```

MUST and STALE are always simulated regardless of budget. The budget N only governs the discretionary REST blocks. If MUST + STALE already exceeds N, we simulate all of them anyway (can't skip critical work) and the EMA will naturally lower N for subsequent ticks to compensate.

## Constants

| Constant | Value | Purpose |
|---|---|---|
| `MUST_SIMULATE_THRESHOLD` | **0.1** | Displacement above this → always simulate |
| `MAX_STALENESS` | **30** | Ticks before a block is force-simulated |
| `TARGET_FRAME_MS` | **33.3** | 30 FPS target |
| `EMA_ALPHA` | **0.1** | Smoothing factor for frame time EMA |
| `BUDGET_MIN` | **32** | Never drop below 32 blocks/tick |
| `BUDGET_MAX` | **total_blocks** | Upper bound = all blocks |
| `BUDGET_STEP` | **4** | Blocks added/removed per adjustment |

## Priority Function

### Per-Block State

```rust
struct BlockState {
    last_displacement: f32,  // max |Δh| observed last time this block was simulated
    last_simulated: u32,     // tick number when last simulated
}
```

`staleness = current_tick - last_simulated` (capped at `MAX_STALENESS`)

### Option 1: Linear Product (Recommended)

```
priority = staleness × displacement
```

| displacement | staleness=1 | staleness=10 | staleness=30 |
|---|---|---|---|
| 0.08 | 0.08 | 0.80 | 2.40 |
| 0.01 | 0.01 | 0.10 | 0.30 |
| 0.001 | 0.001 | 0.01 | 0.03 |

**Why this works:**
- The MUST threshold (0.1) already catches the high-displacement blocks unconditionally
- The stale limit (30) already catches forgotten blocks unconditionally
- The priority function only needs to rank the **middle ground** — blocks with moderate displacement (0.001–0.1) and moderate staleness (1–29)
- In that range, the linear product produces sensible orderings: a block with 0.05 displacement unsimulated for 5 ticks (priority=0.25) ranks above a block with 0.01 displacement unsimulated for 3 ticks (priority=0.03)
- Simple, cheap (one multiply), easy to reason about

### Option 2: Staleness-Weighted (Superlinear)

```
priority = staleness^1.5 × displacement
```

| displacement | staleness=1 | staleness=10 | staleness=30 |
|---|---|---|---|
| 0.08 | 0.08 | 2.53 | 13.14 |
| 0.01 | 0.01 | 0.32 | 1.64 |
| 0.001 | 0.001 | 0.032 | 0.164 |

**Tradeoff:** More aggressively prioritizes stale blocks, which reduces worst-case visual lag. But the stale limit at 30 already handles the worst case, so the superlinear term mostly affects blocks in the 10–25 staleness range. Adds an `f32::powf` call per block (~700 per tick), which is still cheap but not free.

### Option 3: Log-Staleness (Sublinear)

```
priority = ln(1 + staleness) × displacement
```

**Tradeoff:** Diminishing returns on staleness — a block at staleness 20 vs 25 gets almost the same priority boost. This favors displacement more heavily. Probably not what we want since high-displacement blocks are already caught by the MUST threshold.

### Recommendation

**Start with Option 1 (linear product)**. It's the simplest, cheapest, and the two safety nets (MUST threshold + stale limit) cover the edge cases where a fancier function would matter. If testing reveals that blocks in the 10–25 staleness range cause visible artifacts, upgrade to Option 2.

## Adaptive Budget via EMA

```rust
// After each tick:
let frame_ms = frame_timer.elapsed_ms();
ema_frame_ms = EMA_ALPHA * frame_ms + (1.0 - EMA_ALPHA) * ema_frame_ms;

if ema_frame_ms > TARGET_FRAME_MS {
    // Over budget — shed load
    budget_n = (budget_n - BUDGET_STEP).max(BUDGET_MIN);
} else if ema_frame_ms < TARGET_FRAME_MS * 0.85 {
    // Under budget with headroom — simulate more
    budget_n = (budget_n + BUDGET_STEP).min(BUDGET_MAX);
}
// else: in the sweet spot (85%-100% of target), hold steady
```

**Key design choices:**

- **EMA with α=0.1**: Reacts slowly. A single spike won't cause budget changes — it takes ~10 consecutive over-budget frames to meaningfully shift the EMA. Prevents oscillation.
- **Hysteresis band (85%–100%)**: We only *increase* budget when frame time is below 85% of target (28.3ms), not immediately when we're under. This dead zone prevents the budget from bouncing around the target.
- **Fixed step of 4 blocks**: Small enough to avoid overshooting, large enough to converge within a few seconds. At 30 FPS with step=4, we adjust by 120 blocks/second max — for ~700 total blocks, that's full range in ~6 seconds.

### Convergence Example

Starting at budget_n=256, target=33.3ms:

```
Tick 1-30:   frame_time ~25ms  →  EMA drifts to ~25ms  →  below 28.3ms  →  budget grows
Tick 30-60:  budget ~380, frame_time ~30ms  →  EMA ~29ms  →  below 28.3ms  →  grows slower  
Tick 60-90:  budget ~450, frame_time ~33ms  →  EMA ~32ms  →  in dead zone  →  holds steady ✓
```

If a complex pattern starts:

```
Tick 90-100: frame_time spikes to 45ms  →  EMA slowly rises: 32→33→34ms
Tick 100+:   EMA > 33.3ms  →  budget shrinks by 4/tick  →  frame time drops  →  converges
```

## Cross-Block Flow & Dirty Marking

When simulating a block pushes sand into a neighboring block, that neighbor's `last_displacement` should be updated (boosted) even though it wasn't simulated this tick. This ensures flow-receiving blocks get high priority next tick:

```rust
// Inside simulate(block):
if flow_to_neighbor > 0.0 {
    neighbor.last_displacement = neighbor.last_displacement.max(flow_to_neighbor);
    // Don't update neighbor.last_simulated — it's still stale
}
```

This replaces the current `activate_neighbor` mechanism and is simpler — no tier transitions, just a number update.

## Architecture Simplification

| Current (3-Tier LOD) | Proposed (Priority Budget) |
|---|---|
| `BlockActivity` enum with 4 variants | `BlockState { last_displacement, last_simulated }` |
| `should_process_block()` with modulo scheduling | `select_nth_unstable_by` on priority scores |
| `FLOW_FAST/MEDIUM/INACTIVE_THRESHOLD` constants | `MUST_SIMULATE_THRESHOLD` + `MAX_STALENESS` |
| `activate_neighbor()` with tier promotion | Dirty-mark via `last_displacement.max(flow)` |
| No FPS awareness | Closed-loop EMA budget control |

## Performance Estimate

Per-tick overhead of the new system:

| Operation | Cost |
|---|---|
| Score ~700 blocks (1 multiply each) | ~1 μs |
| Partition MUST + STALE | O(n) ≈ ~1 μs |
| `select_nth_unstable` on REST | O(n) ≈ ~1–2 μs |
| EMA update + budget adjust | trivial |
| **Total overhead** | **~3–4 μs** |

vs current tier system overhead (modulo checks, tier transitions, activate_neighbor): comparable or slightly more.

## Implementation Phases

1. **Phase 1 — Tracking**: Add `last_displacement` and `last_simulated` per block alongside the current tier system. No behavior change. Verify values look reasonable.
2. **Phase 2 — Priority Scheduler**: Replace tier-based `should_process_block` with priority scoring + top-N selection. Use a fixed budget N=256 initially. Remove `BlockActivity` enum.
3. **Phase 3 — Adaptive Budget**: Add EMA frame time tracking and budget adjustment. Wire `budget_n` to the WASM/JS layer so it can be displayed in the HUD alongside block counts.
4. **Phase 4 — Cleanup**: Remove old tier constants, simplify `activate_neighbor` to dirty-marking, update HUD to show budget_n and EMA frame time.

## Related Files

- `sandart-sim/src/physics.rs` — `settle_tick` (main simulation loop)
- `sandart-sim/src/lib.rs` — `BlockActivity` enum, `DrawingSimulation` struct
- `sandart-wasm/src/lib.rs` — WASM bindings, `get_active_block_counts()`
- `sandart-wasm/web/demo.js` — HUD rendering
