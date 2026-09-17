# Picture crops -- artifacts/design/upscale-2026-09-14/

Every crop is a 128x128 sample of the 512 grid, magnified 4x nearest-neighbour to 512x512 (no new information -- purely so staircasing is legible). Grayscale = height; orange = the coverage boundary (`h>=0.003` for R0-R4; for R5/R6/R7, `coverage>=0.5 AND coverage*height>=0.003` -- round 3's fix for the film-case fleck defect, see the writeup §4.3). Figure name suffix states the snapshot: `_a_raw_` = raw per-tick snapshot (a), `_c_ema_` = the alpha=0.4 EMA over the last 15 ticks (snapshot c, what the deployed page actually displays), `_b_dry_` = the DrySand snapshot (b, no EMA counterpart). `stream_neck` and `pool_wall_edge` are shown at BOTH (a) and (c), at the IDENTICAL crop region, specifically so temporal smoothing's effect on speckle/jitter can be read directly off the two sheets for the same feature.

## stream_neck_a_raw
crop = (x0=192, y0=246, w=128, h=128), vmax=0.55

`stream_neck_a_raw_contact_sheet.png`: 7 columns x 2 rows. Row 1 = m=2, row 2 = m=4. Columns left to right:

| col 1 | col 2 | col 3 | col 4 | col 5 | col 6 | col 7 |
|---|---|---|---|---|---|---|
| original | R1 bilinear (shipped) | R3 face-match | R4 PLIC | R5 PLIC+AA | R6 strip+AA | R7 strip+sat |

## stream_neck_c_ema
crop = (x0=192, y0=246, w=128, h=128), vmax=0.55

`stream_neck_c_ema_contact_sheet.png`: 7 columns x 2 rows. Row 1 = m=2, row 2 = m=4. Columns left to right:

| col 1 | col 2 | col 3 | col 4 | col 5 | col 6 | col 7 |
|---|---|---|---|---|---|---|
| original | R1 bilinear (shipped) | R3 face-match | R4 PLIC | R5 PLIC+AA | R6 strip+AA | R7 strip+sat |

## pool_wall_edge_a_raw
crop = (x0=0, y0=360, w=128, h=128), vmax=0.55

`pool_wall_edge_a_raw_contact_sheet.png`: 7 columns x 2 rows. Row 1 = m=2, row 2 = m=4. Columns left to right:

| col 1 | col 2 | col 3 | col 4 | col 5 | col 6 | col 7 |
|---|---|---|---|---|---|---|
| original | R1 bilinear (shipped) | R3 face-match | R4 PLIC | R5 PLIC+AA | R6 strip+AA | R7 strip+sat |

## pool_wall_edge_c_ema
crop = (x0=0, y0=360, w=128, h=128), vmax=0.55

`pool_wall_edge_c_ema_contact_sheet.png`: 7 columns x 2 rows. Row 1 = m=2, row 2 = m=4. Columns left to right:

| col 1 | col 2 | col 3 | col 4 | col 5 | col 6 | col 7 |
|---|---|---|---|---|---|---|
| original | R1 bilinear (shipped) | R3 face-match | R4 PLIC | R5 PLIC+AA | R6 strip+AA | R7 strip+sat |

## sand_slope_b_dry
crop = (x0=180, y0=307, w=128, h=128), vmax=0.5

`sand_slope_b_dry_contact_sheet.png`: 7 columns x 2 rows. Row 1 = m=2, row 2 = m=4. Columns left to right:

| col 1 | col 2 | col 3 | col 4 | col 5 | col 6 | col 7 |
|---|---|---|---|---|---|---|
| original | R1 bilinear (shipped) | R3 face-match | R4 PLIC | R5 PLIC+AA | R6 strip+AA | R7 strip+sat |

