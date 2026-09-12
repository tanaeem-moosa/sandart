//! Reader for the binary format `sandart-sim`'s `dump_kernel_bench_snapshots` (an `#[ignore]`d
//! test, see its doc comment in `physics.rs`) writes. Pure byte-slice parsing, no `std::fs` here
//! deliberately: this module is called from both the native binary (which does its own file I/O
//! and hands this the bytes) and, unchanged, from the wasm `cdylib` build (which has no
//! filesystem -- the node runner copies the file's bytes into wasm linear memory and this module
//! parses them from there). See `wasm_api.rs`.
//!
//! Format (little-endian), must match `physics.rs`'s doc comment on `dump_kernel_bench_snapshots`
//! exactly:
//!   magic: [u8; 4] = b"SKB1"
//!   w: u32, h: u32, block_size: u32, cols: u32, rows: u32
//!   time_seed: u32, tick_count: u32
//!   num_sim_blocks: u32, then that many u32 block indices
//!   shape_mask: w*h bytes
//!   heights: w*h f32
//!   cell_props: w*h*4 f32
//!   cell_colors: w*h*4 u8
//!   edge_vel_h: w*h f32
//!   edge_vel_v: w*h f32
//!   column_depth: w*h f32

extern crate alloc;
use alloc::vec::Vec;

pub struct State {
    pub w: usize,
    pub h: usize,
    pub block_size: usize,
    pub cols: usize,
    pub rows: usize,
    pub time_seed: u32,
    pub tick_count: u32,
    /// Block indices `settle_tick`'s own classification marked simulated on the tick this
    /// snapshot was taken from (`active_blocks[b] != Inactive`). Every kernel iterates exactly
    /// this fixed list, on every repeated pass -- see `lib.rs`'s module doc comment for why the
    /// block SET does not change as heights evolve under repeated application.
    pub sim_blocks: Vec<u32>,
    pub shape_mask: Vec<u8>,
    pub heights: Vec<f32>,
    pub cell_props: Vec<f32>,
    pub cell_colors: Vec<u8>,
    pub edge_vel_h: Vec<f32>,
    pub edge_vel_v: Vec<f32>,
    pub column_depth: Vec<f32>,
}

struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn u32(&mut self) -> u32 {
        let v = u32::from_le_bytes(self.bytes[self.pos..self.pos + 4].try_into().unwrap());
        self.pos += 4;
        v
    }
    fn u8_slice(&mut self, n: usize) -> Vec<u8> {
        let v = self.bytes[self.pos..self.pos + n].to_vec();
        self.pos += n;
        v
    }
    fn f32_slice(&mut self, n: usize) -> Vec<f32> {
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            let off = self.pos + i * 4;
            out.push(f32::from_le_bytes(self.bytes[off..off + 4].try_into().unwrap()));
        }
        self.pos += n * 4;
        out
    }
}

pub fn parse(bytes: &[u8]) -> State {
    parse_at(bytes).0
}

/// Same as `parse`, but also returns how many bytes were consumed -- used by
/// `native_bench`'s validation path to read the trailer `dump_kernel_bench_snapshots` appends
/// after an `SKB1`-shaped prefix in `validation_snapshot.bin`.
pub fn parse_at(bytes: &[u8]) -> (State, usize) {
    assert!(&bytes[0..4] == b"SKB1", "bad snapshot magic");
    let mut c = Cursor { bytes, pos: 4 };
    let w = c.u32() as usize;
    let h = c.u32() as usize;
    let block_size = c.u32() as usize;
    let cols = c.u32() as usize;
    let rows = c.u32() as usize;
    let time_seed = c.u32();
    let tick_count = c.u32();
    let n_blocks = c.u32() as usize;
    let mut sim_blocks = Vec::with_capacity(n_blocks);
    for _ in 0..n_blocks {
        sim_blocks.push(c.u32());
    }
    let shape_mask = c.u8_slice(w * h);
    let heights = c.f32_slice(w * h);
    let cell_props = c.f32_slice(w * h * 4);
    let cell_colors = c.u8_slice(w * h * 4);
    let edge_vel_h = c.f32_slice(w * h);
    let edge_vel_v = c.f32_slice(w * h);
    let column_depth = c.f32_slice(w * h);
    (
        State {
            w,
            h,
            block_size,
            cols,
            rows,
            time_seed,
            tick_count,
            sim_blocks,
            shape_mask,
            heights,
            cell_props,
            cell_colors,
            edge_vel_h,
            edge_vel_v,
            column_depth,
        },
        c.pos,
    )
}
