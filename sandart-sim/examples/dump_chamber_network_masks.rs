//! Renders `SandboxShape::ChamberNetwork`'s mask straight from the SHIPPED sim (not the
//! prototype) at grid 256, one PNG per routing, into
//! `artifacts/design/network-2026-09-19/shipped_<routing>_mask.png` -- styled identically to
//! `proto_networks.rs`'s own `mask_image` (same three-colour scheme, same 2x nearest-neighbour
//! magnification) so the two can be compared pixel-for-pixel by eye against
//! `R1_g4_baseline_mask.png` / `R2_neighbours_mask.png` / `R5_butterfly_mask.png`.
//!
//! Also renders the "read as connected" geometry sweep (2026-09-19 task: smaller chamber corner
//! radius/inset, thicker pipes) as two labelled contact sheets:
//!
//!   - `geometry_sweep.png` -- several candidate `(corner radius, pipe half-width, chamber
//!     inset)` settings at routing R1, grid 256, each tile labelled with its numbers (as
//!     `/256`-fractions, matching how the constants are declared in `physics.rs`).
//!   - `geometry_chosen_all_routings.png` -- the geometry that ends up shipped as
//!     `NetGeometry::default()` (i.e. whatever `NET_CHAMBER_R_FRAC`/`NET_PIPE_HW_FRAC`/
//!     `NET_INSET_FRAC` currently are), rendered for all three routings, so the chosen setting
//!     can be checked against every shipped routing, not just R1.
//!
//!   distrobox enter sandart-dev -- bash -lc \
//!     'cd /home/deck/projects/sandart && CARGO_BUILD_JOBS=2 cargo run -p sandart-sim --release --example dump_chamber_network_masks'

use sandart_sim::physics::{chamber_network_mask_with_geometry, NetGeometry};
use sandart_sim::{DrawingSimulation, NetworkRouting, SandboxShape, MASK_BOUNDARY, MASK_OUTSIDE};
use std::fs;
use std::path::Path;

const GRID: usize = 256;

fn mask_image(mask: &[u8], w: usize, h: usize) -> image::RgbImage {
    let mut img = image::RgbImage::new(w as u32, h as u32);
    for y in 0..h {
        for x in 0..w {
            let m = mask[y * w + x];
            let px = match m {
                MASK_OUTSIDE => [40u8, 40, 44],
                MASK_BOUNDARY => [255, 255, 255],
                _ => [190, 190, 196],
            };
            img.put_pixel(x as u32, y as u32, image::Rgb(px));
        }
    }
    magnify_nearest(&img, 2)
}

fn magnify_nearest(img: &image::RgbImage, factor: u32) -> image::RgbImage {
    let (w, h) = img.dimensions();
    let mut out = image::RgbImage::new(w * factor, h * factor);
    for y in 0..h {
        for x in 0..w {
            let p = *img.get_pixel(x, y);
            for dy in 0..factor {
                for dx in 0..factor {
                    out.put_pixel(x * factor + dx, y * factor + dy, p);
                }
            }
        }
    }
    out
}

fn contact_sheet(tiles: &[image::RgbImage], cols: usize) -> image::RgbImage {
    let cols = cols.min(tiles.len().max(1));
    let rows = (tiles.len() + cols - 1) / cols;
    let (tw, th) = tiles.first().map(|t| (t.width(), t.height())).unwrap_or((1, 1));
    let pad = 8u32;
    let mut sheet = image::RgbImage::from_pixel(
        cols as u32 * (tw + pad) + pad,
        rows as u32 * (th + pad) + pad,
        image::Rgb([235, 235, 235]),
    );
    for (i, tile) in tiles.iter().enumerate() {
        let (col, row) = (i % cols, i / cols);
        let ox = pad + col as u32 * (tw + pad);
        let oy = pad + row as u32 * (th + pad);
        image::imageops::overlay(&mut sheet, tile, ox as i64, oy as i64);
    }
    sheet
}

// ---------------------------------------------------------------------------------------------
// Tiny built-in bitmap font (3 wide x 5 tall per glyph) -- just enough characters to label a
// tile with its geometry numbers ("R1 H11 I2") or its routing name ("R1"/"R2"/"R5") without
// pulling in a font-rendering dependency. '#' = ink, '.' = blank.
// ---------------------------------------------------------------------------------------------

fn glyph_rows(c: char) -> [&'static str; 5] {
    match c {
        '0' => ["###", "#.#", "#.#", "#.#", "###"],
        '1' => [".#.", "##.", ".#.", ".#.", "###"],
        '2' => ["###", "..#", "###", "#..", "###"],
        '3' => ["###", "..#", "###", "..#", "###"],
        '4' => ["#.#", "#.#", "###", "..#", "..#"],
        '5' => ["###", "#..", "###", "..#", "###"],
        '6' => ["###", "#..", "###", "#.#", "###"],
        '7' => ["###", "..#", "..#", "..#", "..#"],
        '8' => ["###", "#.#", "###", "#.#", "###"],
        '9' => ["###", "#.#", "###", "..#", "###"],
        '.' => ["...", "...", "...", "...", ".#."],
        ',' => ["...", "...", "...", ".#.", "#.."],
        '=' => ["...", "###", "...", "###", "..."],
        'R' => ["##.", "#.#", "##.", "#.#", "#.#"],
        'H' => ["#.#", "#.#", "###", "#.#", "#.#"],
        'I' => ["###", ".#.", ".#.", ".#.", "###"],
        _ => ["...", "...", "...", "...", "..."],
    }
}

fn draw_text(img: &mut image::RgbImage, text: &str, ox: u32, oy: u32, scale: u32, color: image::Rgb<u8>) {
    let mut cursor_x = ox;
    for c in text.chars() {
        let rows = glyph_rows(c);
        for (ry, row) in rows.iter().enumerate() {
            for (rx, ch) in row.chars().enumerate() {
                if ch == '#' {
                    for dy in 0..scale {
                        for dx in 0..scale {
                            let px = cursor_x + rx as u32 * scale + dx;
                            let py = oy + ry as u32 * scale + dy;
                            if px < img.width() && py < img.height() {
                                img.put_pixel(px, py, color);
                            }
                        }
                    }
                }
            }
        }
        cursor_x += 4 * scale; // 3 glyph columns + 1 column of spacing
    }
}

/// Appends a white label banner below `tile` and draws `text` into it in black.
fn with_label(tile: image::RgbImage, text: &str) -> image::RgbImage {
    let scale = 6u32;
    let banner_h = 5 * scale + 2 * scale; // glyph height + top/bottom margin
    let (w, h) = tile.dimensions();
    let mut out = image::RgbImage::from_pixel(w, h + banner_h, image::Rgb([255, 255, 255]));
    image::imageops::overlay(&mut out, &tile, 0, 0);
    draw_text(&mut out, text, scale, h + scale, scale, image::Rgb([0, 0, 0]));
    out
}

fn main() {
    let out_dir = Path::new("artifacts/design/network-2026-09-19");
    fs::create_dir_all(out_dir).expect("create output dir");

    // ---- Shipped masks, straight from the real sim pipeline (unchanged from before this task)
    for (name, routing) in [
        ("r1", NetworkRouting::R1),
        ("r2", NetworkRouting::R2),
        ("r5", NetworkRouting::R5),
    ] {
        let mut sim = DrawingSimulation::new_with_size(GRID);
        sim.sandbox_shape = SandboxShape::ChamberNetwork;
        sim.network_routing = routing;
        sim.generate_shape_mask();

        let inside = sim.shape_mask.iter().filter(|&&m| m != MASK_OUTSIDE).count();
        println!("{name}: {inside} inside cells of {}", GRID * GRID);

        mask_image(&sim.shape_mask, GRID, GRID)
            .save(out_dir.join(format!("shipped_{name}_mask.png")))
            .expect("write mask png");
    }

    // ---- Geometry sweep: several (corner radius, pipe half-width, chamber inset) candidates,
    // all as /256 fractions (matching how NET_CHAMBER_R_FRAC et al. are declared), rendered at
    // routing R1, grid 256. Ordered from the shipped baseline through progressively thicker
    // pipes / smaller corners and inset, ending in one deliberately-too-thick case to show where
    // it breaks.
    let candidates: [(&str, NetGeometry); 7] = [
        ("R3 H5 I4", NetGeometry { r_frac: 3.0 / 256.0, pipe_hw_frac: 5.0 / 256.0, inset_frac: 4.0 / 256.0 }),
        ("R2 H8 I3", NetGeometry { r_frac: 2.0 / 256.0, pipe_hw_frac: 8.0 / 256.0, inset_frac: 3.0 / 256.0 }),
        ("R1 H11 I2", NetGeometry { r_frac: 1.0 / 256.0, pipe_hw_frac: 11.0 / 256.0, inset_frac: 2.0 / 256.0 }),
        ("R1 H14 I1", NetGeometry { r_frac: 1.0 / 256.0, pipe_hw_frac: 14.0 / 256.0, inset_frac: 1.0 / 256.0 }),
        ("R1 H17 I1", NetGeometry { r_frac: 1.0 / 256.0, pipe_hw_frac: 17.0 / 256.0, inset_frac: 1.0 / 256.0 }),
        ("R0.5 H14 I1", NetGeometry { r_frac: 0.5 / 256.0, pipe_hw_frac: 14.0 / 256.0, inset_frac: 1.0 / 256.0 }),
        // Chosen: found not from the picture but from the sweep's own leak metric
        // (`diag_chamber_network_geometry_sweep`, run and then removed once this was picked) --
        // a large-hw / small-r candidate always leaked a diagonal outlet into its source row's
        // neighbour chamber, so corner radius went UP (buys clearance) while pipe half-width
        // went up more modestly and inset came down (flush pipe-to-wall meet).
        ("R9 H8 I2 (chosen)", NetGeometry { r_frac: 9.0 / 256.0, pipe_hw_frac: 8.0 / 256.0, inset_frac: 2.0 / 256.0 }),
    ];

    let sweep_tiles: Vec<image::RgbImage> = candidates
        .iter()
        .map(|(label, geo)| {
            let mask = chamber_network_mask_with_geometry(GRID, GRID, NetworkRouting::R1, *geo);
            let inside = mask.iter().filter(|&&m| m != MASK_OUTSIDE).count();
            println!("sweep {label}: {inside} inside cells of {}", GRID * GRID);
            with_label(mask_image(&mask, GRID, GRID), label)
        })
        .collect();

    contact_sheet(&sweep_tiles, 3)
        .save(out_dir.join("geometry_sweep.png"))
        .expect("write geometry_sweep.png");

    // ---- Chosen setting, all three routings -- whatever NetGeometry::default() currently is
    // (i.e. the module constants in physics.rs at the time this runs).
    let chosen_tiles: Vec<image::RgbImage> = [
        ("R1", NetworkRouting::R1),
        ("R2", NetworkRouting::R2),
        ("R5", NetworkRouting::R5),
    ]
    .iter()
    .map(|&(name, routing)| {
        let mask = chamber_network_mask_with_geometry(GRID, GRID, routing, NetGeometry::default());
        with_label(mask_image(&mask, GRID, GRID), name)
    })
    .collect();

    contact_sheet(&chosen_tiles, 3)
        .save(out_dir.join("geometry_chosen_all_routings.png"))
        .expect("write geometry_chosen_all_routings.png");
}
