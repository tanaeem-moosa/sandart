# #11 — Add edge sleeping to flux solver and measure perf

**Status:** completed

---

Perf work on the flux solver. Edge sleeping is IN PROGRESS (agent aa3c536c...).
Dirty-region colour sync is DONE and committed (fe2701a8).

--- REMAINING: u16 vs f32 colour buffer — NOW CONTENTIOUS, read this first ---

Original idea: the ~13% suite regression from f32 colour is mostly cache pressure from the
buffer going 1MB -> 4MB and being touched in the inner loop. u16 is 2MB with 256x headroom
over u8; expected error ~1e-5, still ~100x inside the 0.001 tolerance now asserted in
test_hourglass_color_and_property_conservation_under_gravity.

USER OBSERVATION (2026-07-28) THAT COMPLICATES THIS: after the f32 switch the user reported
being "surprised how much color has improved visually" and specifically that they can now
see sand movement causing colour lines to BEND.

Mechanism: u8's failure was not rounding noise, it was systematic ERASURE of slow change. A
blend nudging a channel by < 0.5 LSB rounds to zero and is discarded ENTIRELY, every tick, so
it never accumulates. Slow gradual deformation (a line bending as sand creeps) is exactly the
case that produces sub-LSB increments, so it was absent rather than muted. f32 accumulates at
full precision and quantises once at display.

WHY THIS BLOCKS A NAIVE u16 SWITCH: the benefit the user is seeing is PERCEPTUAL and the
conservation tolerance does not measure it. u16 raises the erasure floor by 256x, which is
probably sufficient — but "probably" is load-bearing and no existing test can tell us. The
7.4% colour-mass loss under u8 was also not evenly spread; it concentrated in slow-moving
regions, which is where the visual detail lives.

IF ATTEMPTED: do not decide on conservation error alone. Need either a perceptual test (e.g.
run a slow shear and assert a colour boundary actually displaces by N cells over M ticks,
rather than staying pinned) or explicit user sign-off on the visual after seeing it. Measure
the actual cache saving FIRST — if it is small, drop the idea entirely and keep f32.

--- context for edge sleeping (in progress) ---

Benchmark baseline (cargo run --release --example bench_sandfall -p sandart-sim):
  material   budget   ms/tick   must  budgeted  stale
  Water        1024    4.4961   87.0     6.6    31.0
  Water          32    4.1913   85.2     0.0    31.3
  DrySand      1024    8.9620  238.1     0.0    26.3
  DrySand        32    8.9330  238.2     0.0    26.3

KEY: budget_n is nearly irrelevant — must-simulate blocks dominate and bypass it. Reducing
must-count is the only real lever. Also: Water on flux is already ~2x FASTER than DrySand on
the CA, which contradicts the "Stage B costs 2x" assumption Stage B was parked on.
