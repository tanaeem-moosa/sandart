---
name: user-tests-via-github-pages-deployment
description: "User verifies sandart changes on the deployed GitHub Pages site, not local desktop or local wasm builds"
metadata: 
  node_type: memory
  type: feedback
  originSessionId: 6dbad8f7-de15-4c1a-aae8-0d4d41f500d8
  modified: 2026-08-06T20:45:18.050Z
---

The user tests sandart on the live deployment at tanaeem-moosa.github.io/sandart/, not by running the desktop binary or serving `sandart-wasm/web` locally. Stated on 2026-07-28 when I handed them a checklist that opened with `distrobox enter sandart-dev -- cargo run --release`.

**Why:** It matters for turnaround. Nothing is visually verifiable until it is **pushed to `main`** — `.github/workflows/deploy.yml` triggers only on push to main, builds with `wasm-pack build sandart-wasm --target web`, and publishes to `gh-pages`. So "ready to look at" means pushed, not committed.

**How to apply:** When writing up work that needs the user's eyes, push first and say the deploy is live, rather than giving local run instructions. Two consequences: (1) desktop-only changes (`sandart/src/app.rs`) are invisible to their testing — the wasm front end is what they see; (2) they may merge PRs on GitHub outside this session, so **check `git fetch origin main` before assuming local `main` is current** — this happened on 2026-07-28 (PR #1, string-keyed material presets, landed while local main sat two commits behind). See [[delegate-to-sonnet-by-default]].

**A push is not a deploy.** On 2026-08-06 the user reported not seeing a shipped feature; four consecutive pushes had landed their refs on GitHub (`git ls-remote origin refs/heads/main` confirmed) but fired **zero** workflow runs and zero check-runs, during a GitHub Actions incident. `gh-pages` stayed four commits behind and I had already told them it was "deploying now". Never say deployed on the strength of a successful push. Confirm the run exists: `curl -s https://api.github.com/repos/tanaeem-moosa/sandart/commits/<sha>/check-runs` (`total_count: 0` means nothing fired), and `git log -1 origin/gh-pages` names the source commit in its message. `gh` is NOT installed on this host; the unauthenticated GitHub API works because the repo is public. `deploy.yml` now also has `workflow_dispatch`, so a stalled deploy can be re-run from the Actions tab without inventing a commit.
