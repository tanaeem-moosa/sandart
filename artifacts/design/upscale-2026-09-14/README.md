# Picture crops -- artifacts/design/upscale-2026-09-14/

Every crop is a 128x128 sample of the original 512 grid, magnified 4x nearest-neighbour to 512x512 (no new information -- purely so staircasing is legible). Grayscale =height; orange = the coverage boundary (h>=0.003 for R0-R4, coverage>=0.5 for R5).

## stream_neck
crop = (x0=192, y0=246, w=128, h=128), vmax=0.55

`stream_neck_contact_sheet.png`: 5 columns x 2 rows. Row 1 = m=2, row 2 = m=4. Columns left to right:

| col 1 | col 2 | col 3 | col 4 | col 5 |
|---|---|---|---|---|
| original | R1 bilinear (shipped) | R3 face-match | R4 PLIC | R5 PLIC+AA |

## pool_wall_edge
crop = (x0=0, y0=360, w=128, h=128), vmax=0.55

`pool_wall_edge_contact_sheet.png`: 5 columns x 2 rows. Row 1 = m=2, row 2 = m=4. Columns left to right:

| col 1 | col 2 | col 3 | col 4 | col 5 |
|---|---|---|---|---|
| original | R1 bilinear (shipped) | R3 face-match | R4 PLIC | R5 PLIC+AA |

## sand_slope
crop = (x0=180, y0=307, w=128, h=128), vmax=0.5

`sand_slope_contact_sheet.png`: 5 columns x 2 rows. Row 1 = m=2, row 2 = m=4. Columns left to right:

| col 1 | col 2 | col 3 | col 4 | col 5 |
|---|---|---|---|---|
| original | R1 bilinear (shipped) | R3 face-match | R4 PLIC | R5 PLIC+AA |

