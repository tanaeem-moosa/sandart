pub mod grid;
pub mod physics;

pub use grid::Heightmap;
pub use physics::{ActiveBounds, displace_line, settle_tick};
use glam::Vec2;
use serde::{Deserialize, Serialize};

pub const GRID_SIZE: usize = 512;
pub const DEFAULT_SAND_HEIGHT: f32 = 0.35;

pub const PROP_WETNESS: usize = 0;
pub const PROP_THRESHOLD: usize = 1;
pub const PROP_FLOW_RATE: usize = 2;
pub const PROP_GRAIN_SIZE: usize = 3;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum SandboxShape {
    Circle,
    Square,
    Oval,
}

impl Default for SandboxShape {
    fn default() -> Self {
        Self::Circle
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum MaterialMode {
    DrySand,
    KineticSand,
    WetSand,
    CoarseSand,
    ButterCream,
    Snow,
    FinePowder,
    Oobleck,
    MoonDust,
    Water,
    Milk,
    VegetableOil,
    CalmWater,
    Yogurt,
}

impl Default for MaterialMode {
    fn default() -> Self {
        Self::DrySand
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[repr(u8)]
pub enum BlockActivity {
    Inactive = 0,
    Slow = 1,
    Medium = 2,
    Fast = 3,
}

impl Default for BlockActivity {
    fn default() -> Self {
        Self::Inactive
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MarbleState {
    pub pos: Vec2,
    pub prev_pos: Vec2,
    pub vel: Vec2,
    pub was_active: bool,
}

impl Default for MarbleState {
    fn default() -> Self {
        Self {
            pos: Vec2::ZERO,
            prev_pos: Vec2::ZERO,
            vel: Vec2::ZERO,
            was_active: false,
        }
    }
}

pub trait HeightmapSimulation {
    fn update(&mut self, dt: f32, cursor_targets: &[Option<glam::Vec2>]);
    fn reset(&mut self);
    fn heightmap(&self) -> &[f32];
    fn dimensions(&self) -> (usize, usize);
    fn marbles(&self) -> &[MarbleState; 5];
    fn active_bounds(&self) -> ActiveBounds;
}

/// Coordinates the state of the marble and the sand bed heightmap.
pub struct DrawingSimulation {
    /// The sand heightmap grid.
    pub heightmap: Heightmap,
    /// Pre-allocated temp buffer for double-buffering settling flows.
    pub temp_heights: Vec<f32>,
    /// Per-cell RGBA color buffer. Advected during CA settling and displacement.
    pub cell_colors: Vec<u8>,
    /// Per-cell physics & render properties. Advected with height.
    /// Layout: [wetness, threshold, flow_rate, grain_size] interleaved.
    pub cell_props: Vec<f32>,
    /// Current position of the primary marble (backward compatibility).
    pub marble_pos: Vec2,
    /// Previous position of the primary marble (backward compatibility).
    pub prev_marble_pos: Vec2,
    /// Last velocity of the primary marble (backward compatibility).
    pub marble_vel: Vec2,
    /// Track whether the primary marble has an active drawing stroke (backward compatibility).
    pub was_active: bool,
    /// Up to 5 marbles tracked in the simulation
    pub marbles: [MarbleState; 5],
    /// Active bounding box for settling updates.
    pub active_bounds: ActiveBounds,
    /// Sliding state tracker for stick-slip shear hysteresis.
    pub sliding: Vec<bool>,
    /// Wave velocity buffer for liquid simulation.
    pub wave_vel: Vec<f32>,
    /// Seed for marble movement noise.
    pub seed: u32,

    // Internal simulation configuration fields
    pub marble_radius: f32,
    pub material_mode: MaterialMode,
    pub sandbox_shape: SandboxShape,

    /// Coarse block activity grid for CA optimization.
    pub active_blocks: Vec<BlockActivity>,
    /// Max displacement observed in each block during the last time it was simulated.
    pub last_displacements: Vec<f32>,
    /// Tick count of when each block was last simulated.
    pub last_simulated_ticks: Vec<u32>,
    /// Current dynamic simulation budget (N blocks).
    pub budget_n: usize,
    /// Exponential moving average of step time in milliseconds.
    pub ema_frame_ms: f32,
    /// Block size (e.g. 32 pixels).
    pub block_size: usize,
    /// Tick count for multi-rate LOD scheduling.
    pub tick_count: u32,
}

fn generate_smooth_noise(seed_val: u32) -> Heightmap {
    let mut heightmap = Heightmap::new(GRID_SIZE, GRID_SIZE, DEFAULT_SAND_HEIGHT);
    let mut seed = seed_val;

    // Helper to generate a low-res random grid via XORShift
    let mut gen_grid = |size: usize| -> Vec<f32> {
        let mut grid = vec![0.0f32; size * size];
        for val in grid.iter_mut() {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            *val = (seed as f32 / u32::MAX as f32) - 0.5; // Range [-0.5, 0.5]
        }
        grid
    };

    // Generate two noise grids at different resolutions (octaves)
    let grid_size1 = 8;
    let grid1 = gen_grid(grid_size1);

    let grid_size2 = 16;
    let grid2 = gen_grid(grid_size2);

    // Bilinear interpolation helper with smoothstep
    let sample_octave = |grid: &[f32], size: usize, x: usize, y: usize| -> f32 {
        let fx = (x as f32 / (GRID_SIZE - 1) as f32) * (size - 1) as f32;
        let fy = (y as f32 / (GRID_SIZE - 1) as f32) * (size - 1) as f32;

        let x0 = fx.floor() as usize;
        let x1 = (x0 + 1).min(size - 1);
        let y0 = fy.floor() as usize;
        let y1 = (y0 + 1).min(size - 1);

        let tx = fx - x0 as f32;
        let ty = fy - y0 as f32;

        // Smoothstep interpolation
        let sx = tx * tx * (3.0 - 2.0 * tx);
        let sy = ty * ty * (3.0 - 2.0 * ty);

        let v00 = grid[y0 * size + x0];
        let v10 = grid[y0 * size + x1];
        let v01 = grid[y1 * size + x0];
        let v11 = grid[y1 * size + x1];

        let h0 = v00 * (1.0 - sx) + v10 * sx;
        let h1 = v01 * (1.0 - sx) + v11 * sx;
        h0 * (1.0 - sy) + h1 * sy
    };

    for y in 0..GRID_SIZE {
        let row_offset = y * GRID_SIZE;
        for x in 0..GRID_SIZE {
            // Combine octaves: 8x8 primary (amp 0.025), 16x16 secondary (amp 0.008)
            let val1 = sample_octave(&grid1, grid_size1, x, y) * 0.025;
            let val2 = sample_octave(&grid2, grid_size2, x, y) * 0.008;

            let combined = val1 + val2;
            heightmap.data[row_offset + x] = (DEFAULT_SAND_HEIGHT + combined).clamp(0.0, 1.0);
        }
    }

    heightmap
}

impl DrawingSimulation {
    pub fn new() -> Self {
        let heightmap = generate_smooth_noise(12345u32);
        let temp_heights = heightmap.data.clone();
        let sliding = vec![false; GRID_SIZE * GRID_SIZE];
        let wave_vel = vec![0.0f32; GRID_SIZE * GRID_SIZE];
        let mut cell_colors = vec![0u8; GRID_SIZE * GRID_SIZE * 4];
        for chunk in cell_colors.chunks_exact_mut(4) {
            chunk[0] = 210;
            chunk[1] = 180;
            chunk[2] = 140;
            chunk[3] = 255;
        }
        let mut cell_props = vec![0.0f32; GRID_SIZE * GRID_SIZE * 4];
        // Initialize with default DrySand preset
        for chunk in cell_props.chunks_exact_mut(4) {
            chunk[PROP_WETNESS] = 0.00;
            chunk[PROP_THRESHOLD] = 0.08;
            chunk[PROP_FLOW_RATE] = 0.25;
            chunk[PROP_GRAIN_SIZE] = 0.45;
        }

        let block_size = 16;
        let cols = (GRID_SIZE + block_size - 1) / block_size;
        let rows = (GRID_SIZE + block_size - 1) / block_size;
        let active_blocks = vec![BlockActivity::Inactive; cols * rows];
        let last_displacements = vec![0.0f32; cols * rows];
        let last_simulated_ticks = vec![0u32; cols * rows];
        let budget_n = 256;
        let ema_frame_ms = 33.3;

        Self {
            heightmap,
            temp_heights,
            cell_colors,
            cell_props,
            marble_pos: Vec2::ZERO,
            prev_marble_pos: Vec2::ZERO,
            marble_vel: Vec2::ZERO,
            was_active: false,
            marbles: [MarbleState::default(); 5],
            active_bounds: ActiveBounds {
                min_x: 0,
                max_x: 0,
                min_y: 0,
                max_y: 0,
                active: false,
            },
            sliding,
            wave_vel,
            seed: 98765u32,
            marble_radius: 0.018,
            material_mode: MaterialMode::default(),
            sandbox_shape: SandboxShape::default(),
            active_blocks,
            last_displacements,
            last_simulated_ticks,
            budget_n,
            ema_frame_ms,
            block_size,
            tick_count: 0,
        }
    }

    /// Reset the simulation state.
    pub fn reset(&mut self) {
        self.heightmap = generate_smooth_noise(54321u32);
        self.temp_heights.copy_from_slice(&self.heightmap.data);
        self.sliding.fill(false);
        self.wave_vel.fill(0.0);
        for chunk in self.cell_colors.chunks_exact_mut(4) {
            chunk[0] = 210;
            chunk[1] = 180;
            chunk[2] = 140;
            chunk[3] = 255;
        }
        self.apply_preset(self.material_mode);
        self.marble_pos = Vec2::ZERO;
        self.prev_marble_pos = Vec2::ZERO;
        self.marble_vel = Vec2::ZERO;
        self.was_active = false;
        self.marbles = [MarbleState::default(); 5];
        self.active_bounds = ActiveBounds {
            min_x: 0,
            max_x: 0,
            min_y: 0,
            max_y: 0,
            active: false,
        };
        self.seed = 98765u32;
        self.active_blocks.fill(BlockActivity::Inactive);
        self.last_displacements.fill(0.0);
        self.last_simulated_ticks.fill(0);
        self.budget_n = 256;
        self.ema_frame_ms = 33.3;
        self.tick_count = 0;
    }

    /// Apply a preset to the per-cell properties buffer.
    pub fn apply_preset(&mut self, mode: MaterialMode) {
        let (wetness, threshold, flow_rate, grain_size) = match mode {
            MaterialMode::DrySand => (0.00, 0.08, 0.25, 0.45),
            MaterialMode::CoarseSand => (0.00, 0.11, 0.22, 0.80),
            MaterialMode::KineticSand => (0.20, 0.10, 0.15, 0.35),
            MaterialMode::WetSand => (0.45, 0.14, 0.08, 0.40),
            MaterialMode::FinePowder => (0.00, 0.05, 0.30, 0.05),
            MaterialMode::Snow => (0.05, 0.15, 0.20, 0.20),
            MaterialMode::MoonDust => (0.00, 0.20, 0.20, 0.10),
            MaterialMode::Oobleck => (0.55, 0.04, 0.12, 0.15),
            MaterialMode::ButterCream => (0.70, 0.04, 0.15, 0.08),
            MaterialMode::Water => (1.00, 0.00, 0.00, 0.00),
            MaterialMode::CalmWater => (0.90, 0.00, 0.00, 0.00),
            MaterialMode::Milk => (0.95, 0.00, 0.00, 0.00),
            MaterialMode::VegetableOil => (0.85, 0.00, 0.00, 0.00),
            MaterialMode::Yogurt => (0.75, 0.00, 0.00, 0.08),
        };
        for chunk in self.cell_props.chunks_exact_mut(4) {
            chunk[PROP_WETNESS] = wetness;
            chunk[PROP_THRESHOLD] = threshold;
            chunk[PROP_FLOW_RATE] = flow_rate;
            chunk[PROP_GRAIN_SIZE] = grain_size;
        }
        self.material_mode = mode;
    }

    /// Copy color patterns into CPU color buffer
    pub fn set_cell_colors(&mut self, rgba_data: &[u8]) {
        let len = self.cell_colors.len().min(rgba_data.len());
        self.cell_colors[..len].copy_from_slice(&rgba_data[..len]);
    }

    /// Copy per-cell properties from a custom buffer
    pub fn set_cell_props(&mut self, props_data: &[f32]) {
        let len = self.cell_props.len().min(props_data.len());
        self.cell_props[..len].copy_from_slice(&props_data[..len]);
    }

    /// Convert normalized Cartesian coordinates ([-1.0, 1.0]) to grid index coordinates.
    #[allow(dead_code)]
    pub fn norm_to_grid(pos: Vec2, width: usize, height: usize) -> (usize, usize) {
        let px = if pos.x.is_finite() { pos.x } else { 0.0 };
        let py = if pos.y.is_finite() { pos.y } else { 0.0 };
        let x = ((px + 1.0) * 0.5 * width as f32).clamp(0.0, (width - 1) as f32) as usize;
        let y = ((1.0 - py) * 0.5 * height as f32).clamp(0.0, (height - 1) as f32) as usize;
        (x, y)
    }

    /// Erase height values inside the marble radius to 0.0 with sub-pixel precision.
    #[allow(dead_code)]
    pub fn draw_point(&mut self, pos: Vec2, radius: f32) {
        displace_line(
            &mut self.heightmap,
            &mut self.cell_colors,
            &mut self.cell_props,
            pos,
            pos,
            radius,
            &mut self.active_bounds,
        );
    }

    /// Draw a line between start and end using interpolation to prevent gaps.
    #[allow(dead_code)]
    pub fn draw_line(&mut self, start: Vec2, end: Vec2, radius: f32) {
        displace_line(
            &mut self.heightmap,
            &mut self.cell_colors,
            &mut self.cell_props,
            start,
            end,
            radius,
            &mut self.active_bounds,
        );
    }

    fn clamp_to_sandbox(pos: Vec2, shape: SandboxShape, marble_radius: f32) -> Vec2 {
        let max_r = (0.92 - marble_radius).max(0.0);
        match shape {
            SandboxShape::Circle => {
                let len = pos.length();
                if len > max_r && len > 1e-5 {
                    pos * (max_r / len)
                } else {
                    pos
                }
            }
            SandboxShape::Square => {
                Vec2::new(
                    pos.x.clamp(-max_r, max_r),
                    pos.y.clamp(-max_r, max_r),
                )
            }
            SandboxShape::Oval => {
                let a = (0.92 - marble_radius).max(0.01);
                let b = (0.60 - marble_radius).max(0.01);
                let d_sq = (pos.x * pos.x) / (a * a) + (pos.y * pos.y) / (b * b);
                if d_sq > 1.0 {
                    let d = d_sq.sqrt();
                    pos / d
                } else {
                    pos
                }
            }
        }
    }

    /// Run a physics frame tick.
    pub fn update(&mut self, dt: f32, targets: &[Option<Vec2>; 5], marble_radius: f32, material: MaterialMode, shape: SandboxShape, last_frame_time_ms: f32, target_frame_time_ms: f32) {
        // Prevent seed degeneracy (XORShift stuck state at 0)
        if self.seed == 0 {
            self.seed = 98765u32;
        }

        // Advance seed every frame to keep settling dynamics active and non-deterministic
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        let time_seed = self.seed;

        let w = self.heightmap.width;
        let h = self.heightmap.height;
        let block_size = self.block_size;
        let cols = (w + block_size - 1) / block_size;
        let rows = (h + block_size - 1) / block_size;

        for j in 0..5 {
            if let Some(target) = targets[j] {
                // Sanitize target coordinate float boundaries against NaNs/Infs
                let tx = if target.x.is_finite() { target.x } else { 0.0 };
                let ty = if target.y.is_finite() { target.y } else { 0.0 };
                let target_sanitized = Vec2::new(tx, ty);

                let clamped_target = Self::clamp_to_sandbox(target_sanitized, shape, marble_radius);

                let mut segment_bounds = ActiveBounds {
                    min_x: 0,
                    max_x: 0,
                    min_y: 0,
                    max_y: 0,
                    active: false,
                };

                if self.marbles[j].was_active {
                    self.marbles[j].prev_pos = self.marbles[j].pos;

                    // Calculate step vector and distance
                    let raw_diff = clamped_target - self.marbles[j].pos;
                    let raw_dist = raw_diff.length();

                    // 1. Generate pseudo-random numbers
                    self.seed ^= self.seed << 13;
                    self.seed ^= self.seed >> 17;
                    self.seed ^= self.seed << 5;
                    let n1 = (self.seed as f32 / u32::MAX as f32 - 0.5) * 2.0; // [-1.0, 1.0]

                    self.seed ^= self.seed << 13;
                    self.seed ^= self.seed >> 17;
                    self.seed ^= self.seed << 5;
                    let n2 = (self.seed as f32 / u32::MAX as f32 - 0.5) * 2.0; // [-1.0, 1.0]

                    let random_offset = Vec2::new(n1, n2);

                    // 2. Micro-jitter: simulate bumping over discrete sand grains (extremely subtle)
                    let jitter_amplitude = marble_radius * 0.04;
                    let jitter = random_offset * jitter_amplitude;

                    // 3. Inertia/drag drift: simulate sand resistance lagging and sliding sideways
                    let mut drift = Vec2::ZERO;
                    if raw_dist > 1e-5 {
                        let dir = raw_diff / raw_dist;
                        let perp = Vec2::new(-dir.y, dir.x);

                        // Minor drag (lag behind magnet/target)
                        let lag = -dir * (raw_dist * 0.08);

                        // Minor sideways slip (uneven resistance)
                        let slip = perp * (raw_dist * 0.05 * n1);

                        drift = lag + slip;
                    }

                    let mut next_pos = clamped_target + jitter + drift;
                    next_pos = Self::clamp_to_sandbox(next_pos, shape, marble_radius);

                    self.marbles[j].pos = next_pos;
                    self.marbles[j].vel = next_pos - self.marbles[j].prev_pos;

                    displace_line(
                        &mut self.heightmap,
                        &mut self.cell_colors,
                        &mut self.cell_props,
                        self.marbles[j].prev_pos,
                        self.marbles[j].pos,
                        marble_radius,
                        &mut segment_bounds,
                    );
                } else {
                    self.marbles[j].pos = clamped_target;
                    self.marbles[j].prev_pos = clamped_target;
                    self.marbles[j].vel = Vec2::ZERO;
                    displace_line(
                        &mut self.heightmap,
                        &mut self.cell_colors,
                        &mut self.cell_props,
                        clamped_target,
                        clamped_target,
                        marble_radius,
                        &mut segment_bounds,
                    );
                    self.marbles[j].was_active = true;
                }

                // Activate blocks overlapping with the new displacement segment
                if segment_bounds.active {
                    let block_min_x = segment_bounds.min_x / block_size;
                    let block_max_x = (segment_bounds.max_x / block_size).min(cols - 1);
                    let block_min_y = segment_bounds.min_y / block_size;
                    let block_max_y = (segment_bounds.max_y / block_size).min(rows - 1);
                    for by in block_min_y..=block_max_y {
                        for bx in block_min_x..=block_max_x {
                            self.last_displacements[by * cols + bx] = 1.0;
                        }
                    }
                }
            } else {
                self.marbles[j].was_active = false;
            }

            // Sync with primary fields for backward compatibility
            if j == 0 {
                self.marble_pos = self.marbles[0].pos;
                self.prev_marble_pos = self.marbles[0].prev_pos;
                self.marble_vel = self.marbles[0].vel;
                self.was_active = self.marbles[0].was_active;
            }
        }



        // Run the gravity-driven settling cellular automata tick
        let has_active = self.last_displacements.iter().any(|&x| x > 3e-4)
            || self.marbles.iter().any(|m| m.was_active);
        if has_active {
            let mut active_marbles = [physics::ActiveMarbleInfo {
                pos: Vec2::ZERO,
                vel: 0.0,
                vel_vec: Vec2::ZERO,
            }; 5];
            let mut active_count = 0;
            for j in 0..5 {
                if self.marbles[j].was_active {
                    let m_vel_vec = if dt > 1e-5 { self.marbles[j].vel / dt } else { Vec2::ZERO };
                    active_marbles[active_count] = physics::ActiveMarbleInfo {
                        pos: self.marbles[j].pos,
                        vel: m_vel_vec.length(),
                        vel_vec: m_vel_vec,
                    };
                    active_count += 1;
                }
            }

            settle_tick(
                &mut self.heightmap,
                &mut self.temp_heights,
                &mut self.cell_colors,
                &mut self.cell_props,
                &mut self.sliding,
                &mut self.active_bounds,
                &mut self.active_blocks,
                &mut self.last_displacements,
                &mut self.last_simulated_ticks,
                self.budget_n,
                self.block_size,
                &active_marbles[..active_count],
                time_seed,
                &mut self.wave_vel,
                shape,
                self.tick_count,
            );
        } else {
            self.active_bounds.active = false;
        }
        self.tick_count = self.tick_count.wrapping_add(1);

        // Update EMA of frame time and adjust budget_n
        const EMA_ALPHA: f32 = 0.1;
        const BUDGET_MIN: usize = 32;
        const BUDGET_STEP_DOWN: usize = 4;
        const BUDGET_STEP_UP: usize = 1;

        if last_frame_time_ms > 0.0 && target_frame_time_ms > 0.0 {
            self.ema_frame_ms = EMA_ALPHA * last_frame_time_ms + (1.0 - EMA_ALPHA) * self.ema_frame_ms;
            
            let budget_max = cols * rows; // e.g. 1024

            // Target 95% of target FPS (Vsync interval * 1.05) to account for browser Vsync-locking
            // and allow the budget to grow back up when running smoothly.
            let adjusted_target = target_frame_time_ms * 1.05;

            if self.ema_frame_ms > adjusted_target {
                self.budget_n = self.budget_n.saturating_sub(BUDGET_STEP_DOWN).max(BUDGET_MIN);
            } else if self.ema_frame_ms < adjusted_target {
                self.budget_n = (self.budget_n + BUDGET_STEP_UP).min(budget_max);
            }
        }
    }
}

impl HeightmapSimulation for DrawingSimulation {
    fn update(&mut self, dt: f32, cursor_targets: &[Option<glam::Vec2>]) {
        let mut targets = [None; 5];
        for (i, target) in cursor_targets.iter().take(5).enumerate() {
            targets[i] = *target;
        }
        let radius = self.marble_radius;
        let mat = self.material_mode;
        let shape = self.sandbox_shape;
        self.update(dt, &targets, radius, mat, shape, dt * 1000.0, dt * 1000.0);
    }

    fn reset(&mut self) {
        self.reset();
    }

    fn heightmap(&self) -> &[f32] {
        self.heightmap.as_slice()
    }

    fn dimensions(&self) -> (usize, usize) {
        (GRID_SIZE, GRID_SIZE)
    }

    fn marbles(&self) -> &[MarbleState; 5] {
        &self.marbles
    }

    fn active_bounds(&self) -> ActiveBounds {
        self.active_bounds
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simulation_reset() {
        let mut sim = DrawingSimulation::new();
        sim.marble_pos = Vec2::new(0.5, -0.5);
        sim.heightmap.set(100, 100, 0.0);
        sim.reset();
        assert_eq!(sim.marble_pos, Vec2::ZERO);
        let val = sim.heightmap.get(100, 100);
        assert!((val - DEFAULT_SAND_HEIGHT).abs() < 0.035);
    }

    #[test]
    fn test_norm_to_grid_mapping() {
        let width = 512;
        let height = 512;

        // Verify corners map to exact boundary indexes
        assert_eq!(
            DrawingSimulation::norm_to_grid(Vec2::new(-1.0, 1.0), width, height),
            (0, 0)
        );
        assert_eq!(
            DrawingSimulation::norm_to_grid(Vec2::new(1.0, -1.0), width, height),
            (width - 1, height - 1)
        );

        // Verify center mapping falls in correct bins (256, 256)
        assert_eq!(
            DrawingSimulation::norm_to_grid(Vec2::new(0.0, 0.0), width, height),
            (256, 256)
        );

        // Verify bounds clamping maps out of bounds coordinates to grid edges safely
        assert_eq!(
            DrawingSimulation::norm_to_grid(Vec2::new(-2.0, 2.0), width, height),
            (0, 0)
        );
        assert_eq!(
            DrawingSimulation::norm_to_grid(Vec2::new(2.0, -2.0), width, height),
            (width - 1, height - 1)
        );
    }

    #[test]
    fn test_norm_to_grid_nan_inf() {
        let width = 512;
        let height = 512;

        // NAN should map safely without panic
        let nan_pos = Vec2::new(f32::NAN, f32::NAN);
        let (x, y) = DrawingSimulation::norm_to_grid(nan_pos, width, height);
        assert!(x < width && y < height);

        // Inf should map safely without panic
        let inf_pos = Vec2::new(f32::INFINITY, f32::NEG_INFINITY);
        let (x, y) = DrawingSimulation::norm_to_grid(inf_pos, width, height);
        assert!(x < width && y < height);
    }

    #[test]
    fn test_marble_movement_noise_and_drift() {
        let mut sim = DrawingSimulation::new();
        let mut targets = [None; 5];
        // Initially target is None, should not be active
        sim.update(0.016, &targets, 0.025, MaterialMode::ButterCream, SandboxShape::Circle, 16.0, 16.0);
        assert!(!sim.was_active);

        // Move to start point (first point is exact target)
        targets[0] = Some(Vec2::new(0.1, 0.2));
        sim.update(0.016, &targets, 0.025, MaterialMode::ButterCream, SandboxShape::Circle, 16.0, 16.0);
        assert!(sim.was_active);
        assert_eq!(sim.marble_pos, Vec2::new(0.1, 0.2));

        // Move to next point, introducing noise, drag, and jitter
        let target = Vec2::new(0.3, 0.4);
        targets[0] = Some(target);
        sim.update(0.016, &targets, 0.025, MaterialMode::ButterCream, SandboxShape::Circle, 16.0, 16.0);

        // Ensure marble position shifted from start and is not exactly the target due to physics drift/noise
        assert_ne!(sim.marble_pos, Vec2::new(0.1, 0.2));
        assert_ne!(sim.marble_pos, target);

        // Verify that it is close to target but slightly drifted/jittered (less than 0.1 delta)
        let dist = (sim.marble_pos - target).length();
        assert!(dist < 0.1);

        // Verify marble velocity is populated
        assert_ne!(sim.marble_vel, Vec2::ZERO);
    }

    #[test]
    fn test_sandbox_shapes_clamping() {
        // Test Circle clamping: length should be clamped to max_r
        let p_circle = DrawingSimulation::clamp_to_sandbox(Vec2::new(1.0, 1.0), SandboxShape::Circle, 0.018);
        assert!((p_circle.length() - (0.92 - 0.018)).abs() < 1e-5);

        // Test Square clamping: X and Y should be clamped to max_r
        let p_square = DrawingSimulation::clamp_to_sandbox(Vec2::new(1.5, 0.2), SandboxShape::Square, 0.018);
        assert_eq!(p_square.x, 0.92 - 0.018);
        assert_eq!(p_square.y, 0.2);

        // Test Oval clamping: should satisfy ellipse equation
        let p_oval = DrawingSimulation::clamp_to_sandbox(Vec2::new(1.0, 1.0), SandboxShape::Oval, 0.018);
        let a = 0.92 - 0.018;
        let b = 0.60 - 0.018;
        let d_sq = (p_oval.x * p_oval.x) / (a * a) + (p_oval.y * p_oval.y) / (b * b);
        assert!((d_sq - 1.0).abs() < 1e-5);
    }

    #[test]
    fn test_simulation_volume_preservation() {
        let mut sim = DrawingSimulation::new();
        let initial_sum: f64 = sim.heightmap.data.iter().map(|&x| x as f64).sum();

        let mut targets = [None; 5];
        // Move marble in a spiral over 200 steps
        for i in 0..200 {
            let angle = i as f32 * 0.1;
            let radius = i as f32 * 0.004;
            targets[0] = Some(Vec2::new(angle.cos() * radius, angle.sin() * radius));
            sim.update(
                0.016,
                &targets,
                0.018,
                MaterialMode::DrySand,
                SandboxShape::Circle,
                16.0,
                16.0,
            );
            
            let current_sum: f64 = sim.heightmap.data.iter().map(|&x| x as f64).sum();
            let diff = (current_sum - initial_sum).abs();
            assert!(diff < 5e-3, "Step {}: Volume leaked! diff = {}, initial = {}, current = {}", i, diff, initial_sum, current_sum);
        }
    }

    #[test]
    fn test_multi_marble_large_spiral_volume_preservation() {
        let mut sim = DrawingSimulation::new();
        let initial_sum: f64 = sim.heightmap.data.iter().map(|&x| x as f64).sum();

        let mut targets = [None; 5];
        // Large marble radius
        let marble_radius = 0.08;
        
        // Move 3 marbles in out-of-phase spirals over 150 steps
        for i in 0..150 {
            for j in 0..3 {
                let angle = i as f32 * 0.15 + (j as f32 * 2.0 * std::f32::consts::PI / 3.0);
                let radius = i as f32 * 0.005;
                targets[j] = Some(Vec2::new(angle.cos() * radius, angle.sin() * radius));
            }
            sim.update(
                0.016,
                &targets,
                marble_radius,
                MaterialMode::DrySand,
                SandboxShape::Circle,
                16.0,
                16.0,
            );
            
            let current_sum: f64 = sim.heightmap.data.iter().map(|&x| x as f64).sum();
            let diff = (current_sum - initial_sum).abs();
            // Use 2e-2 threshold for multi-marble large updates, due to larger accumulated float rounding errors.
            assert!(diff < 2e-2, "Step {}: Multi-marble volume leaked! diff = {}, initial = {}, current = {}", i, diff, initial_sum, current_sum);
        }
    }

    #[test]
    fn test_simulation_color_preservation() {
        let mut sim = DrawingSimulation::new();
        
        // Initialize cell_colors with a gradient/pattern
        let mut initial_colors = vec![0u8; GRID_SIZE * GRID_SIZE * 4];
        for y in 0..GRID_SIZE {
            for x in 0..GRID_SIZE {
                let idx = y * GRID_SIZE + x;
                initial_colors[idx * 4 + 0] = (x % 256) as u8;
                initial_colors[idx * 4 + 1] = (y % 256) as u8;
                initial_colors[idx * 4 + 2] = 128;
                initial_colors[idx * 4 + 3] = 255;
            }
        }
        sim.set_cell_colors(&initial_colors);

        let calculate_color_mass = |s: &DrawingSimulation| -> (f64, f64) {
            let mut red_mass = 0.0f64;
            let mut green_mass = 0.0f64;
            for (idx, &h) in s.heightmap.data.iter().enumerate() {
                let r = s.cell_colors[idx * 4 + 0] as f64;
                let g = s.cell_colors[idx * 4 + 1] as f64;
                red_mass += r * h as f64;
                green_mass += g * h as f64;
            }
            (red_mass, green_mass)
        };

        let (initial_red, initial_green) = calculate_color_mass(&sim);

        let mut targets = [None; 5];
        // Move marble in a spiral over 200 steps
        for i in 0..200 {
            let angle = i as f32 * 0.1;
            let radius = i as f32 * 0.004;
            targets[0] = Some(Vec2::new(angle.cos() * radius, angle.sin() * radius));
            sim.update(
                0.016,
                &targets,
                0.018,
                MaterialMode::DrySand,
                SandboxShape::Circle,
                16.0,
                16.0,
            );
        }

        let (final_red, final_green) = calculate_color_mass(&sim);

        let diff_red = (final_red - initial_red).abs() / initial_red;
        let diff_green = (final_green - initial_green).abs() / initial_green;

        // Verify that the color mass is preserved within 0.5% (to account for u8 integer rounding at each step)
        assert!(diff_red < 0.005, "Red color mass leaked! diff = {:.5}%, initial = {}, final = {}", diff_red * 100.0, initial_red, final_red);
        assert!(diff_green < 0.005, "Green color mass leaked! diff = {:.5}%, initial = {}, final = {}", diff_green * 100.0, initial_green, final_green);
    }

    #[test]
    fn test_multi_marble_large_spiral_color_preservation() {
        let mut sim = DrawingSimulation::new();

        // Initialize cell_colors with a gradient/pattern
        let mut initial_colors = vec![0u8; GRID_SIZE * GRID_SIZE * 4];
        for y in 0..GRID_SIZE {
            for x in 0..GRID_SIZE {
                let idx = y * GRID_SIZE + x;
                initial_colors[idx * 4 + 0] = (x % 256) as u8;
                initial_colors[idx * 4 + 1] = (y % 256) as u8;
                initial_colors[idx * 4 + 2] = 128;
                initial_colors[idx * 4 + 3] = 255;
            }
        }
        sim.set_cell_colors(&initial_colors);

        let calculate_color_mass = |s: &DrawingSimulation| -> (f64, f64) {
            let mut red_mass = 0.0f64;
            let mut green_mass = 0.0f64;
            for (idx, &h) in s.heightmap.data.iter().enumerate() {
                let r = s.cell_colors[idx * 4 + 0] as f64;
                let g = s.cell_colors[idx * 4 + 1] as f64;
                red_mass += r * h as f64;
                green_mass += g * h as f64;
            }
            (red_mass, green_mass)
        };

        let (initial_red, initial_green) = calculate_color_mass(&sim);

        let mut targets = [None; 5];
        let marble_radius = 0.08;
        
        // Move 3 marbles in out-of-phase spirals over 150 steps
        for i in 0..150 {
            for j in 0..3 {
                let angle = i as f32 * 0.15 + (j as f32 * 2.0 * std::f32::consts::PI / 3.0);
                let radius = i as f32 * 0.005;
                targets[j] = Some(Vec2::new(angle.cos() * radius, angle.sin() * radius));
            }
            sim.update(
                0.016,
                &targets,
                marble_radius,
                MaterialMode::DrySand,
                SandboxShape::Circle,
                16.0,
                16.0,
            );
        }

        let (final_red, final_green) = calculate_color_mass(&sim);

        let diff_red = (final_red - initial_red).abs() / initial_red;
        let diff_green = (final_green - initial_green).abs() / initial_green;

        // Verify that the color mass is preserved within 0.5%
        assert!(diff_red < 0.005, "Multi-marble Red color mass leaked! diff = {:.5}%, initial = {}, final = {}", diff_red * 100.0, initial_red, final_red);
        assert!(diff_green < 0.005, "Multi-marble Green color mass leaked! diff = {:.5}%, initial = {}, final = {}", diff_green * 100.0, initial_green, final_green);
    }
}
