use glam::Vec2;
pub mod grid;
pub mod physics;

pub use grid::Heightmap;
pub use physics::{displace_line, settle_tick, ActiveBounds};

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
    /// Track whether the simulation has an active drawing stroke.
    pub was_active: bool,
    /// Active bounding box for settling updates.
    pub active_bounds: ActiveBounds,
}

impl Simulation {
    pub fn new() -> Self {
        Self {
            heightmap: Heightmap::new(512, 512, 0.8),
            temp_heights: vec![0.8; 512 * 512],
            marble_pos: Vec2::ZERO,
            prev_marble_pos: Vec2::ZERO,
            was_active: false,
            active_bounds: ActiveBounds {
                min_x: 0,
                max_x: 0,
                min_y: 0,
                max_y: 0,
                active: false,
            },
        }
    }

    /// Reset the simulation state.
    pub fn reset(&mut self) {
        self.heightmap.reset(0.8);
        self.temp_heights.fill(0.8);
        self.marble_pos = Vec2::ZERO;
        self.prev_marble_pos = Vec2::ZERO;
        self.was_active = false;
        self.active_bounds = ActiveBounds {
            min_x: 0,
            max_x: 0,
            min_y: 0,
            max_y: 0,
            active: false,
        };
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
        displace_line(&mut self.heightmap, pos, pos, radius, &mut self.active_bounds);
    }

    /// Draw a line between start and end using interpolation to prevent gaps.
    #[allow(dead_code)]
    pub fn draw_line(&mut self, start: Vec2, end: Vec2, radius: f32) {
        displace_line(&mut self.heightmap, start, end, radius, &mut self.active_bounds);
    }

    /// Run a physics frame tick.
    pub fn update(&mut self, _dt: f32, target_pos: Option<Vec2>, marble_radius: f32) {
        if let Some(target) = target_pos {
            let max_r = (0.92 - marble_radius).max(0.0);
            let clamped_target = target.clamp(Vec2::splat(-max_r), Vec2::splat(max_r));

            if self.was_active {
                self.prev_marble_pos = self.marble_pos;
                self.marble_pos = clamped_target;
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
        assert_eq!(sim.heightmap.get(100, 100), 0.8);
    }

    #[test]
    fn test_norm_to_grid_mapping() {
        let width = 512;
        let height = 512;

        // Verify corners map to exact boundary indexes
        assert_eq!(Simulation::norm_to_grid(Vec2::new(-1.0, 1.0), width, height), (0, 0));
        assert_eq!(Simulation::norm_to_grid(Vec2::new(1.0, -1.0), width, height), (width - 1, height - 1));

        // Verify center mapping falls in correct bins (256, 256)
        assert_eq!(Simulation::norm_to_grid(Vec2::new(0.0, 0.0), width, height), (256, 256));

        // Verify bounds clamping maps out of bounds coordinates to grid edges safely
        assert_eq!(Simulation::norm_to_grid(Vec2::new(-2.0, 2.0), width, height), (0, 0));
        assert_eq!(Simulation::norm_to_grid(Vec2::new(2.0, -2.0), width, height), (width - 1, height - 1));
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
}
