pub mod grid;
pub mod physics;
pub mod quantiles;

pub use grid::Heightmap;
pub use physics::{ActiveBounds, displace_line, settle_tick};
pub use quantiles::{
    compute_quantile_positions, refresh_row_mass_active, refresh_row_mass_full, QuantileMode,
    DECILE_FRACTIONS, MAX_QUANTILE_LINES, QUARTILE_FRACTIONS,
};
use glam::Vec2;
use serde::{Deserialize, Serialize};

pub const GRID_SIZE: usize = 512;
pub const DEFAULT_SAND_HEIGHT: f32 = 0.35;

/// Shape mask cell values: the single source of truth for container geometry.
pub const MASK_OUTSIDE: u8 = 0;
pub const MASK_INSIDE: u8 = 1;
pub const MASK_BOUNDARY: u8 = 2;

pub const PROP_WETNESS: usize = 0;
pub const PROP_THRESHOLD: usize = 1;
pub const PROP_FLOW_RATE: usize = 2;
pub const PROP_GRAIN_SIZE: usize = 3;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum SimulatorMode {
    Sandbox,
    SandFall,
}

impl Default for SimulatorMode {
    fn default() -> Self {
        Self::Sandbox
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum SandboxShape {
    Circle,
    Square,
    Oval,
    Hourglass,
    MultiStageHourglass,
    GaltonBoard,
    StaircaseCascade,
    ProceduralFunnel,
    MultiNeckHourglass,
}

impl Default for SandboxShape {
    fn default() -> Self {
        Self::Circle
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum MaterialMode {
    DrySand,
    KineticSand,
    WetSand,
    CoarseSand,
    ButterCream,
    Snow,
    FinePowder,
    Oobleck,
    MoonDust,
    Water,
    Milk,
    VegetableOil,
    CalmWater,
    Yogurt,
}

impl Default for MaterialMode {
    fn default() -> Self {
        Self::DrySand
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[repr(u8)]
pub enum BlockActivity {
    Inactive = 0,
    Slow = 1,
    Medium = 2,
    Fast = 3,
}

impl Default for BlockActivity {
    fn default() -> Self {
        Self::Inactive
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MarbleState {
    pub pos: Vec2,
    pub prev_pos: Vec2,
    pub vel: Vec2,
    pub was_active: bool,
}

impl Default for MarbleState {
    fn default() -> Self {
        Self {
            pos: Vec2::ZERO,
            prev_pos: Vec2::ZERO,
            vel: Vec2::ZERO,
            was_active: false,
        }
    }
}

pub trait HeightmapSimulation {
    fn update(&mut self, dt: f32, cursor_targets: &[Option<glam::Vec2>]);
    fn reset(&mut self);
    fn heightmap(&self) -> &[f32];
    fn dimensions(&self) -> (usize, usize);
    fn marbles(&self) -> &[MarbleState; 5];
    fn active_bounds(&self) -> ActiveBounds;
}

/// Coordinates the state of the marble and the sand bed heightmap.
pub struct DrawingSimulation {
    /// The sand heightmap grid.
    pub heightmap: Heightmap,
    /// Pre-allocated temp buffer for double-buffering settling flows.
    pub temp_heights: Vec<f32>,
    /// Per-cell RGBA color buffer, derived from `cell_colors_f32`. This is a render/upload
    /// view only — external consumers (sandart-wasm, the native app) read it for GPU upload
    /// and for `set_cell_colors`'s external `&[u8]` contract. It is NOT the simulation's
    /// source of truth for color and must be regenerated (via `sync_cell_colors_u8`) any
    /// time `cell_colors_f32` changes.
    pub cell_colors: Vec<u8>,
    /// Per-cell RGBA color buffer in f32, RGBA interleaved, same `GRID_SIZE*GRID_SIZE*4`
    /// layout as `cell_colors`. This is the simulation's source of truth: `advect_properties`
    /// and all solver paths blend colors here without ever rounding to u8, so small flows
    /// (sub-1-LSB contributions) are no longer discarded the way they were when colors were
    /// blended directly in u8. `cell_colors` (u8) is regenerated from this buffer for
    /// external consumption; it is never blended directly.
    pub cell_colors_f32: Vec<f32>,
    /// Dither amplitude used when quantizing `cell_colors_f32` down to the `u8` render view.
    /// 0.0 = plain rounding (the original behaviour). 1.0 = true stochastic rounding, spread
    /// over one LSB. Higher values speckle wider for a coarser, grainier look.
    pub color_dither: f32,
    /// Per-cell physics & render properties. Advected with height.
    /// Layout: [wetness, threshold, flow_rate, grain_size] interleaved.
    pub cell_props: Vec<f32>,
    /// Current position of the primary marble (backward compatibility).
    pub marble_pos: Vec2,
    /// Previous position of the primary marble (backward compatibility).
    pub prev_marble_pos: Vec2,
    /// Last velocity of the primary marble (backward compatibility).
    pub marble_vel: Vec2,
    /// Track whether the primary marble has an active drawing stroke (backward compatibility).
    pub was_active: bool,
    /// Up to 5 marbles tracked in the simulation
    pub marbles: [MarbleState; 5],
    /// Active bounding box for settling updates.
    pub active_bounds: ActiveBounds,
    /// Sliding state tracker for stick-slip shear hysteresis.
    pub sliding: Vec<bool>,
    /// Per-edge momentum for the conservative edge-flux liquid solver (see
    /// `physics::flux_edge`). `edge_vel_h[i]` is the horizontal edge between cell `i` and
    /// `i + 1`; `edge_vel_v[i]` the vertical edge between cell `i` and `i + GRID_SIZE`.
    /// Replaces the old per-cell `wave_vel`, which could not be made mass-conservative.
    pub edge_vel_h: Vec<f32>,
    pub edge_vel_v: Vec<f32>,
    /// Seed for marble movement noise.
    pub seed: u32,

    // Internal simulation configuration fields
    pub marble_radius: f32,
    pub material_mode: MaterialMode,
    pub sandbox_shape: SandboxShape,
    pub gravity_dir: Vec2,
    pub neck_width: f32,
    pub hourglass_curve: f32,

    /// Precomputed shape mask grid (GRID_SIZE * GRID_SIZE).
    /// Values: MASK_OUTSIDE (0) = wall, MASK_INSIDE (1) = playable interior,
    /// MASK_BOUNDARY (2) = inside but adjacent to a wall cell.
    pub shape_mask: Vec<u8>,
    /// Set to true when the shape_mask has been regenerated and needs GPU re-upload.
    pub shape_mask_dirty: bool,

    /// Coarse block activity grid for CA optimization.
    pub active_blocks: Vec<BlockActivity>,
    /// Max displacement observed in each block during the last time it was simulated.
    pub last_displacements: Vec<f32>,
    /// Tick count of when each block was last simulated.
    pub last_simulated_ticks: Vec<u32>,
    /// Current dynamic simulation budget (N blocks).
    pub budget_n: usize,
    /// Exponential moving average of step time in milliseconds.
    pub ema_frame_ms: f32,
    /// Block size (e.g. 32 pixels).
    pub block_size: usize,
    /// Tick count for multi-rate LOD scheduling.
    pub tick_count: u32,

    /// Mass-weighted "how much has fallen" overlay setting (Sand-fall mode only). Off by
    /// default; setting this to anything other than `Off` is what turns on the per-row mass
    /// bookkeeping below — see `set_quantile_mode`.
    pub quantile_mode: QuantileMode,
    /// Cached per-*row* (not per-block) mass sum, `heightmap.height` entries long. Refreshed by
    /// `refresh_row_mass_active`/`refresh_row_mass_full`; only touched at all while
    /// `quantile_mode != QuantileMode::Off`, so it costs nothing when the feature is off.
    pub row_mass: Vec<f32>,
    /// The current quantile line targets (normalised 0.0..1.0, 0.0 = top row edge, 1.0 = bottom
    /// row edge), recomputed alongside `row_mass`. Length is 0 (Off), 3 (Quartiles), or 9
    /// (Deciles). These are raw targets, not eased for display — frame-to-frame smoothing is a
    /// rendering concern and belongs to the consumer (sandart-wasm), not the simulation.
    quantile_targets: Vec<f32>,
}

fn generate_smooth_noise(seed_val: u32) -> Heightmap {
    let mut heightmap = Heightmap::new(GRID_SIZE, GRID_SIZE, DEFAULT_SAND_HEIGHT);
    let mut seed = seed_val;

    // Helper to generate a low-res random grid via XORShift
    let mut gen_grid = |size: usize| -> Vec<f32> {
        let mut grid = vec![0.0f32; size * size];
        for val in grid.iter_mut() {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            *val = (seed as f32 / u32::MAX as f32) - 0.5; // Range [-0.5, 0.5]
        }
        grid
    };

    // Generate two noise grids at different resolutions (octaves)
    let grid_size1 = 8;
    let grid1 = gen_grid(grid_size1);

    let grid_size2 = 16;
    let grid2 = gen_grid(grid_size2);

    // Bilinear interpolation helper with smoothstep
    let sample_octave = |grid: &[f32], size: usize, x: usize, y: usize| -> f32 {
        let fx = (x as f32 / (GRID_SIZE - 1) as f32) * (size - 1) as f32;
        let fy = (y as f32 / (GRID_SIZE - 1) as f32) * (size - 1) as f32;

        let x0 = fx.floor() as usize;
        let x1 = (x0 + 1).min(size - 1);
        let y0 = fy.floor() as usize;
        let y1 = (y0 + 1).min(size - 1);

        let tx = fx - x0 as f32;
        let ty = fy - y0 as f32;

        // Smoothstep interpolation
        let sx = tx * tx * (3.0 - 2.0 * tx);
        let sy = ty * ty * (3.0 - 2.0 * ty);

        let v00 = grid[y0 * size + x0];
        let v10 = grid[y0 * size + x1];
        let v01 = grid[y1 * size + x0];
        let v11 = grid[y1 * size + x1];

        let h0 = v00 * (1.0 - sx) + v10 * sx;
        let h1 = v01 * (1.0 - sx) + v11 * sx;
        h0 * (1.0 - sy) + h1 * sy
    };

    for y in 0..GRID_SIZE {
        let row_offset = y * GRID_SIZE;
        for x in 0..GRID_SIZE {
            // Combine octaves: 8x8 primary (amp 0.025), 16x16 secondary (amp 0.008)
            let val1 = sample_octave(&grid1, grid_size1, x, y) * 0.025;
            let val2 = sample_octave(&grid2, grid_size2, x, y) * 0.008;

            let combined = val1 + val2;
            heightmap.data[row_offset + x] = (DEFAULT_SAND_HEIGHT + combined).clamp(0.0, 1.0);
        }
    }

    heightmap
}

impl DrawingSimulation {
    pub fn new() -> Self {
        let heightmap = generate_smooth_noise(12345u32);
        let temp_heights = heightmap.data.clone();
        let sliding = vec![false; GRID_SIZE * GRID_SIZE];
        let edge_vel_h = vec![0.0f32; GRID_SIZE * GRID_SIZE];
        let edge_vel_v = vec![0.0f32; GRID_SIZE * GRID_SIZE];
        let mut cell_colors_f32 = vec![0.0f32; GRID_SIZE * GRID_SIZE * 4];
        for chunk in cell_colors_f32.chunks_exact_mut(4) {
            chunk[0] = 210.0;
            chunk[1] = 180.0;
            chunk[2] = 140.0;
            chunk[3] = 255.0;
        }
        let cell_colors = cell_colors_f32.iter().map(|&v| v.clamp(0.0, 255.0).round() as u8).collect();
        let mut cell_props = vec![0.0f32; GRID_SIZE * GRID_SIZE * 4];
        // Initialize with default DrySand preset
        for chunk in cell_props.chunks_exact_mut(4) {
            chunk[PROP_WETNESS] = 0.00;
            chunk[PROP_THRESHOLD] = 0.08;
            chunk[PROP_FLOW_RATE] = 0.25;
            chunk[PROP_GRAIN_SIZE] = 0.45;
        }

        let block_size = 16;
        let cols = (GRID_SIZE + block_size - 1) / block_size;
        let rows = (GRID_SIZE + block_size - 1) / block_size;
        let active_blocks = vec![BlockActivity::Inactive; cols * rows];
        let last_displacements = vec![0.0f32; cols * rows];
        let last_simulated_ticks = vec![0u32; cols * rows];
        let budget_n = 256;
        let ema_frame_ms = 33.3;

        let mut sim = Self {
            heightmap,
            temp_heights,
            cell_colors,
            cell_colors_f32,
            color_dither: 0.0,
            cell_props,
            marble_pos: Vec2::ZERO,
            prev_marble_pos: Vec2::ZERO,
            marble_vel: Vec2::ZERO,
            was_active: false,
            marbles: [MarbleState::default(); 5],
            active_bounds: ActiveBounds {
                min_x: 0,
                max_x: 0,
                min_y: 0,
                max_y: 0,
                active: false,
            },
            sliding,
            edge_vel_h,
            edge_vel_v,
            seed: 98765u32,
            marble_radius: 0.018,
            material_mode: MaterialMode::default(),
            sandbox_shape: SandboxShape::default(),
            gravity_dir: Vec2::ZERO,
            neck_width: 0.005,
            hourglass_curve: 0.6,
            shape_mask: vec![MASK_OUTSIDE; GRID_SIZE * GRID_SIZE],
            shape_mask_dirty: true,
            active_blocks,
            last_displacements,
            last_simulated_ticks,
            budget_n,
            ema_frame_ms,
            block_size,
            tick_count: 0,
            quantile_mode: QuantileMode::default(),
            row_mass: vec![0.0f32; GRID_SIZE],
            quantile_targets: Vec::new(),
        };
        sim.generate_shape_mask();
        sim
    }

    /// Regenerate the shape mask from the current sandbox_shape, neck_width, and hourglass_curve.
    /// Call this whenever these parameters change. Sets shape_mask_dirty for GPU re-upload.
    pub fn generate_shape_mask(&mut self) {
        let w = GRID_SIZE;
        let h = GRID_SIZE;

        // Pass 1: Evaluate inside/safe for every cell using the existing physics evaluator
        for y in 0..h {
            let offset = y * w;
            for x in 0..w {
                let (inside, _safe) = physics::eval_sandbox_shape(
                    x, y, w, h,
                    self.sandbox_shape,
                    self.neck_width,
                    self.hourglass_curve,
                );
                self.shape_mask[offset + x] = if inside { MASK_INSIDE } else { MASK_OUTSIDE };
            }
        }

        // Pass 2: Mark boundary cells - any INSIDE cell with at least one OUTSIDE neighbor
        // We need a temporary copy to avoid read/write conflict
        let snapshot = self.shape_mask.clone();
        for y in 0..h {
            let offset = y * w;
            for x in 0..w {
                if snapshot[offset + x] == MASK_INSIDE {
                    let has_outside_neighbor =
                        (x == 0 || snapshot[offset + x - 1] == MASK_OUTSIDE) ||
                        (x + 1 >= w || snapshot[offset + x + 1] == MASK_OUTSIDE) ||
                        (y == 0 || snapshot[(y - 1) * w + x] == MASK_OUTSIDE) ||
                        (y + 1 >= h || snapshot[(y + 1) * w + x] == MASK_OUTSIDE);
                    if has_outside_neighbor {
                        self.shape_mask[offset + x] = MASK_BOUNDARY;
                    }
                }
            }
        }

        self.shape_mask_dirty = true;
    }

    /// Return a pointer to the shape mask data for WASM/GPU access.
    pub fn shape_mask_ptr(&self) -> *const u8 {
        self.shape_mask.as_ptr()
    }

    /// Return the length of the shape mask data.
    pub fn shape_mask_len(&self) -> usize {
        self.shape_mask.len()
    }

    /// Force a full regeneration of the `u8` colour view. Needed when the quantisation itself
    /// changes (e.g. `color_dither`), since the per-frame path only reconverts blocks whose
    /// colours moved and would otherwise leave a settled bed on its old quantisation.
    pub fn resync_cell_colors(&mut self) {
        self.sync_cell_colors_u8();
    }

    /// Quantize one f32 colour channel to u8, optionally dithering.
    ///
    /// With `strength == 0.0` this is a plain round, i.e. the previous behaviour exactly.
    ///
    /// Otherwise the fractional part becomes the probability of rounding up, so the result is
    /// unbiased: a channel sitting at 180.3 lands on 181 in 30% of cells and 180 in the other
    /// 70%, and reads as 180.3 across any small region. That is what makes it look grainy
    /// rather than banded — an 8-bit gradient posterizes into flat steps, a dithered one
    /// breaks the step edges up into speckle, which for sand is texture rather than error.
    ///
    /// The "probability" is a hash of the buffer index, NOT a per-frame random draw. That
    /// choice is the whole difference between grain and static:
    ///
    /// - Position-hashed, the pattern is fixed to the grid. A cell's threshold never changes,
    ///   so a still image is still, and only cells whose colour actually crossed their own
    ///   threshold flip. It reads as texture sitting on the sand.
    /// - Re-drawn each frame, the same cell would flip between 180 and 181 at random forever,
    ///   which reads as crawling TV static over the whole bed.
    ///
    /// It also has to be stable for `sync_cell_colors_u8_dirty` to be correct: that only
    /// reconverts blocks which changed this tick, so a frame-varying dither would leave
    /// untouched blocks frozen at an old pattern while active ones shimmered, making the
    /// 32x32 block grid show up as visible seams. Being a pure function of the index, the
    /// dirty path and the full path agree cell-for-cell — which is what
    /// `test_dirty_color_sync_matches_full_sync` already pins down.
    ///
    /// `strength` scales the dither amplitude: 1.0 spreads over one LSB (true stochastic
    /// rounding), higher values spread wider for a coarser, more granular look.
    #[inline]
    fn quantize_color(src: f32, index: usize, strength: f32) -> u8 {
        let v = src.clamp(0.0, 255.0);
        if strength <= 0.0 {
            return v.round() as u8;
        }
        // Cheap integer hash of the buffer index -> a fixed offset in [0, 1) for this channel.
        let mut hsh = index as u32;
        hsh ^= hsh >> 16;
        hsh = hsh.wrapping_mul(0x7feb_352d);
        hsh ^= hsh >> 15;
        hsh = hsh.wrapping_mul(0x846c_a68b);
        hsh ^= hsh >> 16;
        let offset = (hsh >> 8) as f32 / 16_777_216.0; // [0, 1)

        // floor(v + offset) rounds up exactly when frac(v) > 1 - offset, i.e. with probability
        // frac(v) across the population of cells. `strength` widens the window around the
        // value so coarser settings speckle over more than one level.
        let dithered = v + (offset - 0.5) * strength + 0.5;
        dithered.clamp(0.0, 255.0).floor() as u8
    }

    /// Regenerate the `u8` render/upload view (`cell_colors`) from the `f32` simulation
    /// source of truth (`cell_colors_f32`). Must be called any time `cell_colors_f32` is
    /// mutated and `cell_colors` may subsequently be read by an external consumer (GPU
    /// upload in sandart-wasm, the native renderer, or a test reading `sim.cell_colors`
    /// directly). This is the full-buffer regeneration, used on the paths that rewrite the
    /// whole grid (`reset`, `draw_point`/`draw_line`). The per-frame path in `update()` uses
    /// `sync_cell_colors_u8_dirty` instead — see there for why the narrow version is sound.
    fn sync_cell_colors_u8(&mut self) {
        let strength = self.color_dither;
        for (i, (dst, &src)) in self
            .cell_colors
            .iter_mut()
            .zip(self.cell_colors_f32.iter())
            .enumerate()
        {
            *dst = Self::quantize_color(src, i, strength);
        }
    }

    /// Per-frame variant of `sync_cell_colors_u8` that only regenerates the blocks whose colours
    /// can have changed during this tick.
    ///
    /// The full version converts all 1,048,576 elements every frame; measured on the 512x512
    /// Sand-fall benchmark that is ~2.7 ms/frame, which was 31% of a Water frame and 15% of a
    /// DrySand one — more than the whole liquid solver. Almost all of it is wasted, because a
    /// draining hourglass only touches a quarter of the grid per tick.
    ///
    /// **Why the dirty set below is a superset of what actually changed.** Colours are only ever
    /// written by `physics::advect_properties`, which writes the *destination* cell of a transfer
    /// (never the source). Every transfer site — `flux_edge` and `try_move` — calls
    /// `activate_neighbor` on both endpoint blocks before advecting, and `activate_neighbor` does
    /// two things: sets `modified[b]`, and raises `next_displacements[b]` to at least the
    /// transferred flow, which is strictly positive at every call site. `next_displacements`
    /// becomes `last_displacements` at the end of the tick (maxed with the previous value for
    /// blocks that were not simulated, which only ever makes it larger). So any block containing
    /// a changed cell ends the tick with `last_displacements[b] > 0`.
    ///
    /// The other way `modified[b]` is set is from `will_simulate[b]` at the start of the tick, and
    /// `will_simulate` is exactly the union of the must/stale/budget lists — which is exactly the
    /// set `active_blocks[b] != Inactive` marks. That also covers `displace_line`'s marble strokes:
    /// `update()` forces `last_displacements = 1.0` over the stroke's blocks before calling
    /// `settle_tick`, which puts them in the must-simulate list.
    ///
    /// Hence `active_blocks[b] != Inactive || last_displacements[b] > 0` covers every modified
    /// block (and a few extra stale ones, which is harmless — it only means a redundant
    /// conversion). `test_dirty_color_sync_matches_full_sync` pins this down against the full
    /// regeneration so the invariant cannot silently rot.
    fn sync_cell_colors_u8_dirty(&mut self) {
        let strength = self.color_dither;
        let w = self.heightmap.width;
        let h = self.heightmap.height;
        let block_size = self.block_size;
        let cols = (w + block_size - 1) / block_size;
        let rows = (h + block_size - 1) / block_size;
        if self.active_blocks.len() != cols * rows || self.last_displacements.len() != cols * rows {
            self.sync_cell_colors_u8();
            return;
        }

        for by in 0..rows {
            let start_y = by * block_size;
            let end_y = ((by + 1) * block_size).min(h);
            // Merge adjacent dirty block columns into one contiguous span per row, so the
            // conversion runs over long slices instead of 16-cell fragments.
            let mut bx = 0;
            while bx < cols {
                let b = by * cols + bx;
                let dirty = self.active_blocks[b] != BlockActivity::Inactive
                    || self.last_displacements[b] > 0.0;
                if !dirty {
                    bx += 1;
                    continue;
                }
                let run_start = bx;
                while bx < cols {
                    let b2 = by * cols + bx;
                    if self.active_blocks[b2] != BlockActivity::Inactive
                        || self.last_displacements[b2] > 0.0
                    {
                        bx += 1;
                    } else {
                        break;
                    }
                }
                let start_x = run_start * block_size;
                let end_x = (bx * block_size).min(w);
                for y in start_y..end_y {
                    let s = (y * w + start_x) * 4;
                    let e = (y * w + end_x) * 4;
                    for (offset, (dst, &src)) in self.cell_colors[s..e]
                        .iter_mut()
                        .zip(self.cell_colors_f32[s..e].iter())
                        .enumerate()
                    {
                        // Index must be the absolute buffer index, not the slice offset, or the
                        // dither pattern would differ between this path and the full one.
                        *dst = Self::quantize_color(src, s + offset, strength);
                    }
                }
            }
        }
    }

    /// Reset the simulation state.
    pub fn reset(&mut self) {
        self.generate_shape_mask();
        if matches!(
            self.sandbox_shape,
            SandboxShape::Hourglass
                | SandboxShape::MultiStageHourglass
                | SandboxShape::GaltonBoard
                | SandboxShape::StaircaseCascade
                | SandboxShape::ProceduralFunnel
                | SandboxShape::MultiNeckHourglass
        ) {
            self.heightmap.reset(0.0);
            self.initialize_hourglass();
        } else {
            self.heightmap = generate_smooth_noise(54321u32);
            self.temp_heights.copy_from_slice(&self.heightmap.data);
        }
        self.sliding.fill(false);
        self.edge_vel_h.fill(0.0);
        self.edge_vel_v.fill(0.0);
        for chunk in self.cell_colors_f32.chunks_exact_mut(4) {
            chunk[0] = 210.0;
            chunk[1] = 180.0;
            chunk[2] = 140.0;
            chunk[3] = 255.0;
        }
        self.sync_cell_colors_u8();
        self.apply_preset(self.material_mode);
        self.marble_pos = Vec2::ZERO;
        self.prev_marble_pos = Vec2::ZERO;
        self.marble_vel = Vec2::ZERO;
        self.was_active = false;
        self.marbles = [MarbleState::default(); 5];
        self.active_bounds = ActiveBounds {
            min_x: 0,
            max_x: 0,
            min_y: 0,
            max_y: 0,
            active: false,
        };
        self.seed = 98765u32;
        self.active_blocks.fill(BlockActivity::Inactive);
        self.last_displacements.fill(0.0);
        self.last_simulated_ticks.fill(0);
        self.budget_n = 256;
        self.ema_frame_ms = 33.3;
        self.tick_count = 0;
        self.refresh_quantiles_full();
    }

    pub fn initialize_hourglass(&mut self) {
        // Self-sufficient: regenerate the mask here rather than trusting callers to have
        // done so already for the current sandbox_shape/neck_width/hourglass_curve.
        self.generate_shape_mask();

        let w = GRID_SIZE;
        let h = GRID_SIZE;
        let center_y = h as f32 / 2.0;
        
        for y in 0..h {
            let row_offset = y * w;
            let dy = y as f32 - center_y;
            for x in 0..w {
                let idx = row_offset + x;
                let inside = self.shape_mask[idx] != MASK_OUTSIDE;

                let fill_threshold = if self.sandbox_shape == SandboxShape::MultiStageHourglass {
                    -0.14 * h as f32
                } else if self.sandbox_shape == SandboxShape::StaircaseCascade {
                    -0.26 * h as f32
                } else {
                    0.0
                };

                if inside {
                    if dy < fill_threshold {
                        // Upper chamber: filled with smooth sand (1.00 height / 100% capacity)
                        self.heightmap.data[idx] = 1.00;
                    } else {
                        // Lower chamber / lower stages: empty
                        self.heightmap.data[idx] = 0.0;
                    }
                } else {
                    self.heightmap.data[idx] = 0.0;
                }
            }
        }
        self.temp_heights.copy_from_slice(&self.heightmap.data);
    }

    pub fn flip_hourglass(&mut self) {
        let w = self.heightmap.width;
        let h = self.heightmap.height;
        
        // Symmetrical reflection around center_y (h / 2) so row 32 (neck) stays fixed
        for y in 1..=h / 2 {
            let y2 = h.saturating_sub(y);
            if y == y2 || y2 >= h {
                continue;
            }
            for x in 0..w {
                let i1 = y * w + x;
                let i2 = y2 * w + x;
                self.heightmap.data.swap(i1, i2);
                self.temp_heights.swap(i1, i2);
                // (edge momentum is not mirrored here — it is cleared after the loop)
                self.sliding.swap(i1, i2);

                for ch in 0..4 {
                    self.cell_colors.swap(i1 * 4 + ch, i2 * 4 + ch);
                    self.cell_colors_f32.swap(i1 * 4 + ch, i2 * 4 + ch);
                }
                for ch in 0..4 {
                    self.cell_props.swap(i1 * 4 + ch, i2 * 4 + ch);
                }
            }
        }

        // Edge momentum does not survive turning the apparatus over. Mirroring it would mean
        // reversing the sign of every gravity-aligned edge and shifting its index by one row,
        // and the partial row range swapped above does not cover the edge set cleanly anyway.
        // Clearing is both simpler and the physically honest answer: the contents are in free
        // fall from rest the instant the glass is inverted.
        self.edge_vel_h.fill(0.0);
        self.edge_vel_v.fill(0.0);

        // Clean up any sand outside the shape boundary so no specs stay trapped outside/above ceiling
        for y in 0..h {
            for x in 0..w {
                let idx = y * w + x;
                if self.shape_mask[idx] == MASK_OUTSIDE {
                    self.heightmap.data[idx] = 0.0;
                    self.temp_heights[idx] = 0.0;
                }
            }
        }

        self.active_blocks.fill(BlockActivity::Inactive);
        self.last_displacements.fill(0.5); // Force all blocks to be re-simulated
        self.tick_count = 0;
        self.refresh_quantiles_full();
    }

    /// Set the quantile-line overlay mode (off/quartiles/deciles) and immediately bring the
    /// row-mass cache and quantile targets up to date. A full recompute here (rather than
    /// waiting for the next periodic partial refresh) means turning the feature on never shows
    /// a stale/zero reading left over from whatever the cache last held while it was off.
    pub fn set_quantile_mode(&mut self, mode: QuantileMode) {
        self.quantile_mode = mode;
        self.refresh_quantiles_full();
    }

    /// Current quantile line targets, normalised 0.0 (top row edge) .. 1.0 (bottom row edge).
    /// Empty when `quantile_mode` is `Off`. These are raw targets refreshed at most every 5
    /// ticks — easing them for smooth frame-to-frame motion is left to the renderer/consumer.
    pub fn quantile_positions(&self) -> &[f32] {
        &self.quantile_targets
    }

    /// Full (all `GRID_SIZE` rows) row-mass recompute plus a fresh quantile target computation.
    /// O(GRID_SIZE^2); only meant for discontinuities (reset, flip, mode just switched on) — the
    /// steady-state per-tick path is `refresh_quantiles_partial`, which only re-sums rows whose
    /// block was actually simulated this tick. A no-op (aside from clearing the cached targets)
    /// when the feature is off, so resets/flips stay free of this cost in the common case.
    fn refresh_quantiles_full(&mut self) {
        if self.quantile_mode == QuantileMode::Off {
            self.quantile_targets.clear();
            return;
        }
        refresh_row_mass_full(
            &self.heightmap.data,
            self.heightmap.width,
            self.heightmap.height,
            &mut self.row_mass,
        );
        self.quantile_targets =
            compute_quantile_positions(&self.row_mass, self.quantile_mode.fractions());
    }

    /// Steady-state per-tick refresh: only re-sums rows belonging to a block that
    /// `settle_tick` actually simulated this tick (per `active_blocks`), then recomputes the
    /// quantile targets from the (mostly-cached) row_mass array. Called at most once every 5
    /// ticks from `update`, and only while `quantile_mode != Off` — see the call site for the
    /// full cost-gating rationale.
    fn refresh_quantiles_partial(&mut self) {
        refresh_row_mass_active(
            &self.heightmap.data,
            self.heightmap.width,
            self.heightmap.height,
            self.block_size,
            &self.active_blocks,
            &mut self.row_mass,
        );
        self.quantile_targets =
            compute_quantile_positions(&self.row_mass, self.quantile_mode.fractions());
    }

    /// Apply a preset to the per-cell properties buffer.
    pub fn apply_preset(&mut self, mode: MaterialMode) {
        let (wetness, threshold, flow_rate, grain_size) = match mode {
            MaterialMode::DrySand => (0.00, 0.08, 0.25, 0.45),
            MaterialMode::CoarseSand => (0.00, 0.11, 0.22, 0.80),
            MaterialMode::KineticSand => (0.20, 0.10, 0.15, 0.35),
            MaterialMode::WetSand => (0.45, 0.14, 0.08, 0.40),
            MaterialMode::FinePowder => (0.00, 0.05, 0.30, 0.05),
            MaterialMode::Snow => (0.05, 0.15, 0.20, 0.20),
            MaterialMode::MoonDust => (0.00, 0.20, 0.20, 0.10),
            MaterialMode::Oobleck => (0.55, 0.04, 0.12, 0.15),
            MaterialMode::ButterCream => (0.70, 0.04, 0.15, 0.08),
            MaterialMode::Water => (1.00, 0.00, 0.00, 0.00),
            MaterialMode::CalmWater => (0.90, 0.00, 0.00, 0.00),
            MaterialMode::Milk => (0.95, 0.00, 0.00, 0.00),
            MaterialMode::VegetableOil => (0.85, 0.00, 0.00, 0.00),
            MaterialMode::Yogurt => (0.75, 0.00, 0.00, 0.08),
        };
        for chunk in self.cell_props.chunks_exact_mut(4) {
            chunk[PROP_WETNESS] = wetness;
            chunk[PROP_THRESHOLD] = threshold;
            chunk[PROP_FLOW_RATE] = flow_rate;
            chunk[PROP_GRAIN_SIZE] = grain_size;
        }
        self.material_mode = mode;
    }

    /// Copy color patterns into CPU color buffer
    pub fn set_cell_colors(&mut self, rgba_data: &[u8]) {
        let len = self.cell_colors.len().min(rgba_data.len());
        self.cell_colors[..len].copy_from_slice(&rgba_data[..len]);
        // This is the entry point for externally-supplied colors (u8), so the f32 source of
        // truth must be brought into agreement too, or the next advection event would blend
        // against stale f32 values and the just-set u8 colors would be overwritten.
        for i in 0..len {
            self.cell_colors_f32[i] = self.cell_colors[i] as f32;
        }
    }

    /// Copy per-cell properties from a custom buffer
    pub fn set_cell_props(&mut self, props_data: &[f32]) {
        let len = self.cell_props.len().min(props_data.len());
        self.cell_props[..len].copy_from_slice(&props_data[..len]);
    }

    /// Convert normalized Cartesian coordinates ([-1.0, 1.0]) to grid index coordinates.
    #[allow(dead_code)]
    pub fn norm_to_grid(pos: Vec2, width: usize, height: usize) -> (usize, usize) {
        let px = if pos.x.is_finite() { pos.x } else { 0.0 };
        let py = if pos.y.is_finite() { pos.y } else { 0.0 };
        let x = ((px + 1.0) * 0.5 * width as f32).clamp(0.0, (width - 1) as f32) as usize;
        let y = ((1.0 - py) * 0.5 * height as f32).clamp(0.0, (height - 1) as f32) as usize;
        (x, y)
    }

    /// Erase height values inside the marble radius to 0.0 with sub-pixel precision.
    #[allow(dead_code)]
    pub fn draw_point(&mut self, pos: Vec2, radius: f32) {
        displace_line(
            &mut self.heightmap,
            &mut self.cell_colors_f32,
            &mut self.cell_props,
            pos,
            pos,
            radius,
            &mut self.active_bounds,
        );
        self.sync_cell_colors_u8();
    }

    /// Draw a line between start and end using interpolation to prevent gaps.
    #[allow(dead_code)]
    pub fn draw_line(&mut self, start: Vec2, end: Vec2, radius: f32) {
        displace_line(
            &mut self.heightmap,
            &mut self.cell_colors_f32,
            &mut self.cell_props,
            start,
            end,
            radius,
            &mut self.active_bounds,
        );
        self.sync_cell_colors_u8();
    }

    fn clamp_to_sandbox(pos: Vec2, shape: SandboxShape, marble_radius: f32) -> Vec2 {
        let max_r = (0.92 - marble_radius).max(0.0);
        match shape {
            SandboxShape::Circle => {
                let len = pos.length();
                if len > max_r && len > 1e-5 {
                    pos * (max_r / len)
                } else {
                    pos
                }
            }
            SandboxShape::Square => {
                Vec2::new(
                    pos.x.clamp(-max_r, max_r),
                    pos.y.clamp(-max_r, max_r),
                )
            }
            SandboxShape::Oval => {
                let a = (0.92 - marble_radius).max(0.01);
                let b = (0.60 - marble_radius).max(0.01);
                let d_sq = (pos.x * pos.x) / (a * a) + (pos.y * pos.y) / (b * b);
                if d_sq > 1.0 {
                    let d = d_sq.sqrt();
                    pos / d
                } else {
                    pos
                }
            }
            SandboxShape::Hourglass
            | SandboxShape::MultiStageHourglass
            | SandboxShape::GaltonBoard
            | SandboxShape::StaircaseCascade
            | SandboxShape::ProceduralFunnel
            | SandboxShape::MultiNeckHourglass => {
                let chamber_r = 0.92 - marble_radius;  // normalized coords
                let chamber_offset = 0.58;             // normalized vertical offset
                let neck_hw = 0.07 - marble_radius;    // normalized neck half-width

                // Check if in upper chamber, lower chamber, or neck
                let in_upper = Vec2::new(pos.x, pos.y - chamber_offset).length() < chamber_r;
                let in_lower = Vec2::new(pos.x, pos.y + chamber_offset).length() < chamber_r;
                let in_neck = pos.x.abs() < neck_hw && pos.y.abs() < chamber_offset;

                if in_upper || in_lower || in_neck {
                    pos  // already inside
                } else {
                    // Clamp to nearest boundary (upper or lower chamber)
                    let to_upper = Vec2::new(pos.x, pos.y - chamber_offset);
                    let to_lower = Vec2::new(pos.x, pos.y + chamber_offset);
                    if to_upper.length() < to_lower.length() {
                        let dir = to_upper.normalize_or_zero();
                        Vec2::new(0.0, chamber_offset) + dir * chamber_r
                    } else {
                        let dir = to_lower.normalize_or_zero();
                        Vec2::new(0.0, -chamber_offset) + dir * chamber_r
                    }
                }
            }
        }
    }

    /// Run a physics frame tick.
    pub fn update(&mut self, dt: f32, targets: &[Option<Vec2>; 5], marble_radius: f32, _material: MaterialMode, shape: SandboxShape, last_frame_time_ms: f32, target_frame_time_ms: f32) {
        // Prevent seed degeneracy (XORShift stuck state at 0)
        if self.seed == 0 {
            self.seed = 98765u32;
        }

        // Advance seed every frame to keep settling dynamics active and non-deterministic
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        let time_seed = self.seed;

        let w = self.heightmap.width;
        let h = self.heightmap.height;
        let block_size = self.block_size;
        let cols = (w + block_size - 1) / block_size;
        let rows = (h + block_size - 1) / block_size;

        for j in 0..5 {
            if let Some(target) = targets[j] {
                // Sanitize target coordinate float boundaries against NaNs/Infs
                let tx = if target.x.is_finite() { target.x } else { 0.0 };
                let ty = if target.y.is_finite() { target.y } else { 0.0 };
                let target_sanitized = Vec2::new(tx, ty);

                let clamped_target = Self::clamp_to_sandbox(target_sanitized, shape, marble_radius);

                let mut segment_bounds = ActiveBounds {
                    min_x: 0,
                    max_x: 0,
                    min_y: 0,
                    max_y: 0,
                    active: false,
                };

                if self.marbles[j].was_active {
                    self.marbles[j].prev_pos = self.marbles[j].pos;

                    // Calculate step vector and distance
                    let raw_diff = clamped_target - self.marbles[j].pos;
                    let raw_dist = raw_diff.length();

                    // 1. Generate pseudo-random numbers
                    self.seed ^= self.seed << 13;
                    self.seed ^= self.seed >> 17;
                    self.seed ^= self.seed << 5;
                    let n1 = (self.seed as f32 / u32::MAX as f32 - 0.5) * 2.0; // [-1.0, 1.0]

                    self.seed ^= self.seed << 13;
                    self.seed ^= self.seed >> 17;
                    self.seed ^= self.seed << 5;
                    let n2 = (self.seed as f32 / u32::MAX as f32 - 0.5) * 2.0; // [-1.0, 1.0]

                    let random_offset = Vec2::new(n1, n2);

                    // 2. Micro-jitter: simulate bumping over discrete sand grains (extremely subtle)
                    let jitter_amplitude = marble_radius * 0.04;
                    let jitter = random_offset * jitter_amplitude;

                    // 3. Inertia/drag drift: simulate sand resistance lagging and sliding sideways
                    let mut drift = Vec2::ZERO;
                    if raw_dist > 1e-5 {
                        let dir = raw_diff / raw_dist;
                        let perp = Vec2::new(-dir.y, dir.x);

                        // Minor drag (lag behind magnet/target)
                        let lag = -dir * (raw_dist * 0.08);

                        // Minor sideways slip (uneven resistance)
                        let slip = perp * (raw_dist * 0.05 * n1);

                        drift = lag + slip;
                    }

                    let mut next_pos = clamped_target + jitter + drift;
                    next_pos = Self::clamp_to_sandbox(next_pos, shape, marble_radius);

                    self.marbles[j].pos = next_pos;
                    self.marbles[j].vel = next_pos - self.marbles[j].prev_pos;

                    displace_line(
                        &mut self.heightmap,
                        &mut self.cell_colors_f32,
                        &mut self.cell_props,
                        self.marbles[j].prev_pos,
                        self.marbles[j].pos,
                        marble_radius,
                        &mut segment_bounds,
                    );
                } else {
                    self.marbles[j].pos = clamped_target;
                    self.marbles[j].prev_pos = clamped_target;
                    self.marbles[j].vel = Vec2::ZERO;
                    displace_line(
                        &mut self.heightmap,
                        &mut self.cell_colors_f32,
                        &mut self.cell_props,
                        clamped_target,
                        clamped_target,
                        marble_radius,
                        &mut segment_bounds,
                    );
                    self.marbles[j].was_active = true;
                }

                // Activate blocks overlapping with the new displacement segment
                if segment_bounds.active {
                    let block_min_x = segment_bounds.min_x / block_size;
                    let block_max_x = (segment_bounds.max_x / block_size).min(cols - 1);
                    let block_min_y = segment_bounds.min_y / block_size;
                    let block_max_y = (segment_bounds.max_y / block_size).min(rows - 1);
                    for by in block_min_y..=block_max_y {
                        for bx in block_min_x..=block_max_x {
                            self.last_displacements[by * cols + bx] = 1.0;
                        }
                    }
                }
            } else {
                self.marbles[j].was_active = false;
            }

            // Sync with primary fields for backward compatibility
            if j == 0 {
                self.marble_pos = self.marbles[0].pos;
                self.prev_marble_pos = self.marbles[0].prev_pos;
                self.marble_vel = self.marbles[0].vel;
                self.was_active = self.marbles[0].was_active;
            }
        }



        // Run the gravity-driven settling cellular automata tick
        let has_active = self.last_displacements.iter().any(|&x| x > 3e-4)
            || self.marbles.iter().any(|m| m.was_active)
            || self.gravity_dir.length_squared() > 1e-6;
        if has_active {
            let mut active_marbles = [physics::ActiveMarbleInfo {
                pos: Vec2::ZERO,
                vel: 0.0,
                vel_vec: Vec2::ZERO,
            }; 5];
            let mut active_count = 0;
            for j in 0..5 {
                if self.marbles[j].was_active {
                    let m_vel_vec = if dt > 1e-5 { self.marbles[j].vel / dt } else { Vec2::ZERO };
                    active_marbles[active_count] = physics::ActiveMarbleInfo {
                        pos: self.marbles[j].pos,
                        vel: m_vel_vec.length(),
                        vel_vec: m_vel_vec,
                    };
                    active_count += 1;
                }
            }

            let iterations = if self.gravity_dir.length_squared() > 1e-6 { 1 } else { 1 }; // STAGE3 PROBE
            for iter in 0..iterations {
                settle_tick(
                    &mut self.heightmap,
                    &mut self.temp_heights,
                    &mut self.cell_colors_f32,
                    &mut self.cell_props,
                    &mut self.sliding,
                    &mut self.active_bounds,
                    &mut self.active_blocks,
                    &mut self.last_displacements,
                    &mut self.last_simulated_ticks,
                    self.budget_n,
                    self.block_size,
                    &active_marbles[..active_count],
                    time_seed + iter as u32,
                    &mut self.edge_vel_h,
                    &mut self.edge_vel_v,
                    &self.shape_mask,
                    self.tick_count + iter as u32,
                    self.gravity_dir,
                );
            }
        } else {
            self.active_bounds.active = false;
        }
        self.tick_count = self.tick_count.wrapping_add(1);

        // Quantile mass-distribution lines (Sand-fall overlay): recompute at most every 5 ticks,
        // and only while the feature is switched on. `has_active` being false means nothing
        // moved this tick, so row_mass couldn't have changed either — skip in that case too.
        // This is the whole cost-gating story: when `quantile_mode == Off` (the default), this
        // branch — and therefore the O(width) row re-sums it guards — never runs at all.
        if has_active && self.quantile_mode != QuantileMode::Off && self.tick_count % 5 == 0 {
            self.refresh_quantiles_partial();
        }

        // Update EMA of frame time and adjust budget_n
        const EMA_ALPHA: f32 = 0.1;
        const BUDGET_MIN: usize = 32;
        const BUDGET_STEP_DOWN: usize = 4;
        const BUDGET_STEP_UP: usize = 1;

        if last_frame_time_ms > 0.0 && target_frame_time_ms > 0.0 {
            self.ema_frame_ms = EMA_ALPHA * last_frame_time_ms + (1.0 - EMA_ALPHA) * self.ema_frame_ms;
            
            let budget_max = cols * rows; // e.g. 1024

            // Target 95% of target FPS (Vsync interval * 1.05) to account for browser Vsync-locking
            // and allow the budget to grow back up when running smoothly.
            let adjusted_target = target_frame_time_ms * 1.05;

            if self.ema_frame_ms > adjusted_target {
                self.budget_n = self.budget_n.saturating_sub(BUDGET_STEP_DOWN).max(BUDGET_MIN);
            } else if self.ema_frame_ms < adjusted_target {
                self.budget_n = (self.budget_n + BUDGET_STEP_UP).min(budget_max);
            }
        }

        // Regenerate the u8 render/upload view once per tick, after every displace_line and
        // settle_tick call above has finished mutating cell_colors_f32. This is the primary
        // external read path (sandart-wasm's render() reads self.sim.cell_colors for GPU
        // upload right after update() returns), so it must be fresh before returning here.
        // Narrowed to the blocks that can have changed — see `sync_cell_colors_u8_dirty`.
        self.sync_cell_colors_u8_dirty();
    }
}

impl HeightmapSimulation for DrawingSimulation {
    fn update(&mut self, dt: f32, cursor_targets: &[Option<glam::Vec2>]) {
        let mut targets = [None; 5];
        for (i, target) in cursor_targets.iter().take(5).enumerate() {
            targets[i] = *target;
        }
        let radius = self.marble_radius;
        let mat = self.material_mode;
        let shape = self.sandbox_shape;
        self.update(dt, &targets, radius, mat, shape, dt * 1000.0, dt * 1000.0);
    }

    fn reset(&mut self) {
        self.reset();
    }

    fn heightmap(&self) -> &[f32] {
        self.heightmap.as_slice()
    }

    fn dimensions(&self) -> (usize, usize) {
        (GRID_SIZE, GRID_SIZE)
    }

    fn marbles(&self) -> &[MarbleState; 5] {
        &self.marbles
    }

    fn active_bounds(&self) -> ActiveBounds {
        self.active_bounds
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simulation_reset() {
        let mut sim = DrawingSimulation::new();
        sim.marble_pos = Vec2::new(0.5, -0.5);
        sim.heightmap.set(100, 100, 0.0);
        sim.reset();
        assert_eq!(sim.marble_pos, Vec2::ZERO);
        let val = sim.heightmap.get(100, 100);
        assert!((val - DEFAULT_SAND_HEIGHT).abs() < 0.035);
    }

    #[test]
    fn test_norm_to_grid_mapping() {
        let width = 512;
        let height = 512;

        // Verify corners map to exact boundary indexes
        assert_eq!(
            DrawingSimulation::norm_to_grid(Vec2::new(-1.0, 1.0), width, height),
            (0, 0)
        );
        assert_eq!(
            DrawingSimulation::norm_to_grid(Vec2::new(1.0, -1.0), width, height),
            (width - 1, height - 1)
        );

        // Verify center mapping falls in correct bins (256, 256)
        assert_eq!(
            DrawingSimulation::norm_to_grid(Vec2::new(0.0, 0.0), width, height),
            (256, 256)
        );

        // Verify bounds clamping maps out of bounds coordinates to grid edges safely
        assert_eq!(
            DrawingSimulation::norm_to_grid(Vec2::new(-2.0, 2.0), width, height),
            (0, 0)
        );
        assert_eq!(
            DrawingSimulation::norm_to_grid(Vec2::new(2.0, -2.0), width, height),
            (width - 1, height - 1)
        );
    }

    #[test]
    fn test_norm_to_grid_nan_inf() {
        let width = 512;
        let height = 512;

        // NAN should map safely without panic
        let nan_pos = Vec2::new(f32::NAN, f32::NAN);
        let (x, y) = DrawingSimulation::norm_to_grid(nan_pos, width, height);
        assert!(x < width && y < height);

        // Inf should map safely without panic
        let inf_pos = Vec2::new(f32::INFINITY, f32::NEG_INFINITY);
        let (x, y) = DrawingSimulation::norm_to_grid(inf_pos, width, height);
        assert!(x < width && y < height);
    }

    #[test]
    fn test_marble_movement_noise_and_drift() {
        let mut sim = DrawingSimulation::new();
        let mut targets = [None; 5];
        // Initially target is None, should not be active
        sim.update(0.016, &targets, 0.025, MaterialMode::ButterCream, SandboxShape::Circle, 16.0, 16.0);
        assert!(!sim.was_active);

        // Move to start point (first point is exact target)
        targets[0] = Some(Vec2::new(0.1, 0.2));
        sim.update(0.016, &targets, 0.025, MaterialMode::ButterCream, SandboxShape::Circle, 16.0, 16.0);
        assert!(sim.was_active);
        assert_eq!(sim.marble_pos, Vec2::new(0.1, 0.2));

        // Move to next point, introducing noise, drag, and jitter
        let target = Vec2::new(0.3, 0.4);
        targets[0] = Some(target);
        sim.update(0.016, &targets, 0.025, MaterialMode::ButterCream, SandboxShape::Circle, 16.0, 16.0);

        // Ensure marble position shifted from start and is not exactly the target due to physics drift/noise
        assert_ne!(sim.marble_pos, Vec2::new(0.1, 0.2));
        assert_ne!(sim.marble_pos, target);

        // Verify that it is close to target but slightly drifted/jittered (less than 0.1 delta)
        let dist = (sim.marble_pos - target).length();
        assert!(dist < 0.1);

        // Verify marble velocity is populated
        assert_ne!(sim.marble_vel, Vec2::ZERO);
    }

    #[test]
    fn test_sandbox_shapes_clamping() {
        // Test Circle clamping: length should be clamped to max_r
        let p_circle = DrawingSimulation::clamp_to_sandbox(Vec2::new(1.0, 1.0), SandboxShape::Circle, 0.018);
        assert!((p_circle.length() - (0.92 - 0.018)).abs() < 1e-5);

        // Test Square clamping: X and Y should be clamped to max_r
        let p_square = DrawingSimulation::clamp_to_sandbox(Vec2::new(1.5, 0.2), SandboxShape::Square, 0.018);
        assert_eq!(p_square.x, 0.92 - 0.018);
        assert_eq!(p_square.y, 0.2);

        // Test Oval clamping: should satisfy ellipse equation
        let p_oval = DrawingSimulation::clamp_to_sandbox(Vec2::new(1.0, 1.0), SandboxShape::Oval, 0.018);
        let a = 0.92 - 0.018;
        let b = 0.60 - 0.018;
        let d_sq = (p_oval.x * p_oval.x) / (a * a) + (p_oval.y * p_oval.y) / (b * b);
        assert!((d_sq - 1.0).abs() < 1e-5);
    }

    #[test]
    fn test_simulation_volume_preservation() {
        let mut sim = DrawingSimulation::new();
        let initial_sum: f64 = sim.heightmap.data.iter().map(|&x| x as f64).sum();

        let mut targets = [None; 5];
        // Move marble in a spiral over 200 steps
        for i in 0..200 {
            let angle = i as f32 * 0.1;
            let radius = i as f32 * 0.004;
            targets[0] = Some(Vec2::new(angle.cos() * radius, angle.sin() * radius));
            sim.update(
                0.016,
                &targets,
                0.018,
                MaterialMode::DrySand,
                SandboxShape::Circle,
                16.0,
                16.0,
            );
            
            let current_sum: f64 = sim.heightmap.data.iter().map(|&x| x as f64).sum();
            let diff = (current_sum - initial_sum).abs();
            assert!(diff < 5e-3, "Step {}: Volume leaked! diff = {}, initial = {}, current = {}", i, diff, initial_sum, current_sum);
        }
    }

    #[test]
    fn test_multi_marble_large_spiral_volume_preservation() {
        let mut sim = DrawingSimulation::new();
        let initial_sum: f64 = sim.heightmap.data.iter().map(|&x| x as f64).sum();

        let mut targets = [None; 5];
        // Large marble radius
        let marble_radius = 0.08;
        
        // Move 3 marbles in out-of-phase spirals over 150 steps
        for i in 0..150 {
            for j in 0..3 {
                let angle = i as f32 * 0.15 + (j as f32 * 2.0 * std::f32::consts::PI / 3.0);
                let radius = i as f32 * 0.005;
                targets[j] = Some(Vec2::new(angle.cos() * radius, angle.sin() * radius));
            }
            sim.update(
                0.016,
                &targets,
                marble_radius,
                MaterialMode::DrySand,
                SandboxShape::Circle,
                16.0,
                16.0,
            );
            
            let current_sum: f64 = sim.heightmap.data.iter().map(|&x| x as f64).sum();
            let diff = (current_sum - initial_sum).abs();
            // Use 2e-2 threshold for multi-marble large updates, due to larger accumulated float rounding errors.
            assert!(diff < 2e-2, "Step {}: Multi-marble volume leaked! diff = {}, initial = {}, current = {}", i, diff, initial_sum, current_sum);
        }
    }

    #[test]
    fn test_simulation_color_preservation() {
        let mut sim = DrawingSimulation::new();
        
        // Initialize cell_colors with a gradient/pattern
        let mut initial_colors = vec![0u8; GRID_SIZE * GRID_SIZE * 4];
        for y in 0..GRID_SIZE {
            for x in 0..GRID_SIZE {
                let idx = y * GRID_SIZE + x;
                initial_colors[idx * 4 + 0] = (x % 256) as u8;
                initial_colors[idx * 4 + 1] = (y % 256) as u8;
                initial_colors[idx * 4 + 2] = 128;
                initial_colors[idx * 4 + 3] = 255;
            }
        }
        sim.set_cell_colors(&initial_colors);

        let calculate_color_mass = |s: &DrawingSimulation| -> (f64, f64) {
            let mut red_mass = 0.0f64;
            let mut green_mass = 0.0f64;
            for (idx, &h) in s.heightmap.data.iter().enumerate() {
                let r = s.cell_colors[idx * 4 + 0] as f64;
                let g = s.cell_colors[idx * 4 + 1] as f64;
                red_mass += r * h as f64;
                green_mass += g * h as f64;
            }
            (red_mass, green_mass)
        };

        let (initial_red, initial_green) = calculate_color_mass(&sim);

        let mut targets = [None; 5];
        // Move marble in a spiral over 200 steps
        for i in 0..200 {
            let angle = i as f32 * 0.1;
            let radius = i as f32 * 0.004;
            targets[0] = Some(Vec2::new(angle.cos() * radius, angle.sin() * radius));
            sim.update(
                0.016,
                &targets,
                0.018,
                MaterialMode::DrySand,
                SandboxShape::Circle,
                16.0,
                16.0,
            );
        }

        let (final_red, final_green) = calculate_color_mass(&sim);

        let diff_red = (final_red - initial_red).abs() / initial_red;
        let diff_green = (final_green - initial_green).abs() / initial_green;

        // Verify that the color mass is preserved within 0.5% (to account for u8 integer rounding at each step)
        assert!(diff_red < 0.005, "Red color mass leaked! diff = {:.5}%, initial = {}, final = {}", diff_red * 100.0, initial_red, final_red);
        assert!(diff_green < 0.005, "Green color mass leaked! diff = {:.5}%, initial = {}, final = {}", diff_green * 100.0, initial_green, final_green);
    }

    #[test]
    fn test_multi_marble_large_spiral_color_preservation() {
        let mut sim = DrawingSimulation::new();

        // Initialize cell_colors with a gradient/pattern
        let mut initial_colors = vec![0u8; GRID_SIZE * GRID_SIZE * 4];
        for y in 0..GRID_SIZE {
            for x in 0..GRID_SIZE {
                let idx = y * GRID_SIZE + x;
                initial_colors[idx * 4 + 0] = (x % 256) as u8;
                initial_colors[idx * 4 + 1] = (y % 256) as u8;
                initial_colors[idx * 4 + 2] = 128;
                initial_colors[idx * 4 + 3] = 255;
            }
        }
        sim.set_cell_colors(&initial_colors);

        let calculate_color_mass = |s: &DrawingSimulation| -> (f64, f64) {
            let mut red_mass = 0.0f64;
            let mut green_mass = 0.0f64;
            for (idx, &h) in s.heightmap.data.iter().enumerate() {
                let r = s.cell_colors[idx * 4 + 0] as f64;
                let g = s.cell_colors[idx * 4 + 1] as f64;
                red_mass += r * h as f64;
                green_mass += g * h as f64;
            }
            (red_mass, green_mass)
        };

        let (initial_red, initial_green) = calculate_color_mass(&sim);

        let mut targets = [None; 5];
        let marble_radius = 0.08;
        
        // Move 3 marbles in out-of-phase spirals over 150 steps
        for i in 0..150 {
            for j in 0..3 {
                let angle = i as f32 * 0.15 + (j as f32 * 2.0 * std::f32::consts::PI / 3.0);
                let radius = i as f32 * 0.005;
                targets[j] = Some(Vec2::new(angle.cos() * radius, angle.sin() * radius));
            }
            sim.update(
                0.016,
                &targets,
                marble_radius,
                MaterialMode::DrySand,
                SandboxShape::Circle,
                16.0,
                16.0,
            );
        }

        let (final_red, final_green) = calculate_color_mass(&sim);

        let diff_red = (final_red - initial_red).abs() / initial_red;
        let diff_green = (final_green - initial_green).abs() / initial_green;

        // Verify that the color mass is preserved within 0.5%
        assert!(diff_red < 0.005, "Multi-marble Red color mass leaked! diff = {:.5}%, initial = {}, final = {}", diff_red * 100.0, initial_red, final_red);
        assert!(diff_green < 0.005, "Multi-marble Green color mass leaked! diff = {:.5}%, initial = {}, final = {}", diff_green * 100.0, initial_green, final_green);
    }

    #[test]
    fn test_serpentine_no_sand_leaking() {
        let mut sim = super::DrawingSimulation::new();
        sim.sandbox_shape = SandboxShape::MultiStageHourglass;
        sim.gravity_dir = Vec2::new(0.0, 0.04);
        sim.initialize_hourglass();

        let initial_mass: f32 = sim.heightmap.data.iter().sum();
        assert!(initial_mass > 0.0, "MultiStageHourglass should be initialized with sand in Stage 1");

        let targets = [None; 5];
        // Run gravity simulation for 500 ticks across all 3 stages
        for _ in 0..500 {
            sim.update(
                0.016,
                &targets,
                0.08,
                MaterialMode::DrySand,
                SandboxShape::MultiStageHourglass,
                16.0,
                16.0,
            );
        }

        let final_mass: f32 = sim.heightmap.data.iter().sum();
        let mass_err = (final_mass - initial_mass).abs() / initial_mass;

        // Verify 100.0000% sand mass conservation with ZERO leaks out of bounds
        assert!(
            mass_err < 0.0001,
            "Serpentine sandbox leaked sand mass under gravity! Init={:.4}, Final={:.4}, Error={:.6}",
            initial_mass,
            final_mass,
            mass_err
        );
    }

    #[test]
    fn test_quantile_mode_off_by_default_and_costs_nothing() {
        let sim = DrawingSimulation::new();
        assert_eq!(sim.quantile_mode, QuantileMode::Off);
        assert!(sim.quantile_positions().is_empty());
    }

    #[test]
    fn test_quantile_positions_stay_empty_while_off_during_hourglass_run() {
        let mut sim = DrawingSimulation::new();
        sim.sandbox_shape = SandboxShape::Hourglass;
        sim.gravity_dir = Vec2::new(0.0, 0.04);
        sim.initialize_hourglass();

        // 20 ticks is plenty: the assertion is only that the mode gate never fires, and the
        // refresh is scheduled every 5 ticks, so this still covers several would-be refreshes.
        let targets = [None; 5];
        for _ in 0..20 {
            sim.update(0.016, &targets, 0.08, MaterialMode::DrySand, SandboxShape::Hourglass, 16.0, 16.0);
        }

        // Never opted in: no positions should ever be computed.
        assert!(sim.quantile_positions().is_empty());
    }

    #[test]
    fn test_quantile_lines_descend_as_hourglass_drains() {
        let mut sim = DrawingSimulation::new();
        sim.sandbox_shape = SandboxShape::Hourglass;
        sim.gravity_dir = Vec2::new(0.0, 0.04);
        sim.initialize_hourglass();
        sim.set_quantile_mode(QuantileMode::Quartiles);

        // Immediately after init (all mass in the upper chamber), the median line should be
        // some finite position sitting up in the top half of the grid.
        let initial = sim.quantile_positions().to_vec();
        assert_eq!(initial.len(), 3);
        for &p in &initial {
            assert!(p.is_finite() && (0.0..=1.0).contains(&p));
        }
        // Ordered ascending (25% above 50% above 75%, all descending together over time).
        assert!(initial[0] <= initial[1] && initial[1] <= initial[2]);

        let targets = [None; 5];
        for _ in 0..200 {
            sim.update(0.016, &targets, 0.08, MaterialMode::DrySand, SandboxShape::Hourglass, 16.0, 16.0);
        }

        let later = sim.quantile_positions().to_vec();
        assert_eq!(later.len(), 3);
        for &p in &later {
            assert!(p.is_finite() && (0.0..=1.0).contains(&p));
        }
        assert!(later[0] <= later[1] && later[1] <= later[2]);

        // As sand drains from the upper chamber into the lower one, every quantile line should
        // have moved further down the grid (larger normalised position) — mesmerizing descent,
        // not sideways drift or staying put.
        for (i, (&before, &after)) in initial.iter().zip(later.iter()).enumerate() {
            assert!(
                after > before,
                "quantile line {} should have descended: before={}, after={}",
                i,
                before,
                after
            );
        }
    }

    #[test]
    fn test_set_quantile_mode_off_clears_targets() {
        let mut sim = DrawingSimulation::new();
        sim.sandbox_shape = SandboxShape::Hourglass;
        sim.gravity_dir = Vec2::new(0.0, 0.04);
        sim.initialize_hourglass();
        sim.set_quantile_mode(QuantileMode::Deciles);
        assert_eq!(sim.quantile_positions().len(), 9);

        sim.set_quantile_mode(QuantileMode::Off);
        assert!(sim.quantile_positions().is_empty());
    }

    /// The per-frame `sync_cell_colors_u8_dirty` only regenerates the `u8` colour view for blocks
    /// the tick could have touched, on the argument (spelled out on that function) that
    /// `active_blocks[b] != Inactive || last_displacements[b] > 0` is a superset of the blocks
    /// `settle_tick` marked `modified`. That argument reaches across two files and would rot
    /// silently — a missed block shows up as stale colours on screen, not as a failing assert
    /// anywhere else in the suite — so pin it directly: after every tick, the narrow sync must
    /// leave `cell_colors` byte-identical to what the full regeneration would have produced.
    ///
    /// Run for both a granular and a liquid material, and through the full drain lifecycle
    /// (upper chamber settling, streaming through the neck, pooling below), because the three
    /// stages exercise very different dirty-block patterns.
    #[test]
    fn test_color_dither_is_unbiased_and_stable() {
        // strength 0 must be a plain round — the pre-dither behaviour, bit for bit.
        for (v, want) in [(0.0f32, 0u8), (127.4, 127), (127.5, 128), (255.0, 255)] {
            assert_eq!(DrawingSimulation::quantize_color(v, 12345, 0.0), want);
        }

        // Dithering must be unbiased: a constant value quantised across many cells should
        // average back to that value, rather than all snapping to the same level. This is the
        // property that stops slow sub-LSB drift being erased, and it is what makes a gradient
        // speckle instead of band.
        for value in [10.3f32, 128.5, 200.75] {
            let n = 20_000usize;
            let sum: f64 = (0..n)
                .map(|i| DrawingSimulation::quantize_color(value, i, 1.0) as f64)
                .sum();
            let mean = sum / n as f64;
            assert!(
                (mean - value as f64).abs() < 0.05,
                "dither biased at {}: mean {:.4}",
                value,
                mean
            );

            // ...and it must actually spread, not just round consistently.
            let distinct = (0..256usize)
                .map(|i| DrawingSimulation::quantize_color(value, i, 1.0))
                .collect::<std::collections::HashSet<_>>();
            assert!(distinct.len() > 1, "dither produced no speckle at {}", value);
        }

        // Stability: the pattern is a pure function of the buffer index, never of time. If this
        // ever became frame-varying, a still image would crawl and — because the per-frame sync
        // only reconverts changed blocks — the 32x32 block grid would appear as visible seams.
        for i in [0usize, 1, 999, 1_048_575] {
            let a = DrawingSimulation::quantize_color(77.6, i, 2.0);
            let b = DrawingSimulation::quantize_color(77.6, i, 2.0);
            assert_eq!(a, b, "dither not stable at index {}", i);
        }

        // Must stay in range at the rails even with a wide dither.
        for i in 0..512usize {
            let lo = DrawingSimulation::quantize_color(0.0, i, 4.0);
            let hi = DrawingSimulation::quantize_color(255.0, i, 4.0);
            assert!(lo <= 2, "underflowed at index {}: {}", i, lo);
            assert!(hi >= 253, "overflowed at index {}: {}", i, hi);
        }
    }

    #[test]
    fn test_dirty_color_sync_matches_full_sync() {
        for material in [MaterialMode::DrySand, MaterialMode::Water] {
            let mut sim = DrawingSimulation::new();
            sim.sandbox_shape = SandboxShape::Hourglass;
            sim.gravity_dir = Vec2::new(0.0, 0.04);
            sim.apply_preset(material);
            sim.initialize_hourglass();

            let targets = [None; 5];
            for tick in 0..300 {
                sim.update(
                    0.016,
                    &targets,
                    0.08,
                    material,
                    SandboxShape::Hourglass,
                    16.0,
                    16.0,
                );

                let narrow = sim.cell_colors.clone();
                sim.sync_cell_colors_u8();
                let mismatches = narrow
                    .iter()
                    .zip(sim.cell_colors.iter())
                    .filter(|(a, b)| a != b)
                    .count();
                assert_eq!(
                    mismatches, 0,
                    "{:?} tick {}: dirty colour sync missed {} bytes that a full sync would have written",
                    material, tick, mismatches
                );
            }
        }
    }
}
