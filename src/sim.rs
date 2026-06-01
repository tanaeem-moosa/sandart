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
}

impl Simulation {
    pub fn new() -> Self {
        Self {
            // Initializing a 512x512 grid to a default flat sand level of 0.8
            heightmap: Heightmap::new(512, 512, 0.8),
            marble_pos: Vec2::ZERO,
            prev_marble_pos: Vec2::ZERO,
        }
    }

    /// Reset the simulation state.
    pub fn reset(&mut self) {
        self.heightmap.reset(0.8);
        self.marble_pos = Vec2::ZERO;
        self.prev_marble_pos = Vec2::ZERO;
    }

    /// Run a physics frame tick.
    pub fn update(&mut self, _dt: f32, target_pos: Option<Vec2>) {
        if let Some(target) = target_pos {
            self.prev_marble_pos = self.marble_pos;
            // For now, in Block 3, we jump directly to target coordinate.
            // Physics and interpolation will be added in Blocks 4 and 5.
            self.marble_pos = target.clamp(Vec2::splat(-1.0), Vec2::splat(1.0));
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
}
