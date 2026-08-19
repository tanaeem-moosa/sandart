//! If pressure were computed on a 128x128 tile grid (4x4 fine cells per tile at 512), would a
//! falling stream ever look "packed" to it? That is the property that keeps the hourglass intact.
use glam::Vec2;
use sandart_sim::{DrawingSimulation, MaterialMode, SandboxShape};
use sandart_sim::physics::cell_capacity_for;

fn main() {
    let grid: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(512);
    let coarse: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(128);
    let ticks: usize = std::env::args().nth(3).and_then(|s| s.parse().ok()).unwrap_or(400);
    let shape = match std::env::args().nth(4).as_deref() {
        Some("utube") => SandboxShape::UTubeFlowThrough,
        _ => SandboxShape::Hourglass,
    };
    let mut sim = DrawingSimulation::new_with_size(grid);
    sim.sandbox_shape = shape;
    sim.gravity_dir = Vec2::new(0.0, 0.04);
    sim.apply_preset(MaterialMode::Water);
    sim.initialize_hourglass();
    sim.overfill_pressure = true;
    let targets = [None; 5];
    for _ in 0..ticks { sim.budget_n = 1024; sim.update(0.016, &targets, 0.08, MaterialMode::Water, shape, 0.0, 16.6); }

    let (w, h) = (grid, grid);
    let t = grid / coarse; // fine cells per tile edge
    let falling = |i: usize| -> bool {
        let y = i / w;
        if y + 1 >= h { return false; }
        let b = i + w;
        if sim.shape_mask[b] == 0 { return false; }
        let cap = cell_capacity_for(sim.cell_props[b * 4]);
        sim.heightmap.data[b] / cap <= 0.98
    };
    let (mut tiles_wet, mut tiles_over, mut tiles_streamy, mut streamy_over) = (0u64, 0u64, 0u64, 0u64);
    let mut worst_stream_fill = 0.0f64;
    for ty in 0..coarse {
        for tx in 0..coarse {
            let (mut mass, mut capsum, mut n_in, mut n_fall, mut n_mat) = (0.0f64, 0.0f64, 0u64, 0u64, 0u64);
            for dy in 0..t { for dx in 0..t {
                let i = (ty * t + dy) * w + tx * t + dx;
                if sim.shape_mask[i] == 0 { continue; }
                n_in += 1;
                let cap = cell_capacity_for(sim.cell_props[i * 4]) as f64;
                capsum += cap;
                mass += sim.heightmap.data[i] as f64;
                if sim.heightmap.data[i] > 1e-4 { n_mat += 1; if falling(i) { n_fall += 1; } }
            }}
            if n_in == 0 || n_mat == 0 { continue; }
            tiles_wet += 1;
            let fill = mass / capsum.max(1e-12);
            if fill > 1.0 { tiles_over += 1; }
            // "streamy" = a tile where most of the material present is free-falling
            if n_fall * 2 >= n_mat {
                tiles_streamy += 1;
                if fill > 1.0 { streamy_over += 1; }
                if fill > worst_stream_fill { worst_stream_fill = fill; }
            }
        }
    }
    println!("{:?} {grid}x{grid}, pressure tiles {coarse}x{coarse} ({t}x{t} fine cells), {ticks} ticks", shape);
    println!("  tiles holding material          : {tiles_wet}");
    println!("  tiles at/over capacity (pressurised): {tiles_over}");
    println!("  tiles dominated by FALLING material : {tiles_streamy}");
    println!("  ...of those, over capacity          : {streamy_over}   <-- must be 0 to keep the hourglass");
    println!("  worst fill in a falling-dominated tile: {worst_stream_fill:.4} (capacity = 1.0)");
}
