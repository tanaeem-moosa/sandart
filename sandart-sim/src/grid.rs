/// A 2D heightmap representing the sand bed.
pub struct Heightmap {
    pub width: usize,
    pub height: usize,
    pub data: Vec<f32>,
    /// Net external mass change written into each cell *this tick* by
    /// [`Heightmap::apply_external_mass`], from a source outside the simulation's own flux solver
    /// (a waterfall / continuous-pour feature, or a test harness standing in for one).
    /// `settle_tick`'s `column_depth` estimator reads this the same way it reads
    /// `in_transit_at`'s edge-arrived estimate: mass that just arrived from outside is not yet
    /// resting and must not be counted as stacked overburden. Cleared to zero once per tick by
    /// `settle_tick` after it has consumed it (see the clear site there for why that particular
    /// point in the control flow is the correct one) — so between a caller's calls and the next
    /// `tick()`, this buffer holds exactly "what was exchanged since the last clear".
    ///
    /// Signed, not magnitude-only: positive means mass entered the cell from outside (today's only
    /// writer, `apply_external_mass`, only ever adds); negative is reserved for a future drain/sink
    /// (the planned waterfall feature needs an outflow as well as an inflow). **What a negative
    /// value should mean for `column_depth`'s `resting_above` calculation is deliberately left
    /// unresolved by this change** — the source-side justification (freshly injected liquid hasn't
    /// begun falling, so it isn't resting overburden) does not obviously mirror to removal, and
    /// nothing here should be read as having already decided that question. See the read site in
    /// `settle_tick` for the placeholder behavior chosen in the meantime.
    ///
    /// See `apply_external_mass`'s doc comment for why this records the full written height rather
    /// than only the tick-over-tick delta.
    pub external_mass_this_tick: Vec<f32>,
}

impl Heightmap {
    /// Create a new heightmap of the specified dimensions, initialized to a flat value.
    pub fn new(width: usize, height: usize, initial_value: f32) -> Self {
        Self {
            width,
            height,
            data: vec![initial_value; width * height],
            external_mass_this_tick: vec![0.0; width * height],
        }
    }

    /// Apply an external mass change to cell `(x, y)` — a waterfall / continuous-pour source
    /// outside the simulation's own flux solver, as opposed to mass that arrived via internal
    /// flux. Sets the cell's height to `target_height` (clamped to `[0, 1]`, matching `set`'s
    /// clamp), and records the *entire* resulting height into `external_mass_this_tick` so the
    /// physics solver can tell freshly-arrived external mass apart from mass that has had time to
    /// settle.
    ///
    /// Named and shaped as one half of a source/sink pair: today this only ever adds mass (a pour,
    /// or a test harness standing in for one), but a symmetric drain/sink counterpart is planned
    /// for the same waterfall feature, and should read as this method's obvious sibling rather
    /// than a bolted-on special case — see `external_mass_this_tick`'s doc comment for what is and
    /// isn't decided yet about the sink side.
    ///
    /// This records the full `new` value, not just `(new - old)`. That is deliberate, not an
    /// overcount: unlike the internal flux solver, which only ever *adds* to a cell's existing,
    /// possibly-already-settled content, this performs a hard overwrite — whatever provenance the
    /// previous content had is discarded outright. So from the solver's perspective the cell's
    /// entire post-call content arrived from outside this tick, the same way an interior cell of a
    /// falling stream has its entire content freshly arrived via edge flux (see `in_transit_at`,
    /// which for such a cell reads `~= h(above)`, not some smaller delta). A source cell held at a
    /// constant target by repeated calls must keep reading as "fully external" every tick, not just
    /// on the tick its height last changed — using only the delta under-reports this once the cell
    /// reaches its target and stops changing, which is exactly the case that matters for a
    /// continuous pour (verified empirically: delta-only left
    /// `test_liquid_stream_stays_coherent`'s max_width at 9 once `LATERAL_PRESSURE_DEPTH_FLOOR` was
    /// lowered to 0.0; recording the full height brings it back to 8).
    ///
    /// Accumulates (`+=`) rather than overwrites, so a cell called into more than once in the same
    /// tick (e.g. an overlapping pour and splash effect) reports the sum of both calls' full
    /// heights. `resting_above`'s subtraction is followed by a `.max(0.0)` clamp, so an
    /// over-generous sum here can only ever push a cell's apparent resting depth down to (not
    /// below) zero — it cannot manufacture negative depth or otherwise corrupt `column_depth`.
    ///
    /// Out-of-bounds coordinates are ignored, mirroring `set`.
    #[inline]
    pub fn apply_external_mass(&mut self, x: usize, y: usize, target_height: f32) {
        if x < self.width && y < self.height {
            let idx = y * self.width + x;
            let new = target_height.clamp(0.0, 1.0);
            self.external_mass_this_tick[idx] += new;
            self.data[idx] = new;
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
