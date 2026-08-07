# #48 — 2.25 — Rename "colour" to "color" throughout the codebase

**Status:** completed

---

User request 2026-08-01: "I don't know how we ended up with colour. can you rename them to color when you get a chance". Explicitly low priority ("when you get a chance").

Scope: identifiers, comments, doc comments, test names, and user-facing strings across all crates and the web UI. American spelling throughout.

## Sequencing — do NOT start this while other work is in flight
A mass rename conflicts with everything. Run it only when the tree is otherwise clean and nothing else is mid-edit. As of 2026-08-01 there was a concurrent 688-line diff in physics.rs (#47), which is exactly the situation to avoid.

## Cautions
- Do NOT change load-bearing option VALUES in the web UI: `solid`, `gradient`, `stripes`, `concentric`, `checkerboard`, `rainbow_linear`, `rainbow_radial`. These were broken once before by a rewrite that changed values rather than labels. They are already American-spelled; leave them alone.
- Check for "colour" inside string literals that are asserted against in tests, and inside any URL or external identifier, before a blanket replace.
- Watch for case variants: `colour`, `Colour`, `COLOUR`, and compounds like `colours`, `colour_mode`, `cell_colours`.
- Prefer a reviewed pass over a blind `sed`. Run the full suite afterwards; baseline 89 pass / 1 fail (the intentional symmetry test) / ignored count per current main.
- Task titles and descriptions in this task list also use "colour" in places (#12, #16, #31, #43, #46). Cosmetic only, not required.
