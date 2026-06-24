use crate::grid::Heightmap;
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
    pub vel_vec: Vec2,
}

/// Advect color from src cell to dst cell based on the flow amount and dst cell's height before arrival
pub fn advect_color(colors: &mut [u8], src: usize, dst: usize, flow: f32, h_dst: f32) {
    let total = h_dst + flow;
    if total < 1e-6 {
        return;
    }
    let w_keep = h_dst / total;
    let w_arrive = flow / total;

    let src_base = src * 4;
    let dst_base = dst * 4;

    for ch in 0..3 {
        colors[dst_base + ch] = (
            colors[dst_base + ch] as f32 * w_keep
            + colors[src_base + ch] as f32 * w_arrive
        ).clamp(0.0, 255.0).round() as u8;
    }
    colors[dst_base + 3] = 255; // opaque alpha
}

/// Helper function to add sand to a cell, clamping it at max_height (glass top)
/// and distributing any excess volume to its available 4-way neighbors, with color advection.
fn add_sand_with_limit_color(
    heightmap: &mut Heightmap,
    cell_colors: &mut [u8],
    src_idx: usize,
    idx: usize,
    w: usize,
    h: usize,
    amount: f32,
    max_height: f32,
) {
    if amount <= 0.0 {
        return;
    }
    let current_h = heightmap.data[idx];
    if current_h + amount <= max_height {
        advect_color(cell_colors, src_idx, idx, amount, current_h);
        heightmap.data[idx] = current_h + amount;
    } else {
        let allowed = (max_height - current_h).max(0.0);
        advect_color(cell_colors, src_idx, idx, allowed, current_h);
        heightmap.data[idx] = current_h + allowed;
        let mut excess = amount - allowed;
        if excess > 1e-6 {
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
                    let n_idx = neighbors[i];
                    advect_color(cell_colors, idx, n_idx, share, heightmap.data[n_idx]);
                    heightmap.data[n_idx] += share;
                }
            } else {
                // Distribute to room_neighbors proportional to their room
                // Let's do up to 3 passes to distribute everything
                let mut distributed = false;
                for _ in 0..3 {
                    if excess <= 1e-6 {
                        distributed = true;
                        break;
                    }
                    if num_room_neighbors == 0 {
                        break;
                    }
                    let share = excess / num_room_neighbors as f32;
                    let mut next_room = [(0usize, 0.0f32); 4];
                    let mut next_num_room = 0;
                    for i in 0..num_room_neighbors {
                        let (n_idx, room) = room_neighbors[i];
                        if room > 0.0 {
                            let to_add = share.min(room);
                            advect_color(cell_colors, idx, n_idx, to_add, heightmap.data[n_idx]);
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
                        let n_idx = neighbors[i];
                        advect_color(cell_colors, idx, n_idx, share, heightmap.data[n_idx]);
                        heightmap.data[n_idx] += share;
                    }
                }
            }
        }
    }
}

/// Displace sand along a line segment from start to end, carving a groove
/// and depositing the displaced volume into the surrounding ridge area.
pub fn displace_line(
    heightmap: &mut Heightmap,
    cell_colors: &mut [u8],
    start: Vec2,
    end: Vec2,
    radius: f32,
    active_bounds: &mut ActiveBounds,
    material: crate::MaterialMode,
) {
    if !start.is_finite() || !end.is_finite() || !radius.is_finite() || radius <= 0.0 {
        return;
    }

    let residual_factor = match material {
        crate::MaterialMode::ButterCream => 0.00,
        crate::MaterialMode::Oobleck => {
            // Calculate displacement segment speed.
            // Fast marble movement means high shear stress (solid-like), leaving a shallow groove.
            // Slow movement means low shear stress (liquid-like), carving all the way down.
            let speed = (end - start).length();
            let t = (speed / 0.01).clamp(0.0, 1.0);
            0.50 * t * t
        }
        crate::MaterialMode::FinePowder => 0.05,
        crate::MaterialMode::KineticSand => 0.10,
        crate::MaterialMode::DrySand => 0.20,
        crate::MaterialMode::MoonDust => 0.20,
        crate::MaterialMode::Snow => 0.28,
        crate::MaterialMode::WetSand => 0.35,
        crate::MaterialMode::Water => 0.00,
        crate::MaterialMode::Milk => 0.00,
        crate::MaterialMode::VegetableOil => 0.00,
        crate::MaterialMode::CalmWater => 0.00,
        crate::MaterialMode::Yogurt => 0.00,
        crate::MaterialMode::CoarseSand => 0.18,
    };

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
                let h_target_profile = (h_target / r_grid_clamped) * crate::DEFAULT_SAND_HEIGHT;

                let current_idx = row_offset + x;
                let current_h = heightmap.data[current_idx];

                // Scale target height relative to the current height to support multi-pass clearing
                let h_target_norm = residual_factor * current_h.max(h_target_profile) + (1.0 - residual_factor) * h_target_profile;

                // Add a tiny micro-texture noise to the groove base
                let seed = (x as u32).wrapping_mul(73856093) ^ (y as u32).wrapping_mul(19349663);
                let noise = (((seed & 0xFFFF) as f32 / 65535.0) - 0.5) * 0.05; // Range [-0.025, 0.025]
                let h_target_noisy = (h_target_norm + noise).clamp(0.0, 1.0);

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
                    let h_above1 = (heightmap.data[dest1_idx] - crate::DEFAULT_SAND_HEIGHT).max(0.0);

                    let rx2_clamped = rx2.clamp(0, w as isize - 1) as usize;
                    let ry2_clamped = ry2.clamp(0, h as isize - 1) as usize;
                    let dest2_idx = ry2_clamped * w + rx2_clamped;
                    let h_above2 = (heightmap.data[dest2_idx] - crate::DEFAULT_SAND_HEIGHT).max(0.0);

                    let rx3_clamped = rx3.clamp(0, w as isize - 1) as usize;
                    let ry3_clamped = ry3.clamp(0, h as isize - 1) as usize;
                    let dest3_idx = ry3_clamped * w + rx3_clamped;
                    let h_above3 = (heightmap.data[dest3_idx] - crate::DEFAULT_SAND_HEIGHT).max(0.0);

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
                        add_sand_with_limit_color(heightmap, cell_colors, current_idx, dest1_idx, w, h, diff * w1 * m1, 1.5);
                        add_sand_with_limit_color(heightmap, cell_colors, current_idx, dest2_idx, w, h, diff * w2 * m2, 1.5);
                        add_sand_with_limit_color(heightmap, cell_colors, current_idx, dest3_idx, w, h, diff * w3 * m3, 1.5);
                    } else {
                        // Restore height to conserve volume if no deposition can happen
                        heightmap.data[current_idx] = current_h;
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
    cell_colors: &mut Vec<u8>,
    sliding: &mut Vec<bool>,
    active_bounds: &mut ActiveBounds,
    active_blocks: &mut Vec<crate::BlockActivity>,
    last_displacements: &mut Vec<f32>,
    last_simulated_ticks: &mut Vec<u32>,
    budget_n: usize,
    block_size: usize,
    material: crate::MaterialMode,
    active_marbles: &[ActiveMarbleInfo],
    time_seed: u32,
    wave_vel: &mut Vec<f32>,
    shape: crate::SandboxShape,
    tick_count: u32,
) -> f32 {
    let w = heightmap.width;
    let h = heightmap.height;
    if w == 0 || h == 0 {
        return 0.0;
    }

    // Safety checks to prevent panics if heights or sliding buffer are resized
    if temp_heights.len() != heightmap.data.len() {
        temp_heights.resize(heightmap.data.len(), crate::DEFAULT_SAND_HEIGHT);
    }
    if sliding.len() != heightmap.data.len() {
        sliding.resize(heightmap.data.len(), false);
    }
    if wave_vel.len() != heightmap.data.len() {
        wave_vel.resize(heightmap.data.len(), 0.0);
    }

    let cols = (w + block_size - 1) / block_size;
    let rows = (h + block_size - 1) / block_size;
    let expected_len = cols * rows;

    if last_displacements.len() != expected_len {
        last_displacements.resize(expected_len, 0.0);
    }
    if last_simulated_ticks.len() != expected_len {
        last_simulated_ticks.resize(expected_len, 0);
    }
    if active_blocks.len() != expected_len {
        active_blocks.resize(expected_len, crate::BlockActivity::Inactive);
    }

    // Constants from the design doc
    const MUST_SIMULATE_THRESHOLD: f32 = 0.1;
    const MAX_STALENESS: u32 = 30;
    const FLOW_INACTIVE_THRESHOLD: f32 = 3e-4;

    // 1. Identify MUST, STALE, and REST blocks, and calculate priorities
    let mut must_simulate = Vec::new();
    let mut stale_simulate = Vec::new();
    let mut rest_candidates = Vec::new();

    for b in 0..expected_len {
        let displacement = last_displacements[b];
        let staleness = tick_count.saturating_sub(last_simulated_ticks[b]).min(MAX_STALENESS);

        if displacement >= MUST_SIMULATE_THRESHOLD {
            must_simulate.push(b);
        } else if staleness >= MAX_STALENESS {
            stale_simulate.push(b);
        } else if displacement > 0.0 {
            // Priority function: staleness * displacement
            let priority = (staleness as f32) * displacement;
            rest_candidates.push((b, priority));
        }
    }

    // Quick exit check if no blocks are active
    if must_simulate.is_empty() && stale_simulate.is_empty() && rest_candidates.is_empty() {
        active_bounds.active = false;
        active_blocks.fill(crate::BlockActivity::Inactive);
        return 0.0;
    }

    let total_always = must_simulate.len() + stale_simulate.len();
    let remaining_budget = if budget_n > total_always {
        budget_n - total_always
    } else {
        0
    };

    let mut budget_simulate = Vec::new();
    if remaining_budget > 0 && !rest_candidates.is_empty() {
        let n = remaining_budget.min(rest_candidates.len());
        rest_candidates.select_nth_unstable_by(n - 1, |a, b| {
            b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal)
        });
        for i in 0..n {
            budget_simulate.push(rest_candidates[i].0);
        }
    }

    let mut will_simulate = vec![false; expected_len];
    for &b in &must_simulate {
        will_simulate[b] = true;
    }
    for &b in &stale_simulate {
        will_simulate[b] = true;
    }
    for &b in &budget_simulate {
        will_simulate[b] = true;
    }

    // Update active_blocks for HUD statistics
    active_blocks.fill(crate::BlockActivity::Inactive);
    for &b in &must_simulate {
        active_blocks[b] = crate::BlockActivity::Fast;
    }
    for &b in &stale_simulate {
        active_blocks[b] = crate::BlockActivity::Slow;
    }
    for &b in &budget_simulate {
        active_blocks[b] = crate::BlockActivity::Medium;
    }

    // Sandbox boundary helper scaled to current width and height
    let is_inside = |cx: usize, cy: usize| -> bool {
        let center_x = w as f32 / 2.0;
        let center_y = h as f32 / 2.0;
        let dx = cx as f32 - center_x;
        let dy = cy as f32 - center_y;
        let r_x = 0.46 * w as f32;
        let r_y = 0.46 * h as f32;
        let r_oval_y = 0.30 * h as f32;
        match shape {
            crate::SandboxShape::Circle => dx * dx + dy * dy < r_x * r_x,
            crate::SandboxShape::Square => dx.abs() < r_x && dy.abs() < r_y,
            crate::SandboxShape::Oval => {
                (dx * dx) / (r_x * r_x) + (dy * dy) / (r_oval_y * r_oval_y) < 1.0
            }
        }
    };

    // If material is a liquid (Water, Milk, Yogurt, etc.), run the 2D wave propagation solver instead of CA settling.
    if material == crate::MaterialMode::Water
        || material == crate::MaterialMode::Milk
        || material == crate::MaterialMode::VegetableOil
        || material == crate::MaterialMode::CalmWater
        || material == crate::MaterialMode::Yogurt
    {
        let mut modified = will_simulate.clone();
        let mut next_displacements = vec![0.0f32; expected_len];

        // Helper closure to activate neighbor blocks and copy their heights on demand
        let activate_neighbor = |neighbor_b: usize, flow: f32, temp_heights: &mut Vec<f32>, heightmap: &crate::grid::Heightmap, modified: &mut Vec<bool>, next_displacements: &mut Vec<f32>| {
            if !modified[neighbor_b] {
                let nbx = neighbor_b % cols;
                let nby = neighbor_b / cols;
                let start_x = nbx * block_size;
                let end_x = ((nbx + 1) * block_size).min(w);
                let start_y = nby * block_size;
                let end_y = ((nby + 1) * block_size).min(h);
                for y in start_y..end_y {
                    let offset = y * w;
                    temp_heights[offset + start_x..offset + end_x]
                        .copy_from_slice(&heightmap.data[offset + start_x..offset + end_x]);
                }
                modified[neighbor_b] = true;
            }
            if next_displacements[neighbor_b] < flow {
                next_displacements[neighbor_b] = flow;
            }
        };

        // 1. Copy active blocks from heightmap to temp buffer
        for b in 0..expected_len {
            if will_simulate[b] {
                let bx = b % cols;
                let by = b / cols;
                let start_x = bx * block_size;
                let end_x = ((bx + 1) * block_size).min(w);
                let start_y = by * block_size;
                let end_y = ((by + 1) * block_size).min(h);
                for y in start_y..end_y {
                    let offset = y * w;
                    temp_heights[offset + start_x..offset + end_x]
                        .copy_from_slice(&heightmap.data[offset + start_x..offset + end_x]);
                }
            }
        }

        // Wave propagation parameters
        let (c_sq, damping) = match material {
            crate::MaterialMode::Water => (0.24f32, 0.98f32),
            crate::MaterialMode::Milk => (0.16f32, 0.86f32),
            crate::MaterialMode::VegetableOil => (0.18f32, 0.92f32),
            crate::MaterialMode::CalmWater => (0.22f32, 0.88f32),
            crate::MaterialMode::Yogurt => (0.08f32, 0.76f32),
            _ => (0.24f32, 0.98f32),
        };

        let mut flow_occurred = false;
        let mut total_flow = 0.0f32;

        // Run wave equation inside active blocks
        for b in 0..expected_len {
            if will_simulate[b] {
                let bx = b % cols;
                let by = b / cols;
                let start_x = bx * block_size;
                let end_x = ((bx + 1) * block_size).min(w);
                let start_y = by * block_size;
                let end_y = ((by + 1) * block_size).min(h);

                for y in start_y..end_y {
                    let row_offset = y * w;
                    for x in start_x..end_x {
                        let center_idx = row_offset + x;

                        if !is_inside(x, y) {
                            continue;
                        }

                        let h_center = heightmap.data[center_idx];

                        // Neumann boundary reflection conditions
                        let h_left = if x > 0 && is_inside(x - 1, y) { heightmap.data[center_idx - 1] } else { h_center };
                        let h_right = if x + 1 < w && is_inside(x + 1, y) { heightmap.data[center_idx + 1] } else { h_center };
                        let h_top = if y > 0 && is_inside(x, y - 1) { heightmap.data[center_idx - w] } else { h_center };
                        let h_bottom = if y + 1 < h && is_inside(x, y + 1) { heightmap.data[center_idx + w] } else { h_center };

                        let laplacian = h_left + h_right + h_top + h_bottom - 4.0 * h_center;

                        let v_new = (wave_vel[center_idx] + c_sq * laplacian) * damping;
                        wave_vel[center_idx] = v_new;

                        let h_new = (h_center + v_new).clamp(0.0, 1.0);
                        temp_heights[center_idx] = h_new;

                        let height_diff = (h_new - h_center).abs();
                        total_flow += height_diff;

                        if v_new.abs() > 3e-4 || (h_new - crate::DEFAULT_SAND_HEIGHT).abs() > 1e-4 {
                            flow_occurred = true;
                            let flow_val = v_new.abs().max((h_new - crate::DEFAULT_SAND_HEIGHT).abs());
                            activate_neighbor(b, flow_val, temp_heights, heightmap, &mut modified, &mut next_displacements);
                            if bx > 0 { activate_neighbor(b - 1, flow_val, temp_heights, heightmap, &mut modified, &mut next_displacements); }
                            if bx + 1 < cols { activate_neighbor(b + 1, flow_val, temp_heights, heightmap, &mut modified, &mut next_displacements); }
                            if by > 0 { activate_neighbor(b - cols, flow_val, temp_heights, heightmap, &mut modified, &mut next_displacements); }
                            if by + 1 < rows { activate_neighbor(b + cols, flow_val, temp_heights, heightmap, &mut modified, &mut next_displacements); }
                        }
                    }
                }
            }
        }

        // Copy back updated heights for active and next active blocks
        for b in 0..expected_len {
            if modified[b] {
                let bx = b % cols;
                let by = b / cols;
                let start_x = bx * block_size;
                let end_x = ((bx + 1) * block_size).min(w);
                let start_y = by * block_size;
                let end_y = ((by + 1) * block_size).min(h);
                for y in start_y..end_y {
                    let offset = y * w;
                    heightmap.data[offset + start_x..offset + end_x]
                        .copy_from_slice(&temp_heights[offset + start_x..offset + end_x]);
                }
            }
        }

        // Compute active bounds for the GPU
        let mut min_bx = cols;
        let mut max_bx = 0;
        let mut min_by = rows;
        let mut max_by = 0;
        let mut any_modified = false;

        for b in 0..expected_len {
            if modified[b] {
                any_modified = true;
                let bx = b % cols;
                let by = b / cols;
                min_bx = min_bx.min(bx);
                max_bx = max_bx.max(bx);
                min_by = min_by.min(by);
                max_by = max_by.max(by);
            }
        }

        if any_modified {
            active_bounds.min_x = min_bx * block_size;
            active_bounds.max_x = ((max_bx + 1) * block_size - 1).min(w - 1);
            active_bounds.min_y = min_by * block_size;
            active_bounds.max_y = ((max_by + 1) * block_size - 1).min(h - 1);
            active_bounds.active = flow_occurred;
        } else {
            active_bounds.active = false;
        }

        for b in 0..expected_len {
            if !will_simulate[b] {
                next_displacements[b] = next_displacements[b].max(last_displacements[b]);
            } else {
                last_simulated_ticks[b] = tick_count;
            }
        }
        *last_displacements = next_displacements;

        return total_flow;
    }

    let mut modified = will_simulate.clone();

    // 1. Copy active blocks scheduled to run this frame from heightmap to temp buffer
    for b in 0..expected_len {
        if will_simulate[b] {
            let bx = b % cols;
            let by = b / cols;
            let start_x = bx * block_size;
            let end_x = ((bx + 1) * block_size).min(w);
            let start_y = by * block_size;
            let end_y = ((by + 1) * block_size).min(h);
            for y in start_y..end_y {
                let offset = y * w;
                temp_heights[offset + start_x..offset + end_x]
                    .copy_from_slice(&heightmap.data[offset + start_x..offset + end_x]);
            }
        }
    }

    let mut total_flow = 0.0f32;
    let mut next_displacements = vec![0.0f32; expected_len];
    let mut flow_occurred = false;

    // Helper closure to activate neighbor blocks and copy their heights on demand
    let activate_neighbor = |neighbor_b: usize, flow: f32, temp_heights: &mut Vec<f32>, heightmap: &crate::grid::Heightmap, modified: &mut Vec<bool>, next_displacements: &mut Vec<f32>| {
        if !modified[neighbor_b] {
            let nbx = neighbor_b % cols;
            let nby = neighbor_b / cols;
            let start_x = nbx * block_size;
            let end_x = ((nbx + 1) * block_size).min(w);
            let start_y = nby * block_size;
            let end_y = ((nby + 1) * block_size).min(h);
            for y in start_y..end_y {
                let offset = y * w;
                temp_heights[offset + start_x..offset + end_x]
                    .copy_from_slice(&heightmap.data[offset + start_x..offset + end_x]);
            }
            modified[neighbor_b] = true;
        }
        if next_displacements[neighbor_b] < flow {
            next_displacements[neighbor_b] = flow;
        }
    };

    // 2. Cellular automata slope settling update (loop over active blocks)
    let is_complex = material == crate::MaterialMode::Oobleck;
    let is_dynamic = material == crate::MaterialMode::DrySand || material == crate::MaterialMode::CoarseSand;

    let threshold_min = match material {
        crate::MaterialMode::ButterCream => 0.04,
        crate::MaterialMode::Snow => 0.15,
        crate::MaterialMode::WetSand => 0.10,
        crate::MaterialMode::FinePowder => 0.01,
        crate::MaterialMode::MoonDust => 0.20, // threshold is 0.22, but avalanche is 0.20
        crate::MaterialMode::KineticSand => 0.12,
        crate::MaterialMode::DrySand => 0.04,
        crate::MaterialMode::CoarseSand => 0.06,
        crate::MaterialMode::Oobleck => 0.005,
        _ => 0.10,
    };

    if is_complex {
        for b in 0..expected_len {
            if !will_simulate[b] {
                continue;
            }

            let bx = b % cols;
            let by = b / cols;
            let start_x = (bx * block_size).max(1);
            let end_x = (((bx + 1) * block_size) - 1).min(w - 2);
            let start_y = (by * block_size).max(1);
            let end_y = (((by + 1) * block_size) - 1).min(h - 2);

            for y in start_y..=end_y {
                let row_offset = y * w;
                for x in start_x..=end_x {
                    let center_idx = row_offset + x;
                    let h_center = heightmap.data[center_idx];

                    // Load neighbor heights and find minimum
                    let h_left = heightmap.data[center_idx - 1];
                    let h_right = heightmap.data[center_idx + 1];
                    let h_top = heightmap.data[center_idx - w];
                    let h_bottom = heightmap.data[center_idx + w];

                    let min_h = h_left.min(h_right).min(h_top).min(h_bottom);

                    // Fast-path shortcut: check if cell needs any updates
                    if h_center - min_h <= threshold_min {
                        sliding[center_idx] = false;
                        continue;
                    }

                    let seed = (x as u32).wrapping_mul(1299689) ^ (y as u32).wrapping_mul(314159) ^ time_seed.wrapping_mul(7213);
                    
                    let neighbors = [
                        center_idx - 1, // Left
                        center_idx + 1, // Right
                        center_idx - w, // Top
                        center_idx + w, // Bottom
                    ];

                    let mut cell_flowed = false;

                    // A. Absolute gravity-avalanche collapse safety check (to prevent spikes)
                    let mut avalanche_checked = false;
                    for &neighbor_idx in &neighbors {
                        let h_neighbor = heightmap.data[neighbor_idx];
                        let geom_slope = h_center - h_neighbor;

                        if geom_slope > 0.20 {
                            let flow = (0.10 * (geom_slope - 0.20)).max(0.0);
                            if flow > 0.0 {
                                let current_temp_center = temp_heights[center_idx];
                                let current_temp_neighbor = temp_heights[neighbor_idx];
                                let temp_diff = current_temp_center - current_temp_neighbor;
                                let clamped_flow = flow.min(temp_diff * 0.4).max(0.0);
                                if clamped_flow > FLOW_INACTIVE_THRESHOLD {
                                    let nx = neighbor_idx % w;
                                    let ny = neighbor_idx / w;
                                    let neighbor_b = (ny / block_size) * cols + (nx / block_size);
                                    
                                    activate_neighbor(b, clamped_flow, temp_heights, heightmap, &mut modified, &mut next_displacements);
                                    activate_neighbor(neighbor_b, clamped_flow, temp_heights, heightmap, &mut modified, &mut next_displacements);

                                    advect_color(cell_colors, center_idx, neighbor_idx, clamped_flow, temp_heights[neighbor_idx]);
                                    temp_heights[center_idx] -= clamped_flow;
                                    temp_heights[neighbor_idx] += clamped_flow;
                                    total_flow += clamped_flow;
                                    cell_flowed = true;
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
                    let mut closest_marble_idx = None;
                    let mut min_dist_to_marble = f32::MAX;
                    if !active_marbles.is_empty() {
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

                    let oobleck_params = if material == crate::MaterialMode::Oobleck {
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

                    for (_dir_idx, &neighbor_idx) in neighbors.iter().enumerate() {
                        let h_neighbor = heightmap.data[neighbor_idx];
                        let geom_slope = h_center - h_neighbor;

                        // B. Material-specific parameters
                        let threshold;
                        let alpha;
                        let lock_chance;
                        let quantize_size: Option<f32> = None;

                        match material {
                            crate::MaterialMode::Oobleck => {
                                let (th, al, lc) = oobleck_params.unwrap();
                                threshold = th;
                                alpha = al;
                                lock_chance = lc;
                            }
                            _ => {
                                threshold = 0.0;
                                alpha = 0.0;
                                lock_chance = 0.0;
                            }
                        }

                        let slope = geom_slope;

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
                                    if clamped_flow > FLOW_INACTIVE_THRESHOLD {
                                        let nx = neighbor_idx % w;
                                        let ny = neighbor_idx / w;
                                        let neighbor_b = (ny / block_size) * cols + (nx / block_size);
                                        
                                        activate_neighbor(b, clamped_flow, temp_heights, heightmap, &mut modified, &mut next_displacements);
                                        activate_neighbor(neighbor_b, clamped_flow, temp_heights, heightmap, &mut modified, &mut next_displacements);

                                        advect_color(cell_colors, center_idx, neighbor_idx, clamped_flow, temp_heights[neighbor_idx]);
                                        temp_heights[center_idx] -= clamped_flow;
                                        temp_heights[neighbor_idx] += clamped_flow;
                                        total_flow += clamped_flow;
                                        cell_flowed = true;
                                        flow_occurred = true;
                                    }
                                }
                            }
                        }
                    }

                    sliding[center_idx] = cell_flowed;
                }
            }
        }
    } else if is_dynamic {
        for b in 0..expected_len {
            if !will_simulate[b] {
                continue;
            }

            let bx = b % cols;
            let by = b / cols;
            let start_x = (bx * block_size).max(1);
            let end_x = (((bx + 1) * block_size) - 1).min(w - 2);
            let start_y = (by * block_size).max(1);
            let end_y = (((by + 1) * block_size) - 1).min(h - 2);

            for y in start_y..=end_y {
                let row_offset = y * w;
                for x in start_x..=end_x {
                    let center_idx = row_offset + x;
                    let h_center = heightmap.data[center_idx];

                    // Load neighbor heights and find minimum
                    let h_left = heightmap.data[center_idx - 1];
                    let h_right = heightmap.data[center_idx + 1];
                    let h_top = heightmap.data[center_idx - w];
                    let h_bottom = heightmap.data[center_idx + w];

                    let min_h = h_left.min(h_right).min(h_top).min(h_bottom);

                    // Fast-path shortcut
                    if h_center - min_h <= threshold_min {
                        sliding[center_idx] = false;
                        continue;
                    }

                    let seed = (x as u32).wrapping_mul(1299689) ^ (y as u32).wrapping_mul(314159) ^ time_seed.wrapping_mul(7213);
                    
                    let neighbors = [
                        center_idx - 1, // Left
                        center_idx + 1, // Right
                        center_idx - w, // Top
                        center_idx + w, // Bottom
                    ];

                    let mut cell_flowed = false;

                    // A. Absolute gravity-avalanche collapse safety check (to prevent spikes)
                    let mut avalanche_checked = false;
                    for &neighbor_idx in &neighbors {
                        let h_neighbor = heightmap.data[neighbor_idx];
                        let geom_slope = h_center - h_neighbor;

                        if geom_slope > 0.20 {
                            let flow = (0.10 * (geom_slope - 0.20)).max(0.0);
                            if flow > 0.0 {
                                let current_temp_center = temp_heights[center_idx];
                                let current_temp_neighbor = temp_heights[neighbor_idx];
                                let temp_diff = current_temp_center - current_temp_neighbor;
                                let clamped_flow = flow.min(temp_diff * 0.4).max(0.0);
                                if clamped_flow > FLOW_INACTIVE_THRESHOLD {
                                    let nx = neighbor_idx % w;
                                    let ny = neighbor_idx / w;
                                    let neighbor_b = (ny / block_size) * cols + (nx / block_size);
                                    
                                    activate_neighbor(b, clamped_flow, temp_heights, heightmap, &mut modified, &mut next_displacements);
                                    activate_neighbor(neighbor_b, clamped_flow, temp_heights, heightmap, &mut modified, &mut next_displacements);

                                    advect_color(cell_colors, center_idx, neighbor_idx, clamped_flow, temp_heights[neighbor_idx]);
                                    temp_heights[center_idx] -= clamped_flow;
                                    temp_heights[neighbor_idx] += clamped_flow;
                                    total_flow += clamped_flow;
                                    cell_flowed = true;
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
                    for &n_idx in &neighbors {
                        if heightmap.data[n_idx] >= h_center - 1e-4 {
                            higher_neighbors += 1;
                        }
                    }

                    let (base_threshold, alpha, quantize_size, lock_chance_low, lock_chance_high): (f32, f32, Option<f32>, f32, f32) = match material {
                        crate::MaterialMode::DrySand => (0.08, 0.25, Some(0.01), 0.10, 0.80),
                        crate::MaterialMode::CoarseSand => (0.11, 0.22, Some(0.035), 0.15, 0.75),
                        _ => (0.08, 0.25, Some(0.01), 0.10, 0.80),
                    };

                    for &neighbor_idx in &neighbors {
                        let h_neighbor = heightmap.data[neighbor_idx];
                        let geom_slope = h_center - h_neighbor;

                        if geom_slope <= 1e-6 {
                            continue;
                        }

                        let threshold = {
                            let sliding_threshold = if material == crate::MaterialMode::DrySand { 0.04 } else { 0.06 };
                            if sliding[center_idx] { sliding_threshold } else { base_threshold }
                        };

                        let lock_chance = if higher_neighbors >= 3 { lock_chance_high } else { lock_chance_low };

                        // C. Stochastic locking and sliding condition
                        if geom_slope > threshold {
                            let flow_seed = (seed ^ (neighbor_idx as u32).wrapping_mul(997)) & 0xFFFF;
                            let rand_val = flow_seed as f32 / 65535.0;
                            
                            if rand_val >= lock_chance {
                                let alpha_noise = 1.0 + (rand_val - 0.5) * 0.8; // +/- 40% flow rate noise
                                let mut flow = (alpha * (geom_slope - threshold) * alpha_noise).max(0.0);
                                
                                if let Some(q) = quantize_size {
                                    flow = (flow / q).round() * q;
                                }

                                if flow > 0.0 {
                                    let temp_diff = temp_heights[center_idx] - temp_heights[neighbor_idx];
                                    let clamped_flow = flow.min(temp_diff * 0.4).max(0.0);
                                    if clamped_flow > FLOW_INACTIVE_THRESHOLD {
                                        let nx = neighbor_idx % w;
                                        let ny = neighbor_idx / w;
                                        let neighbor_b = (ny / block_size) * cols + (nx / block_size);
                                        
                                        activate_neighbor(b, clamped_flow, temp_heights, heightmap, &mut modified, &mut next_displacements);
                                        activate_neighbor(neighbor_b, clamped_flow, temp_heights, heightmap, &mut modified, &mut next_displacements);

                                        advect_color(cell_colors, center_idx, neighbor_idx, clamped_flow, temp_heights[neighbor_idx]);
                                        temp_heights[center_idx] -= clamped_flow;
                                        temp_heights[neighbor_idx] += clamped_flow;
                                        total_flow += clamped_flow;
                                        cell_flowed = true;
                                        flow_occurred = true;
                                    }
                                }
                            }
                        }
                    }

                    sliding[center_idx] = cell_flowed;
                }
            }
        }
    } else {
        // Static CA loop fully optimized
        let (threshold, alpha, quantize_size, lock_chance): (f32, f32, Option<f32>, f32) = match material {
            crate::MaterialMode::ButterCream => (0.04, 0.15, None, 0.20),
            crate::MaterialMode::Snow => (0.15, 0.04, None, 0.30),
            crate::MaterialMode::WetSand => (0.10, 0.06, None, 0.15),
            crate::MaterialMode::FinePowder => (0.01, 0.35, None, 0.02),
            crate::MaterialMode::MoonDust => (0.22, 0.02, Some(0.015), 0.40),
            crate::MaterialMode::KineticSand => (0.12, 0.12, Some(0.015), 0.75),
            _ => (0.10, 0.06, None, 0.15),
        };

        for b in 0..expected_len {
            if !will_simulate[b] {
                continue;
            }

            let bx = b % cols;
            let by = b / cols;
            let start_x = (bx * block_size).max(1);
            let end_x = (((bx + 1) * block_size) - 1).min(w - 2);
            let start_y = (by * block_size).max(1);
            let end_y = (((by + 1) * block_size) - 1).min(h - 2);

            for y in start_y..=end_y {
                let row_offset = y * w;
                for x in start_x..=end_x {
                    let center_idx = row_offset + x;
                    let h_center = heightmap.data[center_idx];

                    // Load neighbor heights and find minimum
                    let h_left = heightmap.data[center_idx - 1];
                    let h_right = heightmap.data[center_idx + 1];
                    let h_top = heightmap.data[center_idx - w];
                    let h_bottom = heightmap.data[center_idx + w];

                    let min_h = h_left.min(h_right).min(h_top).min(h_bottom);

                    // Fast-path shortcut
                    if h_center - min_h <= threshold_min {
                        sliding[center_idx] = false;
                        continue;
                    }

                    let seed = (x as u32).wrapping_mul(1299689) ^ (y as u32).wrapping_mul(314159) ^ time_seed.wrapping_mul(7213);
                    
                    let neighbors = [
                        center_idx - 1, // Left
                        center_idx + 1, // Right
                        center_idx - w, // Top
                        center_idx + w, // Bottom
                    ];

                    let mut cell_flowed = false;

                    // A. Absolute gravity-avalanche collapse safety check (to prevent spikes)
                    let mut avalanche_checked = false;
                    for &neighbor_idx in &neighbors {
                        let h_neighbor = heightmap.data[neighbor_idx];
                        let geom_slope = h_center - h_neighbor;

                        if geom_slope > 0.20 {
                            let flow = (0.10 * (geom_slope - 0.20)).max(0.0);
                            if flow > 0.0 {
                                let current_temp_center = temp_heights[center_idx];
                                let current_temp_neighbor = temp_heights[neighbor_idx];
                                let temp_diff = current_temp_center - current_temp_neighbor;
                                let clamped_flow = flow.min(temp_diff * 0.4).max(0.0);
                                if clamped_flow > FLOW_INACTIVE_THRESHOLD {
                                    let nx = neighbor_idx % w;
                                    let ny = neighbor_idx / w;
                                    let neighbor_b = (ny / block_size) * cols + (nx / block_size);
                                    
                                    activate_neighbor(b, clamped_flow, temp_heights, heightmap, &mut modified, &mut next_displacements);
                                    activate_neighbor(neighbor_b, clamped_flow, temp_heights, heightmap, &mut modified, &mut next_displacements);

                                    advect_color(cell_colors, center_idx, neighbor_idx, clamped_flow, temp_heights[neighbor_idx]);
                                    temp_heights[center_idx] -= clamped_flow;
                                    temp_heights[neighbor_idx] += clamped_flow;
                                    total_flow += clamped_flow;
                                    cell_flowed = true;
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

                    // Standard Settling Check
                    for &neighbor_idx in &neighbors {
                        let h_neighbor = heightmap.data[neighbor_idx];
                        let geom_slope = h_center - h_neighbor;

                        if geom_slope <= threshold {
                            continue;
                        }

                        // C. Stochastic locking and sliding condition
                        let flow_seed = (seed ^ (neighbor_idx as u32).wrapping_mul(997)) & 0xFFFF;
                        let rand_val = flow_seed as f32 / 65535.0;
                        
                        if rand_val >= lock_chance {
                            let alpha_noise = 1.0 + (rand_val - 0.5) * 0.8; // +/- 40% flow rate noise
                            let mut flow = (alpha * (geom_slope - threshold) * alpha_noise).max(0.0);
                            
                            if let Some(q) = quantize_size {
                                flow = (flow / q).round() * q;
                            }

                            if flow > 0.0 {
                                let temp_diff = temp_heights[center_idx] - temp_heights[neighbor_idx];
                                let clamped_flow = flow.min(temp_diff * 0.4).max(0.0);
                                if clamped_flow > FLOW_INACTIVE_THRESHOLD {
                                    let nx = neighbor_idx % w;
                                    let ny = neighbor_idx / w;
                                    let neighbor_b = (ny / block_size) * cols + (nx / block_size);
                                    
                                    activate_neighbor(b, clamped_flow, temp_heights, heightmap, &mut modified, &mut next_displacements);
                                    activate_neighbor(neighbor_b, clamped_flow, temp_heights, heightmap, &mut modified, &mut next_displacements);

                                    advect_color(cell_colors, center_idx, neighbor_idx, clamped_flow, temp_heights[neighbor_idx]);
                                    temp_heights[center_idx] -= clamped_flow;
                                    temp_heights[neighbor_idx] += clamped_flow;
                                    total_flow += clamped_flow;
                                    cell_flowed = true;
                                    flow_occurred = true;
                                }
                            }
                        }
                    }

                    sliding[center_idx] = cell_flowed;
                }
            }
        }
    }

    // 3. Copy back updated blocks
    for b in 0..expected_len {
        if modified[b] {
            let bx = b % cols;
            let by = b / cols;
            let start_x = bx * block_size;
            let end_x = ((bx + 1) * block_size).min(w);
            let start_y = by * block_size;
            let end_y = ((by + 1) * block_size).min(h);
            for y in start_y..end_y {
                let offset = y * w;
                heightmap.data[offset + start_x..offset + end_x]
                    .copy_from_slice(&temp_heights[offset + start_x..offset + end_x]);
            }
        }
    }

    // Compute updated active bounds for this frame
    let mut min_bx = cols;
    let mut max_bx = 0;
    let mut min_by = rows;
    let mut max_by = 0;
    let mut any_modified = false;

    for b in 0..expected_len {
        if modified[b] {
            any_modified = true;
            let bx = b % cols;
            let by = b / cols;
            min_bx = min_bx.min(bx);
            max_bx = max_bx.max(bx);
            min_by = min_by.min(by);
            max_by = max_by.max(by);
        }
    }

    if any_modified {
        active_bounds.min_x = min_bx * block_size;
        active_bounds.max_x = ((max_bx + 1) * block_size - 1).min(w - 1);
        active_bounds.min_y = min_by * block_size;
        active_bounds.max_y = ((max_by + 1) * block_size - 1).min(h - 1);
        active_bounds.active = flow_occurred;
    } else {
        active_bounds.active = false;
    }

    for b in 0..expected_len {
        if !will_simulate[b] {
            next_displacements[b] = next_displacements[b].max(last_displacements[b]);
        } else {
            last_simulated_ticks[b] = tick_count;
        }
    }
    *last_displacements = next_displacements;

    total_flow
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_draw_point_out_of_bounds() {
        let mut hm = Heightmap::new(512, 512, crate::DEFAULT_SAND_HEIGHT);
        let mut cell_colors = vec![0u8; 512 * 512 * 4];
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
            &mut cell_colors,
            Vec2::new(5.0, 5.0),
            Vec2::new(5.0, 5.0),
            0.1,
            &mut bounds,
            crate::MaterialMode::ButterCream,
        );

        // Assert that heightmap data is unchanged
        for &val in hm.as_slice() {
            assert_eq!(val, crate::DEFAULT_SAND_HEIGHT);
        }
    }

    #[test]
    fn test_draw_point_partial_overlap() {
        let mut hm = Heightmap::new(512, 512, crate::DEFAULT_SAND_HEIGHT);
        let mut cell_colors = vec![0u8; 512 * 512 * 4];
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
            &mut cell_colors,
            Vec2::new(-1.0, 0.0),
            Vec2::new(-1.0, 0.0),
            0.05,
            &mut bounds,
            crate::MaterialMode::ButterCream,
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
        let mut hm = Heightmap::new(512, 512, crate::DEFAULT_SAND_HEIGHT);
        let mut cell_colors = vec![0u8; 512 * 512 * 4];
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
            &mut cell_colors,
            Vec2::new(-0.5, 0.0),
            Vec2::new(0.5, 0.0),
            0.05,
            &mut bounds,
            crate::MaterialMode::ButterCream,
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
        let mut hm = Heightmap::new(512, 512, crate::DEFAULT_SAND_HEIGHT);
        let mut cell_colors = vec![0u8; 512 * 512 * 4];
        let mut bounds = ActiveBounds {
            min_x: 0,
            max_x: 0,
            min_y: 0,
            max_y: 0,
            active: false,
        };

        displace_line(
            &mut hm,
            &mut cell_colors,
            Vec2::new(1e18, 1e18),
            Vec2::new(1e18, 1e18),
            0.1,
            &mut bounds,
            crate::MaterialMode::ButterCream,
        );
        for &val in hm.as_slice() {
            assert_eq!(val, crate::DEFAULT_SAND_HEIGHT);
        }
    }

    #[test]
    fn test_multipass_carving() {
        let mut hm = Heightmap::new(512, 512, crate::DEFAULT_SAND_HEIGHT);
        let mut cell_colors = vec![0u8; 512 * 512 * 4];
        let mut bounds = ActiveBounds {
            min_x: 0,
            max_x: 0,
            min_y: 0,
            max_y: 0,
            active: false,
        };

        // Pass 1: carving at (0.0, 0.0) with DrySand (20% residual factor)
        displace_line(
            &mut hm,
            &mut cell_colors,
            Vec2::ZERO,
            Vec2::ZERO,
            0.05,
            &mut bounds,
            crate::MaterialMode::DrySand,
        );

        let center_idx = 256 * 512 + 256;
        let h1 = hm.data[center_idx];
        // Expect height to be approximately 20% of 0.35 = 0.07
        assert!((h1 - 0.07).abs() < 0.035, "First pass height should be ~0.07, got {}", h1);

        // Pass 2: carving again at (0.0, 0.0)
        displace_line(
            &mut hm,
            &mut cell_colors,
            Vec2::ZERO,
            Vec2::ZERO,
            0.05,
            &mut bounds,
            crate::MaterialMode::DrySand,
        );
        let h2 = hm.data[center_idx];
        // Expect height to be approximately 20% of h1 = 0.20 * 0.07 = 0.014
        assert!((h2 - 0.014).abs() < 0.035, "Second pass height should be ~0.014, got {}", h2);
        assert!(h2 < h1, "Second pass should carve deeper than first pass");
    }

    #[test]
    fn test_volume_conservation() {
        let mut hm = Heightmap::new(512, 512, 0.4);
        let mut cell_colors = vec![0u8; 512 * 512 * 4];
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
            &mut cell_colors,
            Vec2::new(-0.2, 0.2),
            Vec2::new(0.2, -0.2),
            0.03,
            &mut bounds,
            crate::MaterialMode::ButterCream,
        );

        let final_sum: f64 = hm.as_slice().iter().map(|&x| x as f64).sum();
        let diff = (final_sum - initial_sum).abs();
        assert!(diff < 1e-5, "Volume not conserved! diff = {}", diff);
    }

    #[test]
    fn test_draw_line_extreme_coordinates_overflow() {
        let mut hm = Heightmap::new(512, 512, crate::DEFAULT_SAND_HEIGHT);
        let mut cell_colors = vec![0u8; 512 * 512 * 4];
        let mut bounds = ActiveBounds {
            min_x: 0,
            max_x: 0,
            min_y: 0,
            max_y: 0,
            active: false,
        };
        displace_line(
            &mut hm,
            &mut cell_colors,
            Vec2::new(-1e18, 0.0),
            Vec2::new(1e18, 0.0),
            0.1,
            &mut bounds,
            crate::MaterialMode::ButterCream,
        );
    }

    #[test]
    fn test_volume_conservation_with_saturation() {
        let mut hm = Heightmap::new(512, 512, 0.70);
        let mut cell_colors = vec![0u8; 512 * 512 * 4];
        let mut bounds = ActiveBounds {
            min_x: 0,
            max_x: 0,
            min_y: 0,
            max_y: 0,
            active: false,
        };
        let initial_sum: f64 = hm.as_slice().iter().map(|&x| x as f64).sum();

        // Perform displacement at a single point to trigger local saturation in the inner ridge
        displace_line(&mut hm, &mut cell_colors, Vec2::ZERO, Vec2::ZERO, 0.02, &mut bounds, crate::MaterialMode::ButterCream);

        let final_sum: f64 = hm.as_slice().iter().map(|&x| x as f64).sum();
        let diff = (final_sum - initial_sum).abs();
        assert!(diff < 1e-5, "Volume not conserved! diff = {}", diff);
    }

    #[test]
    fn test_settling_flow_and_volume_conservation() {
        let mut hm = Heightmap::new(512, 512, 0.5);
        let mut temp_heights = vec![0.5; 512 * 512];
        let mut cell_colors = vec![0u8; 512 * 512 * 4];

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

        let mut wave_vel = vec![0.0; 512 * 512];
        let mut active_blocks: Vec<crate::BlockActivity> = Vec::new();
        let mut last_displacements = vec![1.0; 256];
        let mut last_simulated_ticks = vec![0; 256];
        let budget_n = 256;
        let mut flow_occurred = false;
        let mut sliding = vec![false; 512 * 512];

        for i in 0..10 {
            let flow = settle_tick(
                &mut hm,
                &mut temp_heights,
                &mut cell_colors,
                &mut sliding,
                &mut bounds,
                &mut active_blocks,
                &mut last_displacements,
                &mut last_simulated_ticks,
                budget_n,
                32,
                crate::MaterialMode::ButterCream,
                &[],
                12345,
                &mut wave_vel,
                crate::SandboxShape::Circle,
                i as u32,
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
        let mut cell_colors = vec![0u8; 512 * 512 * 4];

        let mut bounds = ActiveBounds {
            min_x: 250,
            max_x: 262,
            min_y: 250,
            max_y: 262,
            active: true,
        };

        let mut wave_vel = vec![0.0; 512 * 512];
        let mut active_blocks: Vec<crate::BlockActivity> = Vec::new();
        let mut last_displacements = Vec::new();
        let mut last_simulated_ticks = Vec::new();
        let budget_n = 256;
        let mut sliding = vec![false; 512 * 512];

        let flow = settle_tick(
            &mut hm,
            &mut temp_heights,
            &mut cell_colors,
            &mut sliding,
            &mut bounds,
            &mut active_blocks,
            &mut last_displacements,
            &mut last_simulated_ticks,
            budget_n,
            32,
            crate::MaterialMode::ButterCream,
            &[],
            12345,
            &mut wave_vel,
            crate::SandboxShape::Circle,
            0,
        );
        assert_eq!(flow, 0.0);
        assert!(!bounds.active, "Settling should deactivate when stable");
    }

    #[test]
    fn test_material_presets_and_avalanche() {
        use crate::MaterialMode;
        
        let materials = [
            MaterialMode::ButterCream,
            MaterialMode::DrySand,
            MaterialMode::Snow,
            MaterialMode::KineticSand,
            MaterialMode::WetSand,
            MaterialMode::FinePowder,
            MaterialMode::Oobleck,
            MaterialMode::MoonDust,
            MaterialMode::Water,
            MaterialMode::Milk,
            MaterialMode::VegetableOil,
            MaterialMode::CalmWater,
            MaterialMode::Yogurt,
            MaterialMode::CoarseSand,
        ];

        for &mat in &materials {
            let mut hm = Heightmap::new(64, 64, 0.5);
            let mut temp_heights = vec![0.5; 64 * 64];
            let mut cell_colors = vec![0u8; 64 * 64 * 4];
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

            let mut wave_vel = vec![0.0; 64 * 64];
            let mut active_blocks: Vec<crate::BlockActivity> = Vec::new();
            let mut last_displacements = vec![1.0; 4];
            let mut last_simulated_ticks = vec![0; 4];
            let budget_n = 256;
            let flow = settle_tick(
                &mut hm,
                &mut temp_heights,
                &mut cell_colors,
                &mut sliding,
                &mut bounds,
                &mut active_blocks,
                &mut last_displacements,
                &mut last_simulated_ticks,
                budget_n,
                32,
                mat,
                &[ActiveMarbleInfo { pos: Vec2::ZERO, vel: 0.1, vel_vec: Vec2::new(0.1, 0.0) }],
                9999,
                &mut wave_vel,
                crate::SandboxShape::Circle,
                0,
            );

            assert!(flow > 0.0, "Material {:?} should flow under steep slope", mat);
        }
    }

    #[test]
    fn test_color_conservation() {
        let mut hm = Heightmap::new(128, 128, 0.5);
        // Put a peak in the center so sand flows
        let center_idx = 64 * 128 + 64;
        hm.data[center_idx] = 1.0;

        let mut cell_colors = vec![0u8; 128 * 128 * 4];
        // Initialize cell_colors with a gradient/pattern
        for y in 0..128 {
            for x in 0..128 {
                let idx = y * 128 + x;
                cell_colors[idx * 4 + 0] = (x * 2) as u8; // Red channel
                cell_colors[idx * 4 + 1] = (y * 2) as u8; // Green channel
                cell_colors[idx * 4 + 2] = 100;
                cell_colors[idx * 4 + 3] = 255;
            }
        }

        // Calculate initial total color (Red mass)
        let initial_red_mass: f64 = cell_colors.chunks_exact(4)
            .zip(hm.as_slice())
            .map(|(c, &h)| c[0] as f64 * h as f64)
            .sum();

        let mut temp_heights = vec![0.5; 128 * 128];
        let mut sliding = vec![false; 128 * 128];
        let mut bounds = ActiveBounds {
            min_x: 60,
            max_x: 68,
            min_y: 60,
            max_y: 68,
            active: true,
        };

        let mut wave_vel = vec![0.0; 128 * 128];
        let mut active_blocks: Vec<crate::BlockActivity> = Vec::new();
        let mut last_displacements = vec![1.0; 16];
        let mut last_simulated_ticks = vec![0; 16];

        // Settle a bit to trigger flows
        let flow = settle_tick(
            &mut hm,
            &mut temp_heights,
            &mut cell_colors,
            &mut sliding,
            &mut bounds,
            &mut active_blocks,
            &mut last_displacements,
            &mut last_simulated_ticks,
            256,
            32,
            crate::MaterialMode::DrySand,
            &[],
            12345,
            &mut wave_vel,
            crate::SandboxShape::Circle,
            0,
        );

        assert!(flow > 0.0, "Settling flow must occur for the test");

        // Calculate final total color (Red mass)
        let final_red_mass: f64 = cell_colors.chunks_exact(4)
            .zip(hm.as_slice())
            .map(|(c, &h)| c[0] as f64 * h as f64)
            .sum();

        let diff = (final_red_mass - initial_red_mass).abs();
        assert!(diff < 1e-4, "Color mass not conserved! diff = {}, initial = {}, final = {}", diff, initial_red_mass, final_red_mass);
    }
}

