# #42 — 2.19 — Sand drains as a thin central channel fed from the top, not from the sides at depth

**Status:** completed

---

FIXED and pushed: Stage C (edda8630) + lateral overburden pressure (21ca3843). Live on Pages.

## ROOT CAUSE

Sand had NO depth-integrated lateral pressure. `column_depth` was gated on `cell_liquidity > 0.0`, so a pure granular cell's value stayed 0 and its lateral driving reduced to the local fill difference between adjacent cells. A grain ten cells down felt exactly what a grain at the surface felt. That IS the user's report ("pressure from the side means most sand should drain from the side at some depth") — there was no such pressure for sand.

It also explains why Jacobi transformed liquid and left sand behind: liquid had an overburden term for arbitration to act on; sand had nothing.

## THE FIX

`column_depth` accumulated for every material. How much pushes sideways = LATERAL_EARTH_PRESSURE_K, applied at the READ site (never the store site — the accumulation is a running sum, so scaling at store would compound with depth). Blended per cell by its own liquidity, so liquid stays bit-identical.

K = 0.45 from Jaky (K0 = 1 - sin phi, giving 0.43-0.50 for phi = 30-35 deg). Derived, not fitted.

  K       sand f_50   sand white@10%   liquid f_50
  0.00      0.118        0.439           0.6439423810763675
  0.25      0.631        0.0002          same
  0.45      0.587        0.0009          same
  0.75      0.550        0.0001          same
  1.00      0.523        0.0008          same

Liquid bit-identical to the last digit at every K. K=0 reproduces Stage C alone. Any K>0 transforms sand: white@10% collapses 0.439 -> ~0 (the ideal), f_50 0.118 -> 0.52-0.63 vs liquid 0.644, ideal 0.774.

Higher K gives slightly LOWER f_50 (more lateral flow = more depth mixing). K=0.25 scores marginally best; 0.45 kept as the physical value.

## OTHER RESULTS

- Repose 2.44 -> 5.08 deg (Stage C), still 5.07 with overburden — the flagged trade-off did NOT materialise.
- DrySand 10.61 -> 2.71 ms/tick at 512 (3.9x FASTER), from skipping the CA's RNG draws, get_ca_params, marble search and 4-neighbour loop.
- Mass conservation 4.2e-9..1.1e-8, positivity exact.
- tau lives in GRANULAR_TAU_SCALE — one value to retune the repose angle, as the user asked.

## TEST ASSERTION CORRECTED (not weakened)

test_dry_sand_has_angle_of_repose required CASE 2 (pile built SHALLOWER than repose) to RISE toward the angle and converge with CASE 1. Both encoded creep-dominated behaviour. With a real yield stress a sub-threshold pile is STABLE — measured 0.0532 -> 0.0536. A material that spontaneously steepened under gravity would be unphysical. The user's original design brief said shallower "must STAY PUT"; the assertions had drifted from it.

Replaced with three load-bearing assertions: held position (rules out creep), sits strictly below CASE 1 (else "it held" proves nothing), did not collapse toward flat (the failure the test exists to catch). NON-VACUITY ANCHOR untouched and verified still firing: with GRANULAR_TAU_SCALE=0.0 it fails at dry=0.0143 vs water 0.0000.

## STILL OPEN

- Galton board scatter narrowed under Stage C (std_x 40.97 -> 25.22, confounded by slower drainage). NOT re-measured after the overburden fix — check whether it recovers.
- Repose is 5 deg; real dry sand is 32-35. Raising GRANULAR_TAU_SCALE is the one-value change, to be tuned visually.
- Perf at 512 for WATER still unmeasured since Jacobi.
