---
name: no-working-browser-driver-in-this-environment
description: Agents cannot screenshot the app — flatpak Chrome reports --screenshot success but writes no host-visible file
metadata: 
  node_type: memory
  type: project
  originSessionId: 6dbad8f7-de15-4c1a-aae8-0d4d41f500d8
  modified: 2026-07-31T18:51:16.194Z
---

There is no working browser driver for the sandart app on this machine. Flatpak-sandboxed
Chrome's `--screenshot` exits 0 but the PNG never appears on the host filesystem, and no
playwright or chromium-cli is installed. Do not claim visual verification happened; a
subagent reporting a screenshot read may have read a stale or nonexistent file.

**Why:** I once told the user an agent had "driven the real page and screenshotted it"
because I saw a `Read` of a .png in its log. The agent's own final report said the
screenshot never materialised. That was a false assurance about the one check the user
cannot do for me.

**How to apply:** UI work ships on Rust tests, `node --check`, and code tracing, and the
commit message and task must say plainly that no browser check was done. The user does the
visual pass themselves — see [[user-tests-via-github-pages-deployment]]. Worth revisiting
only if a driver gets installed.
