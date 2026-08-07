---
name: user-reports-mechanisms-not-symptoms
description: "This user describes bug mechanisms, not just appearances — those descriptions have repeatedly exposed bad test instruments"
metadata: 
  node_type: memory
  type: user
  originSessionId: 6dbad8f7-de15-4c1a-aae8-0d4d41f500d8
  modified: 2026-07-31T22:12:28.714Z
---

When this user reports a visual bug they describe the MECHANISM, not just "looks wrong":
"the middle one has no outlet", "a thin line shooting out with nothing under it",
"it used to fully empty in the middle and refill from the side, now it just lowers".
Treat those phrasings as precise technical claims and check them literally.

**Why:** every instrument failure on this project was caught by one of these, not by a
test. The tendril reproduction measured a splash pool while they were reporting
hairlines, and six candidate fixes were judged against it. Ten of twelve cascade
chamber counts had a neck draining onto solid wall while the drainage test passed,
because sand still reached the bottom by piling up and spilling over. The tests
measured a quantity ADJACENT to the defect and stayed green.

**How to apply:** when a report and a green test disagree, suspect the test first.
Before trusting any metric, ask what it would read if the reported defect were
present — if the answer is "the same", it is the wrong metric. Related:
[[no-working-browser-driver-in-this-environment]] means they are the only source of
visual truth, so their wording is the highest-quality signal available.
