//! Renders `SandboxShape::ChamberNetwork`'s mask straight from the SHIPPED sim (not the
//! prototype) at grid 256, one PNG per routing, into
//! `artifacts/design/network-2026-09-19/shipped_<routing>_mask.png` -- styled identically to
//! `proto_networks.rs`'s own `mask_image` (same three-colour scheme, same 2x nearest-neighbour
//! magnification) so the two can be compared pixel-for-pixel by eye against
//! `R1_g4_baseline_mask.png` / `R2_neighbours_mask.png` / `R5_butterfly_mask.png`.
//!
//!   distrobox enter sandart-dev -- bash -lc \
//!     'cd /home/deck/projects/sandart && CARGO_BUILD_JOBS=2 cargo run -p sandart-sim --release --example dump_chamber_network_masks'

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

fn main() {
    let out_dir = Path::new("artifacts/design/network-2026-09-19");
    fs::create_dir_all(out_dir).expect("create output dir");

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
}
