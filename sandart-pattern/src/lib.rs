#![allow(dead_code)]

use glam::Vec2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackState {
    Playing,
    Paused,
    Stopped,
}

pub struct PlaybackController {
    /// Waypoints stored in Cartesian coordinates to eliminate trig computations in frame updates.
    pub waypoints: [Vec<Vec2>; 5],
    pub current_indices: [usize; 5],
    pub speed_multipliers: [f32; 5],
    pub state: PlaybackState,
    pub loop_pattern: bool,
}

impl PlaybackController {
    pub fn new() -> Self {
        Self {
            waypoints: [Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new()],
            current_indices: [0; 5],
            speed_multipliers: [1.0; 5],
            state: PlaybackState::Stopped,
            loop_pattern: true,
        }
    }

    pub fn clear_waypoints(&mut self) {
        for w in &mut self.waypoints {
            w.clear();
        }
        self.current_indices = [0; 5];
        self.speed_multipliers = [1.0; 5];
    }

    /// Randomize speed multipliers for each active marble by up to 10% (range [0.9, 1.1])
    pub fn randomize_speeds(&mut self, count: usize, base_seed: u32) {
        let count = count.clamp(1, 5);
        let mut seed = base_seed;
        for j in 0..5 {
            if j < count {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                let rand_val = seed as f32 / u32::MAX as f32; // [0.0, 1.0]
                let multiplier = 0.9 + rand_val * 0.2;
                self.speed_multipliers[j] = multiplier;
            } else {
                self.speed_multipliers[j] = 1.0;
            }
        }
    }

    /// Advance playback for all active marbles and return their next target positions.
    pub fn step_playback_all(
        &mut self,
        current_positions: &[Vec2; 5],
        count: usize,
        speed: f32,
        dt: f32,
    ) -> [Option<Vec2>; 5] {
        let mut targets = [None; 5];
        if self.state != PlaybackState::Playing {
            return targets;
        }

        let count = count.clamp(1, 5);
        let mut all_stopped = true;

        for j in 0..count {
            let wps = &self.waypoints[j];
            if wps.is_empty() {
                continue;
            }

            let mut idx = self.current_indices[j];
            if idx >= wps.len() {
                continue;
            }

            all_stopped = false;
            targets[j] = Some(current_positions[j]);

            let mut curr_pos = current_positions[j];
            let mut remaining_move = speed * self.speed_multipliers[j] * dt;
            let max_iterations = wps.len() * 4;
            let mut loop_count = 0;

            while remaining_move > 0.0 && idx < wps.len() && loop_count < max_iterations {
                loop_count += 1;
                let target = wps[idx];
                let to_target = target - curr_pos;
                let dist = to_target.length();

                if dist <= remaining_move || dist < 1e-5 {
                    curr_pos = target;
                    remaining_move -= dist;
                    idx += 1;
                    if idx >= wps.len() {
                        if self.loop_pattern {
                            idx = 0;
                        } else {
                            targets[j] = Some(target);
                            break;
                        }
                    }
                    targets[j] = Some(target);
                } else {
                    let step = to_target * (remaining_move / dist);
                    curr_pos += step;
                    targets[j] = Some(curr_pos);
                    remaining_move = 0.0;
                }
            }
            self.current_indices[j] = idx;
        }

        // Check if all active paths are finished (when looping is disabled)
        if !self.loop_pattern {
            let mut finished = true;
            for j in 0..count {
                if !self.waypoints[j].is_empty() && self.current_indices[j] < self.waypoints[j].len() {
                    finished = false;
                    break;
                }
            }
            if finished || all_stopped {
                self.state = PlaybackState::Stopped;
            }
        }

        targets
    }
}

/// Helper to automatically close open paths by traversing them in reverse back to the start.
pub fn close_loop_path(mut path: Vec<Vec2>) -> Vec<Vec2> {
    if path.len() < 2 {
        return path;
    }
    let first = path[0];
    let last = path[path.len() - 1];
    
    // Check if the path is open (first and last points are far apart)
    if first.distance(last) > 0.05 {
        // Create the return path: traverse back along the same waypoints in reverse order.
        // We omit the first and last elements during the reverse sweep to prevent duplicate adjacent waypoints.
        let mut return_path = path.clone();
        return_path.pop(); // remove last element (which is already the end of path)
        return_path.reverse();
        if !return_path.is_empty() {
            return_path.pop(); // remove the first element (which would be duplicate with start when wrapping)
        }
        path.extend(return_path);
    }
    path
}

/// Generates an Archimedean spiral trajectory in Cartesian coordinates: r = a * theta
/// Spacing represents the distance between consecutive turns.
pub fn generate_spiral(spacing: f32) -> Vec<Vec2> {
    if spacing <= 0.005 {
        return Vec::new();
    }

    let mut path = Vec::new();
    let max_r = 0.92;
    let a = spacing / (2.0 * std::f32::consts::PI);
    let total_theta = max_r / a;
    let turns = total_theta / (2.0 * std::f32::consts::PI);
    let steps = (turns * 128.0).ceil() as usize;

    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let theta = t * total_theta;
        let r = a * theta;
        if r <= max_r {
            path.push(Vec2::new(r * theta.cos(), r * theta.sin()));
        }
    }
    path
}

/// Generates a Lissajous curve path: x = sin(a*t + delta), y = sin(b*t)
pub fn generate_lissajous(a: f32, b: f32, delta: f32) -> Vec<Vec2> {
    let mut path = Vec::new();
    let max_r = 0.874f32;
    let steps = 1500;
    // 10 cycles is enough to draw complex overlapping figures
    let t_max = 2.0 * std::f32::consts::PI * 10.0;
    let mut max_len = 0.0f32;
    for i in 0..=steps {
        let t = (i as f32 / steps as f32) * t_max;
        let x = (a * t + delta).sin();
        let y = (b * t).sin();
        let p = Vec2::new(x, y);
        max_len = max_len.max(p.length());
        path.push(p);
    }
    if max_len > 1e-5 {
        let scale = max_r / max_len;
        for p in &mut path {
            *p *= scale;
        }
    }
    path
}

/// Generates a Rose curve path in polar coordinates: r = cos(k * theta)
pub fn generate_rose(k: f32) -> Vec<Vec2> {
    let mut path = Vec::new();
    let max_r = 0.874f32;
    let steps = 1500;
    // theta ranges to cover multiple petals cleanly
    let theta_max = 2.0 * std::f32::consts::PI * 8.0;
    for i in 0..=steps {
        let theta = (i as f32 / steps as f32) * theta_max;
        let r = (k * theta).cos() * max_r;
        let x = r * theta.cos();
        let y = r * theta.sin();
        path.push(Vec2::new(x, y));
    }
    path
}

/// Generates a Hypotrochoid (Spirograph) path rolled inside a unit circle
pub fn generate_hypotrochoid(r_inner: f32, d: f32) -> Vec<Vec2> {
    let mut path = Vec::new();
    let r_inner = r_inner.clamp(0.01, 0.99);
    let r_outer = 1.0f32; // Fixed outer circle radius
    let term1 = r_outer - r_inner;
    let max_possible_r = term1.abs() + d;
    let scale_factor = if max_possible_r > 1e-4 { 0.874 / max_possible_r } else { 1.0 };

    let steps = 2000;
    let theta_max = 2.0 * std::f32::consts::PI * 16.0;
    for i in 0..=steps {
        let theta = (i as f32 / steps as f32) * theta_max;
        let x = term1 * theta.cos() + d * ((term1 / r_inner) * theta).cos();
        let y = term1 * theta.sin() - d * ((term1 / r_inner) * theta).sin();
        path.push(Vec2::new(x * scale_factor, y * scale_factor));
    }
    path
}

/// Generates a Fermat spiral: r = a * sqrt(theta)
pub fn generate_fermat_spiral(turns: f32) -> Vec<Vec2> {
    let mut path = Vec::new();
    let max_r = 0.874f32;
    let theta_max = 2.0 * std::f32::consts::PI * turns.max(1.0);
    let a = max_r / theta_max.sqrt();
    let steps = 1500;
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let theta = t * theta_max;
        let r = a * theta.sqrt();
        path.push(Vec2::new(r * theta.cos(), r * theta.sin()));
    }
    path
}

fn hilbert_recursive(x: f32, y: f32, xi: f32, xj: f32, yi: f32, yj: f32, n: u32, path: &mut Vec<Vec2>) {
    if n == 0 {
        path.push(Vec2::new(x + (xi + yi) / 2.0, y + (xj + yj) / 2.0));
    } else {
        hilbert_recursive(x, y, yi / 2.0, yj / 2.0, xi / 2.0, xj / 2.0, n - 1, path);
        hilbert_recursive(x + xi / 2.0, y + xj / 2.0, xi / 2.0, xj / 2.0, yi / 2.0, yj / 2.0, n - 1, path);
        hilbert_recursive(x + xi / 2.0 + yi / 2.0, y + xj / 2.0 + yj / 2.0, xi / 2.0, xj / 2.0, yi / 2.0, yj / 2.0, n - 1, path);
        hilbert_recursive(x + xi / 2.0 + yi, y + xj / 2.0 + yj, -yi / 2.0, -yj / 2.0, -xi / 2.0, -xj / 2.0, n - 1, path);
    }
}

/// Generates a recursive space-filling Hilbert curve
pub fn generate_hilbert_curve(order: u32) -> Vec<Vec2> {
    let mut path = Vec::new();
    let order = order.clamp(1, 6);
    let side = 1.748f32; // Fits inside [-0.874, 0.874]
    let start_x = -side / 2.0;
    let start_y = -side / 2.0;
    hilbert_recursive(start_x, start_y, side, 0.0, 0.0, side, order, &mut path);
    center_and_scale_path(&mut path, 0.874);
    path
}

fn center_and_scale_path(path: &mut [Vec2], max_limit: f32) {
    if path.is_empty() {
        return;
    }
    let mut min_x = f32::MAX;
    let mut max_x = f32::MIN;
    let mut min_y = f32::MAX;
    let mut max_y = f32::MIN;
    for p in path.iter() {
        min_x = min_x.min(p.x);
        max_x = max_x.max(p.x);
        min_y = min_y.min(p.y);
        max_y = max_y.max(p.y);
    }
    let cx = (min_x + max_x) * 0.5;
    let cy = (min_y + max_y) * 0.5;

    let mut max_r = 0.0f32;
    for p in path.iter_mut() {
        p.x -= cx;
        p.y -= cy;
        let r = p.length();
        max_r = max_r.max(r);
    }

    let scale = if max_r > 1e-4 { max_limit / max_r } else { 1.0 };
    for p in path.iter_mut() {
        *p *= scale;
    }
}

fn gosper_a(level: u32, angle: &mut f32, path: &mut Vec<Vec2>, step_len: f32) {
    if level == 0 {
        let last_pos = path.last().copied().unwrap_or(Vec2::ZERO);
        let next_pos = last_pos + Vec2::new(angle.cos(), angle.sin()) * step_len;
        path.push(next_pos);
    } else {
        gosper_a(level - 1, angle, path, step_len);
        *angle -= std::f32::consts::FRAC_PI_3; // -60 deg
        gosper_b(level - 1, angle, path, step_len);
        *angle -= 2.0 * std::f32::consts::FRAC_PI_3; // -120 deg
        gosper_b(level - 1, angle, path, step_len);
        *angle += std::f32::consts::FRAC_PI_3; // +60 deg
        gosper_a(level - 1, angle, path, step_len);
        *angle += 2.0 * std::f32::consts::FRAC_PI_3; // +120 deg
        gosper_a(level - 1, angle, path, step_len);
        gosper_a(level - 1, angle, path, step_len);
        *angle += std::f32::consts::FRAC_PI_3; // +60 deg
        gosper_b(level - 1, angle, path, step_len);
        *angle -= std::f32::consts::FRAC_PI_3; // -60 deg
    }
}

fn gosper_b(level: u32, angle: &mut f32, path: &mut Vec<Vec2>, step_len: f32) {
    if level == 0 {
        let last_pos = path.last().copied().unwrap_or(Vec2::ZERO);
        let next_pos = last_pos + Vec2::new(angle.cos(), angle.sin()) * step_len;
        path.push(next_pos);
    } else {
        *angle += std::f32::consts::FRAC_PI_3; // +60 deg
        gosper_a(level - 1, angle, path, step_len);
        *angle -= std::f32::consts::FRAC_PI_3; // -60 deg
        gosper_b(level - 1, angle, path, step_len);
        gosper_b(level - 1, angle, path, step_len);
        *angle -= 2.0 * std::f32::consts::FRAC_PI_3; // -120 deg
        gosper_b(level - 1, angle, path, step_len);
        *angle -= std::f32::consts::FRAC_PI_3; // -60 deg
        gosper_a(level - 1, angle, path, step_len);
        *angle += 2.0 * std::f32::consts::FRAC_PI_3; // +120 deg
        gosper_a(level - 1, angle, path, step_len);
        *angle += std::f32::consts::FRAC_PI_3; // +60 deg
        gosper_b(level - 1, angle, path, step_len);
    }
}

/// Generates a recursive space-filling Gosper curve (hexagonal)
pub fn generate_gosper_curve(order: u32) -> Vec<Vec2> {
    let order = order.clamp(1, 5);
    let mut path = Vec::new();
    path.push(Vec2::ZERO);
    let mut angle = 0.0f32;
    let step_len = 1.0f32;
    gosper_a(order, &mut angle, &mut path, step_len);
    center_and_scale_path(&mut path, 0.874);
    path
}

fn sierpinski_a(level: u32, angle: &mut f32, path: &mut Vec<Vec2>, step_len: f32) {
    if level == 0 {
        let last_pos = path.last().copied().unwrap_or(Vec2::ZERO);
        let next_pos = last_pos + Vec2::new(angle.cos(), angle.sin()) * step_len;
        path.push(next_pos);
    } else {
        sierpinski_b(level - 1, angle, path, step_len);
        *angle += std::f32::consts::FRAC_PI_3; // +60 deg
        sierpinski_a(level - 1, angle, path, step_len);
        *angle += std::f32::consts::FRAC_PI_3; // +60 deg
        sierpinski_b(level - 1, angle, path, step_len);
    }
}

fn sierpinski_b(level: u32, angle: &mut f32, path: &mut Vec<Vec2>, step_len: f32) {
    if level == 0 {
        let last_pos = path.last().copied().unwrap_or(Vec2::ZERO);
        let next_pos = last_pos + Vec2::new(angle.cos(), angle.sin()) * step_len;
        path.push(next_pos);
    } else {
        sierpinski_a(level - 1, angle, path, step_len);
        *angle -= std::f32::consts::FRAC_PI_3; // -60 deg
        sierpinski_b(level - 1, angle, path, step_len);
        *angle -= std::f32::consts::FRAC_PI_3; // -60 deg
        sierpinski_a(level - 1, angle, path, step_len);
    }
}

/// Generates a recursive space-filling Sierpinski arrowhead curve
pub fn generate_sierpinski_curve(order: u32) -> Vec<Vec2> {
    let order = order.clamp(1, 7);
    let mut path = Vec::new();
    path.push(Vec2::ZERO);
    let mut angle = 0.0f32;
    let step_len = 1.0f32;
    if order % 2 == 0 {
        sierpinski_a(order, &mut angle, &mut path, step_len);
    } else {
        sierpinski_b(order, &mut angle, &mut path, step_len);
    }
    center_and_scale_path(&mut path, 0.874);
    path
}

/// Generates a deterministic organic Random Walk (Brownian motion)
pub fn generate_random_walk(steps: usize, step_size: f32) -> Vec<Vec2> {
    let mut path = Vec::new();
    let max_r = 0.874f32;
    let mut current = Vec2::ZERO;
    path.push(current);

    let mut state = 12345u32;
    for _ in 0..steps {
        state = state.wrapping_mul(1664525).wrapping_add(1013904223);
        let angle = (state as f32 / u32::MAX as f32) * 2.0 * std::f32::consts::PI;
        let step = Vec2::new(angle.cos(), angle.sin()) * step_size;
        let mut next = current + step;

        let r = next.length();
        if r > max_r {
            let normal = if current.length() > 1e-5 { current.normalize() } else { Vec2::X };
            let reflected = step - 2.0 * step.dot(normal) * normal;
            next = current + reflected;
            if next.length() > max_r {
                next = if next.length() > 1e-5 { next.normalize() * max_r } else { Vec2::ZERO };
            }
        }
        current = next;
        path.push(current);
    }
    path
}

/// Generates a Lemniscate of Bernoulli (figure-8 infinity symbol)
pub fn generate_lemniscate(scale: f32) -> Vec<Vec2> {
    let mut path = Vec::new();
    let a = scale.clamp(0.1, 0.874);
    let steps = 800;
    for i in 0..=steps {
        let t = (i as f32 / steps as f32) * 2.0 * std::f32::consts::PI;
        let sin_t = t.sin();
        let cos_t = t.cos();
        let denom = 1.0 + sin_t * sin_t;
        let x = (a * cos_t) / denom;
        let y = (a * sin_t * cos_t) / denom;
        path.push(Vec2::new(x, y));
    }
    path
}

/// Generates count symmetric spiral arms rotated around the center
pub fn generate_multi_spiral(spacing: f32, count: usize) -> Vec<Vec<Vec2>> {
    let count = count.clamp(1, 5);
    let mut paths = vec![Vec::new(); count];
    if spacing <= 0.005 {
        return paths;
    }
    let max_r = 0.874f32;
    let a = spacing / (2.0 * std::f32::consts::PI);
    let total_theta = max_r / a;
    let turns = total_theta / (2.0 * std::f32::consts::PI);
    let steps = (turns * 128.0).ceil() as usize;

    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let theta = t * total_theta;
        let r = a * theta;
        if r <= max_r {
            for j in 0..count {
                let angle_offset = (j as f32 / count as f32) * 2.0 * std::f32::consts::PI;
                let angle = theta + angle_offset;
                paths[j].push(Vec2::new(r * angle.cos(), r * angle.sin()));
            }
        }
    }
    paths
}

/// Helper to strip comments from a G-code line, handling both ';' and nested '( ... )'.
fn strip_comments(line: &str) -> String {
    let mut result = String::new();
    let mut in_comment = false;
    for c in line.chars() {
        if c == ';' {
            break; // Semicolon comment continues to end of line
        }
        if c == '(' {
            in_comment = true;
            continue;
        }
        if c == ')' {
            in_comment = false;
            continue;
        }
        if !in_comment {
            result.push(c);
        }
    }
    result.trim().to_uppercase()
}

/// Helper to robustly extract a coordinate value following a prefix (e.g. 'X' or 'Y')
/// which supports both spaced (X 10) and spaceless (X10) formatting.
fn parse_coordinate(line: &str, prefix: char) -> Option<f32> {
    if let Some(pos) = line.find(prefix) {
        let remainder = line[pos + prefix.len_utf8()..].trim_start();
        let num_str: String = remainder
            .chars()
            .take_while(|&c| c.is_digit(10) || c == '.' || c == '-' || c == '+')
            .collect();
        num_str.parse::<f32>().ok()
    } else {
        None
    }
}

/// Parses a Theta-Rho (.thr) pattern file.
/// Format: space-separated polar `theta` (radians) and `rho` (normalized radius [0, 1]) lines.
pub fn parse_thr(content: &str) -> Result<Vec<Vec2>, String> {
    let mut waypoints = Vec::new();
    for (line_idx, line) in content.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // Strip inline comments starting with '#'
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 2 {
            return Err(format!(
                "Malformed polar waypoint on line {}: '{}'",
                line_idx + 1,
                line
            ));
        }
        let theta = parts[0].parse::<f32>().map_err(|e| {
            format!(
                "Invalid theta '{}' on line {}: {}",
                parts[0],
                line_idx + 1,
                e
            )
        })?;
        let rho = parts[1]
            .parse::<f32>()
            .map_err(|e| format!("Invalid rho '{}' on line {}: {}", parts[1], line_idx + 1, e))?;

        // Scale rho to 95% of our sand bed boundary (0.92 * 0.95 = 0.874) to leave space for sand ridges
        let max_r = 0.874f32;
        let r = (rho * max_r).clamp(-max_r, max_r);
        waypoints.push(Vec2::new(r * theta.cos(), r * theta.sin()));
    }
    Ok(waypoints)
}

/// Parses G-code files containing X/Y coordinate movements (G0/G1).
/// Automatically centers and scales the coordinates to fit visual boundary (0.92).
pub fn parse_gcode(content: &str) -> Result<Vec<Vec2>, String> {
    let mut raw_points = Vec::new();
    let mut last_x = 0.0f32;
    let mut last_y = 0.0f32;
    let mut has_x = true;
    let mut has_y = true;
    let mut relative_mode = false;

    for line in content.lines() {
        let clean_line = strip_comments(line);
        if clean_line.is_empty() {
            continue;
        }

        // Handle absolute (G90) and relative (G91) positioning modes
        if clean_line.contains("G90") {
            relative_mode = false;
        } else if clean_line.contains("G91") {
            relative_mode = true;
        }

        let x_val = parse_coordinate(&clean_line, 'X');
        let y_val = parse_coordinate(&clean_line, 'Y');

        if x_val.is_some() || y_val.is_some() {
            if let Some(x) = x_val {
                last_x = if relative_mode { last_x + x } else { x };
                has_x = true;
            }
            if let Some(y) = y_val {
                last_y = if relative_mode { last_y + y } else { y };
                has_y = true;
            }
            if has_x && has_y {
                raw_points.push(Vec2::new(last_x, last_y));
            }
        }
    }

    if raw_points.is_empty() {
        return Err("No valid G-code movements found".to_string());
    }

    let mut min_x = f32::MAX;
    let mut max_x = f32::MIN;
    let mut min_y = f32::MAX;
    let mut max_y = f32::MIN;
    for p in &raw_points {
        min_x = min_x.min(p.x);
        max_x = max_x.max(p.x);
        min_y = min_y.min(p.y);
        max_y = max_y.max(p.y);
    }

    let mut waypoints = Vec::new();
    // Corrected check: allow scaling and centering even if only one axis varies (straight lines)
    if min_x < max_x || min_y < max_y {
        let cx = (min_x + max_x) * 0.5;
        let cy = (min_y + max_y) * 0.5;

        let mut max_r = 0.0f32;
        for p in &raw_points {
            let px = p.x - cx;
            let py = p.y - cy;
            let r = (px * px + py * py).sqrt();
            max_r = max_r.max(r);
        }

        // Scale coordinates to 95% of our sand bed boundary (0.92 * 0.95 = 0.874) to leave space for sand ridges
        let max_limit = 0.874f32;
        let scale = if max_r > 1e-4 { max_limit / max_r } else { 1.0 };

        for p in raw_points {
            let px = (p.x - cx) * scale;
            let py = (p.y - cy) * scale;
            waypoints.push(Vec2::new(px, py));
        }
    } else {
        // Single flat coordinate point
        let max_limit = 0.874f32;
        for p in raw_points {
            waypoints.push(p.clamp(Vec2::splat(-max_limit), Vec2::splat(max_limit)));
        }
    }

    Ok(waypoints)
}

/// Generates the classic mathematical butterfly curve (intricate Zen butterfly)
pub fn generate_butterfly_curve() -> Vec<Vec2> {
    let mut path = Vec::new();
    let steps = 1200;
    // Parameter t goes from 0 to 12*PI to capture the full symmetrical density
    let max_t = 12.0 * std::f32::consts::PI;
    let mut max_len = 0.0f32;
    let mut temp_points = Vec::with_capacity(steps + 1);

    for i in 0..=steps {
        let t = (i as f32 / steps as f32) * max_t;
        let cos_t = t.cos();
        let sin_t = t.sin();
        let sin_t12 = (t / 12.0).sin();
        let sin_t12_pow5 = sin_t12.powi(5);
        let cos_4t = (4.0 * t).cos();
        
        // r = e^cos(t) - 2*cos(4t) + sin^5(t/12)
        let r = cos_t.exp() - 2.0 * cos_4t + sin_t12_pow5;
        let pt = Vec2::new(sin_t * r, cos_t * r);
        temp_points.push(pt);
        max_len = max_len.max(pt.length());
    }

    // Scale to standard sand bed boundaries (0.874)
    let limit = 0.874f32;
    let scale = if max_len > 1e-4 { limit / max_len } else { 1.0 };
    for p in temp_points {
        path.push(p * scale);
    }
    path
}

/// Generates nested concentric circles connected via smooth serpentine paths
pub fn generate_zen_waves() -> Vec<Vec2> {
    let mut path = Vec::new();
    // 6 concentric circle radii spanning from r=0.15 to r=0.874
    let radii = [0.15f32, 0.30, 0.45, 0.60, 0.75, 0.874];
    let steps = 200; // Steps per circle

    // Start at center
    path.push(Vec2::ZERO);

    for (idx, &r) in radii.iter().enumerate() {
        let prev_point = *path.last().unwrap_or(&Vec2::ZERO);
        
        // 1. Move from the end of the last circle (or center) to the start of this circle
        // We do a straight line transition
        let start_angle = if idx % 2 == 0 { 0.0f32 } else { 2.0 * std::f32::consts::PI };
        let start_pos = Vec2::new(r * start_angle.cos(), r * start_angle.sin());
        
        let transition_steps = 20;
        for i in 1..=transition_steps {
            let t = i as f32 / transition_steps as f32;
            path.push(prev_point.lerp(start_pos, t));
        }

        // 2. Trace the circle. Alternating direction makes it continuous and fluid
        for i in 0..=steps {
            let t = i as f32 / steps as f32;
            let angle = if idx % 2 == 0 {
                t * 2.0 * std::f32::consts::PI
            } else {
                (1.0 - t) * 2.0 * std::f32::consts::PI
            };
            path.push(Vec2::new(r * angle.cos(), r * angle.sin()));
        }
    }
    path
}

/// Generates a swirling floral star mandala using overlapping parametric loops
pub fn generate_zen_mandala() -> Vec<Vec2> {
    let mut path = Vec::new();
    let steps = 1000;
    let max_t = 2.0 * std::f32::consts::PI;
    let mut temp_points = Vec::with_capacity(steps + 1);
    let mut max_len = 0.0f32;

    for i in 0..=steps {
        let t = (i as f32 / steps as f32) * max_t;
        let x = t.cos() + 0.4 * (8.0 * t).cos() + 0.2 * (16.0 * t).sin();
        let y = t.sin() + 0.4 * (8.0 * t).sin() + 0.2 * (16.0 * t).cos();
        let pt = Vec2::new(x, y);
        temp_points.push(pt);
        max_len = max_len.max(pt.length());
    }

    // Scale to standard sand bed boundaries (0.874)
    let limit = 0.874f32;
    let scale = if max_len > 1e-4 { limit / max_len } else { 1.0 };
    for p in temp_points {
        path.push(p * scale);
    }
    path
}

/// Generates the dynamic clock layout or clearing path based on time and phase:
/// Phase 1 (Drawing/Display): Hour and Minute hands, plus an outer Zen border spiral.
/// Phase 2 (Clearing): A dense Hilbert Curve (order 4) to sweep the sand clean.
pub fn generate_clock_pattern(hours: u32, minutes: u32, _seconds: f32, _phase: u32) -> Vec<Vec2> {
    let mut path: Vec<Vec2> = Vec::new();

    // Drawing Phase: Digital Time oriented for default camera view (azimuth 0.0)
    // Helper to retrieve digital strokes for a character
    let get_digit_stroke = |c: char| -> &'static [(f32, f32)] {
        match c {
            '0' => &[
                (0.0, 0.0), (0.0, 1.0), (1.0, 1.0), (1.0, 0.0), (0.0, 0.0)
            ],
            '1' => &[
                (0.5, 1.0), (0.5, 0.0)
            ],
            '2' => &[
                (0.0, 1.0), (1.0, 1.0), (1.0, 0.5), (0.0, 0.5), (0.0, 0.0), (1.0, 0.0)
            ],
            '3' => &[
                (0.0, 1.0), (1.0, 1.0), (1.0, 0.5), (0.0, 0.5), (1.0, 0.5), (1.0, 0.0), (0.0, 0.0)
            ],
            '4' => &[
                (0.0, 1.0), (0.0, 0.5), (1.0, 0.5), (1.0, 1.0), (1.0, 0.0)
            ],
            '5' => &[
                (1.0, 1.0), (0.0, 1.0), (0.0, 0.5), (1.0, 0.5), (1.0, 0.0), (0.0, 0.0)
            ],
            '6' => &[
                (1.0, 1.0), (0.0, 1.0), (0.0, 0.0), (1.0, 0.0), (1.0, 0.5), (0.0, 0.5)
            ],
            '7' => &[
                (0.0, 1.0), (1.0, 1.0), (0.5, 0.0)
            ],
            '8' => &[
                (0.0, 0.5), (0.0, 1.0), (1.0, 1.0), (1.0, 0.5), (0.0, 0.5), (0.0, 0.0), (1.0, 0.0), (1.0, 0.5), (0.0, 0.5)
            ],
            '9' => &[
                (0.0, 0.5), (1.0, 0.5), (1.0, 1.0), (0.0, 1.0), (0.0, 0.5), (1.0, 0.5), (1.0, 0.0), (0.0, 0.0)
            ],
            ':' => &[
                (0.5, 0.65), (0.5, 0.75), (0.5, 0.65), // tick 1
                (0.5, 0.35), (0.5, 0.25), (0.5, 0.35)  // tick 2
            ],
            _ => &[],
        }
    };

    // Layout the digital clock digits in the center, rotated for default camera view (azimuth 0.0)
    // Screen horizontal is Y increasing (left to right), Screen vertical is X increasing (bottom to top)
    let chars = format!("{:02}:{:02}", hours, minutes);
    let w = 0.14f32; // width of each digit
    let h = 0.28f32; // height of each digit
    let spacing = 0.05f32; // spacing between digits
    let total_w = 5.0 * w + 4.0 * spacing;
    let start_y = -total_w / 2.0;

    let add_outer_transition = |path: &mut Vec<Vec2>, start_pos: Vec2| {
        if let Some(&last_p) = path.last() {
            let r_outer = 0.88f32;
            
            // Go to the closest edge (top if last_p.x <= 0, bottom if last_p.x > 0)
            // to avoid crossing any digit (since digits are spaced horizontally along Y)
            let x_out1 = if last_p.x <= 0.0 {
                -(r_outer * r_outer - last_p.y * last_p.y).sqrt()
            } else {
                (r_outer * r_outer - last_p.y * last_p.y).sqrt()
            };
            let p_out1 = Vec2::new(x_out1, last_p.y);
            
            // Come in from the closest edge to start_pos
            let x_out2 = if start_pos.x <= 0.0 {
                -(r_outer * r_outer - start_pos.y * start_pos.y).sqrt()
            } else {
                (r_outer * r_outer - start_pos.y * start_pos.y).sqrt()
            };
            let p_out2 = Vec2::new(x_out2, start_pos.y);

            let theta_end = p_out1.y.atan2(p_out1.x);
            let theta_start = p_out2.y.atan2(p_out2.x);

            // 1. Move from last_p straight out along X-axis to the outer edge
            let steps_out = 10;
            for i in 1..=steps_out {
                let t = i as f32 / steps_out as f32;
                path.push(last_p.lerp(p_out1, t));
            }

            // 2. Arc sweep along the circular border to the alignment of the next digit
            let mut diff = theta_start - theta_end;
            while diff > std::f32::consts::PI {
                diff -= 2.0 * std::f32::consts::PI;
            }
            while diff < -std::f32::consts::PI {
                diff += 2.0 * std::f32::consts::PI;
            }

            let steps_arc = 15;
            for i in 1..=steps_arc {
                let t = i as f32 / steps_arc as f32;
                let angle = theta_end + t * diff;
                path.push(Vec2::new(r_outer * angle.cos(), r_outer * angle.sin()));
            }

            // 3. Move straight in along X-axis from the outer edge to start_pos
            let steps_in = 10;
            for i in 1..=steps_in {
                let t = i as f32 / steps_in as f32;
                path.push(p_out2.lerp(start_pos, t));
            }
        } else {
            path.push(start_pos);
        }
    };

    for (idx, c) in chars.chars().enumerate() {
        let stroke = get_digit_stroke(c);
        if stroke.is_empty() {
            continue;
        }

        let char_y = start_y + idx as f32 * (w + spacing);

        // Transition from the current last point to the start of this digit
        let start_tx = stroke[0].0;
        let start_ty = stroke[0].1;
        let start_py = char_y + start_tx * w;
        let start_px = -h / 2.0 + start_ty * h;
        let start_pos = Vec2::new(start_px, start_py);

        add_outer_transition(&mut path, start_pos);

        // Draw the digit strokes
        for &(tx, ty) in stroke {
            let py = char_y + tx * w;
            let px = -h / 2.0 + ty * h;
            path.push(Vec2::new(px, py));
        }
    }

    path
}

pub fn generate_dinosaur() -> Vec<Vec2> {
    let raw_pts = [
        (-0.8823, -0.1357),
        (-0.9955, -0.0679),
        (-0.9276, -0.1810),
        (-0.9728, -0.2715),
        (-0.8710, -0.2262),
        (-0.8637, -0.2276),
        (-0.8543, -0.2295),
        (-0.8432, -0.2317),
        (-0.8308, -0.2342),
        (-0.8173, -0.2368),
        (-0.8031, -0.2395),
        (-0.7887, -0.2422),
        (-0.7742, -0.2447),
        (-0.7601, -0.2470),
        (-0.7466, -0.2489),
        (-0.7335, -0.2505),
        (-0.7202, -0.2518),
        (-0.7067, -0.2531),
        (-0.6931, -0.2542),
        (-0.6794, -0.2552),
        (-0.6657, -0.2562),
        (-0.6519, -0.2571),
        (-0.6382, -0.2581),
        (-0.6245, -0.2591),
        (-0.6108, -0.2602),
        (-0.5967, -0.2614),
        (-0.5815, -0.2626),
        (-0.5658, -0.2639),
        (-0.5500, -0.2652),
        (-0.5345, -0.2665),
        (-0.5196, -0.2678),
        (-0.5058, -0.2689),
        (-0.4936, -0.2699),
        (-0.4751, -0.2715),
        (-0.4761, -0.2823),
        (-0.4777, -0.2962),
        (-0.4796, -0.3128),
        (-0.4819, -0.3315),
        (-0.4843, -0.3519),
        (-0.4869, -0.3736),
        (-0.4894, -0.3959),
        (-0.4919, -0.4185),
        (-0.4941, -0.4409),
        (-0.4961, -0.4625),
        (-0.4976, -0.4830),
        (-0.4986, -0.5017),
        (-0.4991, -0.5183),
        (-0.4988, -0.5322),
        (-0.4977, -0.5430),
        (-0.4958, -0.5514),
        (-0.4930, -0.5587),
        (-0.4896, -0.5649),
        (-0.4855, -0.5698),
        (-0.4810, -0.5736),
        (-0.4760, -0.5761),
        (-0.4707, -0.5774),
        (-0.4653, -0.5775),
        (-0.4597, -0.5764),
        (-0.4542, -0.5740),
        (-0.4487, -0.5703),
        (-0.4434, -0.5654),
        (-0.4385, -0.5592),
        (-0.4339, -0.5517),
        (-0.4299, -0.5430),
        (-0.4261, -0.5317),
        (-0.4223, -0.5172),
        (-0.4186, -0.5000),
        (-0.4150, -0.4805),
        (-0.4114, -0.4592),
        (-0.4080, -0.4366),
        (-0.4046, -0.4133),
        (-0.4014, -0.3898),
        (-0.3984, -0.3665),
        (-0.3955, -0.3440),
        (-0.3928, -0.3227),
        (-0.3904, -0.3032),
        (-0.3882, -0.2859),
        (-0.3846, -0.2602),
        (-0.3838, -0.2705),
        (-0.3831, -0.2838),
        (-0.3823, -0.2997),
        (-0.3814, -0.3177),
        (-0.3804, -0.3373),
        (-0.3794, -0.3580),
        (-0.3782, -0.3794),
        (-0.3769, -0.4011),
        (-0.3754, -0.4225),
        (-0.3737, -0.4433),
        (-0.3718, -0.4628),
        (-0.3698, -0.4808),
        (-0.3674, -0.4967),
        (-0.3649, -0.5100),
        (-0.3620, -0.5204),
        (-0.3587, -0.5285),
        (-0.3550, -0.5356),
        (-0.3509, -0.5417),
        (-0.3465, -0.5467),
        (-0.3419, -0.5505),
        (-0.3370, -0.5532),
        (-0.3320, -0.5547),
        (-0.3269, -0.5550),
        (-0.3218, -0.5540),
        (-0.3167, -0.5518),
        (-0.3118, -0.5482),
        (-0.3070, -0.5433),
        (-0.3024, -0.5371),
        (-0.2981, -0.5294),
        (-0.2941, -0.5204),
        (-0.2903, -0.5087),
        (-0.2866, -0.4936),
        (-0.2829, -0.4756),
        (-0.2792, -0.4553),
        (-0.2757, -0.4332),
        (-0.2722, -0.4098),
        (-0.2689, -0.3855),
        (-0.2657, -0.3611),
        (-0.2626, -0.3368),
        (-0.2598, -0.3134),
        (-0.2571, -0.2912),
        (-0.2547, -0.2709),
        (-0.2525, -0.2530),
        (-0.2489, -0.2262),
        (-0.2465, -0.2213),
        (-0.2440, -0.2155),
        (-0.2413, -0.2090),
        (-0.2384, -0.2018),
        (-0.2353, -0.1940),
        (-0.2320, -0.1856),
        (-0.2286, -0.1768),
        (-0.2250, -0.1674),
        (-0.2212, -0.1577),
        (-0.2172, -0.1477),
        (-0.2130, -0.1374),
        (-0.2087, -0.1269),
        (-0.2042, -0.1163),
        (-0.1995, -0.1056),
        (-0.1946, -0.0948),
        (-0.1895, -0.0842),
        (-0.1842, -0.0736),
        (-0.1788, -0.0632),
        (-0.1732, -0.0530),
        (-0.1674, -0.0431),
        (-0.1614, -0.0335),
        (-0.1553, -0.0244),
        (-0.1490, -0.0157),
        (-0.1424, -0.0076),
        (-0.1357, 0.0000),
        (-0.1288, 0.0072),
        (-0.1214, 0.0143),
        (-0.1137, 0.0213),
        (-0.1057, 0.0282),
        (-0.0974, 0.0349),
        (-0.0888, 0.0416),
        (-0.0800, 0.0481),
        (-0.0709, 0.0545),
        (-0.0617, 0.0607),
        (-0.0523, 0.0668),
        (-0.0428, 0.0727),
        (-0.0331, 0.0785),
        (-0.0234, 0.0841),
        (-0.0136, 0.0895),
        (-0.0038, 0.0947),
        (0.0060, 0.0998),
        (0.0158, 0.1047),
        (0.0256, 0.1093),
        (0.0353, 0.1138),
        (0.0449, 0.1180),
        (0.0544, 0.1220),
        (0.0637, 0.1258),
        (0.0728, 0.1294),
        (0.0818, 0.1327),
        (0.0905, 0.1357),
        (0.0991, 0.1386),
        (0.1078, 0.1411),
        (0.1164, 0.1435),
        (0.1251, 0.1456),
        (0.1338, 0.1474),
        (0.1424, 0.1491),
        (0.1511, 0.1505),
        (0.1597, 0.1517),
        (0.1683, 0.1527),
        (0.1768, 0.1536),
        (0.1853, 0.1542),
        (0.1938, 0.1547),
        (0.2021, 0.1550),
        (0.2104, 0.1551),
        (0.2186, 0.1550),
        (0.2268, 0.1548),
        (0.2348, 0.1545),
        (0.2427, 0.1540),
        (0.2504, 0.1533),
        (0.2581, 0.1526),
        (0.2656, 0.1517),
        (0.2730, 0.1507),
        (0.2802, 0.1496),
        (0.2872, 0.1484),
        (0.2941, 0.1471),
        (0.3009, 0.1457),
        (0.3075, 0.1443),
        (0.3141, 0.1428),
        (0.3206, 0.1413),
        (0.3271, 0.1397),
        (0.3334, 0.1380),
        (0.3396, 0.1362),
        (0.3457, 0.1343),
        (0.3517, 0.1323),
        (0.3576, 0.1300),
        (0.3634, 0.1277),
        (0.3691, 0.1251),
        (0.3746, 0.1223),
        (0.3800, 0.1193),
        (0.3853, 0.1161),
        (0.3905, 0.1126),
        (0.3955, 0.1089),
        (0.4003, 0.1049),
        (0.4050, 0.1006),
        (0.4096, 0.0960),
        (0.4140, 0.0911),
        (0.4182, 0.0858),
        (0.4223, 0.0802),
        (0.4261, 0.0742),
        (0.4299, 0.0679),
        (0.4334, 0.0609),
        (0.4367, 0.0530),
        (0.4398, 0.0443),
        (0.4427, 0.0350),
        (0.4454, 0.0250),
        (0.4480, 0.0145),
        (0.4504, 0.0035),
        (0.4526, -0.0079),
        (0.4547, -0.0195),
        (0.4566, -0.0314),
        (0.4585, -0.0434),
        (0.4602, -0.0555),
        (0.4617, -0.0675),
        (0.4632, -0.0795),
        (0.4646, -0.0912),
        (0.4659, -0.1027),
        (0.4671, -0.1139),
        (0.4683, -0.1246),
        (0.4694, -0.1348),
        (0.4704, -0.1444),
        (0.4714, -0.1534),
        (0.4723, -0.1616),
        (0.4733, -0.1690),
        (0.4751, -0.1810),
        (0.4741, -0.1940),
        (0.4725, -0.2108),
        (0.4706, -0.2309),
        (0.4683, -0.2535),
        (0.4659, -0.2782),
        (0.4633, -0.3043),
        (0.4608, -0.3314),
        (0.4583, -0.3587),
        (0.4561, -0.3857),
        (0.4542, -0.4118),
        (0.4526, -0.4365),
        (0.4516, -0.4592),
        (0.4511, -0.4792),
        (0.4514, -0.4960),
        (0.4525, -0.5090),
        (0.4545, -0.5192),
        (0.4573, -0.5278),
        (0.4610, -0.5349),
        (0.4653, -0.5405),
        (0.4701, -0.5446),
        (0.4753, -0.5473),
        (0.4808, -0.5485),
        (0.4864, -0.5484),
        (0.4921, -0.5468),
        (0.4977, -0.5438),
        (0.5031, -0.5395),
        (0.5082, -0.5338),
        (0.5129, -0.5269),
        (0.5170, -0.5186),
        (0.5204, -0.5090),
        (0.5232, -0.4969),
        (0.5258, -0.4813),
        (0.5281, -0.4626),
        (0.5302, -0.4415),
        (0.5321, -0.4185),
        (0.5337, -0.3942),
        (0.5352, -0.3690),
        (0.5365, -0.3436),
        (0.5377, -0.3185),
        (0.5388, -0.2941),
        (0.5397, -0.2711),
        (0.5406, -0.2500),
        (0.5414, -0.2314),
        (0.5430, -0.2036),
        (0.5442, -0.2171),
        (0.5455, -0.2345),
        (0.5470, -0.2552),
        (0.5487, -0.2786),
        (0.5505, -0.3042),
        (0.5525, -0.3312),
        (0.5546, -0.3592),
        (0.5568, -0.3874),
        (0.5593, -0.4154),
        (0.5618, -0.4424),
        (0.5645, -0.4680),
        (0.5674, -0.4914),
        (0.5704, -0.5121),
        (0.5736, -0.5295),
        (0.5769, -0.5430),
        (0.5805, -0.5536),
        (0.5846, -0.5629),
        (0.5890, -0.5708),
        (0.5938, -0.5773),
        (0.5987, -0.5824),
        (0.6038, -0.5859),
        (0.6090, -0.5878),
        (0.6141, -0.5882),
        (0.6193, -0.5870),
        (0.6243, -0.5840),
        (0.6290, -0.5794),
        (0.6336, -0.5730),
        (0.6377, -0.5648),
        (0.6415, -0.5548),
        (0.6448, -0.5430),
        (0.6477, -0.5277),
        (0.6502, -0.5080),
        (0.6526, -0.4845),
        (0.6546, -0.4580),
        (0.6565, -0.4290),
        (0.6582, -0.3984),
        (0.6597, -0.3667),
        (0.6610, -0.3347),
        (0.6622, -0.3030),
        (0.6632, -0.2723),
        (0.6642, -0.2434),
        (0.6651, -0.2168),
        (0.6659, -0.1934),
        (0.6674, -0.1584),
        (0.6707, -0.1574),
        (0.6747, -0.1562),
        (0.6792, -0.1547),
        (0.6842, -0.1531),
        (0.6896, -0.1512),
        (0.6955, -0.1493),
        (0.7017, -0.1473),
        (0.7082, -0.1454),
        (0.7149, -0.1434),
        (0.7219, -0.1416),
        (0.7290, -0.1399),
        (0.7362, -0.1385),
        (0.7434, -0.1372),
        (0.7507, -0.1363),
        (0.7579, -0.1357),
        (0.7653, -0.1356),
        (0.7731, -0.1360),
        (0.7813, -0.1367),
        (0.7898, -0.1378),
        (0.7985, -0.1391),
        (0.8074, -0.1405),
        (0.8163, -0.1421),
        (0.8253, -0.1436),
        (0.8342, -0.1450),
        (0.8430, -0.1462),
        (0.8515, -0.1472),
        (0.8598, -0.1479),
        (0.8677, -0.1481),
        (0.8753, -0.1479),
        (0.8823, -0.1471),
        (0.8892, -0.1458),
        (0.8960, -0.1445),
        (0.9029, -0.1429),
        (0.9096, -0.1411),
        (0.9163, -0.1391),
        (0.9227, -0.1368),
        (0.9289, -0.1343),
        (0.9347, -0.1315),
        (0.9402, -0.1283),
        (0.9452, -0.1249),
        (0.9497, -0.1210),
        (0.9536, -0.1168),
        (0.9570, -0.1122),
        (0.9596, -0.1072),
        (0.9615, -0.1018),
        (0.9627, -0.0956),
        (0.9631, -0.0885),
        (0.9629, -0.0805),
        (0.9621, -0.0719),
        (0.9607, -0.0628),
        (0.9588, -0.0534),
        (0.9565, -0.0437),
        (0.9537, -0.0340),
        (0.9507, -0.0244),
        (0.9473, -0.0151),
        (0.9436, -0.0061),
        (0.9398, 0.0023),
        (0.9358, 0.0100),
        (0.9317, 0.0168),
        (0.9276, 0.0226),
        (0.9231, 0.0275),
        (0.9182, 0.0317),
        (0.9127, 0.0352),
        (0.9069, 0.0382),
        (0.9008, 0.0406),
        (0.8944, 0.0427),
        (0.8878, 0.0445),
        (0.8811, 0.0460),
        (0.8744, 0.0473),
        (0.8677, 0.0486),
        (0.8611, 0.0499),
        (0.8546, 0.0512),
        (0.8485, 0.0527),
        (0.8426, 0.0545),
        (0.8371, 0.0566),
        (0.8318, 0.0589),
        (0.8264, 0.0613),
        (0.8210, 0.0639),
        (0.8156, 0.0665),
        (0.8103, 0.0691),
        (0.8050, 0.0718),
        (0.8000, 0.0744),
        (0.7951, 0.0769),
        (0.7904, 0.0794),
        (0.7860, 0.0817),
        (0.7819, 0.0839),
        (0.7781, 0.0859),
        (0.7747, 0.0877),
        (0.7692, 0.0905),
        (0.7636, 0.0961),
        (0.7556, 0.1042),
        (0.7461, 0.1137),
        (0.7359, 0.1238),
        (0.7250, 0.1358),
        (0.7131, 0.1497),
        (0.7004, 0.1625),
        (0.6874, 0.1714),
        (0.6737, 0.1748),
        (0.6593, 0.1745),
        (0.6449, 0.1727),
        (0.6311, 0.1716),
        (0.6176, 0.1705),
        (0.6040, 0.1685),
        (0.5917, 0.1681),
        (0.5818, 0.1716),
        (0.5753, 0.1805),
        (0.5713, 0.1930),
        (0.5683, 0.2074),
        (0.5648, 0.2216),
        (0.5620, 0.2375),
        (0.5603, 0.2556),
        (0.5566, 0.2708),
        (0.5479, 0.2786),
        (0.5319, 0.2757),
        (0.5107, 0.2653),
        (0.4879, 0.2521),
        (0.4671, 0.2409),
        (0.4477, 0.2301),
        (0.4283, 0.2181),
        (0.4112, 0.2097),
        (0.3984, 0.2094),
        (0.3917, 0.2202),
        (0.3896, 0.2391),
        (0.3892, 0.2616),
        (0.3875, 0.2836),
        (0.3855, 0.3086),
        (0.3842, 0.3374),
        (0.3808, 0.3611),
        (0.3722, 0.3710),
        (0.3563, 0.3612),
        (0.3350, 0.3376),
        (0.3121, 0.3090),
        (0.2913, 0.2842),
        (0.2726, 0.2610),
        (0.2544, 0.2362),
        (0.2379, 0.2172),
        (0.2242, 0.2111),
        (0.2147, 0.2228),
        (0.2083, 0.2475),
        (0.2031, 0.2781),
        (0.1969, 0.3075),
        (0.1902, 0.3404),
        (0.1837, 0.3786),
        (0.1760, 0.4099),
        (0.1654, 0.4225),
        (0.1506, 0.4084),
        (0.1328, 0.3755),
        (0.1142, 0.3359),
        (0.0972, 0.3015),
        (0.0819, 0.2701),
        (0.0672, 0.2369),
        (0.0536, 0.2110),
        (0.0411, 0.2010),
        (0.0306, 0.2129),
        (0.0216, 0.2408),
        (0.0129, 0.2759),
        (0.0033, 0.3095),
        (-0.0076, 0.3468),
        (-0.0192, 0.3901),
        (-0.0309, 0.4256),
        (-0.0424, 0.4398),
        (-0.0537, 0.4236),
        (-0.0652, 0.3860),
        (-0.0760, 0.3408),
        (-0.0857, 0.3016),
        (-0.0932, 0.2661),
        (-0.0991, 0.2289),
        (-0.1053, 0.1994),
        (-0.1137, 0.1866),
        (-0.1247, 0.1969),
        (-0.1373, 0.2240),
        (-0.1514, 0.2586),
        (-0.1670, 0.2911),
        (-0.1861, 0.3264),
        (-0.2082, 0.3672),
        (-0.2288, 0.4004),
        (-0.2436, 0.4130),
        (-0.2504, 0.3963),
        (-0.2517, 0.3589),
        (-0.2508, 0.3142),
        (-0.2506, 0.2753),
        (-0.2498, 0.2406),
        (-0.2474, 0.2047),
        (-0.2474, 0.1756),
        (-0.2534, 0.1613),
        (-0.2676, 0.1674),
        (-0.2877, 0.1885),
        (-0.3106, 0.2161),
        (-0.3335, 0.2421),
        (-0.3590, 0.2702),
        (-0.3873, 0.3031),
        (-0.4127, 0.3298),
        (-0.4293, 0.3396),
        (-0.4336, 0.3254),
        (-0.4294, 0.2942),
        (-0.4217, 0.2570),
        (-0.4155, 0.2249),
        (-0.4095, 0.1965),
        (-0.4018, 0.1676),
        (-0.3970, 0.1439),
        (-0.3995, 0.1313),
        (-0.4120, 0.1340),
        (-0.4316, 0.1481),
        (-0.4547, 0.1670),
        (-0.4777, 0.1842),
        (-0.5033, 0.2024),
        (-0.5320, 0.2234),
        (-0.5578, 0.2402),
        (-0.5743, 0.2456),
        (-0.5777, 0.2345),
        (-0.5721, 0.2120),
        (-0.5630, 0.1853),
        (-0.5558, 0.1621),
        (-0.5496, 0.1417),
        (-0.5423, 0.1211),
        (-0.5376, 0.1036),
        (-0.5396, 0.0926),
        (-0.5506, 0.0905),
        (-0.5680, 0.0950),
        (-0.5886, 0.1022),
        (-0.6090, 0.1081),
        (-0.6311, 0.1137),
        (-0.6558, 0.1205),
        (-0.6782, 0.1250),
        (-0.6934, 0.1238),
        (-0.6986, 0.1146),
        (-0.6968, 0.0997),
        (-0.6924, 0.0826),
        (-0.6897, 0.0671),
        (-0.6883, 0.0529),
        (-0.6865, 0.0385),
        (-0.6864, 0.0255),
        (-0.6900, 0.0152),
        (-0.6987, 0.0089),
        (-0.7111, 0.0054),
        (-0.7252, 0.0031),
        (-0.7390, 0.0007),
        (-0.7521, -0.0005),
        (-0.7657, -0.0005),
        (-0.7794, -0.0022),
        (-0.7928, -0.0089),
        (-0.8061, -0.0230),
        (-0.8195, -0.0422),
        (-0.8324, -0.0628),
        (-0.8441, -0.0809),
        (-0.8552, -0.0967),
        (-0.8657, -0.1118),
        (-0.8823, -0.1357),
    ];
    raw_pts.iter().map(|&(x, y)| Vec2::new(x * 0.90, y * 0.90)).collect()
}

pub fn generate_unicorn() -> Vec<Vec2> {
    let raw_pts = [
        // Muzzle / Nose
        (0.48, 0.18), (0.50, 0.12), (0.47, 0.08), (0.42, 0.09),
        // Under jaw / Chin
        (0.36, 0.12), (0.33, 0.15),
        // Eye loop (curving up inside the head)
        (0.34, 0.22), (0.38, 0.24), (0.39, 0.21), (0.36, 0.20), (0.34, 0.22),
        // Forehead / Head top
        (0.37, 0.27),
        // Horn (extending up-right from forehead)
        (0.48, 0.42), (0.40, 0.31),
        // Ear
        (0.39, 0.36), (0.41, 0.46), (0.36, 0.33),
        // Mane Lock 1 (flowing back)
        (0.31, 0.34), (0.16, 0.36), (0.28, 0.31),
        // Mane Lock 2
        (0.20, 0.30), (0.05, 0.28), (0.18, 0.24),
        // Mane Lock 3
        (0.10, 0.22), (-0.05, 0.20), (0.08, 0.17),
        // Back line / Spine
        (-0.05, 0.16), (-0.18, 0.13), (-0.26, 0.07),
        // Tail (flowing loops)
        (-0.33, 0.05), (-0.30, 0.08), (-0.36, 0.18), (-0.42, 0.08), // Tail Loop 1
        (-0.35, -0.02), (-0.45, -0.06), (-0.48, -0.15), (-0.39, -0.10), // Tail Loop 2
        (-0.32, -0.04), (-0.28, -0.05), // Tail base
        // Butt / Rump
        (-0.25, 0.01),
        // Back Leg 1 (outer hind leg, reaching forward/down)
        (-0.20, -0.08), (-0.15, -0.22), (-0.17, -0.38), (-0.24, -0.52), // Hock to ankle
        (-0.16, -0.53), (-0.18, -0.49), // Hoof
        (-0.12, -0.30), (-0.08, -0.14), // Back up thigh/flank
        // Belly
        (0.00, -0.12), (0.10, -0.11),
        // Front Leg 1 (outer front leg, raised/bent)
        (0.16, -0.13), (0.22, -0.25), (0.16, -0.35), (0.19, -0.38), (0.24, -0.32), (0.26, -0.18), // Bent leg/hoof loop
        // Chest
        (0.28, -0.05), (0.31, 0.05), (0.33, 0.11),
        // Neck front
        (0.39, 0.15),
        // Return to muzzle start
        (0.48, 0.18)
    ];
    raw_pts.iter().map(|&(x, y)| Vec2::new(x * 0.90, y * 0.90)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_spiral() {
        let path = generate_spiral(0.05);
        assert!(!path.is_empty());
        for pos in &path {
            assert!(pos.length() <= 0.9201);
        }
        assert!(path[0].length() < 1e-4);
    }

    #[test]
    fn test_parse_thr() {
        let content = "
        # This is a sample .thr file
        0.00000 0.00000
        1.57079 0.50000
        3.14159 1.00000
        ";
        let parsed = parse_thr(content).unwrap();
        assert_eq!(parsed.len(), 3);

        assert!(parsed[0].length() < 1e-4);
        // Checking polar conversions (scaled to 0.874 instead of 0.92)
        let expected1 = Vec2::new(0.437 * 1.57079f32.cos(), 0.437 * 1.57079f32.sin());
        let expected2 = Vec2::new(0.874 * 3.14159f32.cos(), 0.874 * 3.14159f32.sin());
        assert!((parsed[1] - expected1).length() < 1e-4);
        assert!((parsed[2] - expected2).length() < 1e-4);
    }

    #[test]
    fn test_parse_gcode_spaceless_and_comments() {
        let content = "
        G1X10Y20 (Comment1) (Comment2) ; end of line comment
        (Comment3) X30Y20
        X30Y40
        ";
        let parsed = parse_gcode(content).unwrap();
        assert_eq!(parsed.len(), 3);

        // Bounding box of raw points is: X: [10, 30] (center 20, max offset 10), Y: [20, 40] (center 30, max offset 10)
        // Max radius is sqrt(10^2 + 10^2) = 14.142
        // Scale factor = 0.874 / 14.142 = 0.061798
        let p0 = parsed[0];
        assert!((p0.x + 0.61798).abs() < 1e-3);
        assert!((p0.y + 0.61798).abs() < 1e-3);
    }

    #[test]
    fn test_playback_controller() {
        let mut controller = PlaybackController::new();
        // Use coordinates starting away from 0.0 to make movement math clean
        controller.waypoints[0] = vec![Vec2::new(0.1, 0.0), Vec2::new(0.5, 0.0)];
        controller.state = PlaybackState::Playing;

        // Test default multiplier is 1.0
        assert_eq!(controller.speed_multipliers[0], 1.0);

        // Case 1: Low speed movement. Move towards first waypoint.
        // cur_positions = (0.0, 0.0), speed = 0.5, dt = 0.1 => remaining_move = 0.05
        // Waypoint 0 is at 0.1, which is > 0.05.
        // It should move to (0.05, 0.0) and idx should remain 0.
        let cur_positions = [Vec2::new(0.0, 0.0); 5];
        let targets = controller.step_playback_all(&cur_positions, 1, 0.5, 0.1);
        let pos = targets[0].unwrap();
        assert!((pos.x - 0.05).abs() < 1e-4);
        assert_eq!(pos.y, 0.0);
        assert_eq!(controller.current_indices[0], 0);

        // Case 2: Multi-waypoint movement.
        // cur_positions = (0.0, 0.0), speed = 2.0, dt = 0.1 => remaining_move = 0.2
        // Waypoint 0 is at 0.1 (distance 0.1). Consumed. remaining_move = 0.1. idx becomes 1.
        // Waypoint 1 is at 0.5 (distance 0.4). Move towards it by 0.1 => final pos (0.2, 0.0). idx remains 1.
        let targets = controller.step_playback_all(&cur_positions, 1, 2.0, 0.1);
        let pos = targets[0].unwrap();
        assert!((pos.x - 0.2).abs() < 1e-4);
        assert_eq!(pos.y, 0.0);
        assert_eq!(controller.current_indices[0], 1);

        // Case 3: Fast movement consuming multiple waypoints and wrapping.
        // cur_positions = (0.0, 0.0), speed = 10.0, dt = 0.1 => remaining_move = 1.0
        // Waypoint 0 is at 0.1 (distance 0.1). Consumed. remaining_move = 0.9. idx becomes 1.
        // Waypoint 1 is at 0.5 (distance 0.4). Consumed. remaining_move = 0.5. idx becomes 2.
        // Since loop_pattern is true, wraps to 0.
        // Waypoint 0 is at 0.1 (distance 0.4). Consumed. remaining_move = 0.1. idx becomes 1.
        // Waypoint 1 is at 0.5 (distance 0.4). Move towards it by 0.1 => final pos (0.2, 0.0). idx remains 1.
        let targets = controller.step_playback_all(&cur_positions, 1, 10.0, 0.1);
        let pos = targets[0].unwrap();
        assert!((pos.x - 0.2).abs() < 1e-4);
        assert_eq!(pos.y, 0.0);
        assert_eq!(controller.current_indices[0], 1);

        // Test randomize_speeds
        controller.randomize_speeds(3, 99999);
        for j in 0..3 {
            assert!(controller.speed_multipliers[j] >= 0.899);
            assert!(controller.speed_multipliers[j] <= 1.101);
        }
        assert_eq!(controller.speed_multipliers[3], 1.0);
        assert_eq!(controller.speed_multipliers[4], 1.0);

        // Test multiplier scale on step size
        controller.current_indices[0] = 0;
        controller.speed_multipliers[0] = 0.5;
        // speed = 1.0, dt = 0.1, speed_multiplier = 0.5 => remaining_move = 0.05.
        // Moves from (0.0, 0.0) to (0.05, 0.0).
        let targets = controller.step_playback_all(&cur_positions, 1, 1.0, 0.1);
        let pos = targets[0].unwrap();
        assert!((pos.x - 0.05).abs() < 1e-4);
    }

    #[test]
    fn test_parse_gcode_spaced() {
        let content = "
        G1 X 10.5 Y -20.2
        X -30.0 Y 40.0
        ";
        let parsed = parse_gcode(content).unwrap();
        assert_eq!(parsed.len(), 2);
    }

    #[test]
    fn test_parse_thr_inline_comments() {
        let content = "
        0.00000 0.00000#nospacecomment
        1.57079 0.50000 # spacecomment
        ";
        let parsed = parse_thr(content).unwrap();
        assert_eq!(parsed.len(), 2);
    }

    #[test]
    fn test_parse_gcode_relative_mode() {
        let content = "
        G90 (Absolute)
        G1 X10 Y20
        G91 (Relative)
        G1 X5 Y-5
        ";
        let parsed = parse_gcode(content).unwrap();
        assert_eq!(parsed.len(), 2);
        assert!((parsed[1].x - parsed[0].x).signum() == (parsed[0].y - parsed[1].y).signum());
    }

    #[test]
    fn test_parametric_curves_boundaries() {
        // Lissajous
        let liss = generate_lissajous(3.0, 4.0, 1.5708);
        assert!(!liss.is_empty());
        for p in &liss {
            assert!(p.length() <= 0.92, "Lissajous point {:?} out of bounds", p);
        }

        // Rose
        let rose = generate_rose(5.0);
        assert!(!rose.is_empty());
        for p in &rose {
            assert!(p.length() <= 0.92, "Rose point {:?} out of bounds", p);
        }

        // Hypotrochoid
        let hypo = generate_hypotrochoid(0.28, 0.20);
        assert!(!hypo.is_empty());
        for p in &hypo {
            assert!(p.length() <= 0.92, "Hypotrochoid point {:?} out of bounds", p);
        }

        // Fermat
        let fermat = generate_fermat_spiral(8.0);
        assert!(!fermat.is_empty());
        for p in &fermat {
            assert!(p.length() <= 0.92, "Fermat point {:?} out of bounds", p);
        }

        // Hilbert
        let hilbert = generate_hilbert_curve(4);
        assert!(!hilbert.is_empty());
        for p in &hilbert {
            assert!(p.x.abs() <= 0.875 && p.y.abs() <= 0.875, "Hilbert point {:?} out of bounds", p);
        }

        // Gosper
        let gosper = generate_gosper_curve(3);
        assert!(!gosper.is_empty());
        for p in &gosper {
            assert!(p.length() <= 0.92, "Gosper point {:?} out of bounds", p);
        }

        // Sierpinski
        let sierp = generate_sierpinski_curve(4);
        assert!(!sierp.is_empty());
        for p in &sierp {
            assert!(p.length() <= 0.92, "Sierpinski point {:?} out of bounds", p);
        }

        // Lemniscate
        let lemn = generate_lemniscate(0.8);
        assert!(!lemn.is_empty());
        for p in &lemn {
            assert!(p.length() <= 0.92, "Lemniscate point {:?} out of bounds", p);
        }

        // Multi-spiral
        let multi = multi_spiral_helper(0.03, 3);
        assert_eq!(multi.len(), 3);
        for arm in &multi {
            assert!(!arm.is_empty());
            for p in arm {
                assert!(p.length() <= 0.92, "Multi-spiral point {:?} out of bounds", p);
            }
        }

        // Butterfly Curve
        let butterfly = generate_butterfly_curve();
        assert!(!butterfly.is_empty());
        for p in &butterfly {
            assert!(p.length() <= 0.92, "Butterfly point {:?} out of bounds", p);
        }

        // Zen Waves
        let zen_waves = generate_zen_waves();
        assert!(!zen_waves.is_empty());
        for p in &zen_waves {
            assert!(p.length() <= 0.92, "Zen Waves point {:?} out of bounds", p);
        }

        // Zen Mandala
        let zen_mandala = generate_zen_mandala();
        assert!(!zen_mandala.is_empty());
        for p in &zen_mandala {
            assert!(p.length() <= 0.92, "Zen Mandala point {:?} out of bounds", p);
        }

        // Clock Mode (Draw Phase)
        let clock_draw = generate_clock_pattern(10, 10, 10.0, 1);
        assert!(!clock_draw.is_empty());
        for p in &clock_draw {
            assert!(p.length() <= 0.92, "Clock Draw point {:?} out of bounds", p);
        }

        // Clock Mode (Clear Phase)
        let clock_clear = generate_clock_pattern(10, 10, 50.0, 2);
        assert!(!clock_clear.is_empty());
        for p in &clock_clear {
            assert!(p.length() <= 0.92, "Clock Clear point {:?} out of bounds", p);
        }

        // Dinosaur
        let dinosaur = generate_dinosaur();
        assert!(!dinosaur.is_empty());
        for p in &dinosaur {
            assert!(p.length() <= 0.92, "Dinosaur point {:?} out of bounds", p);
        }

        // Unicorn
        let unicorn = generate_unicorn();
        assert!(!unicorn.is_empty());
        for p in &unicorn {
            assert!(p.length() <= 0.92, "Unicorn point {:?} out of bounds", p);
        }
    }
}

fn multi_spiral_helper(spacing: f32, count: usize) -> Vec<Vec<Vec2>> {
    generate_multi_spiral(spacing, count)
}
