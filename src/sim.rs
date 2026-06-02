use glam::Vec2;

/// A 2D heightmap representing the sand bed.
pub struct Heightmap {
    pub width: usize,
    pub height: usize,
    pub data: Vec<f32>,
}

impl Heightmap {
    /// Create a new heightmap of the specified dimensions, initialized to a flat value.
    pub fn new(width: usize, height: usize, initial_value: f32) -> Self {
        Self {
            width,
            height,
            data: vec![initial_value; width * height],
        }
    }

    /// Reset the heightmap back to a uniform flat value.
    pub fn reset(&mut self, value: f32) {
        self.data.fill(value);
    }

    /// Retrieve the height at a specific grid index with boundary checking.
    #[inline]
    pub fn get(&self, x: usize, y: usize) -> f32 {
        if x < self.width && y < self.height {
            self.data[y * self.width + x]
        } else {
            0.0
        }
    }

    /// Set the height at a specific grid index with boundary checking.
    #[inline]
    pub fn set(&mut self, x: usize, y: usize, value: f32) {
        if x < self.width && y < self.height {
            self.data[y * self.width + x] = value.clamp(0.0, 1.0);
        }
    }

    /// Get a read-only slice of the underlying float data.
    pub fn as_slice(&self) -> &[f32] {
        &self.data
    }
}

/// Coordinates the state of the marble and the sand bed heightmap.
pub struct Simulation {
    /// The sand heightmap grid.
    pub heightmap: Heightmap,
    /// Current position of the marble in normalized coordinates (range [-1.0, 1.0]).
    pub marble_pos: Vec2,
    /// Previous position of the marble (used for path interpolation).
    pub prev_marble_pos: Vec2,
    /// Track whether the simulation has an active drawing stroke.
    pub was_active: bool,
}

impl Simulation {
    pub fn new() -> Self {
        Self {
            // Initializing a 512x512 grid to a default flat sand level of 0.8
            heightmap: Heightmap::new(512, 512, 0.8),
            marble_pos: Vec2::ZERO,
            prev_marble_pos: Vec2::ZERO,
            was_active: false,
        }
    }

    /// Reset the simulation state.
    pub fn reset(&mut self) {
        self.heightmap.reset(0.8);
        self.marble_pos = Vec2::ZERO;
        self.prev_marble_pos = Vec2::ZERO;
        self.was_active = false;
    }

    /// Convert normalized Cartesian coordinates ([-1.0, 1.0]) to grid index coordinates.
    pub fn norm_to_grid(pos: Vec2, width: usize, height: usize) -> (usize, usize) {
        let px = if pos.x.is_finite() { pos.x } else { 0.0 };
        let py = if pos.y.is_finite() { pos.y } else { 0.0 };
        let x = ((px + 1.0) * 0.5 * width as f32).clamp(0.0, (width - 1) as f32) as usize;
        let y = ((1.0 - py) * 0.5 * height as f32).clamp(0.0, (height - 1) as f32) as usize;
        (x, y)
    }

    /// Erase height values inside the marble radius to 0.0 with sub-pixel precision.
    pub fn draw_point(&mut self, pos: Vec2, radius: f32) {
        if !pos.is_finite() || !radius.is_finite() || radius <= 0.0 {
            return;
        }

        let w = self.heightmap.width;
        let h = self.heightmap.height;

        // Sub-pixel grid coordinates of the marble center
        let fx = (pos.x + 1.0) * 0.5 * w as f32;
        let fy = (1.0 - pos.y) * 0.5 * h as f32;

        // Convert marble radius to grid units
        let r_grid = radius * (w as f32 / 2.0);

        // Clamp grid radius to prevent excessive loop sizes (DoS protection)
        let r_grid_clamped = r_grid.min(w as f32);

        // Early out if the center is too far outside the grid to affect any cells (prevents integer overflow on casts)
        let max_bound_x = w as f32 + r_grid_clamped;
        let max_bound_y = h as f32 + r_grid_clamped;
        if fx < -r_grid_clamped || fx > max_bound_x || fy < -r_grid_clamped || fy > max_bound_y {
            return;
        }

        let r_grid_i = r_grid_clamped.ceil() as isize;

        let cx = fx as isize;
        let cy = fy as isize;

        let min_x_raw = cx - r_grid_i;
        let max_x_raw = cx + r_grid_i;
        let min_y_raw = cy - r_grid_i;
        let max_y_raw = cy + r_grid_i;

        // Early out if the bounding box is completely outside the grid
        if max_x_raw < 0 || min_x_raw >= w as isize || max_y_raw < 0 || min_y_raw >= h as isize {
            return;
        }

        let min_x = min_x_raw.max(0) as usize;
        let max_x = max_x_raw.min(w as isize - 1) as usize;
        let min_y = min_y_raw.max(0) as usize;
        let max_y = max_y_raw.min(h as isize - 1) as usize;

        let r_grid_sq = r_grid_clamped * r_grid_clamped;

        for y in min_y..=max_y {
            let dy = (y as f32 + 0.5) - fy;
            let dy_sq = dy * dy;
            let row_offset = y * w;
            let row_slice = &mut self.heightmap.data[row_offset..row_offset + w];
            for x in min_x..=max_x {
                let dx = (x as f32 + 0.5) - fx;
                let dist_sq = dx * dx + dy_sq;
                if dist_sq <= r_grid_sq {
                    row_slice[x] = 0.0;
                }
            }
        }
    }

    /// Draw a line between start and end using interpolation to prevent gaps.
    pub fn draw_line(&mut self, start: Vec2, end: Vec2, radius: f32) {
        let dist = start.distance(end);
        let step = radius * 0.5; // step size is half the radius to ensure overlap and no gaps
        if dist > step && step > 0.0 {
            let steps = (dist / step).ceil() as usize;
            let steps_clamped = steps.min(1000); // Guard against infinite looping
            for i in 0..=steps_clamped {
                let t = i as f32 / steps_clamped as f32;
                let pos = start.lerp(end, t);
                self.draw_point(pos, radius);
            }
        } else {
            self.draw_point(end, radius);
        }
    }

    /// Run a physics frame tick.
    pub fn update(&mut self, _dt: f32, target_pos: Option<Vec2>, marble_radius: f32) {
        if let Some(target) = target_pos {
            let max_r = (0.92 - marble_radius).max(0.0);
            let clamped_target = target.clamp(Vec2::splat(-max_r), Vec2::splat(max_r));

            if self.was_active {
                self.prev_marble_pos = self.marble_pos;
                self.marble_pos = clamped_target;
                // Draw interpolated line to prevent gaps when dragging fast
                self.draw_line(self.prev_marble_pos, self.marble_pos, marble_radius);
            } else {
                // First tick of a new drag/click: teleport without drawing from old position
                self.marble_pos = clamped_target;
                self.prev_marble_pos = clamped_target;
                self.draw_point(clamped_target, marble_radius);
                self.was_active = true;
            }
        } else {
            self.was_active = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_heightmap_bounds() {
        let mut hm = Heightmap::new(10, 10, 0.5);
        assert_eq!(hm.get(5, 5), 0.5);
        hm.set(5, 5, 0.8);
        assert_eq!(hm.get(5, 5), 0.8);
        
        // Out of bounds get/set should not panic
        assert_eq!(hm.get(10, 10), 0.0);
        hm.set(10, 10, 0.9);
        assert_eq!(hm.get(10, 10), 0.0);
    }

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

    #[test]
    fn test_draw_point_out_of_bounds() {
        let mut sim = Simulation::new();
        
        // Drawing completely offscreen should not panic or modify the heightmap
        sim.draw_point(Vec2::new(5.0, 5.0), 0.1);
        
        // Assert that heightmap data is unchanged
        for &val in sim.heightmap.as_slice() {
            assert_eq!(val, 0.8);
        }
    }

    #[test]
    fn test_draw_point_partial_overlap() {
        let mut sim = Simulation::new();
        
        // Position marble so it sits on the left boundary
        sim.draw_point(Vec2::new(-1.0, 0.0), 0.05);
        
        // Check that some points are set to 0.0, and bounds are respected
        let mut modified_count = 0;
        for &val in sim.heightmap.as_slice() {
            if val == 0.0 {
                modified_count += 1;
            }
        }
        assert!(modified_count > 0);
    }

    #[test]
    fn test_draw_line_interpolation() {
        let mut sim = Simulation::new();
        
        // Draw a line from (-0.5, 0.0) to (0.5, 0.0)
        sim.draw_line(Vec2::new(-0.5, 0.0), Vec2::new(0.5, 0.0), 0.05);
        
        // Verify that the path is continuous by checking that the center points are drawn
        let (cx1, cy1) = Simulation::norm_to_grid(Vec2::new(-0.5, 0.0), 512, 512);
        let (cx2, cy2) = Simulation::norm_to_grid(Vec2::new(0.0, 0.0), 512, 512);
        let (cx3, cy3) = Simulation::norm_to_grid(Vec2::new(0.5, 0.0), 512, 512);
        
        assert_eq!(sim.heightmap.get(cx1, cy1), 0.0);
        assert_eq!(sim.heightmap.get(cx2, cy2), 0.0);
        assert_eq!(sim.heightmap.get(cx3, cy3), 0.0);
    }

    #[test]
    fn test_draw_point_extreme_coordinates_overflow() {
        let mut sim = Simulation::new();
        // These coordinates are extremely large and could cause an integer overflow during isize casting.
        // We assert that it early-outs safely and does not modify the heightmap.
        sim.draw_point(Vec2::new(1e18, 1e18), 0.1);
        for &val in sim.heightmap.as_slice() {
            assert_eq!(val, 0.8);
        }
    }
}
