use glam::Vec2;
use crate::sim::grid::Heightmap;

/// Bounding coordinates to optimize Cellular Automata settling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActiveBounds {
    pub min_x: usize,
    pub max_x: usize,
    pub min_y: usize,
    pub max_y: usize,
    pub active: bool,
}

/// Displace sand along a line segment from start to end, carving a groove
/// and depositing the displaced volume into the surrounding ridge area.
pub fn displace_line(
    heightmap: &mut Heightmap,
    start: Vec2,
    end: Vec2,
    radius: f32,
    active_bounds: &mut ActiveBounds,
) {
    if !start.is_finite() || !end.is_finite() || !radius.is_finite() || radius <= 0.0 {
        return;
    }

    let w = heightmap.width;
    let h = heightmap.height;
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

    if active_bounds.active {
        active_bounds.min_x = active_bounds.min_x.min(pad_min_x);
        active_bounds.max_x = active_bounds.max_x.max(pad_max_x);
        active_bounds.min_y = active_bounds.min_y.min(pad_min_y);
        active_bounds.max_y = active_bounds.max_y.max(pad_max_y);
    } else {
        active_bounds.min_x = pad_min_x;
        active_bounds.max_x = pad_max_x;
        active_bounds.min_y = pad_min_y;
        active_bounds.max_y = pad_max_y;
        active_bounds.active = true;
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
                let current_h = heightmap.data[current_idx];
                if current_h > h_target_norm {
                    let diff = current_h - h_target_norm;
                    heightmap.data[current_idx] = h_target_norm;

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
                            heightmap.data[ridx] += diff * weight;
                        }
                    }
                }
            }
        }
    }
}

/// Perform a single gravity flow/settling iteration inside the active bounding box.
pub fn settle_tick(
    heightmap: &mut Heightmap,
    temp_heights: &mut Vec<f32>,
    active_bounds: &mut ActiveBounds,
    threshold: f32,
    alpha: f32,
) -> f32 {
    if !active_bounds.active {
        return 0.0;
    }

    let w = heightmap.width;
    let h = heightmap.height;

    // Safety check to prevent panics if heightmap is resized
    if temp_heights.len() != heightmap.data.len() {
        temp_heights.resize(heightmap.data.len(), 0.8);
    }

    // 1. Determine copy boundaries (expanded by 1 to include neighbors)
    let copy_min_x = active_bounds.min_x.saturating_sub(1);
    let copy_max_x = (active_bounds.max_x + 1).min(w - 1);
    let copy_min_y = active_bounds.min_y.saturating_sub(1);
    let copy_max_y = (active_bounds.max_y + 1).min(h - 1);

    // Copy heightmap to temp buffer inside the expanded bounding box
    for y in copy_min_y..=copy_max_y {
        let offset = y * w;
        temp_heights[offset + copy_min_x..=offset + copy_max_x]
            .copy_from_slice(&heightmap.data[offset + copy_min_x..=offset + copy_max_x]);
    }

    let mut total_flow = 0.0f32;

    // Dynamic active box tracking for the next frame
    let mut next_min_x = w;
    let mut next_max_x = 0;
    let mut next_min_y = h;
    let mut next_max_y = 0;
    let mut flow_occurred = false;

    // 2. Cellular automata slope settling update (loop over core active box)
    for y in active_bounds.min_y..=active_bounds.max_y {
        let row_offset = y * w;
        for x in active_bounds.min_x..=active_bounds.max_x {
            let center_idx = row_offset + x;
            let h_center = heightmap.data[center_idx];

            // Left neighbor
            if x > 0 {
                let nx = x - 1;
                let neighbor_idx = center_idx - 1;
                let h_neighbor = heightmap.data[neighbor_idx];
                if h_center - h_neighbor > threshold {
                    let flow = alpha * (h_center - h_neighbor - threshold);
                    if flow > 0.0 {
                        temp_heights[center_idx] -= flow;
                        temp_heights[neighbor_idx] += flow;
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
                let h_neighbor = heightmap.data[neighbor_idx];
                if h_center - h_neighbor > threshold {
                    let flow = alpha * (h_center - h_neighbor - threshold);
                    if flow > 0.0 {
                        temp_heights[center_idx] -= flow;
                        temp_heights[neighbor_idx] += flow;
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
                let h_neighbor = heightmap.data[neighbor_idx];
                if h_center - h_neighbor > threshold {
                    let flow = alpha * (h_center - h_neighbor - threshold);
                    if flow > 0.0 {
                        temp_heights[center_idx] -= flow;
                        temp_heights[neighbor_idx] += flow;
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
                let h_neighbor = heightmap.data[neighbor_idx];
                if h_center - h_neighbor > threshold {
                    let flow = alpha * (h_center - h_neighbor - threshold);
                    if flow > 0.0 {
                        temp_heights[center_idx] -= flow;
                        temp_heights[neighbor_idx] += flow;
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
        heightmap.data[offset + copy_min_x..=offset + copy_max_x]
            .copy_from_slice(&temp_heights[offset + copy_min_x..=offset + copy_max_x]);
    }

    // 4. Update the active bounding box based on dynamic flow tracking
    if flow_occurred {
        let padding = 1;
        active_bounds.min_x = next_min_x.saturating_sub(padding);
        active_bounds.max_x = (next_max_x + padding).min(w - 1);
        active_bounds.min_y = next_min_y.saturating_sub(padding);
        active_bounds.max_y = (next_max_y + padding).min(h - 1);
        active_bounds.active = true;
    } else {
        active_bounds.active = false;
    }

    total_flow
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_draw_point_out_of_bounds() {
        let mut hm = Heightmap::new(512, 512, 0.8);
        let mut bounds = ActiveBounds { min_x: 0, max_x: 0, min_y: 0, max_y: 0, active: false };
        
        // Drawing completely offscreen should not panic or modify the heightmap
        displace_line(&mut hm, Vec2::new(5.0, 5.0), Vec2::new(5.0, 5.0), 0.1, &mut bounds);
        
        // Assert that heightmap data is unchanged
        for &val in hm.as_slice() {
            assert_eq!(val, 0.8);
        }
    }

    #[test]
    fn test_draw_point_partial_overlap() {
        let mut hm = Heightmap::new(512, 512, 0.8);
        let mut bounds = ActiveBounds { min_x: 0, max_x: 0, min_y: 0, max_y: 0, active: false };
        
        // Position marble so it sits on the left boundary
        displace_line(&mut hm, Vec2::new(-1.0, 0.0), Vec2::new(-1.0, 0.0), 0.05, &mut bounds);
        
        // Check that some points are carved below 0.1, and bounds are respected
        let mut modified_count = 0;
        for &val in hm.as_slice() {
            if val < 0.1 {
                modified_count += 1;
            }
        }
        assert!(modified_count > 0);
        assert!(bounds.active);
    }

    #[test]
    fn test_draw_line_interpolation() {
        let mut hm = Heightmap::new(512, 512, 0.8);
        let mut bounds = ActiveBounds { min_x: 0, max_x: 0, min_y: 0, max_y: 0, active: false };
        
        // Draw a line from (-0.5, 0.0) to (0.5, 0.0)
        displace_line(&mut hm, Vec2::new(-0.5, 0.0), Vec2::new(0.5, 0.0), 0.05, &mut bounds);
        
        // Helper to convert pos to grid index
        let norm_to_grid = |pos: Vec2| {
            let x = ((pos.x + 1.0) * 0.5 * 512.0).clamp(0.0, 511.0) as usize;
            let y = ((1.0 - pos.y) * 0.5 * 512.0).clamp(0.0, 511.0) as usize;
            (x, y)
        };
        
        // Verify that the path is continuous by checking that the center points are drawn
        let (cx1, cy1) = norm_to_grid(Vec2::new(-0.5, 0.0));
        let (cx2, cy2) = norm_to_grid(Vec2::new(0.0, 0.0));
        let (cx3, cy3) = norm_to_grid(Vec2::new(0.5, 0.0));
        
        assert!(hm.get(cx1, cy1) < 0.01);
        assert!(hm.get(cx2, cy2) < 0.01);
        assert!(hm.get(cx3, cy3) < 0.01);
    }

    #[test]
    fn test_draw_point_extreme_coordinates_overflow() {
        let mut hm = Heightmap::new(512, 512, 0.8);
        let mut bounds = ActiveBounds { min_x: 0, max_x: 0, min_y: 0, max_y: 0, active: false };
        
        displace_line(&mut hm, Vec2::new(1e18, 1e18), Vec2::new(1e18, 1e18), 0.1, &mut bounds);
        for &val in hm.as_slice() {
            assert_eq!(val, 0.8);
        }
    }

    #[test]
    fn test_volume_conservation() {
        let mut hm = Heightmap::new(512, 512, 0.4);
        let mut bounds = ActiveBounds { min_x: 0, max_x: 0, min_y: 0, max_y: 0, active: false };
        let initial_sum: f64 = hm.as_slice().iter().map(|&x| x as f64).sum();

        // Perform displacement along a path
        displace_line(&mut hm, Vec2::new(-0.2, 0.2), Vec2::new(0.2, -0.2), 0.03, &mut bounds);

        let final_sum: f64 = hm.as_slice().iter().map(|&x| x as f64).sum();
        let diff = (final_sum - initial_sum).abs();
        assert!(diff < 1e-2, "Volume not conserved! diff = {}", diff);
    }

    #[test]
    fn test_draw_line_extreme_coordinates_overflow() {
        let mut hm = Heightmap::new(512, 512, 0.8);
        let mut bounds = ActiveBounds { min_x: 0, max_x: 0, min_y: 0, max_y: 0, active: false };
        displace_line(&mut hm, Vec2::new(-1e18, 0.0), Vec2::new(1e18, 0.0), 0.1, &mut bounds);
    }

    #[test]
    fn test_volume_conservation_with_saturation() {
        let mut hm = Heightmap::new(512, 512, 0.70);
        let mut bounds = ActiveBounds { min_x: 0, max_x: 0, min_y: 0, max_y: 0, active: false };
        let initial_sum: f64 = hm.as_slice().iter().map(|&x| x as f64).sum();

        // Perform displacement at a single point to trigger local saturation in the inner ridge
        displace_line(&mut hm, Vec2::ZERO, Vec2::ZERO, 0.02, &mut bounds);

        let final_sum: f64 = hm.as_slice().iter().map(|&x| x as f64).sum();
        let diff = (final_sum - initial_sum).abs();
        assert!(diff < 1e-2, "Volume not conserved! diff = {}", diff);
    }

    #[test]
    fn test_settling_flow_and_volume_conservation() {
        let mut hm = Heightmap::new(512, 512, 0.5);
        let mut temp_heights = vec![0.5; 512 * 512];
        
        let center_idx = 256 * 512 + 256;
        hm.data[center_idx] = 0.8;

        let mut bounds = ActiveBounds {
            min_x: 250,
            max_x: 262,
            min_y: 250,
            max_y: 262,
            active: true,
        };

        let initial_sum: f64 = hm.as_slice().iter().map(|&x| x as f64).sum();

        let mut flow_occurred = false;
        for _ in 0..10 {
            let flow = settle_tick(&mut hm, &mut temp_heights, &mut bounds, 0.04, 0.15);
            if flow > 0.0 {
                flow_occurred = true;
            }
        }

        assert!(flow_occurred, "Sand should flow down from the peak");

        let final_sum: f64 = hm.as_slice().iter().map(|&x| x as f64).sum();
        let diff = (final_sum - initial_sum).abs();
        assert!(diff < 1e-5, "Settling did not conserve volume! diff = {}", diff);
        assert!(hm.data[center_idx] < 0.8, "Peak should be lower after flowing");
    }

    #[test]
    fn test_settling_deactivation() {
        let mut hm = Heightmap::new(512, 512, 0.5);
        let mut temp_heights = vec![0.5; 512 * 512];
        
        let mut bounds = ActiveBounds {
            min_x: 250,
            max_x: 262,
            min_y: 250,
            max_y: 262,
            active: true,
        };

        let flow = settle_tick(&mut hm, &mut temp_heights, &mut bounds, 0.04, 0.15);
        assert_eq!(flow, 0.0);
        assert!(!bounds.active, "Settling should deactivate when stable");
    }
}
