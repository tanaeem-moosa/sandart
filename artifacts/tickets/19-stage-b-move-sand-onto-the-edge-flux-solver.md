# #19 — Stage B: move sand onto the edge-flux solver

**Status:** completed

---

Moved the vertical/gravity-aligned edge for granular material onto flux_edge (weight=1.0, tau=0, blended (c_sq,damping) that reduces exactly to wave_params at cell_liquidity=1). CA's avalanche valve + main flow loop now exclude ndy!=0 under gravity to avoid double-counting. Lateral/repose CA behavior left untouched (future increment). DrySand 8.756ms -> 3.43ms/tick @ budget 1024 (2.55x), Water unchanged at 1.3ms. All 69 baseline tests pass + 1 new flowing-state test added (70 total), 4 ignored unchanged.
