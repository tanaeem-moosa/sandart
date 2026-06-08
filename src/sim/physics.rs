use crate::sim::grid::Heightmap;
use glam::Vec2;

/// Bounding coordinates to optimize Cellular Automata settling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActiveBounds {
    pub min_x: usize,
    pub max_x: usize,
    pub min_y: usize,
    pub max_y: usize,
    pub active: bool,
}

/// Active marble state passed to the physics CA simulation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ActiveMarbleInfo {
    pub pos: Vec2,
    pub vel: f32,
}

/// Helper function to add sand to a cell, clamping it at max_height (glass top)
/// and distributing any excess volume to its available 4-way neighbors.
fn add_sand_with_limit(heightmap: &mut Heightmap, idx: usize, w: usize, h: usize, amount: f32, max_height: f32) {
    if amount <= 0.0 {
        return;
    }
    let current_h = heightmap.data[idx];
    if current_h + amount <= max_height {
        heightmap.data[idx] = current_h + amount;
    } else {
        let allowed = (max_height - current_h).max(0.0);
        heightmap.data[idx] = max_height;
        let mut excess = amount - allowed;
        if excess <= 1e-6 {
            return;
        }

        // Distribute excess to neighbors that are below the max_height
        let x = idx % w;
        let y = idx / w;
        
        let mut neighbors = [0usize; 4];
        let mut num_neighbors = 0;
        if x > 0 { neighbors[num_neighbors] = idx - 1; num_neighbors += 1; }
        if x + 1 < w { neighbors[num_neighbors] = idx + 1; num_neighbors += 1; }
        if y > 0 { neighbors[num_neighbors] = idx - w; num_neighbors += 1; }
        if y + 1 < h { neighbors[num_neighbors] = idx + w; num_neighbors += 1; }

        // Filter neighbors that have room (height < max_height)
        let mut room_neighbors = [(0usize, 0.0f32); 4];
        let mut num_room_neighbors = 0;
        for i in 0..num_neighbors {
            let n_idx = neighbors[i];
            let nh = heightmap.data[n_idx];
            if nh < max_height {
                room_neighbors[num_room_neighbors] = (n_idx, max_height - nh);
                num_room_neighbors += 1;
            }
        }

        if num_room_neighbors == 0 {
            // If all neighbors are full, distribute to all neighbors equally (overflowing slightly)
            let num = num_neighbors as f32;
            let share = excess / num;
            for i in 0..num_neighbors {
                heightmap.data[neighbors[i]] += share;
            }
        } else {
            // Distribute to room_neighbors proportional to their room
            // Let's do up to 3 passes to distribute everything
            let mut distributed = false;
            for _ in 0..3 {
                if num_room_neighbors == 0 || excess <= 1e-6 {
                    distributed = true;
                    break;
                }
                let share = excess / num_room_neighbors as f32;
                let mut next_room = [(0usize, 0.0f32); 4];
                let mut next_num_room = 0;
                for i in 0..num_room_neighbors {
                    let (n_idx, room) = room_neighbors[i];
                    if room > 0.0 {
                        let to_add = share.min(room);
                        heightmap.data[n_idx] += to_add;
                        excess -= to_add;
                        let new_room = room - to_add;
                        if new_room > 0.0 {
                            next_room[next_num_room] = (n_idx, new_room);
                            next_num_room += 1;
                        }
                    }
                }
                room_neighbors = next_room;
                num_room_neighbors = next_num_room;
            }
            if !distributed && excess > 1e-6 {
                let num = num_neighbors as f32;
                let share = excess / num;
                for i in 0..num_neighbors {
                    heightmap.data[neighbors[i]] += share;
                }
            }
        }
    }
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
    let min_x_float = (min_center_x - total_radius_clamped)
        .clamp(0.0, w as f32)
        .floor();
    let max_x_float = (max_center_x + total_radius_clamped)
        .clamp(0.0, w as f32)
        .ceil();
    let min_y_float = (min_center_y - total_radius_clamped)
        .clamp(0.0, h as f32)
        .floor();
    let max_y_float = (max_center_y + total_radius_clamped)
        .clamp(0.0, h as f32)
        .ceil();

    let min_x = min_x_float as usize;
    let max_x = (max_x_float as usize).min(w - 1);
    let min_y = min_y_float as usize;
    let max_y = (max_y_float as usize).min(h - 1);

    // Update settling active bounding box
    let padding = 15;
    let pad_min_x = min_x.saturating_sub(padding);
    let pad_max_x = max_x.saturating_add(padding).min(w - 1);
    let pad_min_y = min_y.saturating_sub(padding);
    let pad_max_y = max_y.saturating_add(padding).min(h - 1);

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
                // Scale this target height to flat sand height (DEFAULT_SAND_HEIGHT)
                let h_target_norm = (h_target / r_grid_clamped) * crate::sim::DEFAULT_SAND_HEIGHT;

                // Add a tiny micro-texture noise to the groove base
                let seed = (x as u32).wrapping_mul(73856093) ^ (y as u32).wrapping_mul(19349663);
                let noise = (((seed & 0xFFFF) as f32 / 65535.0) - 0.5) * 0.05; // Range [-0.025, 0.025]
                let h_target_noisy = (h_target_norm + noise).clamp(0.0, 1.0);

                let current_idx = row_offset + x;
                let current_h = heightmap.data[current_idx];
                if current_h > h_target_noisy {
                    let diff = current_h - h_target_noisy;
                    heightmap.data[current_idx] = h_target_noisy;

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

                    // Perturb sample distances with coordinate-locked noise to simulate clumped deposition
                    let base_seed = (x as u32).wrapping_mul(73856093) ^ (y as u32).wrapping_mul(19349663);
                    let seed_d1 = base_seed ^ 12345;
                    let noise_d1 = (((seed_d1 & 0xFFFF) as f32 / 65535.0) - 0.5) * 0.3 * w_grid;
                    let d1_p = (d1 + noise_d1).clamp(r_grid_clamped, total_radius_clamped);

                    let seed_d2 = base_seed ^ 67890;
                    let noise_d2 = (((seed_d2 & 0xFFFF) as f32 / 65535.0) - 0.5) * 0.3 * w_grid;
                    let d2_p = (d2 + noise_d2).clamp(r_grid_clamped, total_radius_clamped);

                    let seed_d3 = base_seed ^ 54321;
                    let noise_d3 = (((seed_d3 & 0xFFFF) as f32 / 65535.0) - 0.5) * 0.3 * w_grid;
                    let d3_p = (d3 + noise_d3).clamp(r_grid_clamped, total_radius_clamped);

                    // Calculate target coordinates
                    let rx1 = (closest_line_x + dir_x * d1_p).floor() as isize;
                    let ry1 = (closest_line_y + dir_y * d1_p).floor() as isize;

                    let rx2 = (closest_line_x + dir_x * d2_p).floor() as isize;
                    let ry2 = (closest_line_y + dir_y * d2_p).floor() as isize;

                    let rx3 = (closest_line_x + dir_x * d3_p).floor() as isize;
                    let ry3 = (closest_line_y + dir_y * d3_p).floor() as isize;

                    // Perturb weights based on the destination cell coordinates (rx, ry)
                    let seed_w1 =
                        (rx1.max(0) as u32).wrapping_mul(1299689) ^ (ry1.max(0) as u32).wrapping_mul(314159) ^ 9991;
                    let seed_w2 =
                        (rx2.max(0) as u32).wrapping_mul(1299689) ^ (ry2.max(0) as u32).wrapping_mul(314159) ^ 9992;
                    let seed_w3 =
                        (rx3.max(0) as u32).wrapping_mul(1299689) ^ (ry3.max(0) as u32).wrapping_mul(314159) ^ 9993;

                    let nf1 = 1.0 + (((seed_w1 & 0xFFFF) as f32 / 65535.0) - 0.5) * 0.6; // +/- 30% variation
                    let nf2 = 1.0 + (((seed_w2 & 0xFFFF) as f32 / 65535.0) - 0.5) * 0.6;
                    let nf3 = 1.0 + (((seed_w3 & 0xFFFF) as f32 / 65535.0) - 0.5) * 0.6;

                    let mut w1 = 0.5 * nf1;
                    let mut w2 = (1.0 / 3.0) * nf2;
                    let mut w3 = (1.0 / 6.0) * nf3;

                    let sum_w = w1 + w2 + w3;
                    if sum_w > 0.0 {
                        let inv_sum = 1.0 / sum_w;
                        w1 *= inv_sum;
                        w2 *= inv_sum;
                        w3 *= inv_sum;
                    } else {
                        w1 = 0.5;
                        w2 = 1.0 / 3.0;
                        w3 = 1.0 / 6.0;
                    }

                    let rx1_clamped = rx1.clamp(0, w as isize - 1) as usize;
                    let ry1_clamped = ry1.clamp(0, h as isize - 1) as usize;
                    let dest1_idx = ry1_clamped * w + rx1_clamped;
                    let h_above1 = (heightmap.data[dest1_idx] - crate::sim::DEFAULT_SAND_HEIGHT).max(0.0);

                    let rx2_clamped = rx2.clamp(0, w as isize - 1) as usize;
                    let ry2_clamped = ry2.clamp(0, h as isize - 1) as usize;
                    let dest2_idx = ry2_clamped * w + rx2_clamped;
                    let h_above2 = (heightmap.data[dest2_idx] - crate::sim::DEFAULT_SAND_HEIGHT).max(0.0);

                    let rx3_clamped = rx3.clamp(0, w as isize - 1) as usize;
                    let ry3_clamped = ry3.clamp(0, h as isize - 1) as usize;
                    let dest3_idx = ry3_clamped * w + rx3_clamped;
                    let h_above3 = (heightmap.data[dest3_idx] - crate::sim::DEFAULT_SAND_HEIGHT).max(0.0);

                    // Scale factor for asymptotic decay based on marble diameter/height in heightmap units
                    let scale = 2.0 * (radius / 0.018).max(0.1);
                    
                    let x1 = h_above1 / scale;
                    let m1 = 1.0 / (1.0 + x1 * x1 * x1 * x1);

                    let x2 = h_above2 / scale;
                    let m2 = 1.0 / (1.0 + x2 * x2 * x2 * x2);

                    let x3 = h_above3 / scale;
                    let m3 = 1.0 / (1.0 + x3 * x3 * x3 * x3);

                    let deposited_volume = diff * (w1 * m1 + w2 * m2 + w3 * m3);
                    if deposited_volume > 1e-6 {
                        heightmap.data[current_idx] = current_h - deposited_volume;
                        add_sand_with_limit(heightmap, dest1_idx, w, h, diff * w1 * m1, 1.5);
                        add_sand_with_limit(heightmap, dest2_idx, w, h, diff * w2 * m2, 1.5);
                        add_sand_with_limit(heightmap, dest3_idx, w, h, diff * w3 * m3, 1.5);
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
    sliding: &mut Vec<bool>,
    active_bounds: &mut ActiveBounds,
    material: crate::config::MaterialMode,
    active_marbles: &[ActiveMarbleInfo],
    time_seed: u32,
) -> f32 {
    if !active_bounds.active {
        return 0.0;
    }

    let w = heightmap.width;
    let h = heightmap.height;
    if w == 0 || h == 0 {
        return 0.0;
    }

    // Safety checks to prevent panics if heights or sliding buffer are resized
    if temp_heights.len() != heightmap.data.len() {
        temp_heights.resize(heightmap.data.len(), crate::sim::DEFAULT_SAND_HEIGHT);
    }
    if sliding.len() != heightmap.data.len() {
        sliding.resize(heightmap.data.len(), false);
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

            let seed = (x as u32).wrapping_mul(1299689) ^ (y as u32).wrapping_mul(314159) ^ time_seed.wrapping_mul(7213);
            
            // Loop over 4 neighbors
            let neighbors = [
                (x > 0, x.wrapping_sub(1), y, center_idx.wrapping_sub(1)),
                (x + 1 < w, x + 1, y, center_idx + 1),
                (y > 0, x, y.wrapping_sub(1), center_idx.wrapping_sub(w)),
                (y + 1 < h, x, y + 1, center_idx + w),
            ];

            let mut cell_flowed = false;

            // A. Absolute gravity-avalanche collapse safety check (to prevent spikes)
            let mut avalanche_checked = false;
            for &(cond, nx, ny, neighbor_idx) in &neighbors {
                if !cond {
                    continue;
                }

                let h_neighbor = heightmap.data[neighbor_idx];
                let geom_slope = h_center - h_neighbor;

                if geom_slope > 0.20 {
                    let flow = (0.10 * (geom_slope - 0.20)).max(0.0);
                    if flow > 0.0 {
                        let current_temp_center = temp_heights[center_idx];
                        let current_temp_neighbor = temp_heights[neighbor_idx];
                        let temp_diff = current_temp_center - current_temp_neighbor;
                        let clamped_flow = flow.min(temp_diff * 0.4).max(0.0);
                        if clamped_flow > 0.0 {
                            temp_heights[center_idx] -= clamped_flow;
                            temp_heights[neighbor_idx] += clamped_flow;
                            total_flow += clamped_flow;
                            cell_flowed = true;
                            
                            next_min_x = next_min_x.min(nx).min(x);
                            next_max_x = next_max_x.max(nx).max(x);
                            next_min_y = next_min_y.min(ny).min(y);
                            next_max_y = next_max_y.max(ny).max(y);
                            flow_occurred = true;
                        }
                    }
                    avalanche_checked = true;
                }
            }
            if avalanche_checked {
                sliding[center_idx] = cell_flowed;
                continue;
            }

            // Cell-invariant properties (calculated once per cell before neighbor loop)
            let mut higher_neighbors = 0;
            if material == crate::config::MaterialMode::DrySand {
                for &(cond, _, _, n_idx) in &neighbors {
                    if cond && heightmap.data[n_idx] >= h_center - 1e-4 {
                        higher_neighbors += 1;
                    }
                }
            }

            let mut closest_marble_idx = None;
            let mut min_dist_to_marble = f32::MAX;
            if (material == crate::config::MaterialMode::Oobleck || material == crate::config::MaterialMode::IronFilings) && !active_marbles.is_empty() {
                let cell_x = (x as f32 / w as f32) * 2.0 - 1.0;
                let cell_y = 1.0 - (y as f32 / h as f32) * 2.0;
                let cell_pos = Vec2::new(cell_x, cell_y);

                for (idx, m) in active_marbles.iter().enumerate() {
                    let dist = (cell_pos - m.pos).length();
                    if dist < min_dist_to_marble {
                        min_dist_to_marble = dist;
                        closest_marble_idx = Some(idx);
                    }
                }
            }

            let oobleck_params = if material == crate::config::MaterialMode::Oobleck {
                let local_vel = if let Some(idx) = closest_marble_idx {
                    active_marbles[idx].vel
                } else {
                    0.0
                };
                let t = ((local_vel - 0.03) / 0.12).clamp(0.0, 1.0);
                let t_steep = t * t;
                Some((
                    0.005 + (0.32 - 0.005) * t_steep,
                    0.40 + (0.005 - 0.40) * t_steep,
                    0.02 + (0.98 - 0.02) * t_steep,
                ))
            } else {
                None
            };

            let iron_filings_threshold = if material == crate::config::MaterialMode::IronFilings && min_dist_to_marble < 0.22 {
                let ripple = (min_dist_to_marble * 2.0 * std::f32::consts::PI / 0.025).cos();
                (0.08 + ripple * 0.05).max(0.01)
            } else {
                0.08
            };

            let to_magnet_norm = if material == crate::config::MaterialMode::IronFilings && min_dist_to_marble > 1e-4 {
                if let Some(idx) = closest_marble_idx {
                    let cell_x = (x as f32 / w as f32) * 2.0 - 1.0;
                    let cell_y = 1.0 - (y as f32 / h as f32) * 2.0;
                    let cell_pos = Vec2::new(cell_x, cell_y);
                    Some((active_marbles[idx].pos - cell_pos).normalize())
                } else {
                    None
                }
            } else {
                None
            };

            for (dir_idx, &(cond, nx, ny, neighbor_idx)) in neighbors.iter().enumerate() {
                if !cond {
                    continue;
                }

                let h_neighbor = heightmap.data[neighbor_idx];
                let geom_slope = h_center - h_neighbor;

                // B. Material-specific parameters
                let threshold;
                let alpha;
                let lock_chance;
                let mut quantize_size = None;
                let mut magnetic_bias = 0.0;

                match material {
                    crate::config::MaterialMode::ButterCream => {
                        threshold = 0.04;
                        alpha = 0.15;
                        lock_chance = 0.20;
                    }
                    crate::config::MaterialMode::DrySand => {
                        threshold = if sliding[center_idx] { 0.04 } else { 0.08 };
                        alpha = 0.25;
                        quantize_size = Some(0.01);
                        lock_chance = if higher_neighbors >= 3 { 0.80 } else { 0.10 };
                    }
                    crate::config::MaterialMode::Snow => {
                        threshold = 0.15;
                        alpha = 0.04;
                        lock_chance = 0.30;
                    }
                    crate::config::MaterialMode::KineticSand => {
                        threshold = 0.12;
                        alpha = 0.12;
                        lock_chance = 0.75;
                        quantize_size = Some(0.015);
                    }
                    crate::config::MaterialMode::WetSand => {
                        threshold = 0.10;
                        alpha = 0.06;
                        lock_chance = 0.15;
                    }
                    crate::config::MaterialMode::FinePowder => {
                        threshold = 0.01;
                        alpha = 0.35;
                        lock_chance = 0.02;
                    }
                    crate::config::MaterialMode::Oobleck => {
                        let (th, al, lc) = oobleck_params.unwrap();
                        threshold = th;
                        alpha = al;
                        lock_chance = lc;
                    }
                    crate::config::MaterialMode::MoonDust => {
                        threshold = 0.22;
                        alpha = 0.02;
                        lock_chance = 0.40;
                        quantize_size = Some(0.015);
                    }
                    crate::config::MaterialMode::IronFilings => {
                        threshold = iron_filings_threshold;
                        alpha = 0.35;
                        lock_chance = 0.05;
                        
                        if min_dist_to_marble < 0.22 {
                            if let Some(to_mag_norm) = to_magnet_norm {
                                let dot_prod = match dir_idx {
                                    0 => -to_mag_norm.x,
                                    1 => to_mag_norm.x,
                                    2 => to_mag_norm.y,
                                    3 => -to_mag_norm.y,
                                    _ => 0.0,
                                };
                                let pull_strength = 0.24 * (1.0 - min_dist_to_marble / 0.22).max(0.0);
                                magnetic_bias = pull_strength * dot_prod;
                            }
                        }
                    }
                }

                let slope = geom_slope + magnetic_bias;

                if slope <= 1e-6 {
                    continue;
                }

                // C. Stochastic locking and sliding condition
                if slope > threshold {
                    let flow_seed = (seed ^ (neighbor_idx as u32).wrapping_mul(997)) & 0xFFFF;
                    let rand_val = flow_seed as f32 / 65535.0;
                    
                    if rand_val >= lock_chance {
                        let alpha_noise = 1.0 + (rand_val - 0.5) * 0.8; // +/- 40% flow rate noise
                        let mut flow = (alpha * (slope - threshold) * alpha_noise).max(0.0);
                        
                        if let Some(q) = quantize_size {
                            flow = (flow / q).round() * q;
                        }

                        if flow > 0.0 {
                            let temp_diff = temp_heights[center_idx] - temp_heights[neighbor_idx];
                            let clamped_flow = flow.min(temp_diff * 0.4).max(0.0);
                            if clamped_flow > 0.0 {
                                temp_heights[center_idx] -= clamped_flow;
                                temp_heights[neighbor_idx] += clamped_flow;
                                total_flow += clamped_flow;
                                cell_flowed = true;
                                
                                next_min_x = next_min_x.min(nx).min(x);
                                next_max_x = next_max_x.max(nx).max(x);
                                next_min_y = next_min_y.min(ny).min(y);
                                next_max_y = next_max_y.max(ny).max(y);
                                flow_occurred = true;
                            }
                        }
                    }
                }
            }

            sliding[center_idx] = cell_flowed;
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
        let mut hm = Heightmap::new(512, 512, crate::sim::DEFAULT_SAND_HEIGHT);
        let mut bounds = ActiveBounds {
            min_x: 0,
            max_x: 0,
            min_y: 0,
            max_y: 0,
            active: false,
        };

        // Drawing completely offscreen should not panic or modify the heightmap
        displace_line(
            &mut hm,
            Vec2::new(5.0, 5.0),
            Vec2::new(5.0, 5.0),
            0.1,
            &mut bounds,
        );

        // Assert that heightmap data is unchanged
        for &val in hm.as_slice() {
            assert_eq!(val, crate::sim::DEFAULT_SAND_HEIGHT);
        }
    }

    #[test]
    fn test_draw_point_partial_overlap() {
        let mut hm = Heightmap::new(512, 512, crate::sim::DEFAULT_SAND_HEIGHT);
        let mut bounds = ActiveBounds {
            min_x: 0,
            max_x: 0,
            min_y: 0,
            max_y: 0,
            active: false,
        };

        // Position marble so it sits on the left boundary
        displace_line(
            &mut hm,
            Vec2::new(-1.0, 0.0),
            Vec2::new(-1.0, 0.0),
            0.05,
            &mut bounds,
        );

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
        let mut hm = Heightmap::new(512, 512, crate::sim::DEFAULT_SAND_HEIGHT);
        let mut bounds = ActiveBounds {
            min_x: 0,
            max_x: 0,
            min_y: 0,
            max_y: 0,
            active: false,
        };

        // Draw a line from (-0.5, 0.0) to (0.5, 0.0)
        displace_line(
            &mut hm,
            Vec2::new(-0.5, 0.0),
            Vec2::new(0.5, 0.0),
            0.05,
            &mut bounds,
        );

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

        assert!(hm.get(cx1, cy1) < 0.03);
        assert!(hm.get(cx2, cy2) < 0.03);
        assert!(hm.get(cx3, cy3) < 0.03);
    }

    #[test]
    fn test_draw_point_extreme_coordinates_overflow() {
        let mut hm = Heightmap::new(512, 512, crate::sim::DEFAULT_SAND_HEIGHT);
        let mut bounds = ActiveBounds {
            min_x: 0,
            max_x: 0,
            min_y: 0,
            max_y: 0,
            active: false,
        };

        displace_line(
            &mut hm,
            Vec2::new(1e18, 1e18),
            Vec2::new(1e18, 1e18),
            0.1,
            &mut bounds,
        );
        for &val in hm.as_slice() {
            assert_eq!(val, crate::sim::DEFAULT_SAND_HEIGHT);
        }
    }

    #[test]
    fn test_volume_conservation() {
        let mut hm = Heightmap::new(512, 512, 0.4);
        let mut bounds = ActiveBounds {
            min_x: 0,
            max_x: 0,
            min_y: 0,
            max_y: 0,
            active: false,
        };
        let initial_sum: f64 = hm.as_slice().iter().map(|&x| x as f64).sum();

        // Perform displacement along a path
        displace_line(
            &mut hm,
            Vec2::new(-0.2, 0.2),
            Vec2::new(0.2, -0.2),
            0.03,
            &mut bounds,
        );

        let final_sum: f64 = hm.as_slice().iter().map(|&x| x as f64).sum();
        let diff = (final_sum - initial_sum).abs();
        assert!(diff < 1e-2, "Volume not conserved! diff = {}", diff);
    }

    #[test]
    fn test_draw_line_extreme_coordinates_overflow() {
        let mut hm = Heightmap::new(512, 512, crate::sim::DEFAULT_SAND_HEIGHT);
        let mut bounds = ActiveBounds {
            min_x: 0,
            max_x: 0,
            min_y: 0,
            max_y: 0,
            active: false,
        };
        displace_line(
            &mut hm,
            Vec2::new(-1e18, 0.0),
            Vec2::new(1e18, 0.0),
            0.1,
            &mut bounds,
        );
    }

    #[test]
    fn test_volume_conservation_with_saturation() {
        let mut hm = Heightmap::new(512, 512, 0.70);
        let mut bounds = ActiveBounds {
            min_x: 0,
            max_x: 0,
            min_y: 0,
            max_y: 0,
            active: false,
        };
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
            let flow = settle_tick(
                &mut hm,
                &mut temp_heights,
                &mut vec![false; 512 * 512],
                &mut bounds,
                crate::config::MaterialMode::ButterCream,
                &[],
                12345,
            );
            if flow > 0.0 {
                flow_occurred = true;
            }
        }

        assert!(flow_occurred, "Sand should flow down from the peak");

        let final_sum: f64 = hm.as_slice().iter().map(|&x| x as f64).sum();
        let diff = (final_sum - initial_sum).abs();
        assert!(
            diff < 1e-5,
            "Settling did not conserve volume! diff = {}",
            diff
        );
        assert!(
            hm.data[center_idx] < 0.8,
            "Peak should be lower after flowing"
        );
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

        let flow = settle_tick(
            &mut hm,
            &mut temp_heights,
            &mut vec![false; 512 * 512],
            &mut bounds,
            crate::config::MaterialMode::ButterCream,
            &[],
            12345,
        );
        assert_eq!(flow, 0.0);
        assert!(!bounds.active, "Settling should deactivate when stable");
    }

    #[test]
    fn test_material_presets_and_avalanche() {
        use crate::config::MaterialMode;
        
        let materials = [
            MaterialMode::ButterCream,
            MaterialMode::DrySand,
            MaterialMode::Snow,
            MaterialMode::KineticSand,
            MaterialMode::WetSand,
            MaterialMode::FinePowder,
            MaterialMode::Oobleck,
            MaterialMode::MoonDust,
            MaterialMode::IronFilings,
        ];

        for &mat in &materials {
            let mut hm = Heightmap::new(64, 64, 0.5);
            let mut temp_heights = vec![0.5; 64 * 64];
            let mut sliding = vec![false; 64 * 64];
            let mut bounds = ActiveBounds {
                min_x: 10,
                max_x: 54,
                min_y: 10,
                max_y: 54,
                active: true,
            };

            // Set a steep spike at center that exceeds the avalanche threshold (0.20 slope)
            let center_idx = 32 * 64 + 32;
            hm.data[center_idx] = 1.0;
            hm.data[center_idx - 1] = 0.5; // slope = 0.5 > 0.20

            // Settle should trigger avalanche flow for all materials
            let flow = settle_tick(
                &mut hm,
                &mut temp_heights,
                &mut sliding,
                &mut bounds,
                mat,
                &[ActiveMarbleInfo { pos: Vec2::ZERO, vel: 0.1 }],
                9999,
            );

            assert!(flow > 0.0, "Material {:?} should flow under steep slope", mat);
        }
    }
}
