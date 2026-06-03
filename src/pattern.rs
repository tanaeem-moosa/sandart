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
    pub waypoints: Vec<Vec2>,
    pub current_idx: usize,
    pub state: PlaybackState,
    pub loop_pattern: bool,
    pub accumulated_theta: f32,
}

impl PlaybackController {
    pub fn new() -> Self {
        Self {
            waypoints: Vec::new(),
            current_idx: 0,
            state: PlaybackState::Stopped,
            loop_pattern: true,
            accumulated_theta: 0.0,
        }
    }

    /// Advance playback and return the target marble position.
    pub fn step_playback(&mut self, current_pos: Vec2, speed: f32, dt: f32) -> Option<Vec2> {
        if self.state != PlaybackState::Playing || self.waypoints.is_empty() {
            return None;
        }

        let target = self.waypoints[self.current_idx];
        let to_target = target - current_pos;
        let dist = to_target.length();
        let max_move = speed * dt;

        // Safe threshold guard to prevent Nan/Inf division when distance is subnormal
        if dist <= max_move || dist < 1e-5 {
            self.current_idx += 1;
            if self.current_idx >= self.waypoints.len() {
                if self.loop_pattern {
                    self.current_idx = 0;
                } else {
                    self.state = PlaybackState::Stopped;
                }
            }
            Some(target)
        } else {
            Some(current_pos + to_target * (max_move / dist))
        }
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
        let remainder = &line[pos + 1..];
        let num_str: String = remainder.chars()
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
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 2 {
            return Err(format!("Malformed polar waypoint on line {}: '{}'", line_idx + 1, line));
        }
        let theta = parts[0].parse::<f32>().map_err(|e| {
            format!("Invalid theta '{}' on line {}: {}", parts[0], line_idx + 1, e)
        })?;
        let rho = parts[1].parse::<f32>().map_err(|e| {
            format!("Invalid rho '{}' on line {}: {}", parts[1], line_idx + 1, e)
        })?;
        
        // Scale rho to our visual sand bed boundary (0.92)
        let r = (rho * 0.92).clamp(-0.92, 0.92);
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

    for line in content.lines() {
        let clean_line = strip_comments(line);
        if clean_line.is_empty() {
            continue;
        }

        let x_val = parse_coordinate(&clean_line, 'X');
        let y_val = parse_coordinate(&clean_line, 'Y');

        if x_val.is_some() || y_val.is_some() {
            if let Some(x) = x_val {
                last_x = x;
                has_x = true;
            }
            if let Some(y) = y_val {
                last_y = y;
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

        let scale = if max_r > 1e-4 { 0.92 / max_r } else { 1.0 };

        for p in raw_points {
            let px = (p.x - cx) * scale;
            let py = (p.y - cy) * scale;
            waypoints.push(Vec2::new(px, py));
        }
    } else {
        // Single flat coordinate point
        for p in raw_points {
            waypoints.push(p.clamp(Vec2::splat(-0.92), Vec2::splat(0.92)));
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
        // Checking polar conversions
        let expected1 = Vec2::new(0.46 * 1.57079f32.cos(), 0.46 * 1.57079f32.sin());
        let expected2 = Vec2::new(0.92 * 3.14159f32.cos(), 0.92 * 3.14159f32.sin());
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
        // Scale factor = 0.92 / 14.142 = 0.06505
        let p0 = parsed[0];
        assert!((p0.x + 0.6505).abs() < 1e-3);
        assert!((p0.y + 0.6505).abs() < 1e-3);
    }

    #[test]
    fn test_playback_controller() {
        let mut controller = PlaybackController::new();
        controller.waypoints = vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(0.5, 0.0),
        ];
        controller.state = PlaybackState::Playing;

        let pos = controller.step_playback(Vec2::new(0.0, 0.0), 0.1, 0.1).unwrap();
        assert_eq!(pos, Vec2::new(0.0, 0.0));
        assert_eq!(controller.current_idx, 1);

        let pos = controller.step_playback(Vec2::new(0.0, 0.0), 0.5, 0.1).unwrap();
        assert!((pos.x - 0.05).abs() < 1e-4);
        assert_eq!(pos.y, 0.0);
        assert_eq!(controller.current_idx, 1);

        let pos = controller.step_playback(Vec2::new(0.0, 0.0), 10.0, 0.1).unwrap();
        assert_eq!(pos, Vec2::new(0.5, 0.0));
        assert_eq!(controller.current_idx, 0);
    }
}
