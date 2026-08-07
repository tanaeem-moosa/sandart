# #29 — 2.6 — Show a build version in the UI so a refresh is verifiable

**Status:** completed

---

The user tests exclusively on the GitHub Pages deployment and currently has no way to tell whether a refresh actually picked up a new build. Every visual verification is therefore untrustworthy — "it still looks wrong" and "I am looking at the old bundle" are indistinguishable.

Show a build identifier in the panel. Must be derived at BUILD time (short git SHA, plus a timestamp), work both in CI (.github/workflows/deploy.yml, `wasm-pack build sandart-wasm --target web`) and in local builds, and degrade gracefully rather than failing the build when git is unavailable.

Note the caching interaction is a FEATURE, not a problem: if the browser serves a stale cached bundle, the stamp shows the old value, which is exactly the signal the user wants. Do not try to defeat caching — just make the displayed value honestly reflect the bundle actually running.
