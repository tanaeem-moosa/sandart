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
    #[allow(dead_code)]
    pub fn reset(&mut self, value: f32) {
        self.data.fill(value);
    }

    /// Retrieve the height at a specific grid index with boundary checking.
    #[inline]
    #[allow(dead_code)]
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

    /// Generate a concentric ripple pattern on the heightmap.
    pub fn generate_ripples(&mut self) {
        let cx = self.width as f32 / 2.0;
        let cy = self.height as f32 / 2.0;
        for y in 0..self.height {
            for x in 0..self.width {
                let dx = x as f32 - cx;
                let dy = y as f32 - cy;
                let dist = (dx * dx + dy * dy).sqrt();
                let val = (dist * 0.1).sin() * 0.3 + 0.5;
                self.set(x, y, val);
            }
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
    fn test_zero_dimension_heightmap() {
        let mut hm = Heightmap::new(0, 0, 0.8);
        assert_eq!(hm.get(0, 0), 0.0);
        hm.set(0, 0, 0.5);
        assert_eq!(hm.get(0, 0), 0.0);
    }
}
