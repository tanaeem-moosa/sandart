---
name: ask-what-visual-quality-words-mean
description: Pin down what a visual quality word means before building — inferring cost three wasted implementations
metadata: 
  node_type: memory
  type: feedback
  originSessionId: 6dbad8f7-de15-4c1a-aae8-0d4d41f500d8
  modified: 2026-08-02T03:05:59.833Z
---

When this user names a visual quality — "grainy", "randomness", "towery", "thunder" — ask
which measurable quantity they mean BEFORE designing anything. Do not infer it from the
word.

**Why:** "I can see the loss of randomness" cost three full implementations aimed at the
wrong quantity. I read it as surface texture and flow scatter, and built dispersion
scaling, randomized capacity arbitration (+50% ms/tick for ~0.1% gain), and grain
quantisation — all rejected. They meant randomness in COLOUR AND PROPERTY MIXING, which
is a different subsystem entirely (`advect_properties`), carries none of the physics
trade-offs, and is nearly free. One question would have saved all three.

**How to apply:** when a quality word arrives, name two or three candidate quantities and
ask which one they see — or state the one you're targeting explicitly before starting, so
a wrong reading is caught in a sentence rather than an agent run. Their answers are
precise once asked; see [[user-reports-mechanisms-not-symptoms]]. Note the mirror-image
risk: a metric can also be right about the quantity and wrong about where in parameter
space to measure it.
