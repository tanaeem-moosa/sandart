use glam::Vec2;
pub mod grid;
pub mod physics;

pub use grid::Heightmap;
pub use physics::{ActiveBounds, displace_line, settle_tick};

/// Coordinates the state of the marble and the sand bed heightmap.
pub struct Simulation {
    /// The sand heightmap grid.
    pub heightmap: Heightmap,
    /// Pre-allocated temp buffer for double-buffering settling flows.
    pub temp_heights: Vec<f32>,
    /// Current position of the marble in normalized coordinates (range [-1.0, 1.0]).
    pub marble_pos: Vec2,
    /// Previous position of the marble (used for path interpolation).
    pub prev_marble_pos: Vec2,
    /// Last velocity of the marble.
    pub marble_vel: Vec2,
    /// Track whether the simulation has an active drawing stroke.
    pub was_active: bool,
    /// Active bounding box for settling updates.
    pub active_bounds: ActiveBounds,
    /// Seed for marble movement noise.
    pub seed: u32,
}

impl Simulation {
    pub fn new() -> Self {
        let mut heightmap = Heightmap::new(512, 512, 0.8);
        // Add random noise to the initial sand bed
        let mut seed = 12345u32;
        for val in heightmap.data.iter_mut() {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            let noise = (seed as f32 / u32::MAX as f32 - 0.5) * 0.05; // Range [-0.025, 0.025]
            *val = (*val + noise).clamp(0.0, 1.0);
        }
        let temp_heights = heightmap.data.clone();

        Self {
            heightmap,
            temp_heights,
            marble_pos: Vec2::ZERO,
            prev_marble_pos: Vec2::ZERO,
            marble_vel: Vec2::ZERO,
            was_active: false,
            active_bounds: ActiveBounds {
                min_x: 0,
                max_x: 0,
                min_y: 0,
                max_y: 0,
                active: false,
            },
            seed: 98765u32,
        }
    }

    /// Reset the simulation state.
    pub fn reset(&mut self) {
        self.heightmap.reset(0.8);
        // Add random noise to the reset sand bed
        let mut seed = 54321u32;
        for val in self.heightmap.data.iter_mut() {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            let noise = (seed as f32 / u32::MAX as f32 - 0.5) * 0.05; // Range [-0.025, 0.025]
            *val = (*val + noise).clamp(0.0, 1.0);
        }
        self.temp_heights.copy_from_slice(&self.heightmap.data);
        self.marble_pos = Vec2::ZERO;
        self.prev_marble_pos = Vec2::ZERO;
        self.marble_vel = Vec2::ZERO;
        self.was_active = false;
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
    pub fn update(&mut self, _dt: f32, target_pos: Option<Vec2>, marble_radius: f32) {
        // Prevent seed degeneracy (XORShift stuck state at 0)
        if self.seed == 0 {
            self.seed = 98765u32;
        }

        // Advance seed every frame to keep settling dynamics active and non-deterministic
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        let time_seed = self.seed;

        if let Some(target) = target_pos {
            // Sanitize target coordinate float boundaries against NaNs/Infs
            let tx = if target.x.is_finite() { target.x } else { 0.0 };
            let ty = if target.y.is_finite() { target.y } else { 0.0 };
            let target_sanitized = Vec2::new(tx, ty);

            let max_r = (0.92 - marble_radius).max(0.0);
            let clamped_target = target_sanitized.clamp(Vec2::splat(-max_r), Vec2::splat(max_r));

            if self.was_active {
                self.prev_marble_pos = self.marble_pos;

                // Calculate step vector and distance
                let raw_diff = clamped_target - self.marble_pos;
                let raw_dist = raw_diff.length();

                // 1. Generate pseudo-random numbers
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
                next_pos = next_pos.clamp(Vec2::splat(-max_r), Vec2::splat(max_r));

                self.marble_pos = next_pos;
                self.marble_vel = next_pos - self.prev_marble_pos;

                displace_line(
                    &mut self.heightmap,
                    self.prev_marble_pos,
                    self.marble_pos,
                    marble_radius,
                    &mut self.active_bounds,
                );
            } else {
                self.marble_pos = clamped_target;
                self.prev_marble_pos = clamped_target;
                self.marble_vel = Vec2::ZERO;
                displace_line(
                    &mut self.heightmap,
                    clamped_target,
                    clamped_target,
                    marble_radius,
                    &mut self.active_bounds,
                );
                self.was_active = true;
            }
        } else {
            self.was_active = false;
        }

        // Run the gravity-driven settling cellular automata tick
        if self.active_bounds.active {
            settle_tick(
                &mut self.heightmap,
                &mut self.temp_heights,
                &mut self.active_bounds,
                0.04, // default repose angle threshold
                0.15, // default flow rate alpha
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
        // Initially target is None, should not be active
        sim.update(0.016, None, 0.025);
        assert!(!sim.was_active);

        // Move to start point (first point is exact target)
        sim.update(0.016, Some(Vec2::new(0.1, 0.2)), 0.025);
        assert!(sim.was_active);
        assert_eq!(sim.marble_pos, Vec2::new(0.1, 0.2));

        // Move to next point, introducing noise, drag, and jitter
        let target = Vec2::new(0.3, 0.4);
        sim.update(0.016, Some(target), 0.025);

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
