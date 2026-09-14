//! Step 0 of "option 4": drives a REAL, unmodified `sandart_sim::DrawingSimulation` through the
//! same scene `sandart-sim/examples/profile_sandfall_water.rs` profiles natively (grid 512,
//! MultiNeckHourglass, water, upper half filled to 0.5, `lateral_substeps` 2.5, `budget_n` 128),
//! and exposes it two ways from ONE source:
//!
//! - `src/bin/native_phase_bench.rs`: a native binary, timed with `std::time::Instant` directly.
//! - the `cdylib` target (this crate, built for `wasm32-unknown-unknown` with `+simd128`),
//!   driven from node via plain `extern "C"` exports + `WebAssembly.instantiate` -- no
//!   wasm-bindgen, same pattern as `sandart-kernel-bench/src/wasm_api.rs` and
//!   `bench_wasm.mjs`, but there is no snapshot format to load here: this crate calls
//!   `sandart_sim`'s own public API to build the scene, in wasm exactly as natively, so it is
//!   the same code path making the same allocations, not a second hand-copy.
//!
//! Both frontends share `Bench` below so the warmup/measurement protocol can't drift between
//! native and wasm.

use sandart_sim::{phase_timing, BlockActivity, DrawingSimulation, MaterialMode, SandboxShape};

pub struct Bench {
    pub sim: DrawingSimulation,
    /// Sum of `active_blocks` non-Inactive counts, accumulated across every `tick()` call since
    /// the last `reset()` -- same definition `profile_sandfall_water.rs`'s `sweep()` uses.
    pub block_ticks: u64,
    pub ticks_run: u32,
}

/// Same scene as `profile_sandfall_water.rs`'s `build()` -- kept in sync by hand (this crate
/// cannot `use` a private example function); see that file if this ever needs to change.
pub fn build_scene(lateral_substeps: f32) -> DrawingSimulation {
    let mut sim = DrawingSimulation::new();
    sim.sandbox_shape = SandboxShape::MultiNeckHourglass;
    sim.apply_preset(MaterialMode::Water);
    sim.generate_shape_mask();
    sim.gravity_dir = glam::Vec2::new(0.0, 0.04);
    sim.lateral_substeps = lateral_substeps;
    sim.budget_n = 128;
    sim.active_bounds.active = true;
    let w = sim.heightmap.width;
    for y in 0..w / 2 {
        for x in 0..w {
            let i = y * w + x;
            if sim.shape_mask[i] != 0 {
                sim.heightmap.data[i] = 0.5;
            }
        }
    }
    sim
}

impl Bench {
    pub fn new(lateral_substeps: f32) -> Self {
        Bench { sim: build_scene(lateral_substeps), block_ticks: 0, ticks_run: 0 }
    }

    #[inline]
    fn step(&mut self) {
        let (r, m, s) = (self.sim.marble_radius, self.sim.material_mode, self.sim.sandbox_shape);
        // 0.0 frame times keep the adaptive controller from touching budget_n -- same as
        // profile_sandfall_water.rs.
        self.sim.update(1.0 / 60.0, &[None; 5], r, m, s, 0.0, 0.0);
    }

    /// Runs `n` ticks with no bookkeeping -- for warmup, where block counts/timers are not read.
    pub fn warmup(&mut self, n: u32) {
        for _ in 0..n {
            self.step();
        }
    }

    /// Runs `n` MEASURED ticks: `phase_timing`'s thread-local accumulators are reset first (so a
    /// caller's own wall-clock bracket around this call lines up with what `phase_timing`
    /// reports), then each tick's `active_blocks` count is folded into `block_ticks`.
    pub fn run_measured(&mut self, n: u32) {
        phase_timing::reset_tick();
        self.block_ticks = 0;
        self.ticks_run = n;
        for _ in 0..n {
            self.step();
            self.block_ticks +=
                self.sim.active_blocks.iter().filter(|b| **b != BlockActivity::Inactive).count() as u64;
        }
    }

    pub fn blocks_per_tick(&self) -> f64 {
        self.block_ticks as f64 / self.ticks_run.max(1) as f64
    }

    pub fn cells_per_tick(&self) -> f64 {
        self.blocks_per_tick() * (self.sim.block_size * self.sim.block_size) as f64
    }
}

#[cfg(target_arch = "wasm32")]
pub mod wasm_api {
    use super::Bench;
    use sandart_sim::phase_timing;
    use std::cell::RefCell;

    thread_local! {
        static BENCH: RefCell<Option<Bench>> = const { RefCell::new(None) };
    }

    /// Builds the scene and runs `warmup_ticks` unmeasured ticks (same role as
    /// `profile_sandfall_water.rs`'s 800-tick warmup before its own timed window).
    #[unsafe(no_mangle)]
    pub extern "C" fn init(lateral_substeps: f32, warmup_ticks: u32) {
        let mut b = Bench::new(lateral_substeps);
        b.warmup(warmup_ticks);
        BENCH.with(|c| *c.borrow_mut() = Some(b));
    }

    /// Runs `n` MEASURED ticks (resets `phase_timing` first) and returns nothing -- the caller
    /// brackets this call with `performance.now()` for the whole-tick wall time, then reads the
    /// per-section breakdown back with `section_ns`/`n_sections`/`blocks_per_tick`/
    /// `cells_per_tick` below.
    #[unsafe(no_mangle)]
    pub extern "C" fn run_measured(n: u32) {
        BENCH.with(|c| {
            let mut b = c.borrow_mut();
            let b = b.as_mut().expect("init not called");
            b.run_measured(n);
        });
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn n_sections() -> u32 {
        phase_timing::N_SECTIONS as u32
    }

    /// `phase_timing::snapshot()[i]`, in nanoseconds, accumulated over the whole `run_measured`
    /// call (not per-tick -- divide by `ticks_run` on the JS side, same as the native harness).
    #[unsafe(no_mangle)]
    pub extern "C" fn section_ns(i: u32) -> f64 {
        phase_timing::snapshot()[i as usize] as f64
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn blocks_per_tick() -> f64 {
        BENCH.with(|c| c.borrow().as_ref().expect("init not called").blocks_per_tick())
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn cells_per_tick() -> f64 {
        BENCH.with(|c| c.borrow().as_ref().expect("init not called").cells_per_tick())
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn budget_n() -> u32 {
        BENCH.with(|c| c.borrow().as_ref().expect("init not called").sim.budget_n as u32)
    }
}
