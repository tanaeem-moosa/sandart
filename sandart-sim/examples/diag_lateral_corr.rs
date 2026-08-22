//! LATERAL-COARSE-CORRECTION.md: does the coarse-grid flow correction buy lateral spread, what
//! does it cost, and where does the damping knob want to sit?
//!
//! Sweeps `--damping` (the under-relaxation on the coarse level's opinion) against the metric the
//! problem is actually about -- `spread`, the mass-weighted std-dev of x over the bottom quarter
//! -- plus the two things that would make a gain in spread worthless: descent (if the correction
//! stalls fall) and `mass_err` (if it is not conservative after all, which it must be by
//! construction, so a nonzero reading here is a bug and not a trade).
//!
//! Also prints the correction's own accounting: how much transport the coarse level asked for,
//! how much survived the availability/headroom limiter, and what fraction of faces were limited.
//! A high limited fraction is the signal that the coarse level is asking for transport the fine
//! level physically cannot supply, which is a finding about the two levels rather than a bug.
use glam::Vec2;
use sandart_sim::{DrawingSimulation, MaterialMode, SandboxShape};
use std::time::Instant;

/// Mass-weighted horizontal spread (std-dev of x, in cells) over the BOTTOM QUARTER -- the same
/// definition `diag_blocks` uses, so the numbers are directly comparable across the two.
fn spread(sim: &DrawingSimulation) -> f64 {
    let w = sim.heightmap.width;
    let h = sim.heightmap.height;
    let y0 = h - h / 4;
    let (mut m, mut mx, mut mxx) = (0.0f64, 0.0f64, 0.0f64);
    for y in y0..h {
        for x in 0..w {
            let v = sim.heightmap.data[y * w + x] as f64;
            if v <= 0.0 {
                continue;
            }
            m += v;
            mx += v * x as f64;
            mxx += v * (x * x) as f64;
        }
    }
    if m <= 0.0 {
        return 0.0;
    }
    let mean = mx / m;
    (mxx / m - mean * mean).max(0.0).sqrt()
}

/// SEAM METRIC. Mean absolute height step across cell boundaries that ARE block boundaries,
/// divided by the same quantity across boundaries that are NOT. A value near 1.0 means block
/// boundaries look like every other column/row; a value well above 1.0 means there is a
/// discontinuity at the block grid -- which is what "I can see block edges" reports.
///
/// Returned as `(lateral_seam, vertical_seam)`: lateral compares column steps at `x % bs == bs-1`,
/// vertical compares row steps at `y % bs == bs-1`. Only counts pairs where both cells hold
/// material, so an empty region cannot dilute the statistic toward 1.0.
fn seam_ratio(sim: &DrawingSimulation, bs: usize) -> (f64, f64) {
    let w = sim.heightmap.width;
    let h = sim.heightmap.height;
    let d = &sim.heightmap.data;
    let (mut b_sum, mut b_n, mut i_sum, mut i_n) = (0.0f64, 0u64, 0.0f64, 0u64);
    for y in 0..h {
        for x in 0..w - 1 {
            let (a, b) = (d[y * w + x], d[y * w + x + 1]);
            if a <= 1e-4 || b <= 1e-4 {
                continue;
            }
            let step = (a - b).abs() as f64;
            if x % bs == bs - 1 {
                b_sum += step;
                b_n += 1;
            } else {
                i_sum += step;
                i_n += 1;
            }
        }
    }
    let lat = if b_n > 0 && i_n > 0 && i_sum > 0.0 {
        (b_sum / b_n as f64) / (i_sum / i_n as f64)
    } else {
        0.0
    };
    let (mut b_sum, mut b_n, mut i_sum, mut i_n) = (0.0f64, 0u64, 0.0f64, 0u64);
    for y in 0..h - 1 {
        for x in 0..w {
            let (a, b) = (d[y * w + x], d[(y + 1) * w + x]);
            if a <= 1e-4 || b <= 1e-4 {
                continue;
            }
            let step = (a - b).abs() as f64;
            if y % bs == bs - 1 {
                b_sum += step;
                b_n += 1;
            } else {
                i_sum += step;
                i_n += 1;
            }
        }
    }
    let vert = if b_n > 0 && i_n > 0 && i_sum > 0.0 {
        (b_sum / b_n as f64) / (i_sum / i_n as f64)
    } else {
        0.0
    };
    (lat, vert)
}

/// Centre of mass, normalised to [0, 1] top to bottom -- descent.
fn com(sim: &DrawingSimulation) -> f64 {
    let w = sim.heightmap.width;
    let h = sim.heightmap.height;
    let (mut num, mut den) = (0.0f64, 0.0f64);
    for y in 0..h {
        let r: f64 = sim.heightmap.data[y * w..(y + 1) * w].iter().map(|&v| v as f64).sum();
        num += r * y as f64;
        den += r;
    }
    num / den / (h - 1) as f64
}

struct Run {
    ms: f64,
    spread0: f64,
    spread1: f64,
    desc: f64,
    mass_err: f64,
    requested: f64,
    applied: f64,
    lateral: f64,
    limited_frac: f64,
    boundaries: f64,
    /// Executed block-steps per frame. SESSION-HANDOVER 2026-08-20 (evening) §2: frame time is
    /// executed block-steps at a steady ~29-31 us each, so this is what separates "the correction
    /// costs because it schedules more real physics" from "the correction costs because its own
    /// bookkeeping is expensive". If ms and block-steps rise together it is the former.
    steps: f64,
    seam_lat: f64,
    seam_vert: f64,
}

#[allow(clippy::too_many_arguments)]
fn run(
    grid: usize,
    mat: MaterialMode,
    ticks: usize,
    warm: usize,
    budget: usize,
    overclock: bool,
    on: bool,
    damping: f32,
    vertical: bool,
) -> Run {
    let mut sim = DrawingSimulation::new_with_size(grid);
    sim.sandbox_shape = SandboxShape::Hourglass;
    sim.gravity_dir = Vec2::new(0.0, 0.04);
    sim.apply_preset(mat);
    sim.initialize_hourglass();
    sim.overfill_pressure = true;
    sim.overclocking_enabled = overclock;
    sim.coarse_flow_correction = on;
    sim.coarse_correction_damping = damping;
    sim.coarse_correction_vertical = vertical;
    let targets = [None; 5];
    for _ in 0..warm {
        sim.budget_n = budget;
        sim.update(0.016, &targets, 0.08, mat, SandboxShape::Hourglass, 0.0, 16.6);
    }
    let m0: f64 = sim.heightmap.data.iter().map(|&v| v as f64).sum();
    let c0 = com(&sim);
    let spread0 = spread(&sim);
    let (mut req, mut app, mut lat, mut lim, mut bnd) = (0.0f64, 0.0f64, 0.0f64, 0u64, 0u64);
    let mut steps = 0u64;
    let t0 = Instant::now();
    for _ in 0..ticks {
        sim.budget_n = budget;
        sim.update(0.016, &targets, 0.08, mat, SandboxShape::Hourglass, 0.0, 16.6);
        let s = sim.last_frame_correction;
        req += s.requested;
        app += s.applied;
        lat += s.lateral_applied;
        lim += s.limited as u64;
        bnd += s.boundaries as u64;
        steps += sim.last_frame_block_steps as u64;
    }
    let ms = t0.elapsed().as_secs_f64() * 1000.0 / ticks as f64;
    let m1: f64 = sim.heightmap.data.iter().map(|&v| v as f64).sum();
    let n = ticks as f64;
    Run {
        ms,
        spread0,
        spread1: spread(&sim),
        desc: com(&sim) - c0,
        mass_err: (m1 - m0).abs() / m0.max(1e-12),
        requested: req / n,
        applied: app / n,
        lateral: lat / n,
        limited_frac: if bnd > 0 { lim as f64 / bnd as f64 } else { 0.0 },
        boundaries: bnd as f64 / n,
        steps: steps as f64 / n,
        seam_lat: seam_ratio(&sim, sim.block_size).0,
        seam_vert: seam_ratio(&sim, sim.block_size).1,
    }
}

fn print_row(
    label: &str,
    r: &Run,
    base_spread: Option<f64>,
    base_ms: Option<f64>,
    base_steps: Option<f64>,
) {
    let gain = r.spread1 - r.spread0;
    let vs = match base_spread {
        Some(b) => format!("{:+.1}%", (r.spread1 - b) / b.abs().max(1e-9) * 100.0),
        None => "   base".into(),
    };
    let vms = match base_ms {
        Some(b) => format!("{:+.0}%", (r.ms - b) / b.max(1e-9) * 100.0),
        None => "  base".into(),
    };
    println!(
        "{label:<26} ms {:>6.2} ({vms:>6})  spread {:>6.2}->{:>6.2} ({gain:+.2}, {vs:>7})  \
         desc {:+.5}  mass_err {:.2e}  req/tick {:>9.2}  applied {:>9.2} ({:>5.1}% of req, {:>5.1}% lateral)  \
         faces {:>6.1}  limited {:>5.1}%  block_steps {:>7.0} ({})  SEAM lat {:>5.2} vert {:>5.2}",
        r.ms,
        r.spread0,
        r.spread1,
        r.desc,
        r.mass_err,
        r.requested,
        r.applied,
        if r.requested > 0.0 { r.applied / r.requested * 100.0 } else { 0.0 },
        if r.applied > 0.0 { r.lateral / r.applied * 100.0 } else { 0.0 },
        r.boundaries,
        r.limited_frac * 100.0,
        r.steps,
        match base_steps {
            Some(b) => format!("{:+.0}%", (r.steps - b) / b.max(1e-9) * 100.0),
            None => "base".into(),
        },
        r.seam_lat,
        r.seam_vert,
    );
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let get = |n: &str| args.iter().position(|a| a == n).and_then(|i| args.get(i + 1)).cloned();
    let ticks: usize = get("--ticks").map(|v| v.parse().unwrap()).unwrap_or(300);
    let warm: usize = get("--warmup").map(|v| v.parse().unwrap()).unwrap_or(60);
    let budget: usize = get("--budget").map(|v| v.parse().unwrap()).unwrap_or(256);
    let grid: usize = get("--grid").map(|v| v.parse().unwrap()).unwrap_or(512);
    let overclock = get("--overclock").map(|v| v == "1").unwrap_or(false);
    let mats: Vec<MaterialMode> = match get("--material").unwrap_or_else(|| "both".into()).as_str() {
        "drysand" => vec![MaterialMode::DrySand],
        "water" => vec![MaterialMode::Water],
        _ => vec![MaterialMode::DrySand, MaterialMode::Water],
    };
    // The sweep. `0.0` is the control -- the correction is disabled, so it is the same physics as
    // the shipped tree, and every other row is measured against it.
    let dampings: Vec<f32> = match get("--damping") {
        Some(v) => v.split(',').map(|s| s.trim().parse().unwrap()).collect(),
        None => vec![0.25, 0.5, 1.0],
    };
    println!(
        "grid={grid} ticks={ticks} warmup={warm} budget={budget} overclock={overclock}\n\
         spread = mass-weighted std-dev of x over the bottom quarter (cells). Higher is more lateral spread.\n"
    );
    for mat in mats {
        println!("=== {mat:?} ===");
        let base = run(grid, mat, ticks, warm, budget, overclock, false, 0.0, true);
        print_row("correction OFF", &base, None, None, None);
        for &vertical in &[true, false] {
        for &d in &dampings {
            let r = run(grid, mat, ticks, warm, budget, overclock, true, d, vertical);
            print_row(
                &format!("{} strength {d:.3}", if vertical { "both  " } else { "lat-only" }),
                &r,
                Some(base.spread1),
                Some(base.ms),
                Some(base.steps),
            );
        }
        }
        println!();
    }
}
