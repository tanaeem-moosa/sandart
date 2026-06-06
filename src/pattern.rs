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

            let idx = self.current_indices[j];
            if idx >= wps.len() {
                continue;
            }

            all_stopped = false;
            let target = wps[idx];
            let to_target = target - current_positions[j];
            let dist = to_target.length();
            let max_move = speed * self.speed_multipliers[j] * dt;

            // Safe threshold guard to prevent Nan/Inf division when distance is subnormal
            if dist <= max_move || dist < 1e-5 {
                self.current_indices[j] += 1;
                if self.current_indices[j] >= wps.len() {
                    if self.loop_pattern {
                        self.current_indices[j] = 0;
                    }
                }
                targets[j] = Some(target);
            } else {
                targets[j] = Some(current_positions[j] + to_target * (max_move / dist));
            }
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

/// Generate a concentric ripple pattern on a heightmap.
pub fn generate_ripples(heightmap: &mut crate::sim::Heightmap) {
    let w = heightmap.width;
    let h = heightmap.height;
    let cx = w as f32 / 2.0;
    let cy = h as f32 / 2.0;
    for y in 0..h {
        for x in 0..w {
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            let dist = (dx * dx + dy * dy).sqrt();
            let val = (dist * 0.1).sin() * 0.3 + 0.5;
            heightmap.set(x, y, val);
        }
    }
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
    let mut has_x = false;
    let mut has_y = false;
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
        controller.waypoints[0] = vec![Vec2::new(0.0, 0.0), Vec2::new(0.5, 0.0)];
        controller.state = PlaybackState::Playing;

        // Test default multiplier is 1.0
        assert_eq!(controller.speed_multipliers[0], 1.0);

        let cur_positions = [Vec2::new(0.0, 0.0); 5];
        let targets = controller.step_playback_all(&cur_positions, 1, 0.1, 0.1);
        let pos = targets[0].unwrap();
        assert_eq!(pos, Vec2::new(0.0, 0.0));
        assert_eq!(controller.current_indices[0], 1);

        let cur_positions = [Vec2::new(0.0, 0.0); 5];
        let targets = controller.step_playback_all(&cur_positions, 1, 0.5, 0.1);
        let pos = targets[0].unwrap();
        assert!((pos.x - 0.05).abs() < 1e-4);
        assert_eq!(pos.y, 0.0);
        assert_eq!(controller.current_indices[0], 1);

        let targets = controller.step_playback_all(&cur_positions, 1, 10.0, 0.1);
        let pos = targets[0].unwrap();
        assert_eq!(pos, Vec2::new(0.5, 0.0));
        assert_eq!(controller.current_indices[0], 0);

        // Test randomize_speeds
        controller.randomize_speeds(3, 99999);
        for j in 0..3 {
            assert!(controller.speed_multipliers[j] >= 0.899);
            assert!(controller.speed_multipliers[j] <= 1.101);
        }
        assert_eq!(controller.speed_multipliers[3], 1.0);
        assert_eq!(controller.speed_multipliers[4], 1.0);

        // Test multiplier scale on step size
        controller.current_indices[0] = 1;
        controller.speed_multipliers[0] = 0.5;
        let targets = controller.step_playback_all(&cur_positions, 1, 1.0, 0.1); // base speed 1.0, dt 0.1, max_move should be 1.0 * 0.5 * 0.1 = 0.05
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

        // Lemniscate
        let lemn = generate_lemniscate(0.8);
        assert!(!lemn.is_empty());
        for p in &lemn {
            assert!(p.length() <= 0.92, "Lemniscate point {:?} out of bounds", p);
        }

        // Multi-spiral
        let multi = generate_multi_spiral(0.03, 3);
        assert_eq!(multi.len(), 3);
        for arm in &multi {
            assert!(!arm.is_empty());
            for p in arm {
                assert!(p.length() <= 0.92, "Multi-spiral point {:?} out of bounds", p);
            }
        }
    }
}
