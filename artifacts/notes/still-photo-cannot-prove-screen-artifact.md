---
name: still-photo-cannot-prove-screen-artifact
description: Never call a visual feature a camera/display artifact from a still photo
metadata: 
  node_type: memory
  type: feedback
  originSessionId: 6dbad8f7-de15-4c1a-aae8-0d4d41f500d8
  modified: 2026-08-05T03:13:04.413Z
---

Do not diagnose a feature in a photo of the screen as moiré, subpixel aliasing, or
any other capture artifact. A still cannot distinguish it from real data, and the
user watching it live can.

**Why:** on 2026-08-04 I called a fine constant-pitch comb with cyan/magenta
fringing "LCD subpixel moiré" and told the user not to investigate it. It was the
negative tendrils — a real advected structure they had already named. Their
correction was one fact I had no access to: "they seems to move." Moiré is fixed to
the screen, so movement settles it instantly.

**How to apply:** when a regular fine pattern shows up in a photo, ask "does it move
with the material?" instead of reasoning from the crop. If it is real, a
constant pitch at the grid scale is a strong clue in itself — a wavelength set by
the discretisation rather than the flow, i.e. a grid-scale odd-even mode.
Related: [[user-reports-mechanisms-not-symptoms]], [[get-a-picture-before-building-metrics]],
[[no-working-browser-driver-in-this-environment]].
