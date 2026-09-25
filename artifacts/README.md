# artifacts/

Working material that used to live outside source control — in an assistant session's task
store and scratchpad — checked in on 2026-08-07 so it survives the session that produced it.
The project was declared done 2026-09-24; this directory was cleaned up and re-indexed at that
point (see `tickets/INDEX.md` for how). What remains is the record, not a live backlog.

**Start with `CLAUDE.md` at the repo root** — it is the authority on what the code currently does
and how to build/test it. `HANDOVER.md` below is historical.

## What is here

| Path | What is in it |
|---|---|
| `tickets/INDEX.md` | Full triage of all 59 exported tickets: shipped, deleted-with-their-subsystem, or still-open ideas. Read this before the ticket files themselves. |
| `tickets/*.md` | Only the 18 tickets still triaged OPEN — real ideas or real unfixed defects, mostly clustered around the hydraulic-head-field subsystem (#55, default-off) and its follow-on bugs (#62–#69). DONE and OBSOLETE tickets were removed; their text is in git history and their outcome is in the index. |
| `design/*.md` | Task briefs and findings written before/during the larger pieces of work, including several **rejected** designs — read before proposing a mechanism that resembles one of these; see `CLAUDE.md`'s "Before proposing a design" section. |
| `design/cascade-2026-09-17/`, `design/network-2026-09-19/`, `design/upscale-2026-09-14/` | Prototype contact sheets and masks for, respectively, the (later deleted) cascade redesign, the chamber-network vessel that replaced it, and the upscale/downscale reconstruction work. Kept in full as the visual evidence behind those design docs. |
| `HANDOVER.md` | Historical (superseded 2026-08-31, see the banner at its top) — the mid-project state before the overfill/coarse/block-clock deletion. Read it for why things were tried, never for what exists. |

Removed 2026-09-24 as process cruft with no reference from any design doc (checked by grep first):
`design/agent-briefs/` (subagent prompts), `notes/` (assistant session memory notes, duplicated
from the harness's own memory store), and `measurements/` (raw diagnostic stdout and two SVGs,
already summarised in the tickets/design docs that used them). None were cited by path from any
surviving document.

## Most important design records

- `design/ASYMMETRY-2026-09-08.md` — the mirror-axis bug (`w/2` vs `(w-1)/2`) that made every
  vessel in the app asymmetric by construction; fixed the persistent left-drift/tendril complaints
  (tickets #44, #56); the residual mid-drain asymmetry is what `test_sandbox_wave_stays_left_right_symmetric`
  and the #56 marker test still track.
- `design/SESSION-HANDOVER-2026-08-30.md` — the bisect and deletion record for the overfill
  pressure model, the hierarchical coarse level, and the block-clock scheduler (~13k lines,
  measured no benefit over three weeks).
- `design/LATERAL-COARSE-CORRECTION.md` — a design that measured *well* (+41% spread) and was
  killed anyway on visible seams; the reason `CLAUDE.md` insists on searching this directory
  before agreeing to a new mechanism.
- `design/KERNEL-BENCH-2026-09-13.md` — SIMD lateral-pass kernels benchmarked and refuted; the
  shipped scalar array-form path stays.
- `network-2026-09-19/README.md` and `design/ASYMMETRY-2026-09-08.md` §6 — the chamber-network
  vessel design (`SandboxShape::ChamberNetwork`) that replaced the deleted cascade, and its
  geometry rework history (thinner pipes, doglegs dropped for single-column routing).
- `upscale-2026-09-14/README.md` — the round-trip method (512 snapshot → downscale → upscale →
  compare) for judging any upscaling/reconstruction scheme.
- `design/TASK55-*.md` — the hydraulic-head-field series (max-propagation head, elliptic solve,
  multigrid attempt, perf work); the field it produced is shipped and default-off, see
  `tickets/INDEX.md` #55.
