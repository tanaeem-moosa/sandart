# artifacts/

Working material that used to live outside source control — in an assistant session's task
store and scratchpad — checked in on 2026-08-07 so it survives the session that produced it.

Start with **[HANDOVER.md](HANDOVER.md)**. Everything else here is reference material it links into.

| Directory | What is in it |
|---|---|
| `tickets/` | 57 tickets exported one file per ticket, plus [INDEX.md](tickets/INDEX.md). These are the real backlog — most carry measured numbers, ruled-out hypotheses and explicit "do not chase X" notes that are worth more than the titles suggest. |
| `design/` | Task briefs written before the larger pieces of work. `TASK55-*.md` are the hydraulic-head-field series; several describe designs that were **abandoned** and say so in HANDOVER. |
| `design/agent-briefs/` | The task descriptions handed to subagents, kept because they state scope and constraints more precisely than the resulting commits do. |
| `measurements/` | Raw stdout from diagnostics, plus two flamegraphs. Conclusions are already summarised in the tickets; these are the underlying evidence. |
| `notes/` | Working agreements and hard-won environment facts accumulated across sessions — how this project is tested, what does not work here, what wording traps to avoid. Short and worth reading before starting. |

## What is deliberately NOT here

- **Screenshots and photos.** The user's phone photos of the deployed build were the primary
  visual evidence for several tickets, but they are excluded by request. Where a photo drove a
  conclusion, the ticket describes what it showed.
- **Source snapshots.** The scratchpad held several ~900KB copies of `physics.rs` from
  mid-investigation states. They are stale copies of tracked source and were dropped.

## Provenance

Tickets are exported verbatim from the task store, including status. The numbering is the
session's own and has gaps (1–9, 17, 21 were deleted before this export). The `2.x` prefixes in
ticket titles are the user's own scheme and do not correspond to the ticket numbers.
