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
    /// Pre-allocated temp buffer for double-buffering settling flows
    pub temp_heights: Vec<f32>,
    /// Current position of the marble in normalized coordinates (range [-1.0, 1.0]).
    pub marble_pos: Vec2,
    /// Previous position of the marble (used for path interpolation).
    pub prev_marble_pos: Vec2,
    /// Track whether the simulation has an active drawing stroke.
    pub was_active: bool,
    
    // Active bounding box for settling updates
    pub active_min_x: usize,
    pub active_max_x: usize,
    pub active_min_y: usize,
    pub active_max_y: usize,
    /// Track if the settling physics is currently active (sand is flowing)
    pub settling_active: bool,
}

impl Simulation {
    pub fn new() -> Self {
        Self {
            // Initializing a 512x512 grid to a default flat sand level of 0.8
            heightmap: Heightmap::new(512, 512, 0.8),
            temp_heights: vec![0.8; 512 * 512],
            marble_pos: Vec2::ZERO,
            prev_marble_pos: Vec2::ZERO,
            was_active: false,
            active_min_x: 0,
            active_max_x: 0,
            active_min_y: 0,
            active_max_y: 0,
            settling_active: false,
        }
    }

    /// Reset the simulation state.
    pub fn reset(&mut self) {
        self.heightmap.reset(0.8);
        self.temp_heights.fill(0.8);
        self.marble_pos = Vec2::ZERO;
        self.prev_marble_pos = Vec2::ZERO;
        self.was_active = false;
        self.active_min_x = 0;
        self.active_max_x = 0;
        self.active_min_y = 0;
        self.active_max_y = 0;
        self.settling_active = false;
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
        self.displace_line(pos, pos, radius);
    }

    /// Draw a line between start and end using interpolation to prevent gaps.
    pub fn draw_line(&mut self, start: Vec2, end: Vec2, radius: f32) {
        self.displace_line(start, end, radius);
    }

    /// Displace sand along a line segment from start to end, carving a groove
    /// and depositing the displaced volume into the surrounding ridge area.
    pub fn displace_line(&mut self, start: Vec2, end: Vec2, radius: f32) {
        if !start.is_finite() || !end.is_finite() || !radius.is_finite() || radius <= 0.0 {
            return;
        }

        let w = self.heightmap.width;
        let h = self.heightmap.height;
        if w == 0 || h == 0 {
            return;
        }

        // Convert coordinates to grid space
        let ax = (start.x + 1.0) * 0.5 * w as f32;
        let ay = (1.0 - start.y) * 0.5 * h as f32;
        let bx = (end.x + 1.0) * 0.5 * w as f32;
        let by = (1.0 - end.y) * 0.5 * h as f32;

        let r_grid = radius * (w as f32 / 2.0);
        let r_grid_clamped = r_grid.min(w as f32);

        // Define ridge width (60% of the marble radius)
        let w_grid = r_grid_clamped * 0.6;
        let total_radius = r_grid_clamped + w_grid;
        let total_radius_clamped = total_radius.min(w as f32);

        // Early out if the swept area is completely outside the grid
        let min_center_x = ax.min(bx);
        let max_center_x = ax.max(bx);
        let min_center_y = ay.min(by);
        let max_center_y = ay.max(by);

        if max_center_x < -total_radius_clamped
            || min_center_x > w as f32 + total_radius_clamped
            || max_center_y < -total_radius_clamped
            || min_center_y > h as f32 + total_radius_clamped
            {
                return;
            }

        // Safe bounding box calculations in float space before casting to usize
        let min_x_float = (min_center_x - total_radius_clamped).clamp(0.0, w as f32).floor();
        let max_x_float = (max_center_x + total_radius_clamped).clamp(0.0, w as f32).ceil();
        let min_y_float = (min_center_y - total_radius_clamped).clamp(0.0, h as f32).floor();
        let max_y_float = (max_center_y + total_radius_clamped).clamp(0.0, h as f32).ceil();

        let min_x = min_x_float as usize;
        let max_x = (max_x_float as usize).min(w - 1);
        let min_y = min_y_float as usize;
        let max_y = (max_y_float as usize).min(h - 1);

        // Update settling active bounding box
        let padding = 15;
        let pad_min_x = min_x.saturating_sub(padding);
        let pad_max_x = (max_x + padding).min(w - 1);
        let pad_min_y = min_y.saturating_sub(padding);
        let pad_max_y = (max_y + padding).min(h - 1);

        if self.settling_active {
            self.active_min_x = self.active_min_x.min(pad_min_x);
            self.active_max_x = self.active_max_x.max(pad_max_x);
            self.active_min_y = self.active_min_y.min(pad_min_y);
            self.active_max_y = self.active_max_y.max(pad_max_y);
        } else {
            self.active_min_x = pad_min_x;
            self.active_max_x = pad_max_x;
            self.active_min_y = pad_min_y;
            self.active_max_y = pad_max_y;
            self.settling_active = true;
        }

        // Segment vector
        let vx = bx - ax;
        let vy = by - ay;
        let len_sq = vx * vx + vy * vy;
        let len = if len_sq >= 1e-6 { len_sq.sqrt() } else { 0.0 };
        let inv_len_sq = if len_sq >= 1e-6 { 1.0 / len_sq } else { 0.0 };

        let r_groove_sq = r_grid_clamped * r_grid_clamped;

        // Ridge ray sampling offsets
        let d1 = r_grid_clamped + w_grid * 0.25;
        let d2 = r_grid_clamped + w_grid * 0.50;
        let d3 = r_grid_clamped + w_grid * 0.75;

        let samples = [
            (d1, 0.5),
            (d2, 1.0 / 3.0),
            (d3, 1.0 / 6.0),
        ];

        // Scan bounding box to carve the groove and displace sand radially/perpendicularly
        for y in min_y..=max_y {
            let py = y as f32 + 0.5;
            let row_offset = y * w;
            for x in min_x..=max_x {
                let px = x as f32 + 0.5;

                // Distance to segment AB (used for carving)
                let (closest_x, closest_y) = if len_sq < 1e-6 {
                    (ax, ay)
                } else {
                    let t = (((px - ax) * vx + (py - ay) * vy) * inv_len_sq).clamp(0.0, 1.0);
                    (ax + t * vx, ay + t * vy)
                };

                let dx = px - closest_x;
                let dy = py - closest_y;
                let dist_sq = dx * dx + dy * dy;

                if dist_sq < r_groove_sq {
                    let dist = dist_sq.sqrt();
                    // Spherical groove profile: z_groove = R - sqrt(R^2 - d^2)
                    let h_target = r_grid_clamped - (r_groove_sq - dist_sq).max(0.0).sqrt();
                    // Scale this target height to flat sand height (0.8)
                    let h_target_norm = (h_target / r_grid_clamped) * 0.8;
                    
                    let current_idx = row_offset + x;
                    let current_h = self.heightmap.data[current_idx];
                    if current_h > h_target_norm {
                        let diff = current_h - h_target_norm;
                        self.heightmap.data[current_idx] = h_target_norm;

                        // Projection on the infinite line (used for perpendicular displacement origin/direction)
                        let (closest_line_x, closest_line_y) = if len_sq < 1e-6 {
                            (ax, ay)
                        } else {
                            let t_unclamped = ((px - ax) * vx + (py - ay) * vy) * inv_len_sq;
                            (ax + t_unclamped * vx, ay + t_unclamped * vy)
                        };

                        let dx_line = px - closest_line_x;
                        let dy_line = py - closest_line_y;
                        let dist_line_sq = dx_line * dx_line + dy_line * dy_line;
                        let dist_line = dist_line_sq.sqrt();

                        // Distribute diff: perpendicular to motion if moving, radial if stationary
                        let (dir_x, dir_y) = if len_sq >= 1e-6 && len > 1e-4 {
                            if dist_line > 1e-4 {
                                (dx_line / dist_line, dy_line / dist_line)
                            } else {
                                // Default perpendicular direction if exactly on the line
                                (-vy / len, vx / len)
                            }
                        } else {
                            if dist > 1e-4 {
                                (dx / dist, dy / dist)
                            } else {
                                (1.0, 0.0)
                            }
                        };

                        for &(d_sample, weight) in &samples {
                            let rx = (closest_line_x + dir_x * d_sample).floor() as isize;
                            let ry = (closest_line_y + dir_y * d_sample).floor() as isize;

                            if rx >= 0 && rx < w as isize && ry >= 0 && ry < h as isize {
                                let ridx = (ry * w as isize + rx) as usize;
                                self.heightmap.data[ridx] += diff * weight;
                            }
                        }
                    }
                }
            }
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
                // Displace along the path line segment
                self.displace_line(self.prev_marble_pos, self.marble_pos, marble_radius);
            } else {
                // First tick of a new drag/click: teleport without drawing from old position
                self.marble_pos = clamped_target;
                self.prev_marble_pos = clamped_target;
                self.displace_line(clamped_target, clamped_target, marble_radius);
                self.was_active = true;
            }
        } else {
            self.was_active = false;
        }

        // Run the gravity-driven settling cellular automata tick
        self.settle_tick();
    }

    /// Perform a single gravity flow/settling iteration inside the active bounding box.
    pub fn settle_tick(&mut self) -> f32 {
        if !self.settling_active {
            return 0.0;
        }

        let w = self.heightmap.width;
        let h = self.heightmap.height;

        // Safety check to prevent panics if heightmap is resized
        if self.temp_heights.len() != self.heightmap.data.len() {
            self.temp_heights.resize(self.heightmap.data.len(), 0.8);
        }

        // 1. Determine copy boundaries (expanded by 1 to include neighbors)
        let copy_min_x = self.active_min_x.saturating_sub(1);
        let copy_max_x = (self.active_max_x + 1).min(w - 1);
        let copy_min_y = self.active_min_y.saturating_sub(1);
        let copy_max_y = (self.active_max_y + 1).min(h - 1);

        // Copy heightmap to temp buffer inside the expanded bounding box
        for y in copy_min_y..=copy_max_y {
            let offset = y * w;
            self.temp_heights[offset + copy_min_x..=offset + copy_max_x]
                .copy_from_slice(&self.heightmap.data[offset + copy_min_x..=offset + copy_max_x]);
        }

        let threshold = 0.04f32; // Slope threshold (angle of repose)
        let alpha = 0.15f32;     // Flow rate factor
        let mut total_flow = 0.0f32;

        // Dynamic active box tracking for the next frame
        let mut next_min_x = w;
        let mut next_max_x = 0;
        let mut next_min_y = h;
        let mut next_max_y = 0;
        let mut flow_occurred = false;

        // 2. Cellular automata slope settling update (loop over core active box)
        for y in self.active_min_y..=self.active_max_y {
            let row_offset = y * w;
            for x in self.active_min_x..=self.active_max_x {
                let center_idx = row_offset + x;
                let h_center = self.heightmap.data[center_idx];

                // Left neighbor
                if x > 0 {
                    let nx = x - 1;
                    let neighbor_idx = center_idx - 1;
                    let h_neighbor = self.heightmap.data[neighbor_idx];
                    if h_center - h_neighbor > threshold {
                        let flow = alpha * (h_center - h_neighbor - threshold);
                        if flow > 0.0 {
                            self.temp_heights[center_idx] -= flow;
                            self.temp_heights[neighbor_idx] += flow;
                            total_flow += flow;
                            if flow > 1e-5 {
                                next_min_x = next_min_x.min(nx);
                                next_max_x = next_max_x.max(x);
                                next_min_y = next_min_y.min(y);
                                next_max_y = next_max_y.max(y);
                                flow_occurred = true;
                            }
                        }
                    }
                }

                // Right neighbor
                if x + 1 < w {
                    let nx = x + 1;
                    let neighbor_idx = center_idx + 1;
                    let h_neighbor = self.heightmap.data[neighbor_idx];
                    if h_center - h_neighbor > threshold {
                        let flow = alpha * (h_center - h_neighbor - threshold);
                        if flow > 0.0 {
                            self.temp_heights[center_idx] -= flow;
                            self.temp_heights[neighbor_idx] += flow;
                            total_flow += flow;
                            if flow > 1e-5 {
                                next_min_x = next_min_x.min(x);
                                next_max_x = next_max_x.max(nx);
                                next_min_y = next_min_y.min(y);
                                next_max_y = next_max_y.max(y);
                                flow_occurred = true;
                            }
                        }
                    }
                }

                // Top neighbor
                if y > 0 {
                    let ny = y - 1;
                    let neighbor_idx = center_idx - w;
                    let h_neighbor = self.heightmap.data[neighbor_idx];
                    if h_center - h_neighbor > threshold {
                        let flow = alpha * (h_center - h_neighbor - threshold);
                        if flow > 0.0 {
                            self.temp_heights[center_idx] -= flow;
                            self.temp_heights[neighbor_idx] += flow;
                            total_flow += flow;
                            if flow > 1e-5 {
                                next_min_x = next_min_x.min(x);
                                next_max_x = next_max_x.max(x);
                                next_min_y = next_min_y.min(ny);
                                next_max_y = next_max_y.max(y);
                                flow_occurred = true;
                            }
                        }
                    }
                }

                // Bottom neighbor
                if y + 1 < h {
                    let ny = y + 1;
                    let neighbor_idx = center_idx + w;
                    let h_neighbor = self.heightmap.data[neighbor_idx];
                    if h_center - h_neighbor > threshold {
                        let flow = alpha * (h_center - h_neighbor - threshold);
                        if flow > 0.0 {
                            self.temp_heights[center_idx] -= flow;
                            self.temp_heights[neighbor_idx] += flow;
                            total_flow += flow;
                            if flow > 1e-5 {
                                next_min_x = next_min_x.min(x);
                                next_max_x = next_max_x.max(x);
                                next_min_y = next_min_y.min(y);
                                next_max_y = next_max_y.max(ny);
                                flow_occurred = true;
                            }
                        }
                    }
                }
            }
        }

        // 3. Copy back the updated heights from the expanded bounding box
        for y in copy_min_y..=copy_max_y {
            let offset = y * w;
            self.heightmap.data[offset + copy_min_x..=offset + copy_max_x]
                .copy_from_slice(&self.temp_heights[offset + copy_min_x..=offset + copy_max_x]);
        }

        // 4. Update the active bounding box based on dynamic flow tracking
        if flow_occurred {
            let padding = 1;
            self.active_min_x = next_min_x.saturating_sub(padding);
            self.active_max_x = (next_max_x + padding).min(w - 1);
            self.active_min_y = next_min_y.saturating_sub(padding);
            self.active_max_y = (next_max_y + padding).min(h - 1);
            self.settling_active = true;
        } else {
            self.settling_active = false;
        }

        total_flow
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
        
        // Check that some points are carved below 0.1, and bounds are respected
        let mut modified_count = 0;
        for &val in sim.heightmap.as_slice() {
            if val < 0.1 {
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
        
        assert!(sim.heightmap.get(cx1, cy1) < 0.01);
        assert!(sim.heightmap.get(cx2, cy2) < 0.01);
        assert!(sim.heightmap.get(cx3, cy3) < 0.01);
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

    #[test]
    fn test_volume_conservation() {
        let mut sim = Simulation::new();
        // Set the heightmap to 0.4 to ensure we have enough headroom (up to 1.0) for ridges
        sim.heightmap.reset(0.4);
        let initial_sum: f64 = sim.heightmap.as_slice().iter().map(|&x| x as f64).sum();

        // Perform displacement along a path
        sim.displace_line(Vec2::new(-0.2, 0.2), Vec2::new(0.2, -0.2), 0.03);

        let final_sum: f64 = sim.heightmap.as_slice().iter().map(|&x| x as f64).sum();
        
        // Assert that the total volume (sum of heightmap) is conserved within floating-point epsilon
        let diff = (final_sum - initial_sum).abs();
        assert!(diff < 1e-2, "Volume not conserved! diff = {}", diff);
    }

    #[test]
    fn test_draw_line_extreme_coordinates_overflow() {
        let mut sim = Simulation::new();
        // Spanning across extreme opposite coordinates should not overflow or panic.
        sim.draw_line(Vec2::new(-1e18, 0.0), Vec2::new(1e18, 0.0), 0.1);
    }

    #[test]
    fn test_zero_dimension_heightmap() {
        let hm = Heightmap::new(0, 0, 0.8);
        let mut sim = Simulation {
            heightmap: hm,
            temp_heights: Vec::new(),
            marble_pos: Vec2::ZERO,
            prev_marble_pos: Vec2::ZERO,
            was_active: false,
            active_min_x: 0,
            active_max_x: 0,
            active_min_y: 0,
            active_max_y: 0,
            settling_active: false,
        };
        // Drawing on a 0x0 heightmap should not panic or loop infinitely.
        sim.draw_line(Vec2::ZERO, Vec2::ZERO, 0.1);
    }

    #[test]
    fn test_volume_conservation_with_saturation() {
        let mut sim = Simulation::new();
        // Initialize to high level to trigger height clamping (saturation)
        sim.heightmap.reset(0.70);
        let initial_sum: f64 = sim.heightmap.as_slice().iter().map(|&x| x as f64).sum();

        // Perform displacement at a single point to trigger local saturation in the inner ridge
        sim.displace_line(Vec2::ZERO, Vec2::ZERO, 0.02);

        let final_sum: f64 = sim.heightmap.as_slice().iter().map(|&x| x as f64).sum();
        let diff = (final_sum - initial_sum).abs();
        // Strict conservation check under saturation conditions
        assert!(diff < 1e-2, "Volume not conserved! diff = {}", diff);
    }

    #[test]
    fn test_settling_flow_and_volume_conservation() {
        let mut sim = Simulation::new();
        sim.heightmap.reset(0.5);

        // Put a pile of height 0.8 at (256, 256)
        let center_idx = 256 * 512 + 256;
        sim.heightmap.data[center_idx] = 0.8;

        // Set active box around the pile manually for testing
        sim.active_min_x = 250;
        sim.active_max_x = 262;
        sim.active_min_y = 250;
        sim.active_max_y = 262;
        sim.settling_active = true;

        let initial_sum: f64 = sim.heightmap.as_slice().iter().map(|&x| x as f64).sum();

        // Run multiple settle ticks and assert flow occurred
        let mut flow_occurred = false;
        for _ in 0..10 {
            let flow = sim.settle_tick();
            if flow > 0.0 {
                flow_occurred = true;
            }
        }

        assert!(flow_occurred, "Sand should flow down from the peak");

        let final_sum: f64 = sim.heightmap.as_slice().iter().map(|&x| x as f64).sum();
        let diff = (final_sum - initial_sum).abs();
        assert!(diff < 1e-5, "Settling did not conserve volume! diff = {}", diff);

        // Verify the peak height decreased as sand flowed to neighbors
        assert!(sim.heightmap.data[center_idx] < 0.8, "Peak should be lower after flowing");
    }

    #[test]
    fn test_settling_deactivation() {
        let mut sim = Simulation::new();
        sim.heightmap.reset(0.5);
        
        // Setup flat heightmap (no slope exceeding threshold)
        sim.active_min_x = 250;
        sim.active_max_x = 262;
        sim.active_min_y = 250;
        sim.active_max_y = 262;
        sim.settling_active = true;

        // Single tick on flat ground should deactivate settling
        let flow = sim.settle_tick();
        assert_eq!(flow, 0.0);
        assert!(!sim.settling_active, "Settling should deactivate when stable");
    }
}
