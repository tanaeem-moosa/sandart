# #14 — Fix sandbox wave instability from flux rewrite

**Status:** completed

---

USER REPORT: "water used to cause beautiful ripples that would reflect in sandbox mode.
now it is fully chaotic. with even emptying on sides."

BISECTED AND CONFIRMED: introduced by cce3b571 (Phase 5 Stage A, edge-flux solver).

Probe (saved at scratchpad/wave_probe_saved.rs): 0.80 gaussian bump on a flat 0.50 Water
pool, Circle shape, gravity=0, 400 ticks. Measures left/right mass asymmetry, peak height,
total mass, and fraction of mass in the outer ring.

  metric              pre-flux (2cbd9026)     post-flux (main)
  peak height         decays 0.80 -> 0.504    GROWS -> 1.0000, pinned
  mass drift          -3.93%                  +0.0004%
  asymmetry           0.0017                  0.0029

So the flux rewrite fixed a real -3.93% mass drift AND broke wave dynamics. Do not simply
revert; both properties matter.

CAPACITY HYPOTHESIS RULED OUT: user suggested the old 1.5 cap gave waves crest headroom and
the Phase 1 liquid cap of 1.0 removed it. Tested by raising liquid cap to 3.0: peak still
saturates, now at 3.0000, and asymmetry gets 33x WORSE (0.0029 -> 0.0996). The amplitude
grows until it hits whatever ceiling exists. This is genuine energy injection, not clipping.
Note the 1.0 cap was MASKING how bad the directional bias is.

ROOT CAUSE (high confidence, not yet proven): the old wave solver read heightmap.data and
wrote temp_heights - a Jacobi update over a fixed snapshot, which is symplectic and stable.
The flux form reads AND writes temp_heights live while sweeping, so each cell's height is
mutated by four separate edge updates within one pass using already-updated neighbour
values, with the sweep direction alternating on tick_count % 2. Gauss-Seidel on a wave
equation with a directional sweep injects energy. Damping is 0.98/tick, which over 400
ticks should crush any wave (0.98^400 ~ 3e-4), yet amplitude grows - so something is
pumping harder than damping removes.

FIX DIRECTION: keep the antisymmetric per-edge flux form (that is what made conservation
structural), but compute edge velocities from a STABLE SNAPSHOT of heights rather than from
temp_heights mid-mutation. Jacobi ordering + conservative flux gives stable symmetric
propagation AND conservation by construction - the old solver had the first, Phase 5 traded
it for the second, and there is no reason it has to be a trade.

SEPARATE BUT REAL (do not conflate): a pool sitting AT capacity has zero headroom for a
crest, so it cannot ripple at all. Distinct from this bug, worth its own fix.

TEST GAP: there is NO test covering sandbox wave dynamics. Every liquid test is
gravity-oriented except test_liquid_mass_conserved_in_sandbox_under_lod, which checks mass
only, never behaviour. Promote the probe to a permanent test: assert peak DECAYS toward
pool level and asymmetry stays bounded.
