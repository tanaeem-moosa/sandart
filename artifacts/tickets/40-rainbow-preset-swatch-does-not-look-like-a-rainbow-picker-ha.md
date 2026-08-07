# #40 — 2.17 — Rainbow preset swatch does not look like a rainbow (picker half DONE)

**Status:** completed

---

Colour-picker half is DONE and pushed in 002bc36b. Root cause was `syncColorTheme`'s preset branch reassigning `colorInput1/2.value` on the very same `input` event the picker fired, overwriting the chosen colour microseconds later. Fixed by having a direct edit to a colour field set the preset select to `custom` first, so authority goes to whichever control the user touched last. The picker's show/hide behaviour was NOT the problem and the user confirmed it is fine as-is.

# REMAINING: the rainbow swatch reads as generic
User: "the rainbow option at the end is gone. but if I select the last one it makes the selection rainbow linear."

It is NOT gone. `#preset-swatches`' last entry is `data-preset="rainbow"`, and clicking it sets `patternSelect.value = 'rainbow_linear'` — exactly the reported behaviour. The problem is purely visual: it uses the same two-colour diagonal split as every other preset, `--c1: #ff2d55; --c2: #2d9bff`, via `.swatch-split`'s `linear-gradient(135deg, var(--c1) 0 50%, var(--c2) 50% 100%)`. It renders as generic pink/blue, so nothing identifies it as the rainbow option.

Fix: give that one swatch a genuine multi-hue gradient instead of the two-stop split. Keep `.swatch` sizing, border, hover and `.active` treatment so it stays consistent with the grid — only the fill changes.

Worth doing at the same time: the grid is `grid-template-columns: repeat(7, 1fr)` with 7 swatches in a 328px sidebar, so each is roughly 34px. A smooth 6-stop spectrum at that size can read as mud — a 3-4 stop gradient may be more legible. Check at the real rendered size.

## Must not regress
- All colour schemes still selectable and rendering.
- Colour must survive reset and shape change (fixed in f652ec3 — do not reintroduce a wipe).
- Do NOT hand-add `<option>` elements to the material `<select>` (populated from `list_materials()`), and do not alter colour-mode option VALUES (`solid`, `gradient`, `stripes`, `concentric`, `checkerboard`, `rainbow_linear`, `rainbow_radial`) — they are load-bearing and were broken once by a rewrite that changed them rather than the labels.
