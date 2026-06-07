use glam::Vec2;
pub mod grid;
pub mod physics;

pub use grid::Heightmap;
pub use physics::{ActiveBounds, displace_line, settle_tick};

pub const GRID_SIZE: usize = 1024;

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

/// Coordinates the state of the marble and the sand bed heightmap.
pub struct Simulation {
    /// The sand heightmap grid.
    pub heightmap: Heightmap,
    /// Pre-allocated temp buffer for double-buffering settling flows.
    pub temp_heights: Vec<f32>,
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
    /// Seed for marble movement noise.
    pub seed: u32,
}

fn generate_smooth_noise(seed_val: u32) -> Heightmap {
    let mut heightmap = Heightmap::new(GRID_SIZE, GRID_SIZE, 0.8);
    let mut noise = vec![0.0f32; GRID_SIZE * GRID_SIZE];
    let mut seed = seed_val;
    for val in noise.iter_mut() {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        *val = (seed as f32 / u32::MAX as f32) - 0.5; // Range [-0.5, 0.5]
    }

    // Separable box filter for smooth 2D noise (2 passes of a 3x3 filter)
    let mut temp = noise.clone();
    let mut buffer = vec![0.0f32; GRID_SIZE * GRID_SIZE];

    for _pass in 0..2 {
        // Horizontal pass
        for y in 0..GRID_SIZE {
            let row_offset = y * GRID_SIZE;
            for x in 0..GRID_SIZE {
                let x_min = x.saturating_sub(1);
                let x_max = (x + 1).min(GRID_SIZE - 1);
                let mut sum = 0.0;
                let mut count = 0.0;
                for nx in x_min..=x_max {
                    sum += temp[row_offset + nx];
                    count += 1.0;
                }
                buffer[row_offset + x] = sum / count;
            }
        }

        // Vertical pass
        for y in 0..GRID_SIZE {
            let y_min = y.saturating_sub(1);
            let y_max = (y + 1).min(GRID_SIZE - 1);
            let row_offset = y * GRID_SIZE;
            for x in 0..GRID_SIZE {
                let mut sum = 0.0;
                let mut count = 0.0;
                for ny in y_min..=y_max {
                    sum += buffer[ny * GRID_SIZE + x];
                    count += 1.0;
                }
                temp[row_offset + x] = sum / count;
            }
        }
    }

    // Find max amplitude to normalize
    let mut max_abs = 0.0f32;
    for &val in &temp {
        max_abs = max_abs.max(val.abs());
    }

    // Scale so that maximum noise deviation is exactly 0.025, and add to base height 0.8
    let scale = if max_abs > 1e-5 { 0.025 / max_abs } else { 1.0 };
    for (i, val) in heightmap.data.iter_mut().enumerate() {
        *val = (0.8 + temp[i] * scale).clamp(0.0, 1.0);
    }

    heightmap
}

impl Simulation {
    pub fn new() -> Self {
        let heightmap = generate_smooth_noise(12345u32);
        let temp_heights = heightmap.data.clone();
        let sliding = vec![false; GRID_SIZE * GRID_SIZE];

        Self {
            heightmap,
            temp_heights,
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
            seed: 98765u32,
        }
    }

    /// Reset the simulation state.
    pub fn reset(&mut self) {
        self.heightmap = generate_smooth_noise(54321u32);
        self.temp_heights.copy_from_slice(&self.heightmap.data);
        self.sliding.fill(false);
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
            start,
            end,
            radius,
            &mut self.active_bounds,
        );
    }

    /// Run a physics frame tick.
    pub fn update(&mut self, dt: f32, targets: &[Option<Vec2>; 5], marble_radius: f32, material: crate::config::MaterialMode) {
        // Prevent seed degeneracy (XORShift stuck state at 0)
        if self.seed == 0 {
            self.seed = 98765u32;
        }

        // Advance seed every frame to keep settling dynamics active and non-deterministic
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        let time_seed = self.seed;

        for j in 0..5 {
            if let Some(target) = targets[j] {
                // Sanitize target coordinate float boundaries against NaNs/Infs
                let tx = if target.x.is_finite() { target.x } else { 0.0 };
                let ty = if target.y.is_finite() { target.y } else { 0.0 };
                let target_sanitized = Vec2::new(tx, ty);

                let max_r = (0.92 - marble_radius).max(0.0);
                let target_len = target_sanitized.length();
                let clamped_target = if target_len > max_r && target_len > 1e-5 {
                    target_sanitized * (max_r / target_len)
                } else {
                    target_sanitized
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
                    let next_len = next_pos.length();
                    if next_len > max_r && next_len > 1e-5 {
                        next_pos = next_pos * (max_r / next_len);
                    }

                    self.marbles[j].pos = next_pos;
                    self.marbles[j].vel = next_pos - self.marbles[j].prev_pos;

                    displace_line(
                        &mut self.heightmap,
                        self.marbles[j].prev_pos,
                        self.marbles[j].pos,
                        marble_radius,
                        &mut self.active_bounds,
                    );
                } else {
                    self.marbles[j].pos = clamped_target;
                    self.marbles[j].prev_pos = clamped_target;
                    self.marbles[j].vel = Vec2::ZERO;
                    displace_line(
                        &mut self.heightmap,
                        clamped_target,
                        clamped_target,
                        marble_radius,
                        &mut self.active_bounds,
                    );
                    self.marbles[j].was_active = true;
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

        // If material is IronFilings, expand the active bounds to cover the magnet's reach
        // around all active marbles so filings can react dynamically as the magnet moves.
        if material == crate::config::MaterialMode::IronFilings {
            for j in 0..5 {
                if self.marbles[j].was_active {
                    let (mx, my) = Self::norm_to_grid(self.marbles[j].pos, GRID_SIZE, GRID_SIZE);
                    let r_reach = 225; // ~0.22 radius in grid cells (matches 0.22 magnet reach)
                    let min_x = mx.saturating_sub(r_reach);
                    let max_x = (mx + r_reach).min(GRID_SIZE - 1);
                    let min_y = my.saturating_sub(r_reach);
                    let max_y = (my + r_reach).min(GRID_SIZE - 1);

                    if self.active_bounds.active {
                        self.active_bounds.min_x = self.active_bounds.min_x.min(min_x);
                        self.active_bounds.max_x = self.active_bounds.max_x.max(max_x);
                        self.active_bounds.min_y = self.active_bounds.min_y.min(min_y);
                        self.active_bounds.max_y = self.active_bounds.max_y.max(max_y);
                    } else {
                        self.active_bounds.min_x = min_x;
                        self.active_bounds.max_x = max_x;
                        self.active_bounds.min_y = min_y;
                        self.active_bounds.max_y = max_y;
                        self.active_bounds.active = true;
                    }
                }
            }
        }

        // Run the gravity-driven settling cellular automata tick
        if self.active_bounds.active {
            let mut active_marbles = Vec::new();
            for j in 0..5 {
                if self.marbles[j].was_active {
                    let m_vel = if dt > 1e-5 { self.marbles[j].vel.length() / dt } else { 0.0 };
                    active_marbles.push(crate::sim::physics::ActiveMarbleInfo {
                        pos: self.marbles[j].pos,
                        vel: m_vel,
                    });
                }
            }

            settle_tick(
                &mut self.heightmap,
                &mut self.temp_heights,
                &mut self.sliding,
                &mut self.active_bounds,
                material,
                &active_marbles,
                time_seed,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simulation_reset() {
        let mut sim = Simulation::new();
        sim.marble_pos = Vec2::new(0.5, -0.5);
        sim.heightmap.set(100, 100, 0.0);
        sim.reset();
        assert_eq!(sim.marble_pos, Vec2::ZERO);
        let val = sim.heightmap.get(100, 100);
        assert!((val - 0.8).abs() < 0.03);
    }

    #[test]
    fn test_norm_to_grid_mapping() {
        let width = 512;
        let height = 512;

        // Verify corners map to exact boundary indexes
        assert_eq!(
            Simulation::norm_to_grid(Vec2::new(-1.0, 1.0), width, height),
            (0, 0)
        );
        assert_eq!(
            Simulation::norm_to_grid(Vec2::new(1.0, -1.0), width, height),
            (width - 1, height - 1)
        );

        // Verify center mapping falls in correct bins (256, 256)
        assert_eq!(
            Simulation::norm_to_grid(Vec2::new(0.0, 0.0), width, height),
            (256, 256)
        );

        // Verify bounds clamping maps out of bounds coordinates to grid edges safely
        assert_eq!(
            Simulation::norm_to_grid(Vec2::new(-2.0, 2.0), width, height),
            (0, 0)
        );
        assert_eq!(
            Simulation::norm_to_grid(Vec2::new(2.0, -2.0), width, height),
            (width - 1, height - 1)
        );
    }

    #[test]
    fn test_norm_to_grid_nan_inf() {
        let width = 512;
        let height = 512;

        // NAN should map safely without panic
        let nan_pos = Vec2::new(f32::NAN, f32::NAN);
        let (x, y) = Simulation::norm_to_grid(nan_pos, width, height);
        assert!(x < width && y < height);

        // Inf should map safely without panic
        let inf_pos = Vec2::new(f32::INFINITY, f32::NEG_INFINITY);
        let (x, y) = Simulation::norm_to_grid(inf_pos, width, height);
        assert!(x < width && y < height);
    }

    #[test]
    fn test_marble_movement_noise_and_drift() {
        let mut sim = Simulation::new();
        let mut targets = [None; 5];
        // Initially target is None, should not be active
        sim.update(0.016, &targets, 0.025, crate::config::MaterialMode::ButterCream);
        assert!(!sim.was_active);

        // Move to start point (first point is exact target)
        targets[0] = Some(Vec2::new(0.1, 0.2));
        sim.update(0.016, &targets, 0.025, crate::config::MaterialMode::ButterCream);
        assert!(sim.was_active);
        assert_eq!(sim.marble_pos, Vec2::new(0.1, 0.2));

        // Move to next point, introducing noise, drag, and jitter
        let target = Vec2::new(0.3, 0.4);
        targets[0] = Some(target);
        sim.update(0.016, &targets, 0.025, crate::config::MaterialMode::ButterCream);

        // Ensure marble position shifted from start and is not exactly the target due to physics drift/noise
        assert_ne!(sim.marble_pos, Vec2::new(0.1, 0.2));
        assert_ne!(sim.marble_pos, target);

        // Verify that it is close to target but slightly drifted/jittered (less than 0.1 delta)
        let dist = (sim.marble_pos - target).length();
        assert!(dist < 0.1);

        // Verify marble velocity is populated
        assert_ne!(sim.marble_vel, Vec2::ZERO);
    }
}
