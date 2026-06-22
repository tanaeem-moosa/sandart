# Future Optimization: Adaptive Budget Simulation

## Status: Idea / Not Started

## Summary

Replace the current 3-tier block LOD system with a **priority-queue based cell scheduler** that simulates the **N most important blocks per tick**, where N is dynamically adjusted to maintain a target frame rate (30 FPS).

## Motivation

The current approach uses fixed flow-rate thresholds to bucket blocks into Fast/Medium/Slow/Inactive tiers, each processed at different frequencies. This works but has downsides:

- **Threshold tuning is manual** — different patterns and materials produce different flow distributions, so no single set of thresholds is universally optimal.
- **Block architecture adds complexity** — 3 separate code paths for tier scheduling, block activation/deactivation logic, neighbor wake-up rules, etc.
- **No frame-rate awareness** — the system doesn't adapt to actual performance; it just hopes the tier distribution keeps things fast enough.

## Core Idea

### Priority Score

Each block gets a priority score that combines two factors:

```
priority(block) = displacement(block) × staleness(block)
```

Where:
- **`displacement`**: Max height delta observed in the block's last simulation tick (how actively sand is flowing). Could also use sum or mean of cell deltas.
- **`staleness`**: Number of ticks since the block was last simulated. A block that hasn't been touched in 10 ticks is more "urgent" than one simulated last tick.

The multiplication means:
- High displacement + stale → **very high priority** (active flow being ignored)
- High displacement + fresh → **medium priority** (active but recently handled)
- Low displacement + stale → **low priority** (not much happening, fine to skip)
- Low displacement + fresh → **lowest priority** (just simulated, nothing to do)

### Adaptive Budget (N)

Instead of a fixed N=256, dynamically adjust the budget:

```
if avg_frame_time > target_frame_time:
    N = N - step       # shed load
elif avg_frame_time < target_frame_time * 0.8:
    N = N + step       # we have headroom, simulate more
```

- **Target**: 33.3 ms (30 FPS) or whatever the display refresh is
- **Step size**: Could be 8–16 blocks per adjustment
- **Smoothing**: Use an exponential moving average of frame time to avoid oscillation
- **Bounds**: `N_min = 32` (always simulate at least something), `N_max = total_blocks` (don't exceed the grid)

### Safety Guardrail

Blocks with displacement above a critical threshold **always get simulated**, regardless of the budget. This prevents visible artifacts like sand "freezing" mid-avalanche.

```
if displacement(block) > CRITICAL_THRESHOLD:
    // always simulate, doesn't count toward budget N
```

The budget N only governs the remaining blocks that fall below the critical threshold.

## Architecture Simplification

This approach could **eliminate the tier system entirely**:

| Current (3-Tier LOD) | Proposed (Priority Budget) |
|---|---|
| `BlockActivity` enum (Fast/Medium/Slow/Inactive) | Single `f32` priority score per block |
| `should_process_block()` with tick modulos | Top-N selection via `select_nth_unstable` |
| 3 fixed frequency rates | Continuous priority spectrum |
| Manual threshold constants | Self-tuning via staleness × displacement |
| No FPS awareness | Closed-loop FPS targeting |

The physics loop simplifies to:

```rust
// 1. Score all blocks
for block in blocks {
    block.priority = block.last_displacement * (tick - block.last_simulated) as f32;
}

// 2. Always-simulate set (above critical threshold)
let (critical, rest) = partition(blocks, |b| b.last_displacement > CRITICAL);

// 3. Budget the rest: pick top N by priority
rest.select_nth_unstable_by(budget_n, |a, b| b.priority.partial_cmp(&a.priority));
let to_simulate = &rest[..budget_n];

// 4. Simulate critical + top-N
for block in critical.chain(to_simulate) {
    simulate(block);
    block.last_simulated = tick;
}

// 5. Adjust budget based on frame time
adjust_budget(&mut budget_n, frame_time, TARGET_FPS);
```

## Performance Estimate

- **Scoring**: ~700 blocks × 1 multiply = trivial
- **Selection**: `select_nth_unstable` is O(n) ≈ 700 ops ≈ ~1–2 μs
- **Overhead vs current**: Comparable or less (no tier bookkeeping, no modulo checks)

## Risks & Considerations

- **Temporal coherence**: Blocks that get deprioritized might cause visible "popping" when they suddenly get simulated after many stale ticks. Mitigation: cap max staleness so nothing goes unsimulated for more than ~15 ticks.
- **Oscillation**: Budget N bouncing up and down could cause inconsistent behavior. Mitigation: use EMA smoothing and hysteresis (only adjust if frame time is consistently over/under target).
- **Cross-block dependencies**: Sand flowing from a simulated block into an unsimulated neighbor could cause inconsistencies. Mitigation: when a block is simulated and pushes flow into a neighbor, mark that neighbor as "dirty" which boosts its priority for the next tick (similar to current `activate_neighbor`).
- **Minimum simulation rate**: Even low-priority blocks should be simulated at least once every ~15 ticks to avoid drift. The staleness factor in the priority score naturally handles this — a block that hasn't been touched in 15 ticks will have high priority regardless of its displacement.

## Implementation Phases

1. **Phase 1**: Add per-block `last_displacement` and `last_simulated_tick` tracking alongside the current tier system (no behavior change).
2. **Phase 2**: Implement priority scoring and top-N selection as an alternative to the tier system, behind a feature flag.
3. **Phase 3**: Add adaptive budget adjustment based on frame time.
4. **Phase 4**: Remove the old tier system if the priority approach proves superior.

## Related

- Current implementation: `settle_tick` in `sandart-sim/src/physics.rs`
- Current tier thresholds: `FLOW_FAST_THRESHOLD`, `FLOW_MEDIUM_THRESHOLD`, `FLOW_INACTIVE_THRESHOLD` constants in `settle_tick`
