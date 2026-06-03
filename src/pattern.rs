#![allow(dead_code)]

use glam::Vec2;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Waypoint {
    Cartesian(Vec2),
    Polar { theta: f32, r: f32 },
}

impl Waypoint {
    pub fn to_cartesian(self) -> Vec2 {
        match self {
            Waypoint::Cartesian(v) => v,
            Waypoint::Polar { theta, r } => Vec2::new(r * theta.cos(), r * theta.sin()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackState {
    Playing,
    Paused,
    Stopped,
}

pub struct PlaybackController {
    pub waypoints: Vec<Waypoint>,
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

        let target = self.waypoints[self.current_idx].to_cartesian();
        let to_target = target - current_pos;
        let dist = to_target.length();
        let max_move = speed * dt;

        if dist <= max_move {
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
            Some(current_pos + (to_target / dist) * max_move)
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
