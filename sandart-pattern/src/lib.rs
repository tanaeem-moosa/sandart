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
        // Head & Snout
        (0.3, 0.6), (0.33, 0.65), (0.4, 0.67), (0.45, 0.65), (0.46, 0.6), 
        (0.43, 0.57), (0.38, 0.55), (0.35, 0.5),
        // Long Neck going down with neck spikes
        (0.32, 0.4), (0.35, 0.38), (0.28, 0.28), (0.31, 0.26), (0.22, 0.15), (0.25, 0.13), (0.15, 0.05),
        // Back curve with spikes
        (0.05, -0.02), (0.07, 0.06), (-0.05, 0.02),
        (-0.1, -0.01), (-0.08, 0.09), (-0.18, 0.04),
        (-0.25, -0.03), (-0.23, 0.08), (-0.33, 0.01),
        (-0.4, -0.08), (-0.38, 0.04), (-0.48, -0.04),
        (-0.55, -0.15), (-0.53, -0.03), (-0.63, -0.11),
        (-0.68, -0.25), (-0.66, -0.13), (-0.74, -0.22),
        // Tail with double spikes (Thagomizer)
        (-0.8, -0.4), 
        (-0.88, -0.35), (-0.84, -0.44),  // Spike 1
        (-0.91, -0.46), (-0.83, -0.51),  // Spike 2
        (-0.8, -0.53), (-0.7, -0.48), (-0.58, -0.4), (-0.48, -0.32),
        // Back leg 1
        (-0.45, -0.45), (-0.45, -0.7), (-0.38, -0.7), (-0.36, -0.45),
        // Belly
        (-0.25, -0.42), (-0.1, -0.41),
        // Back leg 2 (slightly offset)
        (-0.12, -0.45), (-0.12, -0.68), (-0.05, -0.68), (-0.03, -0.42),
        // Belly front
        (0.1, -0.42),
        // Front leg 1
        (0.15, -0.45), (0.15, -0.7), (0.22, -0.7), (0.24, -0.4),
        // Chest
        (0.28, -0.25), (0.3, -0.1), (0.35, 0.1), (0.38, 0.28), (0.36, 0.42), (0.32, 0.52),
    ];
    raw_pts.iter().map(|&(x, y)| Vec2::new(x * 0.90, y * 0.90)).collect()
}

pub fn generate_unicorn() -> Vec<Vec2> {
    let raw_pts = [
        (0.1, -0.7),      // Neck bottom start
        (-0.1, -0.5), (-0.2, -0.3), (-0.25, -0.1), // Chest
        (-0.3, 0.0), (-0.4, 0.05), (-0.45, 0.12),  // Muzzle
        (-0.43, 0.18), (-0.38, 0.2), (-0.4, 0.22), // Nostril loop
        (-0.25, 0.3), (-0.15, 0.38), (-0.1, 0.42), // Nose bridge
        // Horn (long and pointed with spiral texture)
        (0.2, 0.8),       // Horn tip
        (0.1, 0.68), (0.15, 0.69),   // first rib
        (0.02, 0.56), (0.07, 0.57),   // second rib
        (-0.06, 0.44), (-0.01, 0.45), // third rib
        (-0.1, 0.4),      // Horn base
        (-0.05, 0.45),    // Forehead
        // Ear
        (0.0, 0.6), (0.05, 0.7), (-0.02, 0.5),
        (0.05, 0.4),      // Back of head
        // Mane waves
        (0.2, 0.45), (0.12, 0.35),
        (0.25, 0.3), (0.15, 0.2),
        (0.28, 0.1), (0.18, 0.0),
        (0.29, -0.15), (0.19, -0.25),
        (0.30, -0.4), (0.2, -0.5),
        (0.15, -0.6),     // Neck back
        (0.1, -0.7),       // Close loop
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
