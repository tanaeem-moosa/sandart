//! A precomputed noise table standing in for kernel R/A/B's per-edge/per-cell integer hashes
//! (task rule 3: "randomness does not need to come from hashes... a noise buffer filled per pass
//! by a vectorizable generator... or a fixed table read at a per-pass random offset, as a
//! contiguous slice read, NOT a gather").
//!
//! **Generation.** `Noise::new` fills a `NOISE_LEN`-entry table ONCE (at `Scratch::new` time, not
//! per pass) by running the SAME 5-round avalanche mix `scalar_math::hash_u01` uses, but over the
//! table INDEX rather than an edge key -- i.e. exactly the kind of "hash of a running index into a
//! flat buffer" loop that has no cross-iteration data dependency and is in principle
//! auto-vectorizable with integer SIMD lanes (see the report for whether rustc's wasm32 backend
//! actually took that up).
//!
//! **Per-row, per-draw-type offsets, not one global offset.** A single offset shared by every row
//! would make every row read the IDENTICAL slice of the table -- since none of `dispersion`,
//! `lock`, `edge_share_jitter`'s jitter or the colour entropy vary with anything but
//! `(time_seed, edge/cell position)` in this benchmark (real `time_seed` is fixed per snapshot;
//! see `lib.rs`), that would print the same noise value at the same LOCAL edge index `e` on every
//! row: a perfect vertical (lag-1-in-y) correlation, i.e. visible horizontal banding. Folding `y`
//! into the offset hash (`row_offset`) below gives each row (and each of the four draw types) its
//! own pseudo-random window into the table, so lag-1/lag-2 correlation in BOTH x (from the table's
//! own avalanche-mixed contents) and y (from the per-row offset) should be near zero -- reported
//! in the equivalence/RNG section, not assumed.
//!
//! **Distribution matching.** The table itself holds a full-resolution uniform-in-`[0,1)` draw
//! (same 24-bit mantissa precision as `scalar_math::hash_u01`). Callers that need a coarser
//! quantisation (`dispersion_roll`'s 8-bit `& 0xFF`, `lock_roll`'s 16-bit `& 0xFFFF`) requantise a
//! table read at the use site (`kernel_c.rs`) rather than storing multiple tables -- requantising
//! a fine uniform value to N bits reproduces the same coarse uniform distribution the original
//! hash's narrower mask produced.

extern crate alloc;
use alloc::vec;
use alloc::vec::Vec;

/// 64Ki entries: comfortably larger than any span (`w` is at most `REFERENCE_GRID_HEIGHT` =
/// 512 today) times 4 (the colour-entropy draw needs 4 contiguous entries per cell), with room to
/// spare for the per-row offset to actually move.
pub const NOISE_LEN: usize = 1 << 16;

pub const SALT_DISPERSION: u32 = 0x1000_0001;
pub const SALT_LOCK: u32 = 0x1000_0002;
pub const SALT_JITTER: u32 = 0x1000_0003;
pub const SALT_COLOR: u32 = 0x1000_0004;

#[inline]
fn hash_u32(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^= x >> 16;
    x
}

#[inline]
fn hash_u01(x: u32) -> f32 {
    (hash_u32(x) >> 8) as f32 / 16_777_216.0
}

pub struct Noise {
    table: Vec<f32>,
    time_seed: u32,
}

impl Noise {
    /// Fills the table. Timed separately in the bench harness alongside `precompute_head_static`
    /// -- both are one-time, per-snapshot-load costs, not paid by `run_pass`.
    pub fn new(time_seed: u32) -> Self {
        let mut table = vec![0.0f32; NOISE_LEN];
        for (i, slot) in table.iter_mut().enumerate() {
            *slot = hash_u01(time_seed.wrapping_mul(0x2545_F491) ^ (i as u32).wrapping_mul(0x9E37_79B1));
        }
        Noise { table, time_seed }
    }

    /// A pseudo-random start offset for row `y`'s draw of type `salt`, leaving room for `span_len`
    /// contiguous reads (or `4 * span_len` for the colour draw -- pass that in as `span_len`).
    #[inline]
    pub fn row_offset(&self, y: usize, salt: u32, span_len: usize) -> usize {
        let limit = NOISE_LEN.saturating_sub(span_len).max(1);
        let h = hash_u32((self.time_seed ^ salt).wrapping_mul(0x2654_4353) ^ (y as u32).wrapping_mul(0x85EB_CA6B));
        (h as usize) % limit
    }

    #[inline]
    pub fn slice(&self, offset: usize, len: usize) -> &[f32] {
        &self.table[offset..offset + len]
    }
}
