# #23 — Fix Galton board peg lattice so no clear vertical channels form

**Status:** completed

---

sandart-sim/src/physics.rs:1071, GaltonBoard. Sand falls straight through in vertical lines because the row stagger is cancelled out and, separately, the pegs are too small to close the gap even if it weren't.

DEFECT 1 — the stagger is a no-op. Current code:
    let count = row + 3;
    let offset_x = if row % 2 == 1 { spacing * 0.5 } else { 0.0 };
    let start_x = -(count as f32 - 1.0) * spacing * 0.5 + offset_x;
    peg_x = start_x + i * spacing;

`(count - 1)/2 = (row + 2)/2` is an integer on even rows and a half-integer on odd rows. That half-integer already shifts the row by spacing/2, and then `offset_x` adds another spacing/2 on exactly those rows — the two cancel. Worked through with spacing = 8:
  row 0: count 3, start -8  -> pegs -8, 0, 8
  row 1: count 4, start -8  -> pegs -8, 0, 8, 16
  row 2: count 5, start -16 -> pegs -16, -8, 0, 8, 16
Every row lands on multiples of 8. There is no stagger at all, so the gaps at 4, 12, 20... are open shafts from top to bottom.

Fix: anchor pegs to a lattice that does not depend on `count` parity — e.g. peg_x = (j + 0.5 * (row % 2)) * spacing for integer j spanning the allowed half-width — so consecutive rows genuinely offset by half a spacing.

DEFECT 2 — pegs too small to close the gap even when staggered. peg_radius = 1.8, spacing = 8. Even rows cover x-intervals [8j - r, 8j + r]; odd rows [8j + 4 - r, 8j + 4 + r]. The union covers the whole line only if 8j + r >= 8j + 4 - r, i.e. r >= spacing/4 = 2.0. At r = 1.8 a 0.4-wide clear shaft survives at every x = 8j + 2 and 8j + 6. Raise the radius to about 2.2 for margin (or shrink spacing correspondingly).

Both are needed — fixing the stagger alone still leaves thin shafts.

Verify by construction, not by eye: for each column x, check that some peg covers it somewhere down the board. Cheap to assert in a test.
