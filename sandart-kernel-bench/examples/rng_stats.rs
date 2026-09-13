//! Standalone statistics check for kernel C's noise table (task rule 3: "Check there is no
//! visible spatial pattern: report the autocorrelation of each draw at lag 1 and lag 2 in x and in
//! y, and the lock fraction vs `GRAVITY_LOCK_CHANCE * granular_share`.").
//!
//! Not a `#[test]` (this crate's tests need the snapshot files present, see `lib.rs`'s module doc
//! comment) -- run explicitly:
//!   cargo run -p sandart-kernel-bench --release --example rng_stats -- [snapshot_dir]

use sandart_kernel_bench::consts::GRAVITY_LOCK_CHANCE;
use sandart_kernel_bench::noise::{Noise, SALT_DISPERSION, SALT_JITTER, SALT_LOCK};
use sandart_kernel_bench::snapshot;

fn autocorr_lag_x(rows: &[Vec<f32>], lag: usize) -> f64 {
    let mean = rows.iter().flatten().map(|&v| v as f64).sum::<f64>() / rows.iter().map(|r| r.len()).sum::<usize>() as f64;
    let mut num = 0.0f64;
    let mut den = 0.0f64;
    for row in rows {
        for i in 0..row.len() {
            den += (row[i] as f64 - mean).powi(2);
        }
        for i in 0..row.len().saturating_sub(lag) {
            num += (row[i] as f64 - mean) * (row[i + lag] as f64 - mean);
        }
    }
    num / den
}

fn autocorr_lag_y(rows: &[Vec<f32>], lag: usize) -> f64 {
    let w = rows[0].len();
    let mean = rows.iter().flatten().map(|&v| v as f64).sum::<f64>() / rows.iter().map(|r| r.len()).sum::<usize>() as f64;
    let mut num = 0.0f64;
    let mut den = 0.0f64;
    for row in rows {
        for &v in row {
            den += (v as f64 - mean).powi(2);
        }
    }
    for y in 0..rows.len().saturating_sub(lag) {
        for x in 0..w {
            num += (rows[y][x] as f64 - mean) * (rows[y + lag][x] as f64 - mean);
        }
    }
    num / den
}

fn draw_rows(noise: &Noise, salt: u32, w: usize, h: usize, quantize: impl Fn(f32) -> f32) -> Vec<Vec<f32>> {
    (0..h)
        .map(|y| {
            let off = noise.row_offset(y, salt, w);
            noise.slice(off, w).iter().map(|&v| quantize(v)).collect()
        })
        .collect()
}

fn report(name: &str, rows: &[Vec<f32>]) {
    let lag1x = autocorr_lag_x(rows, 1);
    let lag2x = autocorr_lag_x(rows, 2);
    let lag1y = autocorr_lag_y(rows, 1);
    let lag2y = autocorr_lag_y(rows, 2);
    let mean: f64 = rows.iter().flatten().map(|&v| v as f64).sum::<f64>() / rows.iter().map(|r| r.len()).sum::<usize>() as f64;
    println!("  {name}: mean={mean:.4} lag1_x={lag1x:.4} lag2_x={lag2x:.4} lag1_y={lag1y:.4} lag2_y={lag2y:.4}");
}

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| {
        "/tmp/claude-1000/-home-deck-projects-sandart/f1b6526a-1df3-459e-85d7-c652aafd17ae/scratchpad".to_string()
    });
    let dir = std::path::Path::new(&dir);

    for name in ["water", "gradient"] {
        let bytes = std::fs::read(dir.join(format!("{name}_snapshot.bin"))).unwrap();
        let state = snapshot::parse(&bytes);
        let noise = Noise::new(state.time_seed);
        let w = state.w;
        let h = state.h;

        println!("=== RNG statistics -- {name} (time_seed={}, w={w}, h={h}) ===", state.time_seed);

        // Dispersion: 8-bit quantised uniform in [0,1) (matches dispersion_roll's `& 0xFF`).
        let disp_rows = draw_rows(&noise, SALT_DISPERSION, w, h, |v| (v * 256.0).floor().min(255.0) / 255.0);
        report("dispersion (8-bit)", &disp_rows);

        // Lock: 16-bit quantised uniform in [0,1) (matches lock_roll's `& 0xFFFF`).
        let lock_rows = draw_rows(&noise, SALT_LOCK, w, h, |v| (v * 65536.0).floor().min(65535.0) / 65535.0);
        report("lock (16-bit)", &lock_rows);

        // Jitter: full-resolution uniform in [0,1) (matches edge_share_jitter's hash_u01).
        let jit_rows = draw_rows(&noise, SALT_JITTER, w, h, |v| v);
        report("jitter (24-bit)", &jit_rows);

        // Lock fraction vs GRAVITY_LOCK_CHANCE * granular_share, at granular_share = 1.0 (pure
        // granular material -- the case the threshold is actually calibrated against) and 0.5.
        for granular_share in [1.0f32, 0.5f32] {
            let threshold = GRAVITY_LOCK_CHANCE * granular_share;
            let n = lock_rows.iter().map(|r| r.len()).sum::<usize>();
            let locked = lock_rows.iter().flatten().filter(|&&v| v < threshold).count();
            println!(
                "  lock fraction @ granular_share={granular_share}: {:.5} (threshold {:.5}, n={n})",
                locked as f64 / n as f64,
                threshold
            );
        }
        println!();
    }
}
