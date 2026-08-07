---
name: delegate-to-sonnet-by-default
description: "Default to delegating implementation work to Sonnet subagents rather than doing it inline, to conserve tokens"
metadata: 
  node_type: memory
  type: feedback
  originSessionId: 6dbad8f7-de15-4c1a-aae8-0d4d41f500d8
  modified: 2026-07-29T00:01:17.222Z
---

Delegate implementation work to Sonnet subagents by default instead of doing it inline in the main thread.

**Why:** Stated directly on 2026-07-28 — "next time let's delegate work to sonnet to save tokens". The main thread burns Opus tokens on long edit/verify loops (a multi-part shape + UI change ran very long inline), and Sonnet handles well-specified implementation fine. Earlier in the same session they had already asked for Sonnet specifically on #15 and Stage B.

**How to apply:** Write a self-contained brief (baseline numbers, constraints, must-not-regress list, the specific trap that would produce a plausible-but-wrong fix) and hand it to a Sonnet agent. Keep for the main thread: choosing what to work on, verifying the agent's claims independently, and taste-dependent design calls. Always re-run tests/benchmarks yourself rather than trusting reported numbers — agents have reported wrong signs and wrong mechanisms before. See [[verify-subagent-claims-independently]].
