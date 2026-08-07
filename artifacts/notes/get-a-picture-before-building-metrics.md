---
name: get-a-picture-before-building-metrics
description: "For visual defects, get a photo or render frames BEFORE designing a metric — aggregate numbers found the wrong defect twice"
metadata: 
  node_type: memory
  type: feedback
  originSessionId: 6dbad8f7-de15-4c1a-aae8-0d4d41f500d8
  modified: 2026-08-02T04:41:12.375Z
---

When this user reports a visual defect, get an image before building any metric. Ask them
for a photo, or render the sim state to PNG from a Rust test (no browser needed — see
[[no-working-browser-driver-in-this-environment]]). Ask for contact sheets, a grid of many
frames in one PNG, so a whole sequence costs one image read instead of a hundred.

**Why:** on the 2026-08-01 "sand falls in slabs" bug, an agent built a careful void/parity
diagnostic and found clean 2-cell row banding — real, but NOT the reported defect. One
photo from the user showed metre-scale slabs hanging with razor-straight full-width gaps,
which no aggregate statistic had captured. The photo also settled "is it the renderer"
instantly: the gaps' ragged right ends followed the eroded material edge, which a renderer
dropping rows could not produce. Three hypotheses died on one image.

**How to apply:** state what the image would distinguish before asking for it. Sharp binary
edges mean a boolean mask (activation, sleeping, block scheduling); gradations mean a
continuous density field — that single distinction redirected the whole investigation. Also
ask the user for the exact repro recipe early; theirs ("Circle at 64, flip a fully resting
sand") was cheaper and more reliable than the scenario an agent invented, and the detail
that it was WORSE at low resolution was the clue that identified the bug. See
[[user-reports-mechanisms-not-symptoms]] and [[ask-what-visual-quality-words-mean]].
