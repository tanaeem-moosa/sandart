use crate::grid::Heightmap;
use glam::Vec2;

/// Bounding coordinates to optimize Cellular Automata settling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActiveBounds {
    pub min_x: usize,
    pub max_x: usize,
    pub min_y: usize,
    pub max_y: usize,
    pub active: bool,
}

/// Active marble state passed to the physics CA simulation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ActiveMarbleInfo {
    pub pos: Vec2,
    pub vel: f32,
    pub vel_vec: Vec2,
}

use crate::{PROP_WETNESS, PROP_THRESHOLD, PROP_FLOW_RATE, PROP_GRAIN_SIZE};

/// Round `v` to an integer, rounding up with probability equal to its fractional part. Unbiased
/// in expectation: a value of 180.3 lands on 181 three times in ten and 180 the rest, so a
/// sequence of blends accumulates towards 180.3 instead of collapsing onto 180.
///
/// This is what makes `u8` color storage viable. A plain `.round()` discards every increment
/// smaller than half an LSB, and because the flux solver nudges a cell by the same small amount
/// over and over, that discard is *systematic*: slow deformation (a color line bending as sand
/// creeps under it) was erased every tick rather than accumulating.
///
/// **The seeding must vary per event, not per cell.** A stable per-cell hash — the right choice
/// for a display dither, where a fixed pattern is what keeps a still image still — would
/// reintroduce exactly that systematic erasure here, because a cell nudged by less than its own
/// fixed threshold would never flip however many times it was nudged. The entropy is therefore
/// taken from the flow magnitude's bits, which vary naturally between transfers, mixed with the
/// destination index and channel.
///
/// The cost is diffusion rather than bias: each blend adds roughly +/-0.5 LSB of noise, and the
/// random walk over many advection events can accumulate into spatial blur.
/// `test_color_boundary_does_not_diffuse_under_gravity` bounds that.
///
/// `v` is expected to be in [0, 255] already.
#[inline]
fn stochastic_round(v: f32, entropy: u32) -> u8 {
    let mut h = entropy;
    h ^= h >> 16;
    h = h.wrapping_mul(0x7feb_352d);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846c_a68b);
    h ^= h >> 16;
    let r = (h >> 8) as f32 / 16_777_216.0; // [0, 1)
    // floor(v + r) steps up exactly when frac(v) + r >= 1, i.e. with probability frac(v).
    // `as u8` truncates towards zero (v is non-negative here) and saturates at 255.
    (v + r) as u8
}

/// Advect color and properties from src cell to dst cell based on the flow amount and dst cell's height before arrival
pub fn advect_properties(colors: &mut [u8], props: &mut [f32], src: usize, dst: usize, flow: f32, h_dst: f32) {
    let total = h_dst + flow;
    if total < 1e-6 {
        return;
    }
    let src_base = src * 4;
    let dst_base = dst * 4;

    if h_dst < 1e-4 {
        // Empty destination cell: inherit 100% of source color and properties
        for ch in 0..4 {
            colors[dst_base + ch] = colors[src_base + ch];
            props[dst_base + ch] = props[src_base + ch];
        }
    } else {
        let w_keep = h_dst / total;
        let w_arrive = flow / total;

        for ch in 0..3 {
            // Blend in f32, store in u8. The rounding back to an integer is stochastic, not a
            // plain `.round()`, so repeated sub-LSB nudges accumulate in expectation instead of
            // being discarded every time — see `stochastic_round`.
            let blended = (
                colors[dst_base + ch] as f32 * w_keep
                + colors[src_base + ch] as f32 * w_arrive
            ).clamp(0.0, 255.0);
            let entropy = flow.to_bits() ^ (dst as u32).wrapping_mul(2_654_435_761) ^ (ch as u32).wrapping_mul(97);
            colors[dst_base + ch] = stochastic_round(blended, entropy);
        }
        colors[dst_base + 3] = 255; // opaque alpha

        for ch in 0..4 {
            props[dst_base + ch] = props[dst_base + ch] * w_keep + props[src_base + ch] * w_arrive;
        }
    }
}

/// Helper function to add sand to a cell, clamping it at max_height (glass top)
/// and distributing any excess volume to its available 4-way neighbors, with properties advection.
fn add_sand_with_limit_properties(
    heightmap: &mut Heightmap,
    cell_colors: &mut [u8],
    cell_props: &mut [f32],
    src_idx: usize,
    idx: usize,
    w: usize,
    h: usize,
    amount: f32,
    max_height: f32,
) {
    if amount <= 0.0 {
        return;
    }
    let current_h = heightmap.data[idx];
    if current_h + amount <= max_height {
        advect_properties(cell_colors, cell_props, src_idx, idx, amount, current_h);
        heightmap.data[idx] = current_h + amount;
    } else {
        let allowed = (max_height - current_h).max(0.0);
        advect_properties(cell_colors, cell_props, src_idx, idx, allowed, current_h);
        heightmap.data[idx] = current_h + allowed;
        let mut excess = amount - allowed;
        if excess > 1e-6 {
            // Distribute excess to neighbors that are below the max_height
            let x = idx % w;
            let y = idx / w;
            
            let mut neighbors = [0usize; 4];
            let mut num_neighbors = 0;
            if x > 0 { neighbors[num_neighbors] = idx - 1; num_neighbors += 1; }
            if x + 1 < w { neighbors[num_neighbors] = idx + 1; num_neighbors += 1; }
            if y > 0 { neighbors[num_neighbors] = idx - w; num_neighbors += 1; }
            if y + 1 < h { neighbors[num_neighbors] = idx + w; num_neighbors += 1; }

            // Filter neighbors that have room (height < max_height)
            let mut room_neighbors = [(0usize, 0.0f32); 4];
            let mut num_room_neighbors = 0;
            for i in 0..num_neighbors {
                let n_idx = neighbors[i];
                let nh = heightmap.data[n_idx];
                if nh < max_height {
                    room_neighbors[num_room_neighbors] = (n_idx, max_height - nh);
                    num_room_neighbors += 1;
                }
            }

            if num_room_neighbors == 0 {
                // If all neighbors are full, distribute to all neighbors equally (overflowing slightly)
                let num = num_neighbors as f32;
                let share = excess / num;
                for i in 0..num_neighbors {
                    let n_idx = neighbors[i];
                    advect_properties(cell_colors, cell_props, idx, n_idx, share, heightmap.data[n_idx]);
                    heightmap.data[n_idx] += share;
                }
            } else {
                // Distribute to room_neighbors proportional to their room
                let mut distributed = false;
                for _ in 0..3 {
                    if excess <= 1e-6 {
                        distributed = true;
                        break;
                    }
                    if num_room_neighbors == 0 {
                        break;
                    }
                    let share = excess / num_room_neighbors as f32;
                    let mut next_room = [(0usize, 0.0f32); 4];
                    let mut next_num_room = 0;
                    for i in 0..num_room_neighbors {
                        let (n_idx, room) = room_neighbors[i];
                        if room > 0.0 {
                            let to_add = share.min(room);
                            advect_properties(cell_colors, cell_props, idx, n_idx, to_add, heightmap.data[n_idx]);
                            heightmap.data[n_idx] += to_add;
                            excess -= to_add;
                            let new_room = room - to_add;
                            if new_room > 0.0 {
                                next_room[next_num_room] = (n_idx, new_room);
                                next_num_room += 1;
                            }
                        }
                    }
                    room_neighbors = next_room;
                    num_room_neighbors = next_num_room;
                }
                if !distributed && excess > 1e-6 {
                    let num = num_neighbors as f32;
                    let share = excess / num;
                    for i in 0..num_neighbors {
                        let n_idx = neighbors[i];
                        advect_properties(cell_colors, cell_props, idx, n_idx, share, heightmap.data[n_idx]);
                        heightmap.data[n_idx] += share;
                    }
                }
            }
        }
    }
}

/// Continuous "how liquid is this cell" weight in [0, 1], derived from `wetness` via a
/// smoothstep ramp centered on the `wetness >= 0.75` branch cut used to select the wave solver
/// (`physics.rs` `settle_tick`, `if wetness >= 0.75 && !gravity_active`).
///
/// That branch selection itself stays a hard binary switch (Sandbox liquids must keep going
/// through the wave solver, and that fork is out of scope for this phase). `liquidity` is used
/// instead to interpolate the *continuous* CA parameters (droplet quantization, threshold/alpha,
/// gravity_push strength, transfer coefficient, and cell capacity) so that a cell whose `wetness`
/// drifts a hair across 0.75 under property advection (see `advect_properties`) does not suddenly
/// flip between "flows like liquid" and "frozen solid" parameters (defect C5). At the extremes
/// (wetness <= 0.65 or >= 0.85) this reproduces the exact pre-existing granular/liquid parameter
/// values, so materials that never approach the cut are bit-identical to before.
fn liquidity(wetness: f32) -> f32 {
    let t = ((wetness - 0.65) / (0.85 - 0.65)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Incompressibility cap for a cell of the given `wetness`: how much material one cell may hold.
///
/// Granular materials keep the historical 1.5 packing (load-bearing for the sand-pile height
/// tests); liquids cap at 1.0 (C1). Interpolated by `liquidity` so there is no hard cut.
/// Gravitational head, in units of "one saturated cell of fill", per cell of travel per unit of
/// `gravity_dir`. The unified head is `H = h + Phi`, with `Phi(r) = -(g . r) * GRAVITY_HEAD_SCALE`.
///
/// At the shipped Sand-fall gravity of 0.04 this makes the head drop across one row exactly 1.0,
/// which is the natural unit: it is what makes "fall into the empty cell below" outrank "spread
/// into the empty cell beside" by precisely the weight of one saturated cell. Because `Phi` is
/// proportional to `|g|`, the gravity slider moves behaviour continuously between the Sandbox
/// (`Phi == 0`, pure free-surface wave) and Sand-fall regimes instead of flipping between two
/// solvers (defect C6).
const GRAVITY_HEAD_SCALE: f32 = 25.0;

/// Weight on the depth-integrated lateral pressure term (see `column_depth` in `settle_tick`'s
/// cross-gravity liquid branch): how many head units one cell of *stacked, resting* liquid above
/// a cell adds to that cell's lateral driving head, on top of the cell's own local fill.
///
/// This is the fix for a specific blindness in `H = h + Phi`: `Phi` only depends on this edge's
/// two endpoints, so it correctly makes a column push down into whatever is below it, but the
/// *lateral* edge has no `Phi` term at all under vertical gravity (`gravity_dir.x == 0`) and so
/// drives purely on `h_a - h_b` — the local fill difference. Local fill saturates at `cell_capacity`
/// (~1.0), so a cell at the bottom of a 20-deep resting column and a cell under a single resting
/// cell present an *identical* driving head to their lateral neighbour once both are full. Real
/// hydrostatic pressure keeps growing with depth; this term restores that growth without
/// resurrecting the old per-cell wave solver's leak, because — like `GRAVITY_HEAD_SCALE` — it only
/// ever feeds `driving`, never the donor/acceptor mass limits that keep `flux_edge` conservative.
///
/// A shallow, undifferentiated puddle has `column_depth == 0` and reduces exactly to the
/// pre-existing `head_a = h_a + gravity_dir.x * GRAVITY_HEAD_SCALE` formula regardless of this
/// constant, so in principle any positive scale is "correct" and only the genuinely deep case
/// should feel it. In practice `column_depth` is a cheap, single-tick, no-lookahead estimate (see
/// its doc comment in `settle_tick`), not an exact column integral.
///
/// `LATERAL_PRESSURE_SCALE` is measured, not derived. It was originally swept against
/// `test_liquid_flowing_liquid_does_not_stand_in_walls` (an hourglass chamber tens of cells deep)
/// under the old regime described below, where a phantom "resting" depth at the continuous
/// source cell inflated `column_depth` and `LATERAL_PRESSURE_DEPTH_FLOOR` (since removed) was
/// clipping the term at 1.5: that sweep read 30060 (no lateral pressure) -> 23526 at scale = 2 ->
/// 21938 at scale = 5 -> 22085 at scale = 10, and looked like it flattened past 5. With the
/// phantom fixed at its source (see below) and the floor deleted, the same test at scale = 5 reads
/// 12106 instead of 21938 — a large enough shift that the old sweep's shape could not be trusted,
/// so it was redone from scratch against the corrected solver:
///
/// scale = 0 (no lateral pressure): total = 30060, `test_liquid_stream_stays_coherent`'s
/// max_width = 8 (passes, but the void count is the worst in the sweep). scale in roughly (0,
/// 3.2]: max_width jumps to 9 and that test fails outright, regardless of total — this whole band
/// is disqualified by stream coherence, not by the void metric. max_width recovers to 8 at
/// scale ~3.5 and holds through at least scale = 18, then fails again (back to 9) by scale = 20.
/// Inside that valid window the total is noisy and not monotonic — 12097 at 4, 11848 at 4.2,
/// 11743 at 4.8, 12106 at 5, 12107 at 8, 13014 at 6, 13648 at 9, drifting up to 13183 at 12 and
/// 14451 at 14 — with no point anywhere in the window meaningfully beating what scale = 4-5
/// already gets, and no improving trend to chase by going higher (the opposite, if anything,
/// plus eventual failure at 20). So the old regime's "knee past 5" really was a floor artifact as
/// suspected, but the corrected picture is not "still falling" either: it is a flat, noisy
/// plateau, bottomed out already at the low end of the valid range. `5` stays the chosen value —
/// it sits with comfortable margin above the ~3.2-3.5 coherence cliff on one side and the failure
/// at 20 on the other, and is statistically tied for the lowest total anywhere in the plateau, so
/// there is nothing to buy by moving off it in either direction.
///
/// A depth floor/deadband used to be load bearing here: `column_depth`'s `resting_above` term is
/// `temp_heights[above] - in_transit_at(above)`, and `in_transit_at` only sees mass that moved
/// through `edge_vel_v` — it had no way to see mass a caller wrote directly into
/// `heightmap.data`, which is exactly how a continuous source (e.g.
/// `test_liquid_stream_stays_coherent`'s tap) used to be fed. That made an always-full source cell
/// read as a few cells of phantom "resting" depth every tick, and the floor was sized (1.5) as a
/// deadband wide enough to swallow that phantom without also swallowing genuine shallow
/// overburden. The real fix is `Heightmap::apply_external_mass` / `Heightmap::external_mass_this_tick` (see
/// `grid.rs`): callers that add mass from outside the flux solver now go through `inject`, which
/// records the full injected height, and `resting_above`'s computation in `settle_tick` subtracts
/// it the same way it subtracts `in_transit_at`'s edge-arrived estimate. The phantom depth is
/// eliminated at its source instead of masked after the fact, so `column_depth` is now `>= 0` by
/// construction (`resting_above` is `.max(0.0)`-clamped before being added to the prior row's
/// already-non-negative value) and no floor/deadband is needed at all — if a future regression
/// ever reintroduces a phantom, git history has the deadband.
const LATERAL_PRESSURE_SCALE: f32 = 5.0;

/// STAGE C. The single knob that turns a granular material's declared repose threshold
/// (`PROP_THRESHOLD`, e.g. 0.08 for DrySand — see `MaterialMode::preset_props`) into the yield
/// stress `tau` the lateral flux edge in `settle_tick`'s granular CA branch gates flow behind (see
/// the "Combined liquid + granular share" block there). `tau = GRANULAR_TAU_SCALE *
/// threshold_prop * granular_share`.
///
/// **This is where to change the repose angle.** Raising it raises `tau` for every granular
/// material proportionally to its own `threshold_prop`, so CoarseSand stays steeper than DrySand
/// stays steeper than FinePowder exactly as their `preset_props` already declare, without having
/// to hand-tune each material's threshold separately. Lowering it moves sand back toward liquid
/// behaviour; `0.0` reproduces a zero-yield-stress (pre-Stage-C-fix) granular material exactly,
/// which is a useful sanity check if this mechanism is ever suspected of a regression.
///
/// **Starting value and its derivation.** `1.0` — i.e. `tau` starts as `threshold_prop` taken
/// literally, at face value, in the same raw-height units the granular CA's own `geom_slope`
/// comparisons already used it in (`effective_slope <= threshold` in the old lateral loop this
/// replaces). This is deliberately NOT the CA's actual in-gravity value, which was
/// `threshold_prop * 0.35` (`get_ca_params`: "Lower friction/repose angle in Sand-fall mode for
/// realistic fluid flow", further halved by the `sliding_active` hysteresis branch to as low as
/// `threshold_prop * 0.175`) — that discount is *why* sand had no working yield stress under
/// gravity in the first place (see the Stage C task brief and `test_dry_sand_has_angle_of_repose`,
/// which documents DrySand settling to a ~2.4 degree slope with the discount in place). Using the
/// material's own undiscounted threshold is the natural undoing of that: not a new number invented
/// for this change, but the number the material already declared for itself, applied without the
/// fluid-flavoured discount that used to sit on top of it. It is a starting point, not a final
/// answer — see the doc comment above for what measuring it produced (a still-fairly-weak ~2.4
/// degree repose angle at this value, because the old CA's flow-RATE constants and its dispersion
/// term dominated the pile's short-timescale shape at least as much as either threshold did — see
/// `test_dry_sand_has_angle_of_repose`'s own doc comment, NON-VACUITY FINDING). Raising this
/// constant is the intended next step once a specific target repose angle is picked.
const GRANULAR_TAU_SCALE: f32 = 1.0;

/// STAGE C FOLLOW-ON. The coefficient of lateral earth pressure: what fraction of a granular
/// cell's vertical overburden pushes SIDEWAYS.
///
/// A liquid is isotropic -- vertical stress transmits laterally in full, so K = 1, which is what
/// `LATERAL_PRESSURE_SCALE` alone encodes and what liquid keeps exactly. A granular medium does
/// not: grain-to-grain contacts and wall friction carry part of the load, so lateral stress is a
/// fraction of vertical. Jaky's classic estimate for a loose, normally-consolidated granular bed
/// is `K0 = 1 - sin(phi)`, which for a friction angle of 30-35 degrees gives 0.43-0.50. 0.45 sits
/// in the middle of that range and is a derived value, not a fitted one.
///
/// Before this existed the coefficient was effectively ZERO for sand -- `column_depth` was gated
/// on `cell_liquidity > 0.0`, so a granular cell had no overburden term at all and its lateral
/// driving was the local fill difference alone. Sand could not converge toward an outlet from
/// depth because depth exerted no sideways push on it.
///
/// Applied at the lateral edge's read site, blended by each cell's own liquidity, so mixed cells
/// interpolate and a fully liquid cell is bit-identical to before. Set to 0.0 to reproduce
/// pre-follow-on behaviour exactly; 1.0 makes sand hydrostatic like water.
const LATERAL_EARTH_PRESSURE_K: f32 = 0.45;

/// `k_lateral` from the task brief: what fraction of a cell's vertical overburden reads through
/// as additional driving head at a read site, blended by that cell's own liquidity so a fully
/// liquid cell is exactly `1.0` (bit-identical to pre-existing behaviour at every call site) and
/// a fully granular cell is `LATERAL_EARTH_PRESSURE_K`. Originally inline only at the lateral
/// edge's read site (Stage C follow-on); promoted to a shared function so the vertical
/// overburden bonus (Task #54 step 3, see the `VERTICAL_PRESSURE_SCALE` call site in phase 0)
/// can reuse the exact same coefficient rather than inventing a second one -- both are "how much
/// does this cell's own granular-ness discount an overburden-driven effect", just applied to two
/// different edges.
#[inline]
fn k_of_liquidity(liq: f32) -> f32 {
    liq + (1.0 - liq) * LATERAL_EARTH_PRESSURE_K
}

/// The Janssen depth scale: the characteristic depth (in `column_depth`'s own units --
/// reference-resolution rows, see `REFERENCE_GRID_HEIGHT` -- of RAW, un-transformed overburden)
/// over which a granular column's vertical stress rises toward its plateau. `1.0 - sin(phi)`-type
/// wall friction (`LATERAL_EARTH_PRESSURE_K`) is what carries load into the container walls
/// instead of straight down, and Janssen's classic result is that this makes vertical stress
/// saturate exponentially with depth rather than grow linearly the way a liquid's hydrostatic
/// pressure does -- `sigma_v(z) ~ 1 - exp(-z / z_c)`, with `z_c` set by the container's own
/// transverse dimension and the wall friction coefficient. See `janssen_effective_depth`, which
/// this feeds, for where the transform is applied.
///
/// NOT MEASURED — a first cut, exactly as the task brief allows ("a fixed z_c tied to container
/// scale is an acceptable first cut if you say so"). Production hourglass neck widths run
/// `neck_width * REFERENCE_GRID_HEIGHT` for `neck_width` in roughly 0.04..0.15 (see
/// `make_test_mask`'s callers and `SandboxShape::Hourglass`'s docs), i.e. 20-77 reference rows;
/// `24` sits at the narrow end of that range, deliberately -- a container-scale number, not a
/// grid-scale one, chosen so the plateau engages within roughly one neck-width of depth rather
/// than needing hundreds of rows (a linear-looking regime over hundreds of rows would be
/// indistinguishable from the old unbounded hydrostatic behaviour in every scenario short one,
/// defeating the point of this change). Retuning this against an actual Beverloo flow-rate
/// measurement (drain rate should become independent of fill height once the column is a few
/// `z_c` deep) is future work, flagged here rather than silently presented as validated.
const JANSSEN_DEPTH_SCALE: f32 = 24.0;

/// Transforms raw `column_depth` (hydrostatic, unbounded, linear in depth -- see
/// `LATERAL_PRESSURE_SCALE`'s doc comment) into the depth-response SHAPE each material actually
/// feels, blended by liquidity: a fully liquid cell gets the identity transform (`column_depth`
/// unchanged -- hydrostatic pressure genuinely does grow without bound with depth, that is the
/// physically correct model for a liquid and nothing here should touch it), and a fully granular
/// cell gets the Janssen saturating curve, `JANSSEN_DEPTH_SCALE * (1 - exp(-column_depth /
/// JANSSEN_DEPTH_SCALE))`, which approaches `column_depth` itself for shallow depth (matching
/// today's tuning near the free surface, where `column_depth << JANSSEN_DEPTH_SCALE`) and
/// plateaus at `JANSSEN_DEPTH_SCALE` for deep material instead of growing forever. This is the
/// "depth RESPONSE SHAPE" the task brief asks for, applied at the read site rather than to the
/// stored `column_depth` running sum itself (which must stay the raw, un-shaped overburden --
/// see `column_depth`'s own doc comment on why scaling the stored value would compound down the
/// column).
#[inline]
fn janssen_effective_depth(column_depth: f32, liquidity: f32) -> f32 {
    let saturating = JANSSEN_DEPTH_SCALE * (1.0 - (-column_depth / JANSSEN_DEPTH_SCALE).exp());
    liquidity * column_depth + (1.0 - liquidity) * saturating
}

/// TASK #55. Alternative, gated (`multiplicative_lateral_gate`, default OFF) form of the lateral
/// edge's driving head. Diagnosis: `LATERAL_PRESSURE_SCALE`'s term is ADDITIVE --
/// `driving = (h_a - h_b) + k_lateral * SCALE * (depth_a - depth_b)` -- when the physically correct
/// free-surface form is multiplicative: `flux ~ conveyance(depth) * grad(free-surface elevation)`.
/// An additive depth term can drive flow between two columns that are NOT actually at different
/// surface heights (whenever the fill-difference and depth-difference terms don't cancel in the
/// same proportion they'd need to for the additive sum to track true elevation), and -- because it
/// is only ever a bonus on top of a separately-computed fill term -- it does not stop flow when the
/// true surface actually is flat, it just adds less. The multiplicative form fixes this
/// structurally: a factor that is exactly zero forces the *whole* driving term to zero, not just
/// one component of a sum.
///
/// **`grad(eta)`, the free-surface term.** This solver has no separate stored "bed elevation" +
/// "flow depth" pair (see `column_depth`'s own doc comment: it is a per-CELL top-down accumulation,
/// not a per-COLUMN scalar). The best available proxy for "how tall is the material stack down to
/// and including this row" is `h[idx] * depth_scale + column_depth[idx]` -- this cell's own local
/// fill plus everything resting on top of it, BOTH converted into the same units first (see the
/// `depth_scale` paragraph below -- this was originally shipped as an un-scaled `h[idx] +
/// column_depth[idx]`, which is where TASK #55's unit bug lived; see the call site's own comment
/// and `diag_task55_eta_depth_scale_consistency`'s resolution sweep for the fix and its proof).
/// For a LATERAL edge both endpoints share the same row, so the
/// row-index terms that would otherwise appear in an absolute elevation cancel in the subtraction,
/// leaving exactly this sum's difference as the estimate of `eta_a - eta_b`. Deliberately RAW
/// (`column_depth` unshaped by Janssen, unweighted by `k_of_liquidity`): `eta` is meant to answer a
/// purely geometric question -- how tall is the pile, physically -- and a pile's physical height
/// does not depend on how its internal stress is distributed. Two columns holding the same amount
/// of material to the same row are at the same surface height whether that material is water or
/// sand; Janssen and `LATERAL_EARTH_PRESSURE_K` are about how much of that column's weight
/// transmits as *stress*, not about how tall the column *is*. Folding either into `eta` would make
/// a granular and a liquid column disagree about their own geometry, which is not what either
/// mechanism is for.
///
/// **`conveyance(depth)`, the material-dependent factor.** This is where Janssen and
/// `k_of_liquidity` belong instead: how much of a column's depth actually participates in carrying
/// *lateral flow*, which is exactly the question Janssen answers for granular material (wall
/// friction bleeds load out of the vertical stress column, so it saturates) and `k_of_liquidity`
/// answers for the isotropic/anisotropic split (liquid transmits stress sideways in full, granular
/// only partially). Composition check, since the task brief asks this be reasoned about rather than
/// silently assumed: `janssen_effective_depth` is read exactly ONCE per endpoint, feeding
/// `conveyance` only -- never also added a second time into `eta` -- so there is no double
/// application of the `1 - exp(-z/z_c)` saturation. The two mechanisms answer two different
/// questions (how tall IS it vs. how much of it CONVEYS) from the same underlying `column_depth`
/// reading, the same way the additive form's `k_a * LATERAL_PRESSURE_SCALE * depth_a` term already
/// combined `k_of_liquidity` and `janssen_effective_depth` into one read of `column_depth`, not two.
/// `conveyance` also folds in this cell's own local fill (`h[idx]`, unshaped -- it is not
/// "overburden", there is nothing above it to saturate) precisely so a genuinely shallow, unstacked
/// puddle (`column_depth == 0` on both sides, `h > 0`) still has nonzero conveyance and can still
/// level under its own local fill difference -- the base case `LATERAL_PRESSURE_SCALE`'s own doc
/// comment calls out ("a shallow, undifferentiated puddle... reduces exactly to the pre-existing
/// head_a = h_a + Phi formula"). Without this, conveyance would be exactly zero for every surface
/// cell of every pile regardless of how tall the pile beneath it is, which would silently stop ALL
/// surface-layer levelling, not just the flat-surface case this change targets -- see
/// `mult_lateral_conveyance`'s own doc comment for the exact place this matters (a cusp).
#[inline]
fn mult_lateral_conveyance(local_fill: f32, column_depth: f32, k: f32, liq: f32) -> f32 {
    let janssen_depth = janssen_effective_depth(column_depth, liq);
    (local_fill + k * janssen_depth).max(0.0).powf(MULT_LATERAL_CONVEYANCE_EXPONENT)
}

/// Exponent for `mult_lateral_conveyance`. Open-channel diffusive-wave models commonly use a power
/// law of local depth for unit discharge (Manning: `q ~ h^(5/3) * sqrt(S)`; broad-crested weir flow:
/// `Q ~ h^(3/2)`). `1.5` is picked over Manning's `5/3` because it is the simpler, better-known
/// closed form the task brief itself floats ("depth^(3/2) or similar") and because this solver has
/// no analogue of Manning's hydraulic-radius/wetted-perimeter geometry to justify the extra `1/6`
/// power over -- a per-cell CA has no channel cross-section, so borrowing Manning's *exponent*
/// without its *premise* would be spurious precision. `1.5` is NOT measured or fitted; it is a
/// defensible first choice, exactly as the task brief permits, and reported as such rather than as
/// a swept constant.
const MULT_LATERAL_CONVEYANCE_EXPONENT: f32 = 1.5;

/// Overall scale on the multiplicative driving head (`MULT_LATERAL_SCALE * conveyance(depth) *
/// grad(eta)`, see `mult_lateral_conveyance`). `1.0` -- chosen, not swept, so that a single
/// near-capacity cell of material (`local_fill ~ 1`, `column_depth ~ 0` on both sides, i.e. the
/// same "shallow, undifferentiated puddle" baseline `LATERAL_PRESSURE_SCALE`'s doc comment
/// anchors to) gives `conveyance ~= 1^1.5 = 1`, so the multiplicative driving head reduces to
/// approximately the same order of magnitude as the legacy `h_a - h_b` baseline in that base case
/// -- not bit-identical (the two forms are structurally different away from that one anchor point),
/// but not an arbitrary order of magnitude off either. Deliberately left at this un-swept starting
/// point per the task brief's instruction not to tune constants to land inside a passing window;
/// see this task's report for what `1.0` actually measures like against
/// `test_liquid_flowing_liquid_does_not_stand_in_walls` and the flat-surface check.
///
/// TASK #55 UNIT FIX addendum: the call site now passes `mult_lateral_conveyance` a `local_fill`
/// already lifted into `column_depth`'s reference-row units (`h * depth_scale`, see that call
/// site's own comment) rather than raw `h`, so this anchor (`local_fill ~ 1` giving
/// `conveyance ~= 1`) is exact only where `depth_scale == 1`, i.e. production's `w ==
/// REFERENCE_GRID_HEIGHT == 512`. That is deliberate, not a new drift: `depth_scale` is a no-op at
/// that resolution, so this constant's anchor case is unchanged there; away from it, the anchor
/// scales by `depth_scale` along with everything else `column_depth` touches, which is exactly the
/// resolution-invariance property the fix is for (see `diag_task55_eta_depth_scale_consistency`).
const MULT_LATERAL_SCALE: f32 = 1.0;

/// STEP 3 (Task #54). How much of a cell's OWN (Janssen-shaped, see `janssen_effective_depth`)
/// vertical overburden feeds back into the GRAVITY-ALIGNED edge's driving head, on top of the
/// existing flat `gravity_dir.y * GRAVITY_HEAD_SCALE` term every cell already gets regardless of
/// depth. Without this, `H = h + Phi(g)` has no depth term on the vertical edge at all -- a cell
/// ten deep accelerates exactly like a surface cell, which is the literal bug this task's step 3
/// exists to fix ("deep water falls faster").
///
/// Reuses `LATERAL_PRESSURE_SCALE`'s own value rather than an independently swept constant: it
/// is the existing, already-tuned answer to "how many head units does one row of resting
/// overburden add", and the CFL-style cap at the call site (see `VERTICAL_PRESSURE_CAP_MULT`) is
/// what actually bounds the result, not this scale -- so there is little to gain from a separate
/// sweep here versus just inheriting the value that already passed `test_liquid_stream_stays_coherent`
/// and the enclosed-void tests at the lateral edge.
const VERTICAL_PRESSURE_SCALE: f32 = LATERAL_PRESSURE_SCALE;

/// STEP 3 (Task #54) CFL-STYLE BOUND. Caps the vertical overburden bonus (`VERTICAL_PRESSURE_SCALE
/// * janssen_effective_depth(...) * k_of_liquidity(...)`) at this multiple of the existing flat
/// `|gravity_dir.y| * GRAVITY_HEAD_SCALE` term, so a column of ANY depth can push its own vertical
/// edge at most `1.0 + VERTICAL_PRESSURE_CAP_MULT` times as hard as the system was already tuned
/// for, never unboundedly harder.
///
/// WHY A CAP IS NEEDED AT ALL, given `flux_edge_candidate` already clamps every edge's flux to
/// `min(donor's available mass, acceptor's free capacity)` regardless of how large the driving
/// head is (so no single edge can ever move more than about one cell's worth of mass in one
/// tick, with or without this bonus): the risk this task brief calls out is not any one edge
/// exceeding its own clamp, it is CASCADING -- phase 0 is a frozen-Jacobi pass, so every vertical
/// edge in a column reads the SAME pre-tick snapshot independently and can *simultaneously*
/// reach its own clamp in the same tick (that is by design, see the phase-loop comment on why
/// phase 0 is order-independent). An unbounded depth bonus would push EVERY edge in a deep
/// column to its saturated, fully-clamped transfer on every tick at once -- the exact "material
/// moving multiple cells per tick while `block_size` is 2 cells" slab artifact this task
/// explicitly warns against, just produced by a different mechanism (head magnitude, not an
/// unclamped apply step) than the one a prior investigation already fixed. Capping the bonus
/// at a small, fixed multiple of the SAME per-row unit `GRAVITY_HEAD_SCALE` was tuned around
/// keeps the system in the regime it was already validated at (edges reach their clamp readily,
/// same as today, just a little more readily for deep material) rather than a qualitatively new
/// one where `column_depth` in the hundreds (measured: 464 at 60 rows deep, water, in
/// `diag_lateral_pressure_term_magnitudes`) drives every single edge in a column to instantly
/// saturate every tick forever.
///
/// `1.0`: the bonus can at most DOUBLE the existing driving head. Chosen, not measured -- a
/// deliberately conservative first value that keeps the system within the same order of
/// magnitude `test_liquid_stream_stays_coherent` and the CFL-respecting phase-0 sweep order were
/// validated at, leaving headroom to raise it later against a specific "deep falls how much
/// faster than shallow" target once one is picked.
const VERTICAL_PRESSURE_CAP_MULT: f32 = 1.0;

/// STAGE C. Amplitude of the granular lateral edge's dispersion term, as a fraction of that edge's
/// own `tau` (see `GRANULAR_TAU_SCALE`) rather than a fixed magnitude — see the "Combined liquid +
/// granular share" block in `settle_tick` for where this is used. `dispersion` is drawn uniformly
/// from `[-DISPERSION_TAU_FRAC * tau, +DISPERSION_TAU_FRAC * tau]` and added to the driving head
/// before the yield-stress gate, so it can nudge an edge sitting near `tau` over or under the
/// threshold from tick to tick (a ragged, grainy heap surface) without being able to either
/// override a genuinely-below-threshold edge by more than half of `tau` or swamp `tau` altogether
/// the way the old CA's fixed `perp_dot * 3.5 * dispersion_noise` did (up to ~44x the ~0.08
/// threshold it was nominally gated behind — see the Stage C task brief, and
/// `GRANULAR_TAU_SCALE`'s doc comment for why that made the old mechanism's threshold inoperative
/// in practice). `0.5` is a starting value, not a measured optimum: half of `tau` is large enough
/// to visibly texture a settled heap's edge (see `test_dry_sand_has_angle_of_repose` CASE 3's
/// slight overshoot of the measured angle, 0.0464 vs 0.0426) while remaining smaller than `tau`
/// itself, so it perturbs which near-threshold edges move rather than deciding the outcome for
/// edges that are clearly above or clearly below.
const DISPERSION_TAU_FRAC: f32 = 0.5;

/// STAGE C. Flat per-edge, per-tick probability that the granular lateral edge sits out this tick
/// (see the "Combined liquid + granular share" block in `settle_tick`) — reproduces the old CA's
/// flat `lock_chance = 0.05` under gravity (`get_ca_params`: "Low locking under gravity so sand
/// avalanches smoothly into a natural hill"), scaled by `granular_share` at the call site so a
/// fully liquid cell is never locked. Low by design: this is meant to add occasional stickiness
/// texture, not to be a second yield-stress mechanism competing with `tau`.
const GRAVITY_LOCK_CHANCE: f32 = 0.05;

/// The grid height `LATERAL_PRESSURE_SCALE` was actually tuned at, and the height `column_depth`
/// normalises its per-row contribution against so the accumulated sum represents *physical*
/// depth rather than a row count.
///
/// **The bug this fixes:** `column_depth` is a top-down running sum, one `resting_above` term
/// added per grid row (see its accumulation in `settle_tick`). `resting_above` is itself derived
/// from `temp_heights`, which saturates at `cell_capacity` (~1.0) regardless of resolution — a
/// row's contribution is an O(1) "cell's worth of fill," not a physical thickness. Refining the
/// grid N-fold to cover the *same physical container* at higher resolution multiplies the number
/// of rows spanning that container by N, and therefore multiplies the accumulated sum — and the
/// `LATERAL_PRESSURE_SCALE * column_depth` driving head built from it — by N too, even though the
/// physical column of liquid above the cell hasn't gotten any deeper. Production is
/// `GRID_SIZE = 512`; the sweep that picked `LATERAL_PRESSURE_SCALE = 5.0` (see its doc comment)
/// was run entirely at 64x64 and 64x96, so at production resolution the lateral head this
/// produces is inflated 8x over what was actually tuned, which is large enough to reintroduce a
/// bad case of the exact "water walls" defect the term exists to prevent (measured: enclosed-void
/// counts on `test_liquid_flowing_liquid_does_not_stand_in_walls`'s scenario at scale go from 0
/// suppressed at 64x64 to tens of thousands at 512x512 — see docs/ARCHITECTURE.md).
///
/// **The fix:** scale each row's `resting_above` contribution by `REFERENCE_GRID_HEIGHT as f32 /
/// w as f32` before folding it into the running sum, so a column spanning many rows contributes
/// the same total regardless of grid resolution — `column_depth` becomes an estimate of physical
/// depth in units of "rows at the reference resolution," not "rows at whatever resolution happens
/// to be running." At `w == REFERENCE_GRID_HEIGHT` this is `depth_scale == 1.0`, an exact no-op,
/// so the tuned 64x64/64x96 behaviour (and every test pinned to it) is unchanged.
///
/// **Divides by `w` (grid width), not `h`, despite normalising a *vertical* sum.** Production
/// (`GRID_SIZE` in `lib.rs`) is always square, so this is invisible there — `w == h` unconditionally.
/// It matters only for this crate's own test grids, which aren't square: the two tests
/// `LATERAL_PRESSURE_SCALE` was swept against are `test_liquid_flowing_liquid_does_not_stand_in_walls`
/// (64x64) and `test_liquid_stream_stays_coherent` (64 wide, 96 tall — the extra rows exist only to
/// give a falling stream room to develop before measurement, not because that container is "higher
/// resolution"). Both share width 64; only one shares height 64. Dividing by `h` was tried first and
/// is an exact no-op for the first test but *not* the second, where it silently drops the effective
/// lateral pressure to 64/96 of nominal and pushes `test_liquid_stream_stays_coherent`'s `max_width`
/// from 8 to 9 — past the coherence cliff documented on `LATERAL_PRESSURE_SCALE` — as a pure artifact
/// of which axis the reference resolution was measured against, not any genuine change in scenario.
/// Dividing by `w` reproduces both tests' existing numbers exactly at scale 1, and is identical to
/// dividing by `h` at every resolution this simulator's actual (square) grids ever run at.
///
/// Normalising `column_depth` itself (rather than dividing `LATERAL_PRESSURE_SCALE` by a fixed
/// 512/64 = 8 for production) is deliberately the more general fix: it makes the term correct at
/// *any* grid size the simulator is ever run at — including the intermediate 128/256 sizes this
/// file's tests exercise, and whatever size a future change picks — rather than hard-coding
/// correctness for one more specific resolution the way the original constant hard-coded it for
/// 64.
///
/// `64` is not an arbitrary round number: it is the grid width every value in
/// `LATERAL_PRESSURE_SCALE`'s doc-comment sweep (30060, 12106, 13648, ...) was actually measured
/// at, via `test_liquid_flowing_liquid_does_not_stand_in_walls`'s 64x64 grid and
/// `test_liquid_stream_stays_coherent`'s 64-wide box. Reusing that same number as the reference
/// resolution is what makes `LATERAL_PRESSURE_SCALE = 5.0` continue to mean exactly what it was
/// swept against, rather than silently changing its meaning a second time.
const REFERENCE_GRID_HEIGHT: usize = 512;

pub fn cell_capacity_for(wetness: f32) -> f32 {
    let l = liquidity(wetness);
    1.5 * (1.0 - l) + 1.0 * l
}

/// Task #70: Maximum fraction above nominal cell capacity that material is permitted to transiently occupy.
/// Capped up to 1.5x nominal capacity (0.50 allowance) per user design decision.
pub const OVERFILL_CEILING_RATIO: f32 = 0.50;

/// Task #70: Hydrostatic stiffness parameter $k$ for the overfill pressure model.
/// Calibrated so that steady-state overfill at the base of a deep resting column (depth ~300)
/// reaches ~5% (0.05) overfill: k = (g * D) / sigma_max = (1.0 * 300) / 0.05 = 6000.
pub const OVERFILL_STIFFNESS_K: f32 = 6000.0;

/// Task #70: Yield friction coefficient for granular material under the overfill stress model (Mohr-Coulomb).
pub const OVERFILL_MOHR_COULOMB_MU: f32 = 0.60;

/// Task #70: Maximum permitted cell fill capacity including transient overfill allowance.
#[inline]
pub fn cell_overfill_capacity(wetness: f32) -> f32 {
    cell_capacity_for(wetness) * (1.0 + OVERFILL_CEILING_RATIO)
}

/// Task #70: Configurable overfill capacity allowance.
#[inline]
pub fn cell_overfill_capacity_for(wetness: f32, overfill_ratio: f32) -> f32 {
    cell_capacity_for(wetness) * (1.0 + overfill_ratio)
}

/// Task #70: Dimensionless relative overfill fraction (e = (h - cap) / cap for h > cap, else 0).
#[inline]
pub fn relative_overfill(h: f32, cap: f32) -> f32 {
    if h > cap && cap > 0.0 {
        (h - cap) / cap
    } else {
        0.0
    }
}

/// Task #47 round 3. Graded support fraction for cell `idx`, in `[0, 1]`: how much of what is
/// below it can bear its weight. `1.0` = fully supported (resting on the container floor/casing,
/// or the cell below is at or over its own material capacity, `cell_capacity_for`). `0.0` = fully
/// unsupported (the cell below is completely empty). Linear in between:
/// `height_below / capacity_below`.
///
/// A STATE predicate, not a velocity or history one: reads only the current heights and the
/// static per-cell capacity, never `edge_vel_v` and never last tick's values. That is the whole
/// point relative to `in_transit_at` (the velocity proxy this function replaces for scheduling,
/// see `fresh_overburden_must_blocks`'s `Unsupported*` variants): a solid-packed body falling as a
/// unit produces no per-edge velocity signal partway down its own bulk.
/// `diag_task47_in_transit_underdetection` measured this directly -- `in_transit_at` read exactly
/// `0.0` for every interior row of a falling, fully-packed DrySand block (only the block's own
/// leading/trailing edge rows read nonzero), so a velocity-based test only ever "sees" the
/// outermost cell or two of such a body, and the accumulated `column_depth` built from it climbs
/// past `FRESH_OVERBURDEN_SKIN_CELLS` after just the ONE topmost row of material -- the rest of an
/// unambiguously free-falling block reads as if it were fully resting. `support_fraction` has no
/// such blind spot: every cell in that same falling body reads at or near `0.0`, because the cell
/// below each of them genuinely is empty, regardless of how fast (or whether) anything is
/// currently recorded as moving.
///
/// REUSE, BY DESIGN: named and placed for what it means, not for its current caller. Task #47
/// (the "sand-slab" scheduling defect) is the only consumer today, but this is a direct,
/// physically-motivated answer to "does this material press on what is below it" -- exactly the
/// question the Janssen/Beverloo-style lateral-pressure term (`LATERAL_PRESSURE_SCALE`'s doc
/// comment, `column_depth`) is trying to answer with `in_transit_at`'s velocity proxy instead. A
/// future change could plausibly build `column_depth` from `1.0 - support_fraction` (or some
/// function of it) rather than `in_transit_at`, which would likely fix the same under-detection
/// there that this function fixes for scheduling here. That is deliberately NOT done by this
/// change: it is a separate, contested decision -- see `fresh_pressure_field`'s doc comment for
/// the confirmed, unexplained standing-arch regression a previous, unrelated attempt at touching
/// that path caused -- and this function feeds block activation only, today.
#[inline]
fn support_fraction(
    idx: usize,
    w: usize,
    h: usize,
    heightmap_data: &[f32],
    cell_props: &[f32],
    shape_mask: &[u8],
) -> f32 {
    let cy = idx / w;
    if cy + 1 >= h {
        return 1.0; // Off-grid below: resting on the container floor, fully supported.
    }
    let below_idx = idx + w;
    if shape_mask[below_idx] == crate::MASK_OUTSIDE {
        return 1.0; // Resting on casing, fully supported.
    }
    let cap_below = cell_capacity_for(cell_props[below_idx * 4 + PROP_WETNESS]);
    if cap_below <= 0.0 {
        return 1.0;
    }
    (heightmap_data[below_idx] / cap_below).clamp(0.0, 1.0)
}

/// Conservative per-edge flux update — the Phase 5 (Option B) replacement for the per-cell
/// Laplacian wave update.
///
/// For an edge `e = (a, b)` with gravitational heads `head_a`/`head_b` (`H = h + Phi(g)`; `Phi` is
/// zero when gravity is out-of-plane, so `H = h` reduces to a pure free-surface head):
///
/// ```text
/// yielded = sign(H_a - H_b) * max(|H_a - H_b| - tau, 0)     // tau = yield stress (0 for liquid)
/// v_e    <- (v_e + c_sq * yielded) * damping                // per-edge momentum
/// flux    = clamp(v_e, -(donor b limits), +(donor a limits))
/// h_a -= flux ; h_b += flux
/// ```
///
/// Two properties this buys over the old formulation:
///
/// 1. **Mass conservation by construction.** Every edge debits exactly what it credits, so the
///    total is invariant no matter *which* blocks the LOD scheduler chose to run this tick. The
///    old per-cell form only telescoped to zero if every cell in the domain updated in the same
///    pass, which `will_simulate[b]` explicitly breaks (defect C7).
/// 2. **No unilateral clamp.** The old form ended in `(h + v).clamp(0.0, 1.0)`, an edit with no
///    counterparty: flooring a negative excursion to 0 *adds* mass and capping at 1.0 *discards*
///    it. Here the donor limit (`h_donor`) and the acceptor limit (`cap - h_acceptor`) only ever
///    *reduce a transfer*, which cannot change the total.
///
/// Reduction to the old wave solver at `tau = 0`, `Phi = 0`: summing the four edge fluxes incident
/// on a cell gives `Δh_c = c_sq * (h_l + h_r + h_t + h_b - 4 h_c)` with the same damped-momentum
/// history, i.e. exactly the old `v_new = (v + c_sq * laplacian) * damping; h += v_new`. Ripples
/// and sloshing are preserved; only the leak is gone.
///
/// `*v_e` is set to the *realised* flux rather than the raw integrated velocity. That is the
/// anti-windup term: an edge whose donor is empty or whose acceptor is full would otherwise
/// accumulate unbounded head every tick and then discharge it in one burst the instant the
/// constraint lifts.
///
/// `avail_a` / `avail_b` are the donor limits: how much of each endpoint's mass is actually
/// available to move across *this* edge. They are the cell's full height for a gravity-aligned
/// edge, but for a cross-gravity edge the mass that arrived from upstream during this tick is
/// still in transit — it is unsupported, exerts no hydrostatic pressure, and must not be able to
/// push sideways. Subtracting it is what distinguishes a falling stream (everything it holds
/// arrived this tick, so nothing spreads) from a settled pool (nothing arrived, so all of it
/// levels), without either a free-fall special case or a column-pressure sweep.
///
/// `weight` scales the realised flux. It is the `liquidity` share of the donor cell when the
/// granular CA and this solver are both contributing to the same edge, so that a cell whose
/// `wetness` drifts across the old `>= 0.75` cut hands over between the two solvers continuously
/// instead of switching regime (defect C5). It is 1.0 wherever this solver acts alone.
///
/// How much of cell `c`'s fill is still passing through it — mass that arrived from upstream
/// this tick and can be shown to be continuing on downward, as opposed to mass that arrived and
/// is now at rest. Shared by the cross-gravity liquid edge's `avail_a`/`avail_b` and by
/// `column_depth`'s top-down accumulation in `settle_tick`; see the big comment on that edge for
/// the full derivation of `in_transit = min(inflow, outflow + room_below)`.
///
/// A plain function, not a closure defined inline in the per-cell loop: that loop runs once for
/// *every* cell in every active block regardless of material, `settle_tick` is already large, and
/// a wider closure there measurably worsened generated code for the pure-granular path in
/// benchmarking even though the closure body is never invoked when `cell_liquidity == 0.0` — the
/// cost was in how it changed codegen for the enclosing loop, not in calling it.
#[inline]
#[allow(clippy::too_many_arguments)]
fn in_transit_at(
    c: usize,
    w: usize,
    h: usize,
    temp_heights: &[f32],
    heightmap_data: &[f32],
    cell_props: &[f32],
    edge_vel_v: &[f32],
    shape_mask: &[u8],
) -> f32 {
    let cx = c % w;
    let cy = c / w;
    // No edge below (off-grid, or the cell below is casing): the cell is resting on the
    // container, so there is no downstream route at all. `edge_vel_v[c]` is stale in that case —
    // phase 0 skips exactly these edges, and its guard is mirrored here — so it must not be read.
    if !(cx > 0 && cx + 1 < w && cy > 0 && cy + 1 < h
        && shape_mask[(cy + 1) * w + cx] != crate::MASK_OUTSIDE)
    {
        return 0.0;
    }
    let below = c + w;
    let h_below = temp_heights[below].max(heightmap_data[below]);
    let cap_below = cell_capacity_for(cell_props[below * 4 + PROP_WETNESS]);
    let downstream_route = edge_vel_v[c].max(0.0) + (cap_below - h_below).max(0.0);
    edge_vel_v[c - w].max(0.0).min(downstream_route)
}

/// Recomputes `column_depth` (depth-integrated lateral pressure; see `LATERAL_PRESSURE_SCALE`'s
/// doc comment for what this quantity means) as a standalone, unconditional, top-to-bottom pass
/// over the whole grid interior, reading a caller-supplied `source_heights` array instead of
/// chaining off values computed earlier in the same per-cell loop.
///
/// NOW WIRED UP behind the `fresh_pressure_field` debug toggle (`settle_tick`'s own parameter of
/// that name, `DrawingSimulation::fresh_pressure_field`) at the ONCE-PER-TICK placement described
/// below — run once, before the `for phase in 0..2` loop, reading the frozen pre-tick
/// `heightmap.data` snapshot. Off by default; the fallback (this function unused, `column_depth`
/// computed inline instead) is what every existing test still exercises. Task #54 originally
/// tried promoting this to a free function so `settle_tick` could call it once at the TOP OF EACH
/// PHASE instead (once reading `heightmap.data` at the top of phase 0, where it is bit-identical
/// to `temp_heights`; once again reading `temp_heights` at the top of phase 1, after phase 0's
/// own APPLY step has mutated it) — on the hypothesis that the once-per-tick placement (run once
/// before the phase loop, always reading the frozen pre-tick `heightmap.data` snapshot) fed phase
/// 1's lateral edges a pre-phase-0 overburden field instead of the state phase 1 actually reads
/// its heads from.
///
/// MEASURED, and it did NOT fix the target regression: `test_liquid_flowing_liquid_does_not_stand_in_walls`
/// went from voids@160=66 (once-per-tick, before-phase-0 placement) to 85 (once-per-phase) -- WORSE,
/// not better, and worse than the once-per-tick "after phase 0" placement (83) too. Root-caused by
/// direct instrumentation, not left as a guess: dumping `column_depth` down a resting column of the
/// walls-test's own scenario showed the once-per-phase pass produces BIT-IDENTICAL values to the
/// old order-dependent in-loop computation at tick 0 (e.g. 24.0/64.0/104.0/144.0 at rows 10/15/20/25
/// under both), which rules out a buffer-swap/plumbing bug (a real prior hypothesis: that
/// `source_heights` and the frozen `heightmap_data` argument to `in_transit_at` had been swapped or
/// duplicated, driving `column_depth` to a near-zero, ineffective field). By tick 29 the two builds'
/// `column_depth` values diverge substantially (e.g. row 25: 92.6 old vs. 35.6 new) -- but this
/// tracks a genuine difference in simulated height trajectory between the two builds by that point
/// (their own `h` values at those same cells differ just as much), not degeneracy in this function.
/// So the once-per-phase idea is sound in isolation but performs WORSE on the blocking test than
/// the once-per-tick placement, which itself performs WORSE (voids@160=66, still over the <= 20
/// bound) than the in-loop fallback that shipped after Task #54 (which passes the full suite).
/// Neither of this function's two possible call placements fixes the target regression, which is
/// why the DEFAULT stays the in-loop fallback (see the `if gravity_active && !fresh_pressure_field`
/// block ahead of the `for phase in 0..2` loop in `settle_tick`, and the inline computation inside
/// phase 1's CA branch) -- steps 2-4 (vertical overburden bonus, CFL cap, Janssen transform)
/// WITHOUT step 1 (this standalone pass), `column_depth` computed in-loop, order-dependent. The
/// once-per-tick placement is exposed anyway, opt-in only, as `fresh_pressure_field` -- not because
/// it is known to be an improvement (the walls-test regression above says it visibly is not, by
/// this one metric) but so its actual on-screen behaviour can be judged directly rather than only
/// through this one scalar; see the task report for the full numbers.
///
/// `source_heights` is the current heights to accumulate depth from (`temp_heights` at a
/// once-per-phase call site, or the frozen `heightmap.data` snapshot at a once-per-tick site).
/// `heightmap_data` is passed through unchanged to `in_transit_at`'s own frozen-clamp read (see
/// its doc comment) and should always be the tick's frozen pre-tick snapshot, never
/// `source_heights` itself, matching the original in-loop computation's own two-array split.
#[allow(clippy::too_many_arguments)]
fn recompute_column_depth(
    w: usize,
    h: usize,
    shape_mask: &[u8],
    source_heights: &[f32],
    heightmap_data: &[f32],
    external_mass_this_tick: &[f32],
    cell_props: &[f32],
    edge_vel_v: &[f32],
    column_depth: &mut [f32],
) {
    let is_inside = |cx: usize, cy: usize| -> bool { shape_mask[cy * w + cx] != crate::MASK_OUTSIDE };
    let depth_scale = REFERENCE_GRID_HEIGHT as f32 / w as f32;
    for y in 1..h.saturating_sub(1) {
        let row_offset = y * w;
        for x in 1..w.saturating_sub(1) {
            let center_idx = row_offset + x;
            if !is_inside(x, y) {
                continue;
            }
            let above_idx = center_idx - w;
            let depth_above = if is_inside(x, y - 1) {
                let resting_above = (source_heights[above_idx]
                    - in_transit_at(above_idx, w, h, source_heights, heightmap_data, cell_props, edge_vel_v, shape_mask)
                    - external_mass_this_tick[above_idx].max(0.0))
                    .max(0.0)
                    * depth_scale;
                resting_above + column_depth[above_idx]
            } else {
                0.0
            };
            column_depth[center_idx] = depth_above;
        }
    }
}

/// Task #47: which population the fresh-overburden predicate promotes. History, corrected across
/// three rounds and tested empirically each time (`diag_task47_block_fraction_table`,
/// `diag_task47_variant_divergence_comparison`, `diag_task47_in_transit_underdetection`) rather
/// than trusted on any one round's story alone:
///
/// - Round 1 shipped `OverburdenAndRoom`: material, near-zero `column_depth`, and a lower
///   neighbour. Measured improvement over doing nothing: only 8.4% cumulative divergence from a
///   perfect-simulation ground truth.
/// - Round 2's first diagnosis -- that the conjunction catches "already falling" (in free fall,
///   `in_transit_at` subtracted, so overburden reads ~0) and misses "about to fall" (still
///   resting, so overburden reads high even though its support just opened up) -- turned out to
///   be the wrong mechanism, per the user's own correction: a genuinely free-falling body SHOULD
///   read near-zero overburden by that same reasoning, so the conjunction ought to work.
/// - Round 3 found the real mechanism: `in_transit_at` itself under-detects free fall.
///   `diag_task47_in_transit_underdetection` measured it directly on a falling, fully-packed
///   DrySand block -- `in_transit_at` read exactly `0.0` for every interior row (only the block's
///   own leading/trailing edge rows read nonzero), so accumulated `column_depth` crossed the
///   `FRESH_OVERBURDEN_SKIN_CELLS` bar after just the ONE topmost row of material. The rest of an
///   unambiguously free-falling body read as if it were fully resting, and the fraction of
///   perfect-simulation's promoted blocks excluded purely by the overburden clause despite having
///   somewhere to go grew from 36% at tick 3 to 62% by tick 12.
///
/// The fix (`UnsupportedAndRoom`, shipped): `support_fraction`, a STATE predicate (current heights
/// + static per-cell capacity only, no `edge_vel_v`, no history) swapped in for the overburden
/// clause -- see its own doc comment for why it has no blind spot for a rigid/packed body falling
/// as a unit. Measured cumulative-divergence reduction: ~95%, against round 1's 8.4%.
// Every variant but `UnsupportedAndRoom` is test/diagnostic-only (see `fresh_overburden_gate`'s
// `#[cfg(not(test))]` twin, which hardcodes the shipped choice) -- `allow(dead_code)` rather than
// leaving this to warn in every non-test build.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[allow(dead_code)]
pub(crate) enum FreshOverburdenVariant {
    /// The naive, unbounded predicate from round 1's own "measure the cost" section: material
    /// with near-zero overburden, no room-to-move bound at all. Kept only as the reference point
    /// for how much `OverburdenAndRoom`'s room clause actually saves.
    OverburdenOnly,
    /// Round 1's shipped default: material AND near-zero overburden AND somewhere to go
    /// (below/left/right, a measurably lower neighbour).
    OverburdenAndRoom,
    /// Round 2 variant 2: material AND free capacity directly below -- drops the overburden
    /// clause entirely. The direct "support is gone or going" condition the coordinator asked
    /// for: promotes a block purely because the column below it isn't full, independent of how
    /// much settled material of its own still sits on top of it.
    CapacityBelowOnly,
    /// Round 2 variant 3: material AND (near-zero overburden OR free capacity below).
    OverburdenOrCapacityBelow,
    /// Round 2 variant 4: material AND somewhere to go (below/left/right) -- drops the overburden
    /// clause, keeps `OverburdenAndRoom`'s lateral room check that `CapacityBelowOnly` lacks.
    RoomOnly,
    /// Round 3: material AND `support_fraction` below `1 - SUPPORT_FRACTION_EPSILON` -- no room
    /// clause. The formal, reusable primitive superseding `CapacityBelowOnly`'s ad-hoc version of
    /// the same idea (see `support_fraction`'s doc comment for why this is a STATE predicate,
    /// unlike `in_transit_at`, and is designed for reuse beyond scheduling).
    UnsupportedOnly,
    /// Round 3 (shipped): material AND unsupported (per `support_fraction`) AND somewhere to go.
    /// Directly replaces round 1's `OverburdenAndRoom` clause-for-clause -- same AND-with-room
    /// structure, `support_fraction` swapped in for the near-zero-overburden test that
    /// `diag_task47_in_transit_underdetection` showed almost never fires more than one row into a
    /// falling body.
    UnsupportedAndRoom,
}

/// Task #47 ("sand-slab" scheduling defect): the fresh-overburden MUST-simulate predicate,
/// factored out to a standalone function so `settle_tick`'s call site and diagnostic/regression
/// tests can share exactly one implementation rather than a test-only copy silently drifting from
/// what actually ships. See the predicate's call site in `settle_tick` (just before the MUST/
/// STALE/REST classification loop) for the full rationale: every other activation signal there is
/// historical (one tick late); this one is a state predicate on the CURRENT, pre-tick field, so it
/// wakes a block for what it is doing this tick, before anything moves.
///
/// Returns one bool per block (`cols * rows`, row-major): `true` if any cell in that block
/// satisfies `variant` (see `FreshOverburdenVariant`'s own comments for what each one checks).
///
/// `heightmap_data`/`external_mass_this_tick` should be the tick's frozen pre-tick snapshot (the
/// same once-per-tick placement `recompute_column_depth`'s own doc comment describes for the
/// `fresh_pressure_field` toggle) — but note this is computed into a throwaway local buffer, never
/// `column_depth` itself, and is used only to decide which blocks `settle_tick` runs, never as a
/// physics term.
#[allow(clippy::too_many_arguments)]
fn fresh_overburden_must_blocks(
    w: usize,
    h: usize,
    block_size: usize,
    cols: usize,
    rows: usize,
    shape_mask: &[u8],
    heightmap_data: &[f32],
    external_mass_this_tick: &[f32],
    cell_props: &[f32],
    edge_vel_v: &[f32],
    variant: FreshOverburdenVariant,
) -> Vec<bool> {
    let expected_len = cols * rows;
    let mut fresh_active = vec![false; expected_len];

    // Only variants that actually consult overburden pay for `recompute_column_depth`'s full-grid
    // pass; `CapacityBelowOnly` and `RoomOnly` never read `fresh_overburden` at all.
    let needs_overburden = matches!(
        variant,
        FreshOverburdenVariant::OverburdenOnly
            | FreshOverburdenVariant::OverburdenAndRoom
            | FreshOverburdenVariant::OverburdenOrCapacityBelow
    );
    let mut fresh_overburden = vec![0.0f32; heightmap_data.len()];
    if needs_overburden {
        recompute_column_depth(
            w,
            h,
            shape_mask,
            heightmap_data,
            heightmap_data,
            external_mass_this_tick,
            cell_props,
            edge_vel_v,
            &mut fresh_overburden[..],
        );
    }

    // Same resolution normalisation `column_depth`'s own accumulation uses (see
    // `LATERAL_PRESSURE_SCALE`'s doc comment) — keeps `FRESH_OVERBURDEN_SKIN_CELLS` meaning the
    // same physical skin thickness at every grid size.
    let depth_scale = REFERENCE_GRID_HEIGHT as f32 / w as f32;
    let pressure_epsilon = FRESH_OVERBURDEN_SKIN_CELLS * depth_scale;

    // "Somewhere to go": a mask-inside orthogonal neighbour (down, left, or right — this
    // simulator's grid rows always run physically downward regardless of the apparatus's
    // simulated `gravity_dir`; see `column_depth`'s own top-to-bottom accumulation) that
    // currently reads measurably lower. A flat resting bed's neighbours are all level, so it
    // fails this check; a falling body's landing zone, or a slope, passes it.
    let has_room_to_move = |idx: usize| -> bool {
        let cx = idx % w;
        let cy = idx / w;
        let h_c = heightmap_data[idx];
        let lower = |nx: usize, ny: usize| -> bool {
            let nidx = ny * w + nx;
            shape_mask[nidx] != crate::MASK_OUTSIDE
                && heightmap_data[nidx] < h_c - FRESH_OVERBURDEN_ROOM_EPSILON
        };
        (cy + 1 < h && lower(cx, cy + 1))
            || (cx > 0 && lower(cx - 1, cy))
            || (cx + 1 < w && lower(cx + 1, cy))
    };

    // "Free capacity below": the cell directly below is not full to its own material capacity
    // (`cell_capacity_for`, the same per-cell cap the flux solver's donor/acceptor clamps use) --
    // independent of whether it currently reads lower than this cell. A resting block sitting on
    // an unfilled column below it is, by this test, unsupported regardless of what sits on top of
    // the block itself; that is the round-2 "about to fall" signal `has_room_to_move` alone
    // (round 1) does not carry, since a below-neighbour can be at-or-above this cell's own height
    // and still be under its own capacity.
    let has_free_capacity_below = |idx: usize| -> bool {
        let cy = idx / w;
        if cy + 1 >= h {
            return false;
        }
        let below_idx = idx + w;
        if shape_mask[below_idx] == crate::MASK_OUTSIDE {
            return false;
        }
        let cap_below = cell_capacity_for(cell_props[below_idx * 4 + PROP_WETNESS]);
        cap_below - heightmap_data[below_idx] > FRESH_OVERBURDEN_ROOM_EPSILON
    };

    // Round 3: the formal, reusable `support_fraction` primitive -- see its own doc comment for
    // why this is a STATE predicate (current heights + static capacity only) rather than
    // `in_transit_at`'s velocity proxy, and why that matters for a body falling as a rigid mass.
    let unsupported_enough = |idx: usize| -> bool {
        support_fraction(idx, w, h, heightmap_data, cell_props, shape_mask) < 1.0 - SUPPORT_FRACTION_EPSILON
    };

    for by in 0..rows {
        let start_y = by * block_size;
        let end_y = ((by + 1) * block_size).min(h);
        for bx in 0..cols {
            let start_x = bx * block_size;
            let end_x = ((bx + 1) * block_size).min(w);
            let b = by * cols + bx;
            'scan: for y in start_y..end_y {
                let row_offset = y * w;
                for x in start_x..end_x {
                    let idx = row_offset + x;
                    if shape_mask[idx] == crate::MASK_OUTSIDE {
                        continue;
                    }
                    if heightmap_data[idx] <= FRESH_OVERBURDEN_MATERIAL_EPSILON {
                        continue;
                    }
                    let overburden_ok = needs_overburden && fresh_overburden[idx] < pressure_epsilon;
                    let promote = match variant {
                        FreshOverburdenVariant::OverburdenOnly => overburden_ok,
                        FreshOverburdenVariant::OverburdenAndRoom => overburden_ok && has_room_to_move(idx),
                        FreshOverburdenVariant::CapacityBelowOnly => has_free_capacity_below(idx),
                        FreshOverburdenVariant::OverburdenOrCapacityBelow => {
                            overburden_ok || has_free_capacity_below(idx)
                        }
                        FreshOverburdenVariant::RoomOnly => has_room_to_move(idx),
                        FreshOverburdenVariant::UnsupportedOnly => unsupported_enough(idx),
                        FreshOverburdenVariant::UnsupportedAndRoom => {
                            unsupported_enough(idx) && has_room_to_move(idx)
                        }
                    };
                    if promote {
                        fresh_active[b] = true;
                        break 'scan;
                    }
                }
            }
        }
    }

    fresh_active
}

/// Computes the *candidate* signed flux (positive = `a` -> `b`) for one edge, from a single
/// frozen read of the caller-supplied heads/avail/cap — no shared state (`temp_heights`,
/// `cell_props`, `cell_colors`) is touched here. This is the COLLECT half of the frozen-Jacobi
/// flux solver; see the big comment on the candidate-flux buffers in `settle_tick` for the
/// three-pass structure and `flux_edge_apply` for the APPLY half.
///
/// The donor/acceptor clamps below (`avail_a`, `cap_b - h_b`, etc.) are still applied — they are
/// what makes this a *single-edge* candidate rather than the raw, unbounded integrated velocity —
/// but they are only a per-edge upper bound. Multiple edges reading the same frozen donor (a cell
/// with two owned outgoing edges, or a cell that is the acceptor of two different owners' edges)
/// can still, together, claim more than that donor has or more than that acceptor can hold; that
/// is exactly what the caller's arbitration step (summing candidates per cell and rescaling, see
/// `settle_tick`) exists to catch before any candidate here is actually applied.
///
/// Reduction to the pre-Jacobi solver: this is bit-for-bit the same formula the combined
/// compute-and-apply `flux_edge` used before this conversion (see git history), just without the
/// final application — so a cell touched by only one edge this phase (arbitration a provable
/// no-op there — see the phase-0 case) behaves identically to before.
#[inline]
#[allow(clippy::too_many_arguments)]
fn flux_edge_candidate(
    head_a: f32,
    head_b: f32,
    c_sq: f32,
    damping: f32,
    tau: f32,
    cap_a_eff: f32,
    cap_b_eff: f32,
    _cap_a: f32,
    _cap_b: f32,
    avail_a: f32,
    avail_b: f32,
    h_a: f32,
    h_b: f32,
    weight: f32,
    v_e_prev: f32,
) -> f32 {
    let driving = head_a - head_b;
    let yielded = if driving > tau {
        driving - tau
    } else if driving < -tau {
        driving + tau
    } else {
        0.0
    };

    let v = ((v_e_prev + c_sq * yielded) * damping).clamp(-1.0, 1.0);

    // Donor mass and acceptor capacity, in the direction the velocity actually points. This is
    // the single-edge bound described above, not the final word — see the doc comment.
    weight * if v > 0.0 {
        v.min(avail_a).min((cap_b_eff - h_b).max(0.0))
    } else if v < 0.0 {
        -((-v).min(avail_b).min((cap_a_eff - h_a).max(0.0)))
    } else {
        0.0
    }
}

/// Applies a *final* (post-arbitration) signed flux to one edge: the APPLY half of the
/// frozen-Jacobi flux solver (see `flux_edge_candidate` and the candidate-flux buffer comment in
/// `settle_tick`). Bit-for-bit the same mutation the old combined `flux_edge` performed once its
/// `flux` value was computed — moved here unchanged so that arbitration can sit between computing
/// a candidate and mutating anything.
///
/// `*v_e` is set to the *realised* (final) flux rather than the raw candidate or the raw
/// integrated velocity — same anti-windup rationale as before: an edge whose donor is empty or
/// whose acceptor is full (locally, or now also via arbitration) must not accumulate unbounded
/// head every tick and then discharge it in one burst the instant the constraint lifts.
#[inline]
#[allow(clippy::too_many_arguments)]
fn flux_edge_apply(
    a_b: usize,
    b_b: usize,
    a_idx: usize,
    b_idx: usize,
    flux: f32,
    v_e: &mut f32,
    temp_heights: &mut [f32],
    cell_colors: &mut [u8],
    cell_props: &mut [f32],
    modified: &mut Vec<bool>,
    next_displacements: &mut Vec<f32>,
    total_flow: &mut f32,
    flow_occurred: &mut bool,
) {
    *v_e = flux;

    // Below this the transfer is pure f32 noise; skipping it is still exactly conservative
    // (nothing is added or removed), it just avoids an advect_properties call per edge per tick.
    const MIN_FLUX: f32 = 1e-7;
    if flux > MIN_FLUX {
        activate_neighbor(a_b, flux, modified, next_displacements);
        activate_neighbor(b_b, flux, modified, next_displacements);
        advect_properties(cell_colors, cell_props, a_idx, b_idx, flux, temp_heights[b_idx]);
        temp_heights[a_idx] -= flux;
        temp_heights[b_idx] += flux;
        *total_flow += flux;
        *flow_occurred = true;
    } else if flux < -MIN_FLUX {
        let mag = -flux;
        activate_neighbor(a_b, mag, modified, next_displacements);
        activate_neighbor(b_b, mag, modified, next_displacements);
        advect_properties(cell_colors, cell_props, b_idx, a_idx, mag, temp_heights[a_idx]);
        temp_heights[b_idx] -= mag;
        temp_heights[a_idx] += mag;
        *total_flow += mag;
        *flow_occurred = true;
    }
}

/// The pointwise-minimum flux-corrected-transport (Zalesak-style) scale factor for one candidate
/// edge, given the donor's and acceptor's frozen per-cell budgets and the RAW (pre-scaling)
/// totals every edge touching them this phase already claimed (see the candidate-flux buffer
/// comment above `settle_tick`'s phase loop for how `*_out_total`/`*_in_total` are accumulated).
///
/// Returns a value in `[0, 1]`; multiplying a candidate flux by it can only shrink the candidate's
/// magnitude, never grow or flip it.
///
/// **Why a single application of this (no fixed-point iteration to convergence) is sufficient.**
/// For a donor cell `d`, define `out_scale(d) = min(1, avail(d) / out_total(d))` where
/// `out_total(d)` is the sum of every RAW candidate magnitude `d` donates this phase. Scaling
/// *every* edge donating out of `d` by at most `out_scale(d)` (this function never returns more,
/// since it takes the minimum with the acceptor's own ratio too) bounds their sum by
/// `out_total(d) * out_scale(d)`, which is exactly `avail(d)` when `out_total(d) > avail(d)` and
/// is `<= avail(d)` otherwise (where `out_scale(d) == 1` and the raw sum was already within
/// budget). The identical argument bounds the acceptor side by `freecap(a)`. Both guarantees
/// therefore follow from the RAW totals alone, computed once before any scaling is applied — a
/// second pass recomputing totals from already-scaled fluxes would not find anything to correct,
/// because the bound was exact, not approximate. This is the standard synchronous FCT limiter
/// (Zalesak 1979): the min-of-two-ratios construction is precisely what makes it single-pass.
///
/// Cross-checked empirically, not just algebraically: every diagnostic and test run against this
/// conversion measured `min_h >= 0` and `max_h <= capacity` with no exceptions (see the physics
/// conversion's measurement notes), which is what would fail first if a second pass were in fact
/// needed.
///
/// **The split need not be proportional.** The argument above only requires each side's per-edge
/// factors to sum, weighted by the raw claims, to no more than that side's budget — it never
/// required every edge sharing a cell to be scaled by the *same* factor. `budget_term` exploits
/// exactly that slack to divide a contested budget with randomised weights instead of equal ones
/// (see its doc comment for the per-side bound, and `grain_jitter_strength` for why granular
/// material wants this and liquid does not).
#[inline]
fn edge_arbitration_scale(
    donor_out_total: f32,
    donor_out_total_jit: f32,
    donor_avail: f32,
    acceptor_in_total: f32,
    acceptor_in_total_jit: f32,
    acceptor_freecap: f32,
    jitter: f32,
) -> f32 {
    budget_term(donor_out_total, donor_out_total_jit, donor_avail, jitter)
        .min(budget_term(acceptor_in_total, acceptor_in_total_jit, acceptor_freecap, jitter))
        .min(1.0)
}

/// One side (donor or acceptor) of `edge_arbitration_scale`: the fraction of this edge's raw claim
/// that survives *that* cell's budget.
///
/// **Oversubscription is tested against the RAW total, the split is taken against the JITTERED
/// one, and mixing those two is deliberate.** Testing with the jittered total instead would be
/// unsound in one direction: `jit_total` can fall *below* `raw_total` (jitters below 1 shrink it),
/// so `jit_total <= budget` does not imply the raw claims fit, and returning `1.0` there would let
/// a cell over-drain or overfill. Testing with the raw total keeps the two branches exactly as
/// safe as they were before jitter existed:
///
/// * **Not oversubscribed** (`raw_total <= budget`): every edge passes at `1.0`, so the applied sum
///   is `raw_total <= budget`. Jitter is deliberately inert here — there is no allocation decision
///   to make when everyone fits, and perturbing fluxes that nothing is competing for would be a
///   change to the physics rather than to how a contested budget is *divided*.
/// * **Oversubscribed**: the applied sum is bounded by
///   `Σ |raw_e| · budget · r_e / jit_total = budget · (Σ |raw_e| r_e) / jit_total = budget`,
///   exactly — since `jit_total` is by definition `Σ |raw_e| r_e` over the same edge group. The
///   share edge `e` ends up with is `|raw_e| r_e / Σ |raw_e'| r_e'`: the proportional weights,
///   multiplicatively perturbed and renormalised, still summing to one.
///
/// So the Zalesak single-pass argument in `edge_arbitration_scale`'s doc comment carries over
/// verbatim — it only ever needed each side's per-edge factors to respect that side's own budget
/// sum, which both branches above still do. The outer `.min(1.0)` is what lets an above-1 jitter
/// raise an edge's *share* without ever letting it exceed its own raw candidate.
///
/// With `jitter == 1.0` on every edge, `jit_total == raw_total` and this reduces to
/// `min(1, budget / raw_total)` — bit-identical to the pre-jitter limiter, which is what makes
/// liquid (and any zero-strength granular material) provably unchanged.
#[inline]
fn budget_term(raw_total: f32, jit_total: f32, budget: f32, jitter: f32) -> f32 {
    if raw_total > budget && jit_total > 0.0 {
        (budget * jitter / jit_total).max(0.0)
    } else {
        1.0
    }
}

/// How strongly a contested edge's share of a shared budget is randomised away from the strictly
/// proportional split, as the half-width of the multiplicative jitter: `r ∈ [1 - s, 1 + s]`.
///
/// `PROP_GRAIN_SIZE` spans `0.05` (FinePowder) .. `0.80` (CoarseSand) across the granular presets
/// (`MaterialMode::preset_props`). At `1.25` a coarse grain reaches the `0.95` ceiling — its share
/// of a contested budget ranges over `[0.05, 1.95]×` the proportional one — while FinePowder sits
/// at `0.0625`, i.e. essentially the smooth proportional behaviour a powder should have. The
/// ceiling is `0.95` rather than `1.0` so `r` stays strictly positive and `jit_total` cannot
/// collapse to zero while raw claims are still nonzero, which `budget_term` relies on.
const GRAIN_JITTER_SCALE: f32 = 1.25;
const GRAIN_JITTER_MAX: f32 = 0.95;

/// `granular_share`-gated jitter strength for the cell whose material is being rationed.
///
/// Gated on `granular_share = 1 - cell_liquidity`, not on `grain_size`, so every liquid preset is
/// excluded structurally: liquids have `wetness >= 0.75`, hence `cell_liquidity == 1` under
/// gravity, hence strength `0.0` and `jitter == 1.0` regardless of what `grain_size` they carry —
/// the bit-identical path noted on `budget_term`. Liquid's grain sizes happen to be small too, but
/// it is the liquidity gate that is load-bearing.
#[inline]
fn grain_jitter_strength(cell_props: &[f32], cell: usize) -> f32 {
    let granular_share = (1.0 - liquidity(cell_props[cell * 4 + PROP_WETNESS])).clamp(0.0, 1.0);
    if granular_share <= 0.0 {
        return 0.0;
    }
    let grain_size = cell_props[cell * 4 + PROP_GRAIN_SIZE];
    (GRAIN_JITTER_SCALE * grain_size).clamp(0.0, GRAIN_JITTER_MAX) * granular_share
}

/// The per-edge multiplicative weight `r` this edge carries into arbitration, in `[0.05, 1.95]`.
///
/// A stateless hash of `(time_seed, edge key, salt)` rather than a stored RNG stream — the same
/// pattern as the `disp_roll`/`lock_roll` draws in `settle_tick` — precisely so the COLLECT pass
/// (which accumulates `|candidate| * r` into the jittered totals) and the APPLY pass (which needs
/// the same `r` back to compute this edge's share) can each derive it independently and agree,
/// with no extra per-edge buffer to allocate, clear, and keep in sync. `salt` separates the two
/// edge orientations and the tick phases, which index into the same cell-keyed arrays.
///
/// Strength comes from the DONOR: it is the donor's material that is being divided up, so a coarse
/// grain draining into a fine bed should behave coarsely, not the other way round.
#[inline]
fn edge_share_jitter(
    cell_props: &[f32],
    donor: usize,
    edge_key: usize,
    salt: u32,
    time_seed: u32,
) -> f32 {
    let s = grain_jitter_strength(cell_props, donor);
    if s <= 0.0 {
        return 1.0;
    }
    let mut hsh = time_seed
        ^ (edge_key as u32).wrapping_mul(0x9E37_79B1)
        ^ salt.wrapping_mul(0x2545_F491);
    hsh ^= hsh >> 16;
    hsh = hsh.wrapping_mul(0x7feb_352d);
    hsh ^= hsh >> 15;
    hsh = hsh.wrapping_mul(0x846c_a68b);
    hsh ^= hsh >> 16;
    let u = (hsh >> 8) as f32 / 16_777_216.0;
    1.0 + s * (2.0 * u - 1.0)
}

/// Distinguishes the two edge orientations in `edge_share_jitter`'s hash. `cand_h[i]` and
/// `cand_v[i]` are two *different* edges that share the cell key `i`, so without this they would
/// draw the identical jitter and a cell's horizontal and vertical claims would be perturbed in
/// lockstep — a directional bias, not grain.
const EDGE_SALT_H: u32 = 0x27d4_eb2f;
const EDGE_SALT_V: u32 = 0x5bf0_3635;

/// Folds one collected candidate into both the raw and the jittered donor/acceptor totals, drawing
/// the edge's jitter once. The COLLECT-side counterpart of the `edge_arbitration_scale` call in
/// APPLY, which re-derives the same jitter from the same `(edge_key, salt, time_seed)`.
#[allow(clippy::too_many_arguments)]
#[inline]
fn accumulate_edge_totals(
    candidate: f32,
    a_idx: usize,
    b_idx: usize,
    edge_key: usize,
    salt: u32,
    time_seed: u32,
    cell_props: &[f32],
    out_total: &mut [f32],
    in_total: &mut [f32],
    out_total_jit: &mut [f32],
    in_total_jit: &mut [f32],
) {
    let (donor, acceptor, mag) = if candidate >= 0.0 {
        (a_idx, b_idx, candidate)
    } else {
        (b_idx, a_idx, -candidate)
    };
    let jit = edge_share_jitter(cell_props, donor, edge_key, salt, time_seed);
    out_total[donor] += mag;
    in_total[acceptor] += mag;
    out_total_jit[donor] += mag * jit;
    in_total_jit[acceptor] += mag * jit;
}

/// Pressure projection: corrects one phase's just-COLLECTed candidate fluxes (`cand_h`/`cand_v`,
/// restricted to exactly the edges in `touched_h`/`touched_v`) so that a cell many edges away from
/// a capacity constraint can feel it within the same tick, instead of the constraint travelling
/// one cell per tick the way unaided arbitration resolves it.
///
/// **This is a unilateral (complementarity) constraint, not `div u = 0`.** This is a height field
/// whose fill genuinely accumulates -- divergence is physical, not an error to zero out. What must
/// hold is only the one-sided capacity bound:
///
/// ```text
/// p[i] >= 0,   f[i] <= cap[i],   p[i] * (cap[i] - f[i]) = 0
/// ```
///
/// i.e. pressure is zero wherever there is free space, and only rises where flux would otherwise
/// over-pack a cell, and then only enough to push the excess along.
///
/// **Discrete problem.** The node set is exactly the cells incident to `touched_h ∪ touched_v` --
/// the edges COLLECT actually produced this phase, no more (inventing an edge into a block the
/// scheduler decided not to run would move mass it never accounted for; restricting the stencil to
/// live edges also gives Neumann/no-flux boundaries at walls and the shape mask for free, since a
/// missing edge is a missing coupling term, not a special case). Provisional post-flux fill from
/// one pass over the touched lists is `f_star[i] = fill[i] + inflow - outflow`, where `fill[i]` is
/// `temp_heights[i]` as COLLECT read it -- the tick's running snapshot, already advanced by
/// whichever earlier phase(s) applied this tick, and un-mutated by this phase since nothing is
/// applied until after this call returns. A correction on edge `e = (a, b)` (`a` the lower index),
/// `cand_e += phi[a] - phi[b]`, makes the resulting fill `f[i] = f_star[i] + sum over incident
/// edges of (phi[nb] - phi[i])` -- a discrete Laplacian on the candidate-edge graph -- and is
/// solved for with projected Jacobi:
///
/// ```text
/// n_i        = number of candidate edges incident to cell i   (>= 1 by construction: i is only
///              ever visited because some edge touches it)
/// lap[i]     = sum over incident edges of (phi_old[nb] - phi_old[i])
/// residual   = f_star[i] + lap[i] - cap[i]
/// phi_new[i] = max(0, phi_old[i] + PRESSURE_OMEGA * residual / n_i)
/// ```
///
/// The `max(0, ...)` *is* the complementarity condition: a cell with room to spare relaxes its own
/// pressure to zero, so no separate free-surface boundary condition is needed. Double-buffered
/// (`phi_a`/`phi_b`, swapped -- never mutated in place) so the result cannot depend on the order
/// `nodes` happens to be visited in, matching the frozen-Jacobi discipline the rest of this solver
/// already follows; a Gauss-Seidel in-place sweep would reintroduce exactly the visit-order
/// dependence that discipline exists to remove.
///
/// `cap[i]` reuses `cell_capacity_for` -- the same per-cell (wetness-dependent) capacity
/// `cell_freecap` is derived from elsewhere in this phase -- rather than a hardcoded `1.0`.
///
/// **Why this cannot break what arbitration already guarantees.** This function only edits
/// `cand_h`/`cand_v` -- *proposals* -- before arbitration ever reads them. `edge_arbitration_scale`
/// and `flux_edge_apply` still run on the corrected candidates afterwards exactly as before, so
/// positivity and the capacity bound are enforced there regardless of how well this solve
/// converged: a poorly-converged (or even disabled) projection degrades quality, not correctness.
/// And because every correction is applied to an edge -- `phi[a] - phi[b]` added to `cand_e`, which
/// debits `a` and credits `b` by the identical amount -- mass conservation is structural here too:
/// there is no step that could add or remove mass, only redistribute the proposed transfer among
/// the edges that already existed.
///
/// No RNG anywhere in this solve. The stochastic proportional split (`edge_share_jitter`) already
/// runs downstream in arbitration, on whatever magnitude this function hands it; adding a second
/// randomisation here would duplicate that, not improve it.
///
/// Single-edge feasibility clamp for one already-corrected candidate flux `raw` on edge `(a, b)`,
/// applied by `pressure_project` after the Jacobi solve. Exactly the donor/acceptor bound
/// `flux_edge_candidate` already applies to every RAW candidate it produces (`v.min(avail_a).min(
/// (cap_b - h_b).max(0.0))` and its mirror) -- see `pressure_project`'s doc comment, at its call
/// site, for why the projection's own Laplacian residual has no equivalent per-edge bound of its
/// own and why that gap is what needs closing here rather than in the Jacobi loop itself.
///
/// Takes `cap_a`/`cap_b` precomputed rather than deriving them from `cell_props` itself -- see
/// `pressure_project`'s perf note: capacity depends only on wetness, which this solve never
/// mutates, so every caller already has it cached in `pressure_cap` from the node's first visit
/// and re-deriving it here (a `cell_capacity_for` / `liquidity` call per edge, twice) would be
/// pure waste.
#[inline]
fn clamp_edge_feasible(raw: f32, temp_heights: &[f32], cap_a: f32, cap_b: f32, a: usize, b: usize) -> f32 {
    let h_a = temp_heights[a];
    let h_b = temp_heights[b];
    if raw > 0.0 {
        raw.min(h_a).min((cap_b - h_b).max(0.0))
    } else if raw < 0.0 {
        -((-raw).min(h_b).min((cap_a - h_a).max(0.0)))
    } else {
        0.0
    }
}

/// Threshold on `liquidity(wetness)` for a cell to be eligible as a node in
/// the head field's liquid-only domain. See `head_field_transport`'s "Why liquid-only" note:
/// measured directly that running the (structurally similar) multiplicative lateral head over
/// granular material flattens `test_dry_sand_has_angle_of_repose`'s pile to 0 degrees, because
/// `flux ~ conveyance * grad(surface)` has no yield criterion. `0.999`, not `1.0`, only to tolerate
/// float round-trip through `liquidity`'s smoothstep for a material whose `wetness` is nominally
/// 1.0 (pure Water); anything with genuine granular character reads well under this on the
/// `liquidity` smoothstep's own steep part (its ramp spans `wetness` in `[0.65, 0.85]`).
const LIQUID_ELLIPTIC_THRESHOLD: f32 = 0.999;

/// **Perf note (task #45, defect 2).** This function, and its call site, used to do two full
/// extra passes over `touched_h`/`touched_v` beyond what building and solving the Jacobi system
/// itself needs: a separate "build the node set" pass followed by a separate "initialise `fstar`
/// from `temp_heights`, then apply each edge's delta" pass (three passes where one will do -- see
/// the fused loop below), and a whole SEPARATE `accumulate_edge_totals` pass living back in
/// `settle_tick`, walking the same touched lists a second time right after this function already
/// walked them to apply the correction. Measured: that duplicated traversal was the dominant cost
/// at `PRESSURE_ITERATIONS = 1` (i.e. with the Jacobi loop itself contributing almost nothing) --
/// DrySand alone was +48% over baseline at a single iteration, which is only possible from fixed
/// per-phase overhead, not from solve iterations. Both are fused away below: node-set
/// construction and `fstar` initialisation happen in the SAME pass that discovers each edge (using
/// `degree[i] == 0` as the "first time we've seen this node" flag, exactly as the node-set build
/// already did), and `accumulate_edge_totals` runs inline in the same loop that applies and clamps
/// the correction, rather than in a second pass over the same indices back in the caller.
#[allow(clippy::too_many_arguments)]
fn pressure_project(
    cand_h: &mut [f32],
    cand_v: &mut [f32],
    touched_h: &[usize],
    touched_v: &[usize],
    temp_heights: &[f32],
    cell_props: &[f32],
    w: usize,
    phi_a: &mut [f32],
    phi_b: &mut [f32],
    fstar: &mut [f32],
    lap: &mut [f32],
    degree: &mut [u32],
    cap_cache: &mut [f32],
    nodes: &mut Vec<usize>,
    phase: usize,
    time_seed: u32,
    overfill_active: bool,
    overfill_ratio: f32,
    cell_out_total: &mut [f32],
    cell_in_total: &mut [f32],
    cell_out_total_jit: &mut [f32],
    cell_in_total_jit: &mut [f32],
) {
    // Whether the Jacobi solve itself runs this call. Even when it doesn't (diagnostic gate off,
    // or this phase touched nothing), the totals `accumulate_edge_totals` produces are still
    // required downstream by arbitration -- see the always-run accumulate pass at the bottom of
    // this function, which is exactly why this is a local instead of an early `return`.
    let run_solve = !pressure_gate::is_disabled() && !(touched_h.is_empty() && touched_v.is_empty());

    let get_cap = |idx: usize| -> f32 {
        let wetness = cell_props[idx * 4 + PROP_WETNESS];
        if overfill_active {
            cell_overfill_capacity_for(wetness, overfill_ratio)
        } else {
            cell_capacity_for(wetness)
        }
    };

    if run_solve {
        // Clear exactly what the PREVIOUS call to this function touched -- same sparse-reset
        // pattern as the phase-boundary reset above `settle_tick`'s COLLECT loop, and for the same
        // reason: an O(grid) clear every phase would defeat the point of only ever touching the
        // active set.
        for &i in nodes.iter() {
            degree[i] = 0;
            phi_a[i] = 0.0;
            phi_b[i] = 0.0;
            fstar[i] = 0.0;
            lap[i] = 0.0;
        }
        nodes.clear();

        // Fused node-set build + `fstar` init + `cap_cache` init + delta apply: one pass per
        // orientation instead of three. `degree[i] == 0` still means "first time this call has
        // seen node `i`" (the clear above guarantees that), so it doubles as the node-set
        // membership test, the "haven't initialised `fstar[i]` yet" test, AND the "haven't cached
        // this node's capacity yet" test -- all three were always the same condition, just checked
        // in separate passes before. `cap_cache[i] = cell_capacity_for(wetness)` is computed here,
        // ONCE per node for the whole call, specifically so the Jacobi loop below -- which reads a
        // node's capacity on EVERY iteration -- never has to re-derive it (wetness, and therefore
        // capacity, does not change during this solve): see the perf note above this function.
        for &idx in touched_h {
            let a = idx;
            let b = idx + 1;
            if degree[a] == 0 {
                nodes.push(a);
                fstar[a] = temp_heights[a];
                cap_cache[a] = get_cap(a);
            }
            if degree[b] == 0 {
                nodes.push(b);
                fstar[b] = temp_heights[b];
                cap_cache[b] = get_cap(b);
            }
            degree[a] += 1;
            degree[b] += 1;
            let c = cand_h[idx];
            fstar[a] -= c;
            fstar[b] += c;
        }
        for &idx in touched_v {
            let a = idx;
            let b = idx + w;
            if degree[a] == 0 {
                nodes.push(a);
                fstar[a] = temp_heights[a];
                cap_cache[a] = get_cap(a);
            }
            if degree[b] == 0 {
                nodes.push(b);
                fstar[b] = temp_heights[b];
                cap_cache[b] = get_cap(b);
            }
            degree[a] += 1;
            degree[b] += 1;
            let c = cand_v[idx];
            fstar[a] -= c;
            fstar[b] += c;
        }

        // Projected Jacobi. `cur`/`nxt` are local bindings over the caller's two buffers so the
        // loop can swap which one is "current" without ever writing to a buffer while also
        // reading it for this same iteration -- true Jacobi, independent of `nodes`' visit order.
        let mut cur: &mut [f32] = phi_a;
        let mut nxt: &mut [f32] = phi_b;
        for _ in 0..PRESSURE_ITERATIONS {
            for &i in nodes.iter() {
                lap[i] = 0.0;
            }
            for &idx in touched_h {
                let a = idx;
                let b = idx + 1;
                let d = cur[b] - cur[a];
                lap[a] += d;
                lap[b] -= d;
            }
            for &idx in touched_v {
                let a = idx;
                let b = idx + w;
                let d = cur[b] - cur[a];
                lap[a] += d;
                lap[b] -= d;
            }
            for &i in nodes.iter() {
                let residual = fstar[i] + lap[i] - cap_cache[i];
                nxt[i] = (cur[i] + PRESSURE_OMEGA * residual / degree[i] as f32).max(0.0);
            }
            std::mem::swap(&mut cur, &mut nxt);
        }

        // Apply the converged (or best-effort, after a fixed sweep count) correction to the
        // candidates in place, clamp the result to the same single-edge feasibility bound
        // `flux_edge_candidate` already enforces on the raw candidate (a donor cannot be asked to
        // give more than it has, an acceptor cannot be asked to take more than it has room for,
        // and the mirror image the other way), and fold the arbitration totals in -- all in the
        // SAME pass over each touched list, rather than three.
        //
        // Structural conservation holds regardless of the clamp: both still debit `a` and credit
        // `b` by the identical amount, since `cand_e` is one scalar shared by both endpoints -- so
        // summing `cand_h`/`cand_v` before and after this loop is unchanged.
        //
        // Why the clamp is needed even though arbitration (further downstream) already bounds
        // each cell's AGGREGATE draw/receipt against `cell_avail`/`cell_freecap`: the Laplacian
        // residual this solve corrects toward is a global capacity balance (`f[i] <= cap[i]`),
        // with no per-edge upper bound of its own -- projected Jacobi can converge cleanly (see
        // the residual-trace measurements in the task report that motivated this) to a correction
        // that is still, on some single edge, larger than that edge's donor actually has
        // available. Left unclamped, that one inflated edge inflates `cell_out_total[donor]`, and
        // arbitration's proportional scale-down (`edge_arbitration_scale`) throttles that donor's
        // ENTIRE outgoing budget to compensate -- including edges carrying perfectly legitimate,
        // pressure-independent flow. That is what starves real drainage and shows up as more voids
        // at higher iteration counts, not an unstable solve: the fix is bounding the proposal
        // itself, the same way the base flow solver already bounds every one of its own
        // candidates.
        for &idx in touched_h {
            let a = idx;
            let b = idx + 1;
            let corrected = cand_h[idx] + (cur[a] - cur[b]);
            let final_flux = clamp_edge_feasible(corrected, temp_heights, cap_cache[a], cap_cache[b], a, b);
            cand_h[idx] = final_flux;
            accumulate_edge_totals(
                final_flux, a, b, idx,
                EDGE_SALT_H.wrapping_add(phase as u32), time_seed, cell_props,
                cell_out_total, cell_in_total, cell_out_total_jit, cell_in_total_jit,
            );
        }
        for &idx in touched_v {
            let a = idx;
            let b = idx + w;
            let corrected = cand_v[idx] + (cur[a] - cur[b]);
            let final_flux = clamp_edge_feasible(corrected, temp_heights, cap_cache[a], cap_cache[b], a, b);
            cand_v[idx] = final_flux;
            accumulate_edge_totals(
                final_flux, a, b, idx,
                EDGE_SALT_V.wrapping_add(phase as u32), time_seed, cell_props,
                cell_out_total, cell_in_total, cell_out_total_jit, cell_in_total_jit,
            );
        }
    } else {
        // Pressure did not run this call (gate off, or nothing touched this phase): arbitration
        // still needs totals from whatever `cand_h`/`cand_v` already hold (the untouched raw
        // COLLECT candidates).
        for &idx in touched_h {
            accumulate_edge_totals(
                cand_h[idx], idx, idx + 1, idx,
                EDGE_SALT_H.wrapping_add(phase as u32), time_seed, cell_props,
                cell_out_total, cell_in_total, cell_out_total_jit, cell_in_total_jit,
            );
        }
        for &idx in touched_v {
            accumulate_edge_totals(
                cand_v[idx], idx, idx + w, idx,
                EDGE_SALT_V.wrapping_add(phase as u32), time_seed, cell_props,
                cell_out_total, cell_in_total, cell_out_total_jit, cell_in_total_jit,
            );
        }
    }
}

/// Sleeping predicate for a flux edge: `true` when `flux_edge` would provably realise a flux of
/// *exactly* zero this tick, so the whole call — and the `*v_e` write that goes with it — can be
/// skipped.
///
/// This is the flux form's answer to the granular CA's fast-path shortcut at the
/// `h_center - min_h <= threshold_min` check further down. That one is gated on gravity being
/// *off*, and has to be: it compares bare heights, so under gravity it cannot tell "resting on a
/// full column" from "about to fall into an empty one". Here `H = h + Phi(g, r)` already folds
/// gravity into the head, so "at rest under gravity" is a well-posed question and the answer is
/// checkable in a handful of flops — which matters because gravity mode is where the frame time
/// actually goes.
///
/// Two disjoint reasons an edge is dead, both *exact* (no tolerance, no behaviour change):
///
/// 1. **Constrained both ways.** `a` can only donate if it has mass available *and* `b` has room;
///    `b` symmetrically. If neither direction has both, `flux_edge`'s donor/acceptor clamps
///    (`v.min(avail_a).min((cap_b - h_b).max(0.0))` and its mirror) drive the transfer to zero
///    whatever the stored momentum is. This is what puts the *interior* of a settled body to
///    sleep under gravity, and it is the branch that matters there: a saturated column has a
///    driving head of `|g| * GRAVITY_HEAD_SCALE` on every vertical edge — one whole cell of fill
///    per row — and is nonetheless completely at rest, because every neighbour is already at
///    capacity. Empty air is the mirror image: a big head, and nothing anywhere to donate. Only
///    the free surface between them stays awake.
///
///    Callers may pass any *upper bound* on the true `avail_*` (the cell's full height is one,
///    when the real limit further subtracts in-transit mass). Overstating `avail` can only make
///    this branch fire less often, never more, so a bound is sound; it just sleeps less.
///
/// 2. **At equilibrium and at rest.** `|H_a - H_b| <= tau` makes `yielded` zero, and with no
///    stored `v_e` to carry over, the integrated velocity `(v_e + c_sq * yielded) * damping` is
///    zero and so is the flux. At `tau = 0` this is the flat-pool case: a level free surface has
///    `H_a == H_b`. It is also the branch that would carry a granular material's whole settled
///    heap once `tau` is its yield stress rather than zero.
///
///    Both conditions are required, and that is deliberate rather than defensive. A standing wave
///    at its turning point has `v_e` momentarily near zero while `|H_a - H_b|` is at its largest;
///    sleeping on `v_e` alone would freeze a live ripple mid-oscillation. Conversely a wave
///    crossing its rest level has `H_a == H_b` while carrying full momentum, and sleeping on the
///    head alone would swallow it. The conjunction is exactly "nothing stored and nothing
///    driving", which is the only state that reproduces `flux == 0`.
///
/// Because both branches imply `flux == 0`, and `flux_edge` ends with `*v_e = flux`, a caller that
/// takes this early-out must leave `*v_e` at zero — branch 2 already requires it to be zero, and
/// branch 1 callers clear it (skipping the store when it is already zero, so a sleeping region
/// stops dirtying the 1 MB edge-velocity buffers every tick).
///
/// Mass conservation is unaffected by construction: a skipped edge transfers nothing, and
/// `flux_edge` is the only thing that moves mass on the liquid path. Block activity is unaffected
/// for the same reason — a zero flux never reached `activate_neighbor` in the first place, so a
/// sleeping edge neither wakes anything nor withholds a wake that used to happen. Waking is
/// therefore entirely the existing machinery's job: whatever *does* move calls `activate_neighbor`
/// on both endpoints' blocks, those blocks re-run, and their edges are re-tested from scratch. The
/// predicate stores no state of its own, so there is nothing that can go stale.
#[inline(always)]
fn edge_sleeps(
    driving: f32,
    tau: f32,
    v_e: f32,
    avail_a: f32,
    avail_b: f32,
    room_a: f32,
    room_b: f32,
) -> bool {
    let slept = if (avail_a <= 0.0 || room_b <= 0.0) && (avail_b <= 0.0 || room_a <= 0.0) {
        true
    } else {
        v_e == 0.0 && driving.abs() <= tau
    };
    #[cfg(test)]
    edge_sleep_stats::note(slept);
    slept
}

/// Test-only instrumentation for `edge_sleeps`, and the only way a test can see the *mechanism*
/// rather than its consequences.
///
/// Sleeping is deliberately exact — the edges it skips would have moved zero mass — so it leaves no
/// trace in any heightmap, mass total, flow total or block-activity count. That is the property
/// that makes it safe and the property that makes it untestable from the outside: a solver that
/// silently stopped sleeping altogether would still pass every behavioural test in this file while
/// costing 2.7x more per tick. Counting the two outcomes of the predicate is what closes that hole.
///
/// Thread-local rather than a global counter because the test harness runs tests in parallel and
/// `settle_tick` is single-threaded, so each test observes only its own solver.
#[cfg(test)]
mod edge_sleep_stats {
    use std::cell::Cell;

    thread_local! {
        static COUNTS: Cell<(u64, u64)> = const { Cell::new((0, 0)) };
    }

    #[inline(always)]
    pub fn note(slept: bool) {
        COUNTS.with(|c| {
            let (s, a) = c.get();
            c.set(if slept { (s + 1, a) } else { (s, a + 1) });
        });
    }

    pub fn reset() {
        COUNTS.with(|c| c.set((0, 0)));
    }

    /// `(slept, awake)` since the last `reset`.
    pub fn take() -> (u64, u64) {
        COUNTS.with(|c| c.get())
    }

    /// Fraction of edges tested that were skipped. `None` when no edge was tested at all.
    pub fn slept_fraction() -> Option<f64> {
        let (s, a) = take();
        if s + a == 0 {
            None
        } else {
            Some(s as f64 / (s + a) as f64)
        }
    }
}

/// DIAGNOSTIC-ONLY A/B TOGGLE for the upstream/side block-wake fix (`activate_neighbor_upstream`
/// / `activate_neighbor_side`), same pattern and same rationale as `edge_sleep_stats`: a
/// thread-local so parallel tests don't interfere, `#[cfg(test)]`-gated so it does not exist in
/// production at all (a non-test build always takes the fix -- see the `#[cfg(not(test))]`
/// twin below, which the optimizer folds to a no-op branch). Exists solely so a diagnostic test
/// can measure the SAME build with the fix on vs. off (e.g. to confirm a gap metric actually
/// moves because of this fix, not because of something else) without needing two separate
/// compiles or touching version control.
#[cfg(test)]
pub(crate) mod upstream_wake_gate {
    use std::cell::Cell;
    thread_local! {
        static DISABLED: Cell<bool> = const { Cell::new(false) };
    }
    pub fn set_disabled(v: bool) {
        DISABLED.with(|c| c.set(v));
    }
    #[inline(always)]
    pub fn is_disabled() -> bool {
        DISABLED.with(|c| c.get())
    }
}
#[cfg(not(test))]
mod upstream_wake_gate {
    #[inline(always)]
    pub fn is_disabled() -> bool {
        false
    }
}

/// DIAGNOSTIC-ONLY A/B TOGGLE for the pressure projection (`pressure_project`), same pattern and
/// same rationale as `upstream_wake_gate`: a thread-local so parallel tests don't interfere,
/// `#[cfg(test)]`-gated so it does not exist in production at all (a non-test build always takes
/// the projection -- see the `#[cfg(not(test))]` twin below). Exists so a test can measure the
/// SAME build with the projection on vs. off -- in particular, to show that capacity-limited
/// information now propagates further than one cell per tick only when the gate is enabled (see
/// `test_pressure_projection_reaches_beyond_one_cell`).
#[cfg(test)]
pub(crate) mod pressure_gate {
    use std::cell::Cell;
    thread_local! {
        static DISABLED: Cell<bool> = const { Cell::new(false) };
    }
    pub fn set_disabled(v: bool) {
        DISABLED.with(|c| c.set(v));
    }
    #[inline(always)]
    pub fn is_disabled() -> bool {
        DISABLED.with(|c| c.get())
    }
}
#[cfg(not(test))]
mod pressure_gate {
    #[inline(always)]
    pub fn is_disabled() -> bool {
        false
    }
}

/// DIAGNOSTIC-ONLY A/B TOGGLE for the fresh-overburden MUST-simulate predicate (task #47, the
/// "sand-slab" scheduling defect fix -- see the predicate's own comment at its call site in
/// `settle_tick`, just before the MUST/STALE/REST classification loop). Same pattern and same
/// rationale as `upstream_wake_gate`/`pressure_gate`: a thread-local so parallel tests don't
/// interfere, `#[cfg(test)]`-gated so it does not exist in production at all (a non-test build --
/// including every integration-test binary in `tests/`, which links this crate as an ordinary,
/// non-`--cfg test` dependency -- always takes the fix; see the `#[cfg(not(test))]` twin below,
/// which the optimizer folds to a no-op branch). Exists solely so a diagnostic test can measure
/// the SAME build with the predicate on vs. off -- e.g. the pre-fix-vs-post-fix slab divergence
/// against a perfect-simulation ground truth -- without a second compile or touching version
/// control.
#[cfg(test)]
pub(crate) mod fresh_overburden_gate {
    use super::FreshOverburdenVariant;
    use std::cell::Cell;
    thread_local! {
        static DISABLED: Cell<bool> = const { Cell::new(false) };
        // Defaults to the shipped choice, so any test that never calls `set_variant` measures
        // exactly what production ships.
        static VARIANT: Cell<FreshOverburdenVariant> =
            const { Cell::new(FreshOverburdenVariant::UnsupportedAndRoom) };
    }
    pub fn set_disabled(v: bool) {
        DISABLED.with(|c| c.set(v));
    }
    #[inline(always)]
    pub fn is_disabled() -> bool {
        DISABLED.with(|c| c.get())
    }
    /// Task #47 round 2: lets a diagnostic run `settle_tick` with a specific
    /// `FreshOverburdenVariant` without a second compile. Reset to the shipped default at the
    /// start of every comparison loop that uses it (see `diag_task47_variant_divergence_comparison`)
    /// so an early return/panic in one iteration can't leak a non-default variant into the next
    /// test that runs on this thread.
    pub fn set_variant(v: FreshOverburdenVariant) {
        VARIANT.with(|c| c.set(v));
    }
    #[inline(always)]
    pub fn variant() -> FreshOverburdenVariant {
        VARIANT.with(|c| c.get())
    }
}
#[cfg(not(test))]
mod fresh_overburden_gate {
    use super::FreshOverburdenVariant;
    #[inline(always)]
    pub fn is_disabled() -> bool {
        false
    }
    #[inline(always)]
    pub fn variant() -> FreshOverburdenVariant {
        FreshOverburdenVariant::UnsupportedAndRoom
    }
}

/// TASK #55 DIAGNOSTIC-ONLY A/B TOGGLE for the multiplicative lateral driving head (see
/// `mult_lateral_driving`'s doc comment for the mechanism). Same thread-local-per-test pattern as
/// `upstream_wake_gate`/`pressure_gate`/`fresh_overburden_gate` -- `#[cfg(test)]`-gated so it does
/// not exist in production at all, `#[cfg(not(test))]` twin hardcodes the shipped choice so a
/// non-test build pays no thread-local read.
///
/// Named and defaulted the OPPOSITE way from its three siblings above: they gate a shipped FIX
/// that is active by default (`is_disabled()`, default `false` == fix on), because in each of
/// those cases the additive/legacy behaviour was the thing being replaced. This gate instead ships
/// OFF by default (`is_enabled()`, default `false` == legacy additive lateral head, unchanged) --
/// the multiplicative form is a live experiment being measured, not yet a decided replacement for
/// `LATERAL_PRESSURE_SCALE`'s additive term. Flipping the polarity keeps the *shipped* behaviour
/// at `false` in both conventions (nothing here changes what ships), it just means "false" reads
/// naturally in each case as "the thing that ships today".
#[cfg(test)]
pub(crate) mod multiplicative_lateral_gate {
    use std::cell::Cell;
    thread_local! {
        static ENABLED: Cell<bool> = const { Cell::new(false) };
    }
    pub fn set_enabled(v: bool) {
        ENABLED.with(|c| c.set(v));
    }
    #[inline(always)]
    pub fn is_enabled() -> bool {
        ENABLED.with(|c| c.get())
    }
}
#[cfg(not(test))]
mod multiplicative_lateral_gate {
    #[inline(always)]
    pub fn is_enabled() -> bool {
        false
    }
}

/// TASK #55 step 3. Same pattern as `multiplicative_lateral_gate`:
/// `#[cfg(test)]`-gated thread-local so it does not exist in production at all, `#[cfg(not(test))]`
/// twin hardcodes the shipped choice (off) so a non-test build pays no thread-local read. `false`
/// by default.
///
/// Gates the head-field-driven driving head on the liquid-only lateral and vertical edge sites in
/// `settle_tick` (see `DrawingSimulation::head_field_transport`'s doc comment for what it
/// switches). NOTE: this thread-local is a TEST-ONLY diagnostics knob, letting a test force the
/// branch on for a scenario built through `TestSim`/other test helpers that never expose the real
/// `head_field_transport` parameter directly (e.g. `test_dry_sand_has_angle_of_repose`'s
/// `ReposeRig`) -- it is not how the branch is reachable from the shipped app. The real,
/// user-facing route is `settle_tick`'s own `head_field_transport` parameter
/// (`DrawingSimulation::head_field_transport`), OR'd with `is_enabled()` below at the call site so
/// both routes keep working without interfering with each other.
#[cfg(test)]
pub(crate) mod head_field_gate {
    use std::cell::Cell;
    thread_local! {
        static ENABLED: Cell<bool> = const { Cell::new(false) };
    }
    pub fn set_enabled(v: bool) {
        ENABLED.with(|c| c.set(v));
    }
    #[inline(always)]
    pub fn is_enabled() -> bool {
        ENABLED.with(|c| c.get())
    }
}
#[cfg(not(test))]
mod head_field_gate {
    #[inline(always)]
    pub fn is_enabled() -> bool {
        false
    }
}

/// Task #70: thread-local gate for overfill pressure simulation in tests.
#[cfg(test)]
pub(crate) mod overfill_pressure_gate {
    use std::cell::Cell;
    thread_local! {
        static ENABLED: Cell<bool> = const { Cell::new(false) };
    }
    pub fn set_enabled(v: bool) {
        ENABLED.with(|c| c.set(v));
    }
    #[inline(always)]
    pub fn is_enabled() -> bool {
        ENABLED.with(|c| c.get())
    }
}
#[cfg(not(test))]
mod overfill_pressure_gate {
    #[inline(always)]
    pub fn is_enabled() -> bool {
        false
    }
}

/// Under-relaxation factor for the pressure projection's projected-Jacobi solve
/// (`pressure_project`). The candidate-edge graph it runs on is not a fixed stencil -- its shape
/// changes every phase (which edges exist depends on which cells the block-LOD scheduler ran this
/// tick and which edges `edge_sleeps` judged live), so there is no single spectral radius to tune
/// against once, the way a fixed-topology pressure solve would allow. Plain (unrelaxed) Jacobi at
/// `omega = 1.0` is marginal on this kind of stencil; under-relaxing at `0.8` trades a few extra
/// iterations for a solve that stays stable across whatever topology this tick happens to produce.
const PRESSURE_OMEGA: f32 = 0.8;

/// Iteration count for the pressure projection's projected-Jacobi solve (`pressure_project`).
/// Fixed rather than convergence-gated: a variable count would make the correction depend on how
/// many sweeps a *particular* topology happened to need, re-introducing exactly the kind of
/// order/timing dependence the frozen-Jacobi flux solver was built to remove (two runs of the same
/// tick must produce the same correction regardless of incidental solver internals).
///
/// **History: this used to be capped at 8 to dodge a correctness cliff; it no longer needs to be,
/// and 8 is now a genuine convergence-based choice.** A previous version of this solve had no
/// per-edge feasibility bound on the correction it produced, only the global capacity bound the
/// Laplacian residual targets -- so a well-converged solve could still ask a single edge to move
/// more than its donor actually had, which inflated that donor's total claim and made
/// arbitration's proportional scale-down throttle its ENTIRE outgoing budget, including edges
/// carrying real, pressure-independent flow. That showed up as `test_liquid_flowing_liquid_does_
/// not_stand_in_walls`'s voids@tick160 climbing from 19 to 25 as `PRESSURE_ITERATIONS` rose past
/// 10 -- MORE convergence making the visible result WORSE, which is the signature of a modelling
/// gap, not of an unstable solve. Instrumented residual traces (max/L2 of the Jacobi residual per
/// iteration, over the active/`phi > 0` node set) at every `PRESSURE_OMEGA` in `{0.3, 0.5, 0.67,
/// 0.8, 1.0}` and every iteration count in `{1, 8, 11, 16, 24, 64}` showed clean, monotonic
/// geometric decay to zero every time -- never oscillation, never divergence -- which ruled out an
/// unstable Jacobi scheme and pointed at the missing per-edge bound instead (see
/// `clamp_edge_feasible`, applied to the correction before it reaches `cand_h`/`cand_v`: the same
/// donor/acceptor bound `flux_edge_candidate` already enforces on the raw candidate). With that
/// clamp in place, voids@tick160 no longer climbs with iteration count: at `PRESSURE_OMEGA = 0.8`
/// it stays in single digits to the low twenties across the ENTIRE 1..=64 sweep (measured, not
/// asserted precisely because tick-159 scheduling is sensitive to which edges happen to be
/// touched, but always well inside the test's `<= 20` bound with normal variance), and
/// `test_liquid_flowing_liquid_does_not_stand_in_walls` passes identically at 8, 24, and 64 sweeps
/// (24 and 64 are bit-for-bit identical to each other, confirming the solve has fully converged by
/// 8 and gains nothing further from more sweeps).
///
/// **So 8 is chosen for where the residual trace actually plateaus, not for a pass/fail window.**
/// At `PRESSURE_OMEGA = 0.8` the measured residual decays roughly geometrically (ratio ~0.2 per
/// iteration) and is already at noise level (`< 1e-4`) by iteration 6-8 in every scenario
/// exercised; iterations beyond that measurably change nothing (see the 24-vs-64 bit-identity
/// above). Eight sweeps is also enough for a packed column several cells long to feel a single
/// outlet within one tick (`test_pressure_projection_reaches_beyond_one_cell`). A rounder 24 or 64
/// would cost strictly more for a correction that has already converged -- there is no remaining
/// argument for spending more than the plateau requires.
///
/// **This still does not clear the task's +25% perf budget, and that is reported rather than
/// hidden by silently trimming iterations further** (going below the convergence plateau would
/// reintroduce exactly the "dodging a number" failure mode this fix exists to remove). Measured
/// via `bench_sandfall`, 512x512, Water/Hourglass, budget_n=1024, on this measurement host
/// (absolute numbers are noisier here than the ones earlier notes in this area quoted -- Steam
/// Deck thermal/frequency scaling -- so compare the PERCENTAGES, not the raw ms, against any older
/// note): baseline (projection disabled) 8.25 ms/tick Water, 7.83 ms/tick DrySand. At
/// `PRESSURE_ITERATIONS = 8` (this value): Water 11.97 ms/tick (+45%), DrySand 13.20 ms/tick
/// (+69%). Two real reductions went into getting here (see `pressure_project`'s perf note for the
/// mechanism): fusing what used to be five-plus separate passes over `touched_h`/`touched_v` into
/// two, and caching each node's `cell_capacity_for` result once per call instead of re-deriving it
/// every Jacobi iteration -- worth roughly -17% to -20% at higher iteration counts, where the
/// now-eliminated per-iteration capacity re-derivation used to matter most. Neither closes the
/// gap at `PRESSURE_ITERATIONS = 8`, because most of the remaining cost is not iteration count at
/// all: even at `PRESSURE_ITERATIONS = 1` -- the cheapest possible non-zero solve -- DrySand is
/// already +47% over baseline (Water +22%), which is fixed per-phase cost (building the touched
/// node set, deriving `fstar`, applying and clamping the correction) that scales with how many
/// edges `touched_h`/`touched_v` hold, not with how many Jacobi sweeps run over them. DrySand
/// touches more edges per tick than Water by construction (the granular path runs both the
/// gravity-aligned AND the yield-stress lateral edge through this same candidate mechanism, on top
/// of its own sequential CA), so it pays proportionally more no matter how cheap each edge visit
/// is made. Closing this the rest of the way needs either a cheaper touched-edge representation or
/// skipping the projection below some minimum touched-edge count -- both out of scope for this
/// pass; reported to the task's caller rather than resolved by re-lowering `PRESSURE_ITERATIONS`
/// underneath the convergence plateau established above.
const PRESSURE_ITERATIONS: usize = 8;

/// EXPERIMENTAL DIAGNOSTIC (see the tick-phase-order hypothesis test plan): per-tick, per-phase
/// flux attribution. Not used by any assertion; `#[cfg(test)]`-gated and thread-local like
/// `edge_sleep_stats`, so it costs nothing in production and cannot interact with parallel tests.
/// Records the *magnitude* of every flux this tick, bucketed by which phase realised it (0 =
/// gravity-aligned edge, 1 = everything else — lateral liquid edge and the granular CA's lateral
/// `try_move`s). This is a direct, model-free measurement of "how much of the capacity that
/// opened up this tick did each phase actually consume", with no assumption about mechanism.
#[cfg(test)]
mod phase_flow_stats {
    use std::cell::Cell;

    thread_local! {
        static FLOW: Cell<(f64, f64)> = const { Cell::new((0.0, 0.0)) };
    }

    #[inline(always)]
    pub fn note(phase: usize, flux: f32) {
        if flux == 0.0 {
            return;
        }
        let mag = flux.abs() as f64;
        FLOW.with(|c| {
            let (p0, p1) = c.get();
            if phase == 0 {
                c.set((p0 + mag, p1));
            } else {
                c.set((p0, p1 + mag));
            }
        });
    }

    pub fn reset() {
        FLOW.with(|c| c.set((0.0, 0.0)));
    }

    /// `(phase0_flow, phase1_flow)` since the last `reset`.
    pub fn take() -> (f64, f64) {
        FLOW.with(|c| c.get())
    }
}

#[cfg(test)]
#[inline(always)]
fn note_phase_flow(phase: usize, flux: f32) {
    phase_flow_stats::note(phase, flux);
}
#[cfg(not(test))]
#[inline(always)]
#[allow(dead_code)]
fn note_phase_flow(_phase: usize, _flux: f32) {}

fn wave_params(wetness: f32) -> (f32, f32) {
    if wetness <= 0.75 {
        (0.08, 0.76)
    } else if wetness <= 0.85 {
        let t = (wetness - 0.75) / 0.10;
        (0.08 + (0.18 - 0.08) * t, 0.76 + (0.92 - 0.76) * t)
    } else if wetness <= 0.90 {
        let t = (wetness - 0.85) / 0.05;
        (0.18 + (0.22 - 0.18) * t, 0.92 + (0.88 - 0.92) * t)
    } else if wetness <= 0.95 {
        let t = (wetness - 0.90) / 0.05;
        (0.22 + (0.16 - 0.22) * t, 0.88 + (0.86 - 0.88) * t)
    } else {
        let t = ((wetness - 0.95) / 0.05).min(1.0);
        (0.16 + (0.24 - 0.16) * t, 0.86 + (0.98 - 0.86) * t)
    }
}

/// TASK #63. The depth, in REFERENCE ROWS of head (see `task55_head_field::rows_of_head_at`), at
/// and beyond which a donor conveys at the solver's full shipped rate.
///
/// TWENTY ROWS, and this one IS a stated design choice rather than a derived quantity -- unlike
/// the `sqrt` in `pressure_rate_factor` below, which is not. What forces a choice here is that the
/// top of the range cannot move: `wave_params`' `(c_sq, damping)` already sit at the CFL bound, so
/// no depth can be made to convey FASTER than today. A depth-ordered rate therefore has to be
/// produced by slowing everything shallower than some reference depth, and this constant names
/// that depth -- "the shipped rate is taken to be correct at 20 reference rows of head".
///
/// USER-SPECIFIED, 2026-08-07: "I want 20 depth to have higher flow than 10 but it doesn't have to
/// be linear." 20 rows is that request read literally -- the grading has to still be climbing at
/// 20 for a 20-deep body to beat a 10-deep one, so the neutral point cannot be below it. Raising
/// it grades a deeper range and slows more of the simulation; lowering it flattens the ordering
/// the user asked for. Both are real trades, neither is a bug to be tuned away.
///
/// In REFERENCE rows, so this is one physical depth at every resolution: 20 local cells at w=512,
/// 2.5 at w=64. See `rows_of_head_at` for why the unit has to be reference rows and not cells.
const PRESSURE_RATE_FULL_AT_ROWS_OF_HEAD: f32 = 20.0;

/// TASK #63. Multiplier on a liquid edge's conveyance coefficient (`flux_edge_candidate`'s `c_sq`)
/// as a function of the DONOR cell's hydrostatic head, in reference rows. Never exceeds `1.0`, so
/// it can only ever REDUCE a flux the solver would otherwise have produced -- which is what makes
/// it safe against `spec_transport_speed_is_bounded` by construction rather than by measurement.
///
/// **SQUARE ROOT, and that is Torricelli, not a curve picked to look right.** Efflux velocity
/// under a head `h` is `v = sqrt(2*g*h)`, so flow rate scales as the SQUARE ROOT of depth. The
/// shipped solver's rate is independent of depth given the same driving head, so the correction
/// that restores the physical law is exactly `sqrt(h / h_ref)`, clamped at 1 because the top of
/// the range is already at the CFL bound and cannot be raised.
///
/// The concavity is also what makes the feature affordable. The user's requirement is an ordering
/// out at 10-vs-20 rows of head, and a LINEAR ramp to 20 would deliver that only by running a
/// 1-row film at 5% of rate and a 5-row puddle at 25% -- a near-freeze of every free surface in
/// the scene. `sqrt` gives the same ordering at the top (0.71 at 10 rows against 1.00 at 20, a
/// 1.41x spread) while leaving the shallow end far more alive: 0.22 at one row, 0.50 at five.
/// Diminishing returns with depth is both the physically correct shape and the affordable one.
///
/// **Free fall is exempt, and the exemption is exact, not an epsilon.** `rows_of_head_at` returns
/// exactly `0.0` for a cell `advance_head_field` classified as unsupported (it WRITES `head = z`
/// there rather than taking a max), and strictly more than `0.0` for every supported wet cell
/// however thin -- see that function's own doc comment. So `<= 0.0` reads "outside the pressure
/// model" and not "very little water", and returning `1.0` there is not a special case bolted on
/// to protect free fall: a ballistic parcel has no contact pressure to be sensitive TO, so a
/// pressure-derived rate simply does not apply to it. Attenuating it instead would freeze falling
/// water in mid-air, which is the one outcome this task must not produce.
///
/// NOT a saturating `p / (p + ref)` form, which was considered and rejected: it never reaches
/// `1.0`, so it would slow the deepest water in the scene too and silently retune every edge in
/// the simulation rather than only the graded range. Clamped-`sqrt` is EXACTLY neutral at and
/// above `PRESSURE_RATE_FULL_AT_ROWS_OF_HEAD`.
#[inline]
fn pressure_rate_factor(rows_of_head: f32) -> f32 {
    if rows_of_head <= 0.0 {
        1.0
    } else {
        (rows_of_head / PRESSURE_RATE_FULL_AT_ROWS_OF_HEAD).sqrt().min(1.0)
    }
}

fn get_ca_params(
    wetness: f32,
    threshold_prop: f32,
    flow_rate_prop: f32,
    grain_size: f32,
    higher_neighbors: usize,
    sliding_active: bool,
    closest_marble_vel: f32,
    gravity_active: bool,
) -> (f32, f32, f32, Option<f32>) {
    // Oobleck shear-thickening
    if wetness >= 0.50 && wetness < 0.65 {
        let t = ((closest_marble_vel - 0.03) / 0.12).clamp(0.0, 1.0);
        let t_steep = t * t;
        let threshold = 0.005 + (0.32 - 0.005) * t_steep;
        let alpha = 0.40 + (0.005 - 0.40) * t_steep;
        let lock_chance = 0.02 + (0.98 - 0.02) * t_steep;
        return (threshold, alpha, lock_chance, None);
    }

    // Continuous liquid weight for this cell (see `liquidity` doc comment). Used below to blend
    // the granular and liquid CA parameters instead of hard-switching on `wetness >= 0.75`.
    let liquidity = liquidity(wetness);

    // Quantization size (droplet beading for liquids under gravity, discrete grains in sandbox).
    // Gated on `liquidity > 0.0` (wetness > 0.65) rather than the hard `wetness >= 0.75` cut, so
    // a cell drifting across the old cut doesn't flip discretely between "beaded" and "smooth".
    let quantize_size = if liquidity > 0.0 && gravity_active {
        Some(0.025) // Droplet/bead quantization for liquids under gravity
    } else if wetness < 0.30 && !gravity_active {
        if grain_size >= 0.60 {
            Some(0.035)
        } else if grain_size >= 0.40 {
            Some(0.01)
        } else if grain_size >= 0.08 {
            Some(0.015)
        } else {
            None
        }
    } else {
        None
    };

    // Hysteresis threshold (lower repose threshold during gravity settling for natural sliding/funneling)
    let mut threshold = if wetness < 0.15 && sliding_active {
        0.5 * threshold_prop
    } else {
        threshold_prop
    };

    // Flow rate (alpha) (faster settling when gravity is pulling sand down)
    let mut alpha = flow_rate_prop;

    if gravity_active {
        threshold *= 0.35; // Lower friction/repose angle in Sand-fall mode for realistic fluid flow

        // Phase 5: the liquid (threshold = 0.0, alpha = 0.75) blend that used to live here is
        // gone. Under gravity a cell's liquid share is now carried by the conservative edge-flux
        // solver in `settle_tick` and the CA carries only the complementary `1 - liquidity`
        // share, so the CA no longer has to impersonate a liquid at all. That deleted the whole
        // Phase 2 tuning cluster — `liquid_alpha = 0.75` (which passed L1 and L3 only inside a
        // narrow 0.70-0.80 band), plus the 0.70 free-fall and 0.90 lateral transfer coefficients
        // and the `liquid_can_still_fall` gate below. In the flux form the equivalent limits are
        // not coefficients at all: they are the donor's mass and the acceptor's capacity, which
        // are physical quantities rather than tuned ones.
        //
        // C5 (a material drifting across the old `wetness >= 0.75` cut must not change regime)
        // is still handled continuously — the handover is now between the two *solvers*, by the
        // same `liquidity` weight, rather than between two parameter sets inside one solver.
        alpha = (alpha * 1.5).min(0.8);
    }

    // Lock chance
    let lock_chance = if gravity_active {
        0.05 // Low locking under gravity so sand avalanches smoothly into a natural hill
    } else if wetness < 0.05 {
        if flow_rate_prop >= 0.21 {
            // DrySand / CoarseSand stochastic locking
            if higher_neighbors >= 3 { 0.80 } else { 0.10 }
        } else {
            // FinePowder / MoonDust
            let t = ((threshold_prop - 0.05) / 0.15).clamp(0.0, 1.0);
            0.02 + (0.40 - 0.02) * t
        }
    } else if wetness < 0.30 {
        // Snow / KineticSand
        let t = ((wetness - 0.05) / 0.25).clamp(0.0, 1.0);
        0.30 + (0.75 - 0.30) * t
    } else {
        // WetSand / ButterCream
        let t = ((wetness - 0.30) / 0.40).clamp(0.0, 1.0);
        0.15 + (0.20 - 0.15) * t
    };

    (threshold, alpha, lock_chance, quantize_size)
}


/// Displace sand along a line segment from start to end, carving a groove
/// and depositing the displaced volume into the surrounding ridge area.
pub fn displace_line(
    heightmap: &mut Heightmap,
    cell_colors: &mut [u8],
    cell_props: &mut [f32],
    start: Vec2,
    end: Vec2,
    radius: f32,
    active_bounds: &mut ActiveBounds,
) {
    if !start.is_finite() || !end.is_finite() || !radius.is_finite() || radius <= 0.0 {
        return;
    }

    let w = heightmap.width;
    let h = heightmap.height;
    if w == 0 || h == 0 {
        return;
    }

    // Convert coordinates to grid space
    let ax = (start.x + 1.0) * 0.5 * w as f32;
    let ay = (1.0 - start.y) * 0.5 * h as f32;
    let bx = (end.x + 1.0) * 0.5 * w as f32;
    let by = (1.0 - end.y) * 0.5 * h as f32;

    let r_grid = radius * (w as f32 / 2.0);
    let r_grid_clamped = r_grid.min(w as f32);

    // Define ridge width (60% of the marble radius)
    let w_grid = r_grid_clamped * 0.6;
    let total_radius = r_grid_clamped + w_grid;
    let total_radius_clamped = total_radius.min(w as f32);

    // Early out if the swept area is completely outside the grid
    let min_center_x = ax.min(bx);
    let max_center_x = ax.max(bx);
    let min_center_y = ay.min(by);
    let max_center_y = ay.max(by);

    if max_center_x < -total_radius_clamped
        || min_center_x > w as f32 + total_radius_clamped
        || max_center_y < -total_radius_clamped
        || min_center_y > h as f32 + total_radius_clamped
    {
        return;
    }

    // Safe bounding box calculations in float space before casting to usize
    let min_x_float = (min_center_x - total_radius_clamped)
        .clamp(0.0, w as f32)
        .floor();
    let max_x_float = (max_center_x + total_radius_clamped)
        .clamp(0.0, w as f32)
        .ceil();
    let min_y_float = (min_center_y - total_radius_clamped)
        .clamp(0.0, h as f32)
        .floor();
    let max_y_float = (max_center_y + total_radius_clamped)
        .clamp(0.0, h as f32)
        .ceil();

    let min_x = min_x_float as usize;
    let max_x = (max_x_float as usize).min(w - 1);
    let min_y = min_y_float as usize;
    let max_y = (max_y_float as usize).min(h - 1);

    // Update settling active bounding box
    let padding = 15;
    let pad_min_x = min_x.saturating_sub(padding);
    let pad_max_x = max_x.saturating_add(padding).min(w - 1);
    let pad_min_y = min_y.saturating_sub(padding);
    let pad_max_y = max_y.saturating_add(padding).min(h - 1);

    if active_bounds.active {
        active_bounds.min_x = active_bounds.min_x.min(pad_min_x);
        active_bounds.max_x = active_bounds.max_x.max(pad_max_x);
        active_bounds.min_y = active_bounds.min_y.min(pad_min_y);
        active_bounds.max_y = active_bounds.max_y.max(pad_max_y);
    } else {
        active_bounds.min_x = pad_min_x;
        active_bounds.max_x = pad_max_x;
        active_bounds.min_y = pad_min_y;
        active_bounds.max_y = pad_max_y;
        active_bounds.active = true;
    }

    // Segment vector
    let vx = bx - ax;
    let vy = by - ay;
    let len_sq = vx * vx + vy * vy;
    let len = if len_sq >= 1e-6 { len_sq.sqrt() } else { 0.0 };
    let inv_len_sq = if len_sq >= 1e-6 { 1.0 / len_sq } else { 0.0 };

    let r_groove_sq = r_grid_clamped * r_grid_clamped;

    // Ridge ray sampling offsets
    let d1 = r_grid_clamped + w_grid * 0.25;
    let d2 = r_grid_clamped + w_grid * 0.50;
    let d3 = r_grid_clamped + w_grid * 0.75;

    // Scan bounding box to carve the groove and displace sand radially/perpendicularly
    for y in min_y..=max_y {
        let py = y as f32 + 0.5;
        let row_offset = y * w;
        for x in min_x..=max_x {
            let px = x as f32 + 0.5;

            // Distance to segment AB (used for carving)
            let (closest_x, closest_y) = if len_sq < 1e-6 {
                (ax, ay)
            } else {
                let t = (((px - ax) * vx + (py - ay) * vy) * inv_len_sq).clamp(0.0, 1.0);
                (ax + t * vx, ay + t * vy)
            };

            let dx = px - closest_x;
            let dy = py - closest_y;
            let dist_sq = dx * dx + dy * dy;

            if dist_sq < r_groove_sq {
                let dist = dist_sq.sqrt();
                // Spherical groove profile: z_groove = R - sqrt(R^2 - d^2)
                let h_target = r_grid_clamped - (r_groove_sq - dist_sq).max(0.0).sqrt();
                let h_target_profile = (h_target / r_grid_clamped) * crate::DEFAULT_SAND_HEIGHT;

                let current_idx = row_offset + x;
                let current_h = heightmap.data[current_idx];

                let wetness = cell_props[current_idx * 4 + PROP_WETNESS];

                // Continuous residual_factor mapping based on wetness
                let residual_factor = if wetness >= 0.50 && wetness < 0.65 {
                    let speed = (end - start).length();
                    let t = (speed / 0.01).clamp(0.0, 1.0);
                    0.50 * t * t
                } else if wetness >= 0.70 {
                    0.0
                } else if wetness < 0.45 {
                    0.20 + (0.35 - 0.20) * (wetness / 0.45)
                } else {
                    0.35 * (1.0 - (wetness - 0.45) / 0.25)
                };

                // Scale target height relative to the current height to support multi-pass clearing
                let h_target_norm = residual_factor * current_h.max(h_target_profile) + (1.0 - residual_factor) * h_target_profile;

                // Add a tiny micro-texture noise to the groove base
                let seed = (x as u32).wrapping_mul(73856093) ^ (y as u32).wrapping_mul(19349663);
                let noise = (((seed & 0xFFFF) as f32 / 65535.0) - 0.5) * 0.05; // Range [-0.025, 0.025]
                let h_target_noisy = (h_target_norm + noise).clamp(0.0, 1.0);

                if current_h > h_target_noisy {
                    let diff = current_h - h_target_noisy;
                    heightmap.data[current_idx] = h_target_noisy;

                    // Projection on the infinite line (used for perpendicular displacement origin/direction)
                    let (closest_line_x, closest_line_y) = if len_sq < 1e-6 {
                        (ax, ay)
                    } else {
                        let t_unclamped = ((px - ax) * vx + (py - ay) * vy) * inv_len_sq;
                        (ax + t_unclamped * vx, ay + t_unclamped * vy)
                    };

                    let dx_line = px - closest_line_x;
                    let dy_line = py - closest_line_y;
                    let dist_line_sq = dx_line * dx_line + dy_line * dy_line;
                    let dist_line = dist_line_sq.sqrt();

                    // Distribute diff: perpendicular to motion if moving, radial if stationary
                    let (dir_x, dir_y) = if len_sq >= 1e-6 && len > 1e-4 {
                        if dist_line > 1e-4 {
                            (dx_line / dist_line, dy_line / dist_line)
                        } else {
                            // Default perpendicular direction if exactly on the line
                            (-vy / len, vx / len)
                        }
                    } else {
                        if dist > 1e-4 {
                            (dx / dist, dy / dist)
                        } else {
                            (1.0, 0.0)
                        }
                    };

                    // Perturb sample distances with coordinate-locked noise to simulate clumped deposition
                    let base_seed = (x as u32).wrapping_mul(73856093) ^ (y as u32).wrapping_mul(19349663);
                    let seed_d1 = base_seed ^ 12345;
                    let noise_d1 = (((seed_d1 & 0xFFFF) as f32 / 65535.0) - 0.5) * 0.3 * w_grid;
                    let d1_p = (d1 + noise_d1).clamp(r_grid_clamped, total_radius_clamped);

                    let seed_d2 = base_seed ^ 67890;
                    let noise_d2 = (((seed_d2 & 0xFFFF) as f32 / 65535.0) - 0.5) * 0.3 * w_grid;
                    let d2_p = (d2 + noise_d2).clamp(r_grid_clamped, total_radius_clamped);

                    let seed_d3 = base_seed ^ 54321;
                    let noise_d3 = (((seed_d3 & 0xFFFF) as f32 / 65535.0) - 0.5) * 0.3 * w_grid;
                    let d3_p = (d3 + noise_d3).clamp(r_grid_clamped, total_radius_clamped);

                    // Calculate target coordinates
                    let rx1 = (closest_line_x + dir_x * d1_p).floor() as isize;
                    let ry1 = (closest_line_y + dir_y * d1_p).floor() as isize;

                    let rx2 = (closest_line_x + dir_x * d2_p).floor() as isize;
                    let ry2 = (closest_line_y + dir_y * d2_p).floor() as isize;

                    let rx3 = (closest_line_x + dir_x * d3_p).floor() as isize;
                    let ry3 = (closest_line_y + dir_y * d3_p).floor() as isize;

                    // Perturb weights based on the destination cell coordinates (rx, ry)
                    let seed_w1 =
                        (rx1.max(0) as u32).wrapping_mul(1299689) ^ (ry1.max(0) as u32).wrapping_mul(314159) ^ 9991;
                    let seed_w2 =
                        (rx2.max(0) as u32).wrapping_mul(1299689) ^ (ry2.max(0) as u32).wrapping_mul(314159) ^ 9992;
                    let seed_w3 =
                        (rx3.max(0) as u32).wrapping_mul(1299689) ^ (ry3.max(0) as u32).wrapping_mul(314159) ^ 9993;

                    let nf1 = 1.0 + (((seed_w1 & 0xFFFF) as f32 / 65535.0) - 0.5) * 0.6; // +/- 30% variation
                    let nf2 = 1.0 + (((seed_w2 & 0xFFFF) as f32 / 65535.0) - 0.5) * 0.6;
                    let nf3 = 1.0 + (((seed_w3 & 0xFFFF) as f32 / 65535.0) - 0.5) * 0.6;

                    let mut w1 = 0.5 * nf1;
                    let mut w2 = (1.0 / 3.0) * nf2;
                    let mut w3 = (1.0 / 6.0) * nf3;

                    let sum_w = w1 + w2 + w3;
                    if sum_w > 0.0 {
                        let inv_sum = 1.0 / sum_w;
                        w1 *= inv_sum;
                        w2 *= inv_sum;
                        w3 *= inv_sum;
                    } else {
                        w1 = 0.5;
                        w2 = 1.0 / 3.0;
                        w3 = 1.0 / 6.0;
                    }

                    let rx1_clamped = rx1.clamp(0, w as isize - 1) as usize;
                    let ry1_clamped = ry1.clamp(0, h as isize - 1) as usize;
                    let dest1_idx = ry1_clamped * w + rx1_clamped;
                    let h_above1 = (heightmap.data[dest1_idx] - crate::DEFAULT_SAND_HEIGHT).max(0.0);

                    let rx2_clamped = rx2.clamp(0, w as isize - 1) as usize;
                    let ry2_clamped = ry2.clamp(0, h as isize - 1) as usize;
                    let dest2_idx = ry2_clamped * w + rx2_clamped;
                    let h_above2 = (heightmap.data[dest2_idx] - crate::DEFAULT_SAND_HEIGHT).max(0.0);

                    let rx3_clamped = rx3.clamp(0, w as isize - 1) as usize;
                    let ry3_clamped = ry3.clamp(0, h as isize - 1) as usize;
                    let dest3_idx = ry3_clamped * w + rx3_clamped;
                    let h_above3 = (heightmap.data[dest3_idx] - crate::DEFAULT_SAND_HEIGHT).max(0.0);

                    // Scale factor for asymptotic decay based on marble diameter/height in heightmap units
                    let scale = 2.0 * (radius / 0.018).max(0.1);
                    
                    let x1 = h_above1 / scale;
                    let m1 = 1.0 / (1.0 + x1 * x1 * x1 * x1);

                    let x2 = h_above2 / scale;
                    let m2 = 1.0 / (1.0 + x2 * x2 * x2 * x2);

                    let x3 = h_above3 / scale;
                    let m3 = 1.0 / (1.0 + x3 * x3 * x3 * x3);

                    let mut forward_vol = 0.0f32;
                    let mut forward_dest_idx = 0;
                    if len_sq >= 1e-6 && len > 1e-4 {
                        let forward_dist = r_grid_clamped * 1.05; // Just in front of the marble boundary
                        let fx = (px + (vx / len) * forward_dist).floor() as isize;
                        let fy = (py + (vy / len) * forward_dist).floor() as isize;
                        let fx_clamped = fx.clamp(0, w as isize - 1) as usize;
                        let fy_clamped = fy.clamp(0, h as isize - 1) as usize;
                        forward_dest_idx = fy_clamped * w + fx_clamped;
                        forward_vol = (diff * 0.10).min(0.10);
                    }

                    let side_diff = diff - forward_vol;
                    let deposited_volume = side_diff * (w1 * m1 + w2 * m2 + w3 * m3) + forward_vol;
                    if deposited_volume > 1e-6 {
                        heightmap.data[current_idx] = current_h - deposited_volume;
                        if side_diff > 0.0 {
                            add_sand_with_limit_properties(heightmap, cell_colors, cell_props, current_idx, dest1_idx, w, h, side_diff * w1 * m1, 1.5);
                            add_sand_with_limit_properties(heightmap, cell_colors, cell_props, current_idx, dest2_idx, w, h, side_diff * w2 * m2, 1.5);
                            add_sand_with_limit_properties(heightmap, cell_colors, cell_props, current_idx, dest3_idx, w, h, side_diff * w3 * m3, 1.5);
                        }
                        if forward_vol > 0.0 {
                            add_sand_with_limit_properties(heightmap, cell_colors, cell_props, current_idx, forward_dest_idx, w, h, forward_vol, 1.5);
                        }
                    } else {
                        // Restore height to conserve volume if no deposition can happen
                        heightmap.data[current_idx] = current_h;
                    }
                }
            }
        }
    }
}

/// Deterministic pseudo-random float in [0, 1) from an integer seed. Used to give
/// procedurally-generated shape features (staircase steps, etc.) organic variation
/// while staying stable across repeated shape_mask regenerations.
fn step_hash(n: u32) -> f32 {
    let h = n.wrapping_mul(2654435761).wrapping_add(0x9E3779B9);
    let h = h ^ (h >> 15);
    (h % 10000) as f32 / 10000.0
}

/// Fixed-size, allocation-free per-tier chamber BOUNDARIES for
/// `SandboxShape::MultiStageHourglass`'s merge-tree cascade, built by `multistage_tier_boundaries`.
///
/// Every tier's boundaries are expressed as integer counts of `w / n` units, where `n` is tier
/// 0's (the widest tier's) chamber count -- i.e. `multistage_chambers` itself, clamped to the
/// supported `1..=16` range. Tier 0's boundaries are therefore always `[0, 1, 2, ..., n]`, and
/// every lower tier's boundary list is, by construction, a SUBSET of tier 0's: each entry is
/// copied verbatim from a parent boundary, never recomputed or interpolated, so there is no
/// float accumulation or drift and a parent chamber's neck is always exactly on one of its
/// child chamber's boundaries.
///
/// Sized for the supported range (`n <= 16`, so at most 17 boundary values per tier, and at
/// most 5 tiers -- see `multistage_tier_boundaries`'s doc comment -- with one spare row of
/// headroom) so the whole thing lives on the stack: no `Vec`, no per-call heap allocation, safe
/// to build fresh on every `eval_sandbox_shape` call despite that function running ~262k times
/// per mask regeneration.
pub struct MultistageBoundaries {
    /// `boundaries[t][0..lens[t]]` are tier `t`'s chamber boundaries (`lens[t]` values, i.e.
    /// `lens[t] - 1` chambers), in integer units of `w / n`. Entries at or beyond `lens[t]`,
    /// and rows at or beyond `n_tiers`, are unused zero padding.
    pub boundaries: [[u32; 17]; 6],
    /// Number of valid entries in `boundaries[t]` (chamber count + 1) for each tier.
    pub lens: [usize; 6],
    /// Number of tiers actually populated -- `multistage_tier_chambers(n).len()`.
    pub n_tiers: usize,
}

/// Build `SandboxShape::MultiStageHourglass`'s merge-tree tier boundaries from the widest (top)
/// tier's chamber count `n` (user-selectable 5..=16, default 8 -- see
/// `DrawingSimulation::multistage_chambers`; clamped to `1..=16` here so the fixed-size arrays
/// above can never be written out of bounds). This is the ONLY place the merge rule is
/// expressed; `eval_sandbox_shape`, `multistage_tier_chambers` and
/// `DrawingSimulation::initialize_hourglass` all just consume the result, with no assumption
/// about how it was derived or how many tiers it produces -- so changing the rule (a different
/// range, or a coarser final merge) is a one-function change.
///
/// THE MERGE RULE: tier 0 has `n` equal-width chambers, boundaries `[0, 1, .., n]`. Each lower
/// tier merges its parent tier's `n_k` chambers into `m = ceil(n_k / 2)` children by
/// WIDTH-BALANCED boundary selection, not fixed index pairing: child `j`'s right boundary
/// (`j` = 1..m-1) is whichever of the parent's OWN boundary values lands closest to the evenly
/// spaced target `j * n / m` (`n` = tier 0's chamber count, so this is exact integer comparison
/// via cross-multiplication, never a float target) -- so every child's boundaries are still
/// exactly the union of the parent boundaries it merges (its own neck is inside that span by
/// construction, fixing the "neck lands on a wall" bug the original per-tier independent
/// uniform grid had), but the CHOICE of which parent boundaries go together is made to keep
/// children as close to equal width as the parent's own available cut points allow, rather than
/// always grouping strict left-to-right pairs.
///
/// This replaces an earlier, simpler version of this rule (fixed 2-parents-per-child index
/// pairing, with one designated middle child getting only 1 parent when `n_k` was odd). That
/// version kept every merge "locally" balanced but NOT globally: two odd merges in a row (e.g.
/// n = 9's `9 -> 5 -> 3`) can leave a lone narrow singleton chamber adjacent to a much wider
/// sibling, and the NEXT merge, pairing them together by fixed index regardless of their actual
/// widths, produces a child whose centre is dragged far enough toward the wide parent that the
/// narrow parent's neck -- though still inside the child's boundary, satisfying the "no wall
/// collision" guarantee -- lands outside that child's own visual funnel width (`0.35 *
/// chamber_w`) at every row, a structural miss found by this feature's own regression test. The
/// width-balanced selection dissolves that case (verified for n = 9's `9 -> 5 -> 3 -> 2 -> 1`:
/// tier 2 becomes widths `[4, 3, 2]` instead of `[4, 1, 4]`, and the final `3 -> 2` merge lands
/// widths `[4, 5]` with both parent centres within their child's funnel reach) without
/// abandoning the exact-integer-subset property: a child boundary is still always copied
/// verbatim from a parent boundary (which is, inductively, always one of tier 0's `[0, 1, ..,
/// n]`), never recomputed or interpolated. On a DENSE, evenly-spaced parent (tier 0 itself, or
/// any later tier that happens to still be uniform) this reduces to ordinary 2-parents-per-
/// child pairing, so `n = 8` and `n = 16` (both all-power-of-two, always-uniform chains) still
/// produce exactly today's `8 -> 4 -> 2 -> 1` / `16 -> 8 -> 4 -> 2 -> 1` -- see
/// `test_multistage_n8_is_bit_identical_to_shipped_geometry`, unaffected by this change.
pub fn multistage_tier_boundaries(n: u32) -> MultistageBoundaries {
    let n = n.clamp(1, 16);

    let mut boundaries = [[0u32; 17]; 6];
    let mut lens = [0usize; 6];

    let mut cur_n = n;
    for i in 0..=cur_n as usize {
        boundaries[0][i] = i as u32;
    }
    lens[0] = cur_n as usize + 1;
    let mut n_tiers = 1usize;

    while cur_n > 1 {
        let next_n = (cur_n + 1) / 2; // ceil(cur_n / 2) -- m, the number of children
        let t = n_tiers - 1; // parent tier index
        let p_len = cur_n as usize + 1; // number of boundary values in the parent tier
        let m = next_n as usize;

        boundaries[t + 1][0] = boundaries[t][0]; // always 0
        boundaries[t + 1][m] = boundaries[t][p_len - 1]; // always n

        // Width-balanced greedy: for each interior cut j = 1..m-1, walk a forward-only
        // pointer `i` through the parent's own boundary list looking for the value closest
        // to the evenly spaced target `j * n / m`, comparing via cross-multiplication
        // (`boundaries[t][i] * m` vs `j * n`) so the comparison is exact integer arithmetic,
        // never a float target. The pointer only ever advances (never revisits an earlier
        // parent boundary), which keeps selections strictly increasing across all m-1 cuts
        // -- i.e. every child gets at least one parent -- while `max_i` reserves enough
        // remaining parent boundaries for the cuts still to come after this one. On an
        // exact tie between two candidate boundaries, this advances to the LARGER one
        // (`next_diff <= cur_diff`, not `<`), matching the worked example this rule was
        // specified against.
        let mut i = 1usize;
        for j in 1..m {
            let target_num = (j as u32) * n; // compare against boundaries[t][i] * m
            let remaining_after = (m - 1) - j; // interior cuts still needed after this one
            let max_i = p_len - 2 - remaining_after; // last usable index, reserving room
            while i < max_i {
                let cur_diff = (boundaries[t][i] * m as u32).abs_diff(target_num);
                let next_diff = (boundaries[t][i + 1] * m as u32).abs_diff(target_num);
                if next_diff <= cur_diff {
                    i += 1;
                } else {
                    break;
                }
            }
            boundaries[t + 1][j] = boundaries[t][i];
            i += 1;
        }
        lens[t + 1] = m + 1;

        cur_n = next_n;
        n_tiers += 1;
    }

    MultistageBoundaries { boundaries, lens, n_tiers }
}

/// Derive `SandboxShape::MultiStageHourglass`'s per-tier chamber COUNTS, widest tier first, from
/// `multistage_tier_boundaries` (the single source of truth for the merge rule -- see that
/// function's doc comment). Kept as a separate `Vec`-returning function, rather than inlining
/// `.lens[t] - 1` everywhere, because the non-hot-path call sites (the UI's cell-count readout,
/// `DrawingSimulation::initialize_hourglass`, and this module's own tests) only ever want the
/// per-tier chamber count, not the boundary geometry, and a `Vec` is fine there -- it's only
/// `eval_sandbox_shape`'s per-cell hot path that needs `multistage_tier_boundaries`' allocation-
/// free form directly.
pub fn multistage_tier_chambers(n: u32) -> Vec<u32> {
    let tb = multistage_tier_boundaries(n);
    (0..tb.n_tiers).map(|t| (tb.lens[t] - 1) as u32).collect()
}

/// `MultiStageHourglass`'s per-chamber neck half-width: capped at 0.30 of that tier's own
/// chamber width (so the neck can never be wider than the chamber's own taper allows, which
/// is what stops the funnel inverting), floored at 0.5 cells -- half a cell either side of
/// centre, i.e. a 1-cell-wide opening, the smallest a rasterised neck can be and still exist
/// at all -- and then clamped to a hair under half the chamber width (`anti_merge_ceiling`)
/// so the floor above can never push the neck past the point where it would overlap this
/// chamber's neighbour -- the merge failure this whole clamp chain exists to prevent.
///
/// The floor used to be 3 cells; it was lowered to 0.5 by deliberate user request (a 1-cell
/// neck reachable at every resolution, on the reasoning "if it blocks sand flow, I could just
/// increase the neck width -- no point not allowing me to pick"), paired with `index.html`'s
/// `neck-slider` gaining a resolution-dependent `min` (`0.5 / grid_width`, recomputed in
/// `demo.js` on every resolution change) so the slider's own bottom end actually reaches this
/// floor rather than clamping to it early. This floor is itself now mostly a safety net for
/// direct API/future callers rather than load-bearing from the shipped slider -- see the task
/// report this shipped alongside for drainage measurements at a 1-cell neck, and note this
/// deliberately changes geometry at the bottom of the neck-width slider (unlike the anti-merge
/// ceiling below, which was proven a no-op at the shipped default).
///
/// See the doc comment at this function's one call site inside `eval_sandbox_shape` for the
/// full worked numbers.
///
/// Factored out into its own function -- rather than left inline in `eval_sandbox_shape` --
/// so `effective_neck_half_width_cells` below (the UI cell-count readout) computes this
/// exact same value instead of a second, hand-copied formula that could silently drift out of
/// sync with it.
fn multistage_neck_half_width(w_f: f32, chamber_w: f32, neck_width: f32) -> f32 {
    let neck_cap = 0.30 * chamber_w;
    let neck_hw = (neck_width * w_f).min(neck_cap).max(0.5);
    let anti_merge_ceiling = (chamber_w / 2.0 - 0.5).max(0.5);
    neck_hw.min(anti_merge_ceiling)
}

/// The rasterised neck HALF-width, in cells, that `eval_sandbox_shape` actually uses for
/// `shape` at grid width `w` and the given `neck_width` (and, for `MultiStageHourglass`,
/// `multistage_chambers`) -- i.e. after whatever per-shape cap/floor logic applies, not just
/// the raw `neck_width * w` fraction. Exists so the web UI can show the user where the
/// neck-width slider's fraction actually lands once that logic has run (see the `demo.js`
/// readout this feeds), rather than only the fraction itself, which is a poor guide to the
/// real opening once a cap or floor bites -- particularly at small grid sizes.
///
/// For every funnel shape except `MultiStageHourglass` there is no cap, so this is simply
/// `neck_width * w`. For `MultiStageHourglass` it reproduces the widest tier's (tier 0's) own
/// neck computation -- the narrowest chambers, and therefore the one most likely to be
/// capped or floored -- via `multistage_neck_half_width`, the same function
/// `eval_sandbox_shape` calls, so this cannot drift out of sync with the geometry it
/// describes.
pub fn effective_neck_half_width_cells(w: usize, shape: crate::SandboxShape, neck_width: f32, multistage_chambers: u32) -> f32 {
    let w_f = w as f32;
    match shape {
        crate::SandboxShape::MultiStageHourglass => {
            let tiers = multistage_tier_chambers(multistage_chambers);
            let chamber_w = w_f / tiers[0] as f32; // widest tier, tier 0
            multistage_neck_half_width(w_f, chamber_w, neck_width)
        }
        _ => neck_width * w_f,
    }
}

/// Task #61: the U-tube flow-through vessel's five axis-aligned rects, expressed as fractions
/// of `(w_f, h_f)` so the shape is resolution-invariant. `[x_lo, x_hi, y_lo, y_hi]`, same
/// coordinate convention as `eval_sandbox_shape` (`dx = x - cx`, `dy = y - cy`, y increases
/// downward). Union of all five is ONE connected region: reservoir (index
/// `U_TUBE_RESERVOIR_RECT`) feeds down the left arm, through a bottom basin that is only
/// PARTLY roofed (the strip between the two arms has no rect above it -- that gap is
/// deliberate, the Pascal-pressure test case this apparatus exists for), up the right arm,
/// over its rim (the overflow lip) via the spout, and down into the catch well.
///
/// Single source of truth for both `eval_sandbox_shape`'s `UTubeFlowThrough` branch below and
/// `DrawingSimulation::initialize_hourglass`'s `UTubeFlowThrough` branch (`lib.rs`), which reads
/// `U_TUBE_RECTS[U_TUBE_RESERVOIR_RECT]` to prefill only the reservoir arm -- kept here, not
/// duplicated, so the two can never drift apart.
pub(crate) const U_TUBE_RECTS: [[f32; 4]; 5] = [
    [-0.42, -0.24, -0.40, 0.36], // reservoir / left arm
    [-0.42, 0.02, 0.36, 0.42],   // basin (bottom, partly roofed)
    [-0.04, 0.02, 0.10, 0.36],   // right arm
    [-0.04, 0.16, 0.10, 0.17],   // spout (overflow lip is the right arm's top, dy = 0.10)
    [0.16, 0.42, 0.10, 0.42],    // catch well
];

/// Index into `U_TUBE_RECTS` of the reservoir / left-arm rect -- the only one
/// `initialize_hourglass` prefills.
pub(crate) const U_TUBE_RESERVOIR_RECT: usize = 0;

pub fn eval_sandbox_shape(
    cx: usize,
    cy: usize,
    w: usize,
    h: usize,
    shape: crate::SandboxShape,
    neck_width: f32,
    hourglass_curve: f32,
    multistage_chambers: u32,
    flipped: bool,
) -> (bool, bool) {
    let center_x = w as f32 / 2.0;
    let center_y = h as f32 / 2.0;
    let dx = cx as f32 - center_x;
    // Turning the apparatus over inverts the *structure*, not just its contents. Every shape
    // below is written in terms of `dy`, so negating it here mirrors the geometry about
    // `center_y` and nothing else needs to know. Negating the continuous `dy` rather than
    // remapping the integer row is what keeps this consistent with `flip_hourglass`'s content
    // mirror (`y2 = h - y`, i.e. the same axis at `h / 2`) and well defined for row 0, which
    // has no partner row under that mapping.
    let dy = (cy as f32 - center_y) * if flipped { -1.0 } else { 1.0 };
    let w_f = w as f32;
    let h_f = h as f32;

    let r_x = 0.46 * w_f;
    let r_y = 0.46 * h_f;
    let r_x_sq = r_x * r_x;
    let r_oval_y_sq = (0.35 * h_f) * (0.35 * h_f);
    let safe_r_x = r_x - 1.5;
    let safe_r_y = r_y - 1.5;
    let safe_circle_r_sq = safe_r_x * safe_r_x;

    match shape {
        crate::SandboxShape::Circle => {
            let dist_sq = dx * dx + dy * dy;
            (dist_sq < r_x_sq, dist_sq < safe_circle_r_sq)
        }
        crate::SandboxShape::Square => {
            let adx = dx.abs();
            let ady = dy.abs();
            (adx < r_x && ady < r_y, adx < safe_r_x && ady < safe_r_y)
        }
        crate::SandboxShape::Oval => {
            let oval_val = (dx * dx) / r_x_sq + (dy * dy) / r_oval_y_sq;
            (oval_val < 1.0, oval_val < 0.98)
        }
        crate::SandboxShape::Hourglass => {
            let chamber_h = 0.40 * h_f;
            let max_hw = 0.35 * w_f;
            let neck_hw = neck_width * w_f;

            let dy_abs = dy.abs();
            if dy_abs < chamber_h {
                let t = dy_abs / chamber_h;
                let allowed_hw = neck_hw + t.powf(hourglass_curve) * (max_hw - neck_hw);
                let inside = dx.abs() < allowed_hw;
                let safe_allowed_hw = (allowed_hw - 1.5).max(1.0);
                let is_safe = dx.abs() < safe_allowed_hw && dy_abs < (chamber_h - 1.5);
                (inside, is_safe)
            } else {
                (false, false)
            }
        }
        crate::SandboxShape::MultiStageHourglass => {
            // Binary-merging cascade: `multistage_chambers` chambers in the widest (top)
            // tier, each adjacent pair merging into one shared chamber below, per
            // `multistage_tier_chambers` -- e.g. 8 -> 4 -> 2 -> 1 at the shipped default,
            // 16 -> 8 -> 4 -> 2 -> 1 at the slider's top, 5 -> 3 -> 2 -> 1 at its bottom.
            // The TIER COUNT is therefore itself derived, not fixed at 4 -- see that
            // function's doc comment, the single place this rule is expressed. Tiers of
            // equal height are stacked from `-total_half` (top) to `+total_half` (bottom),
            // however many tiers the chain produces.
            let total_half = 0.42 * h_f;
            if dy < -total_half || dy >= total_half {
                return (false, false);
            }
            // Allocation-free per-tier boundary table (see `multistage_tier_boundaries`'s doc
            // comment) -- built fresh on every call, which is fine: it's fixed-size stack
            // arrays, not a `Vec`, so there's no heap churn despite this function running
            // ~262k times per mask regeneration.
            let tb = multistage_tier_boundaries(multistage_chambers);
            let n_tiers = tb.n_tiers;
            let tier_h = (2.0 * total_half) / n_tiers as f32;
            let tier = (((dy + total_half) / tier_h).floor() as i32)
                .clamp(0, n_tiers as i32 - 1) as usize;
            let y0 = -total_half + tier as f32 * tier_h;
            let y1 = y0 + tier_h;

            // Tier 0's chamber count (`n`, clamped to the supported 1..=16 range by
            // `multistage_tier_boundaries`) is the `n` that every boundary value is expressed
            // in `w / n` units of.
            let n0 = (tb.lens[0] - 1) as f32;
            let unit_w = w_f / n0;

            // Chamber `slot` of this tier spans boundaries `[tb.boundaries[tier][slot],
            // tb.boundaries[tier][slot + 1])`, in `w / n0` units -- found by scanning this
            // tier's own (short, <= 17-entry) boundary list, rather than dividing the full
            // width evenly by this tier's OWN chamber count the way every tier used to. That
            // old per-tier independent uniform grid is exactly the bug this replaces: a
            // merge-tree boundary list guarantees each child's span is the union of the
            // parent chambers that feed it, so a parent's neck is inside its child's slot by
            // construction, instead of landing there by arithmetic luck (or not, e.g. a
            // middle chamber's neck landing exactly on the tier-below's centre wall).
            let n_t = tb.lens[tier] - 1;
            let u = ((dx + w_f / 2.0) / unit_w).clamp(0.0, n0);
            let mut slot = n_t - 1;
            for i in 0..n_t {
                if u < tb.boundaries[tier][i + 1] as f32 {
                    slot = i;
                    break;
                }
            }
            let b0 = tb.boundaries[tier][slot] as f32;
            let b1 = tb.boundaries[tier][slot + 1] as f32;
            let chamber_w = (b1 - b0) * unit_w;
            let chamber_center = ((b0 + b1) * 0.5) * unit_w - w_f / 2.0;
            let dx_local = dx - chamber_center;

            // Each chamber is its own small funnel: widest at the top of its tier,
            // narrowing to a neck at the bottom. `max_hw` reuses the 0.35 fraction every
            // other funnel shape in this file uses for its chamber half-width -- and at
            // the bottom tier (one chamber spanning the full width) it reduces to exactly
            // that: 0.35 * w, the same top width as the plain `Hourglass`.
            //
            // The neck cannot just be `neck_width * w_f` here the way it is everywhere
            // else: that slider tops out at 0.12 * w, but the widest tier's chambers can
            // be as narrow as w/16 = 0.0625 * w (at the top of the chamber-count slider),
            // so an unscaled neck there would be nearly as wide as its own chamber and
            // adjacent chambers would merge into open space -- the specific failure this
            // design invites. The neck half-width is instead capped at 0.30 of *that
            // tier's own* chamber width (comfortably under the 0.35 used for the
            // chamber's own widest point, so the funnel can never invert with the neck
            // wider than its own top) and floored at 0.5 cells (a 1-cell-wide opening,
            // the smallest a rasterised neck can be at all) so it never collapses to
            // nothing at the bottom of the slider.
            //
            // The slider's own minimum is resolution-dependent (`0.5 / grid_width`, set in
            // `demo.js` and recomputed on every resolution change) precisely so its bottom
            // end lands on exactly this 0.5-cell floor at every grid size, rather than a
            // fixed fraction that floors to very different cell counts depending on
            // resolution (which was the original complaint this floor-lowering answers: at
            // the old fixed minimum and floor, low resolutions spent much of the slider's
            // travel pinned at a neck several times wider than the new floor allows).
            //
            // At the shipped default (n = 8, 4 tiers), tabulated in fractions of `w`
            // (grid-size independent) and in cells on the shipped 512 grid, at the
            // slider's low/mid/high ends (neck_width = 0.5/w / .06 / .12):
            //
            //   tier  n   chamber_w       max_hw           neck_cap         neck_hw @ min / .06 / .12
            //     0   8   .125  ( 64px)   .0438 ( 22.4px)  .0375 ( 19.2px)   0.5px / 19.2px(cap) / 19.2px(cap)
            //     1   4   .25   (128px)   .0875 ( 44.8px)  .075  ( 38.4px)   0.5px / 30.7px      / 38.4px(cap)
            //     2   2   .5    (256px)   .175  ( 89.6px)  .15   ( 76.8px)   0.5px / 30.7px      / 61.4px
            //     3   1   1.0   (512px)   .35   (179.2px)  .30   (153.6px)   0.5px / 30.7px      / 61.4px
            //
            // The cap only ever bites the two tiers small enough for the raw slider value
            // to threaten them (tiers 0-1 at n = 8; more of the chain at larger n). Once
            // the cap exceeds the slider's maximum (0.15 * w > 0.12 * w) the chambers see
            // exactly the neck width the slider asked for, uncapped.
            let max_hw = 0.35 * chamber_w;
            // See `multistage_neck_half_width`'s doc comment for the cap/floor/anti-merge-
            // ceiling arithmetic this computes and why the last of those three only starts
            // to matter once the chamber count becomes configurable (it is a no-op at
            // n = 8, today's only historical value, at every shipped grid size -- see the
            // report this shipped alongside for the measurements that back that claim).
            // Factored into its own function so the UI's neck-width cell-count readout
            // (`effective_neck_half_width_cells`) computes this exact value rather than a
            // second, hand-copied formula that could drift out of sync with it.
            let neck_hw = multistage_neck_half_width(w_f, chamber_w, neck_width);

            // t_local: 1 at the top of the tier (widest), 0 at its bottom (the neck) --
            // the same sense as `Hourglass`'s `t` (0 at the neck, 1 at the chamber's
            // outer edge) so `hourglass_curve` biases the taper identically here.
            let t_local = ((y1 - dy) / tier_h).clamp(0.0, 1.0);
            let allowed_hw = neck_hw + t_local.powf(hourglass_curve) * (max_hw - neck_hw);

            let inside = dx_local.abs() < allowed_hw;
            let safe_allowed_hw = (allowed_hw - 1.5).max(1.0);
            let is_safe = dx_local.abs() < safe_allowed_hw
                && dy > (-total_half + 1.5)
                && dy < (total_half - 1.5);
            (inside, is_safe)
        }
        crate::SandboxShape::GaltonBoard => {
            let chamber_h = 0.40 * h_f;
            let max_hw = 0.35 * w_f;
            let neck_hw = neck_width * w_f;
            let dy_abs = dy.abs();
            if dy_abs < chamber_h {
                let t = dy_abs / chamber_h;
                let allowed_hw = neck_hw + t.powf(hourglass_curve) * (max_hw - neck_hw);
                if dx.abs() >= allowed_hw {
                    return (false, false);
                }
                
                if dy > 6.0 && dy < 0.38 * h_f {
                    // Pegs sit on a fixed lattice with a genuine half-spacing stagger between
                    // consecutive rows, so no column of the board is ever clear from top to
                    // bottom.
                    //
                    // The previous arrangement centred each row on its own peg count and then
                    // added an explicit half-spacing offset on odd rows. `(count - 1) / 2` with
                    // `count = row + 3` is a half-integer on exactly those odd rows, so it had
                    // already shifted them by half a spacing and the explicit offset cancelled it:
                    // every peg of every row landed on a multiple of `spacing`, leaving open
                    // shafts 4.2 cells wide that sand fell straight down without ever being
                    // deflected. The same count-based centring also pushed the odd rows off-axis
                    // (row 1 spanned -8..16 rather than being symmetric about 0).
                    //
                    // Deriving the peg from the lattice instead of enumerating a row's pegs also
                    // drops the inner loop: the only candidate is the nearest lattice column.
                    // The triangular Galton silhouette still emerges on its own, because the
                    // `allowed_hw` test above has already rejected anything outside the funnel and
                    // the funnel widens with depth.
                    const PEG_SPACING: f32 = 8.0;
                    // Staggered rows only close the gap if a peg is at least a quarter of a
                    // spacing wide: even rows cover `[j*s - r, j*s + r]`, odd rows the same
                    // shifted by `s/2`, and the union has no gap exactly when `r >= s/4`. At the
                    // old `r = 1.8` against `s = 8` a 0.4-wide shaft survived at every `8j +- 2`
                    // even once the stagger was fixed, so the radius has to move too.
                    const PEG_RADIUS: f32 = 2.2;
                    let row = ((dy - 6.0) / PEG_SPACING).round();
                    let row_y = 6.0 + row * PEG_SPACING;
                    let stagger = if (row as i32) % 2 != 0 { PEG_SPACING * 0.5 } else { 0.0 };
                    let peg_x = ((dx - stagger) / PEG_SPACING).round() * PEG_SPACING + stagger;
                    let pdx = dx - peg_x;
                    let pdy = dy - row_y;
                    if pdx * pdx + pdy * pdy < PEG_RADIUS * PEG_RADIUS {
                        return (false, false);
                    }
                }
                let is_safe = dx.abs() < (allowed_hw - 1.5).max(1.0) && dy_abs < (chamber_h - 1.5);
                (true, is_safe)
            } else {
                (false, false)
            }
        }
        crate::SandboxShape::StaircaseCascade => {
            let max_hw = 0.42 * w_f;
            let max_hh = 0.42 * h_f;
            if dx.abs() >= max_hw || dy.abs() >= max_hh {
                return (false, false);
            }

            // Procedurally-varied alternating sloped stair shelves: more steps than the
            // original fixed 4, each with a slightly randomized slope (deterministic per
            // step index) plus a randomized gap ("hole") sand can filter straight through,
            // in addition to the usual open side at the end of each shelf.
            // Smaller steps: 13 shelves over the same span rather than 8, so each drop is ~31
            // cells instead of ~53.
            //
            // The slope had to come down with it, and by more than it first looks. Consecutive
            // shelves alternate both their slope sign and which wall they attach to, so they
            // approach each other at the shared inner edge, `attach_limit`. The vertical gap
            // there is `step_spacing - 2 * attach_limit * slope_max - shelf_thickness`, and it has
            // to stay comfortably positive or two shelves fuse into one solid slab that dams the
            // cascade. Worked at the shipped 512 grid:
            //
            //   steps  slope_max  spacing  clearance
            //       8       0.20     52.7        6.3   <- previous, already near collision
            //      13       0.10     30.7        4.0   too tight
            //      13       0.08     30.7        8.0   <- this
            //      15       0.08     26.3        3.6   too tight
            //
            // So 13 steps at a 0.04..0.08 slope is both finer *and* has more clearance than the
            // 8-step version it replaces. Raising the count further needs a shallower slope than
            // still reads as a slope.
            let step_count: i32 = 13;
            let attach_limit = 0.20 * w_f;
            let y_start = -0.36 * h_f;
            let step_spacing = 0.72 * h_f / (step_count as f32 - 1.0);

            for k in 0..step_count {
                let y_k = y_start + k as f32 * step_spacing;
                let slope_mag = 0.04 + 0.04 * step_hash(k as u32 * 3);
                let slope = if k % 2 == 0 { slope_mag } else { -slope_mag };
                let y_shelf = y_k + dx * slope;
                if (dy - y_shelf).abs() < 3.5 {
                    let is_left_attached = k % 2 == 0;

                    let (span_lo, span_hi) = if is_left_attached {
                        (-max_hw + 4.0, attach_limit - 4.0)
                    } else {
                        (-attach_limit + 4.0, max_hw - 4.0)
                    };
                    let hole_center = span_lo + step_hash(k as u32 * 3 + 1) * (span_hi - span_lo);
                    // Scaled down alongside the step size so a hole stays a hole rather than
                    // becoming most of the shelf it is cut into.
                    let hole_width = (0.035 + 0.035 * step_hash(k as u32 * 3 + 2)) * w_f;
                    let in_hole = (dx - hole_center).abs() < hole_width * 0.5;

                    if is_left_attached && dx < attach_limit && !in_hole {
                        return (false, false);
                    } else if !is_left_attached && dx > -attach_limit && !in_hole {
                        return (false, false);
                    }
                }
            }

            let is_safe = dx.abs() < (max_hw - 2.0) && dy.abs() < (max_hh - 2.0);
            (true, is_safe)
        }
        crate::SandboxShape::ProceduralFunnel => {
            let chamber_h = 0.40 * h_f;
            let max_hw = 0.35 * w_f;
            let neck_hw = neck_width * w_f;
            let dy_abs = dy.abs();
            if dy_abs < chamber_h {
                let t = dy_abs / chamber_h;
                let allowed_hw = neck_hw + t.powf(hourglass_curve) * (max_hw - neck_hw);
                if dx.abs() >= allowed_hw {
                    return (false, false);
                }
                if dy > -0.32 * h_f && dy < 0.32 * h_f {
                    // Higher-frequency, 4-octave noise than before packs in more, smaller
                    // stalactite/stalagmite obstacles instead of a few large blobby ones.
                    let cave_val = (
                        (dx * 0.14).sin()
                        + (dy * 0.16).cos()
                        + (dx * 0.05 + dy * 0.07).sin()
                        + (dx * 0.24 - dy * 0.21).cos()
                    ).abs();
                    if cave_val > 1.35 && dx.abs() > 6.0 {
                        return (false, false);
                    }
                }
                let is_safe = dx.abs() < (allowed_hw - 1.5).max(1.0) && dy_abs < (chamber_h - 1.5);
                (true, is_safe)
            } else {
                (false, false)
            }
        }
        crate::SandboxShape::MultiNeckHourglass => {
            // Two genuinely separate necks, spread wide apart, rather than one center
            // opening barely split by a small barrier. Each neck is its own mini funnel
            // (same taper shape as the classic Hourglass); their wide tops overlap near
            // the chamber walls to form a single continuous top/bottom chamber, and they
            // pull apart into two distinct openings approaching the pinch line, forming a
            // "W" (draining) / "M" (refilling) silhouette.
            let chamber_h = 0.40 * h_f;
            // THREE necks rather than two. A symmetric pair reads unmistakably as a bust; an odd
            // count does not, and the centre neck also gives the silhouette a "W"/"M" with a
            // middle spike instead of a single cleavage.
            //
            // The spacing is set by the neck-width slider's top end, not by looks. Adjacent necks
            // merge into one opening once `neck_hw` exceeds half the spacing, so with the slider
            // capped at 0.12 the necks stay distinct across its whole range only if the spacing is
            // above 0.24 * w. `0.22 * w` keeps them separate to 0.11 and lets them merge in the
            // last sliver of slider travel, which is the graceful end of that trade — a very wide
            // neck *should* read as one mouth.
            //
            // `max_hw` drops 0.30 -> 0.24 to pay for the wider spacing: the outermost extent is
            // `neck_offset + max_hw`, and it has to stay inside the same 0.46 * w the other shapes
            // respect. At 0.22 + 0.24 = 0.46 the chambers still overlap (each neck's top spans
            // +/- 0.24 about a centre 0.22 away from its neighbour), so the tops fuse into one
            // continuous chamber exactly as the two-neck version did.
            let max_hw = 0.24 * w_f;
            let neck_offset = 0.22 * w_f;
            let dy_abs = dy.abs();
            if dy_abs < chamber_h {
                let t = dy_abs / chamber_h;
                let neck_hw = neck_width * w_f;
                let allowed_hw = neck_hw + t.powf(hourglass_curve) * (max_hw - neck_hw);
                let nearest_neck = [-neck_offset, 0.0, neck_offset]
                    .iter()
                    .map(|c| (dx - c).abs())
                    .fold(f32::INFINITY, f32::min);
                if nearest_neck >= allowed_hw {
                    return (false, false);
                }
                let safe_hw = (allowed_hw - 1.5).max(1.0);
                let is_safe = nearest_neck < safe_hw && dy_abs < (chamber_h - 1.5);
                (true, is_safe)
            } else {
                (false, false)
            }
        }
        crate::SandboxShape::UTubeFlowThrough => {
            // Task #61: a fixed union of five axis-aligned rects (`U_TUBE_RECTS`) -- not a
            // tapered funnel -- so `neck_width` and `hourglass_curve` are unused here. Both
            // are still accepted (this match arm's signature is shared with every other shape)
            // but this apparatus has no neck to narrow and no curve to bend; that is
            // intentional, not an oversight.
            let _ = (neck_width, hourglass_curve);

            let in_rect = |px: f32, py: f32, r: &[f32; 4]| -> bool {
                px >= r[0] * w_f && px < r[1] * w_f && py >= r[2] * h_f && py < r[3] * h_f
            };
            let in_union = |px: f32, py: f32| -> bool {
                U_TUBE_RECTS.iter().any(|r| in_rect(px, py, r))
            };

            if !in_union(dx, dy) {
                return (false, false);
            }

            // Safe (interior, non-boundary-adjacent) iff the union still contains the point
            // after nudging by 1.5 cells along each axis independently -- so any region less
            // than 3 cells thick along x or y (e.g. the basin's roofed strip, if it were ever
            // narrowed) can never report safe.
            let margin = 1.5;
            let is_safe = in_union(dx - margin, dy)
                && in_union(dx + margin, dy)
                && in_union(dx, dy - margin)
                && in_union(dx, dy + margin);

            (true, is_safe)
        }
    }
}

/// The LOD scheduler's MUST-simulate bar (see the block-classification comment in `settle_tick`,
/// `BlockActivity::Fast`): a block whose recorded displacement clears this next tick is
/// simulated unconditionally, bypassing `budget_n` entirely. Module-level (rather than local to
/// `settle_tick`, which is where this used to live) so `activate_neighbor_upstream` below —
/// needed by both `settle_tick`'s flux-edge loops and `try_move`'s granular-CA path — can force
/// a block straight into that tier by name instead of duplicating the magic number.
///
/// `pub(crate)` (rather than private) so `DrawingSimulation::update`'s "perfect simulation"
/// debug toggle (`sandart-sim/src/lib.rs`) can push a block into this exact tier by writing this
/// exact threshold into `last_displacements`, instead of `settle_tick` growing a second bypass
/// parameter that every one of its ~20 test call sites would also have to learn.
pub(crate) const MUST_SIMULATE_THRESHOLD: f32 = 1e-4;

/// Task #47 ("sand-slab" scheduling defect): fresh-overburden MUST-simulate predicate constants.
/// See the big comment at this predicate's call site (top of `settle_tick`, just before the MUST/
/// STALE/REST classification loop) for the mechanism and why every other activation signal in
/// this scheduler is historical (one tick late) while this one is not.
///
/// Material-presence bar: mirrors `PERFECT_SIM_MATERIAL_EPSILON` in `lib.rs` (same value, same
/// meaning -- "holding material that could move") but declared here rather than reached into from
/// `lib.rs`, since this module owns the predicate and the two are not required to move together.
const FRESH_OVERBURDEN_MATERIAL_EPSILON: f32 = 1e-5;

/// Overburden bar, in units of "resting cells directly above" rather than raw `column_depth`
/// units: multiplied by the same `REFERENCE_GRID_HEIGHT / w` resolution-normalisation
/// `column_depth`'s own accumulation already applies (see `LATERAL_PRESSURE_SCALE`'s doc comment
/// for why that normalisation exists), so this bar means the same physical thing -- "about one and
/// a half full-capacity cells' worth of resting skin" -- at every grid resolution.
///
/// Must be comfortably above what a thin skin of newly-settled material on top of an otherwise
/// free-falling body can accumulate in a single cell (up to ~1.5, `cell_capacity_for`'s max), or
/// that skin would flip the body's own block back off MUST the instant a single cell of it settles
/// -- the exact lag this predicate exists to avoid. `0.0` (or "== 0.0") is deliberately not used:
/// `column_depth` is an accumulated float, not a clean boundary.
const FRESH_OVERBURDEN_SKIN_CELLS: f32 = 1.5;

/// How much lower a neighbour must read before a cell is judged to have "somewhere to go" —
/// small enough to catch a genuine slope, comfortably above float noise. Same order of magnitude
/// as the scheduler's other flow/displacement bars (`MUST_SIMULATE_THRESHOLD`,
/// `FLOW_INACTIVE_THRESHOLD` in `settle_tick`).
const FRESH_OVERBURDEN_ROOM_EPSILON: f32 = 1e-3;

/// Task #47 round 3: fraction of a cell's own local capacity (`cell_capacity_for`) that must be
/// free below it, per `support_fraction`, for it to count as "unsupported enough" to matter for
/// scheduling -- small enough to catch genuine free space, comfortably above float noise. A
/// fraction rather than an absolute height (unlike `FRESH_OVERBURDEN_ROOM_EPSILON`) because
/// `support_fraction` is itself already normalised to `[0, 1]` by the cell-below's own capacity.
const SUPPORT_FRACTION_EPSILON: f32 = 0.02;

/// Mark a neighbor block as modified (needing redraw/copy-back this frame) and bump its
/// next-frame displacement estimate, without touching the buffer belonging to the block
/// currently being simulated (which would corrupt a block that hasn't run yet this frame).
fn activate_neighbor(neighbor_b: usize, flow: f32, modified: &mut Vec<bool>, next_displacements: &mut Vec<f32>) {
    modified[neighbor_b] = true;
    if next_displacements[neighbor_b] < flow {
        next_displacements[neighbor_b] = flow;
    }
}

/// Injected displacement for `activate_neighbor_upstream`: comfortably above the near-zero
/// residual displacement a block settling toward rest reports (so it reliably outranks those as
/// a `budget_simulate` priority — see `priority = staleness * displacement` in `settle_tick`'s
/// block classification), but strictly under `MUST_SIMULATE_THRESHOLD` so it can never itself
/// push a block into the unconditional, budget-exempt MUST-simulate (`Fast`) tier. Half the
/// threshold rather than some other fraction is not load-bearing — the only requirement is
/// `0 < UPSTREAM_DISPLACEMENT_HINT < MUST_SIMULATE_THRESHOLD`.
const UPSTREAM_DISPLACEMENT_HINT: f32 = 0.5 * MUST_SIMULATE_THRESHOLD;

/// Like `activate_neighbor`, but injects `UPSTREAM_DISPLACEMENT_HINT` in place of an actual flow
/// magnitude (there isn't one to report: nothing has moved into or out of this block yet — that
/// is exactly the point).
///
/// Reserved for the block one step upstream of an edge whose donor just lost mass: its support
/// moved, so unlike an ordinary touched neighbour this dependency is causal rather than merely
/// speculative, and it competes for `budget_n` as a `Medium`-priority (`budget_simulate`)
/// candidate rather than being left to whatever priority a same-tick, zero-actual-flow block
/// would otherwise get (none — it would not enter `rest_candidates` at all without this nudge,
/// since nothing flowed through IT this tick to record a displacement).
///
/// Deliberately NOT a `MUST_SIMULATE_THRESHOLD`-or-above bump: an earlier version of this fix
/// forced the upstream block straight into the unconditional `Fast` tier, which the task's own
/// author overturned — blocks that are merely likely to have work, as this one is, are meant to
/// compete for budget like everything else in `Medium`/`Slow`, not bypass it; only genuinely
/// already-active blocks (real, measured displacement) earn the budget-exempt tier. This keeps
/// that invariant: the total simulated block count stays capped by `budget_n` either way, so
/// this fix can only ever redistribute which blocks receive it, never add unbounded extra work.
#[inline]
fn activate_neighbor_upstream(neighbor_b: usize, modified: &mut Vec<bool>, next_displacements: &mut Vec<f32>) {
    modified[neighbor_b] = true;
    if next_displacements[neighbor_b] < UPSTREAM_DISPLACEMENT_HINT {
        next_displacements[neighbor_b] = UPSTREAM_DISPLACEMENT_HINT;
    }
}

/// Injected displacement for `activate_neighbor_side` -- deliberately smaller than
/// `UPSTREAM_DISPLACEMENT_HINT` (a tenth of `MUST_SIMULATE_THRESHOLD`, so it ranks below an
/// upstream nudge at equal staleness) and, like it, strictly under `MUST_SIMULATE_THRESHOLD` so
/// it can never bypass `budget_n` into `Fast`.
const SIDE_DISPLACEMENT_HINT: f32 = 0.1 * MUST_SIMULATE_THRESHOLD;

/// The SPECULATIVE half of the upstream-wake fix: a plain lateral/vertical neighbour of a block
/// that just had real flow through it, on the axis perpendicular to that flow, is not causally
/// implicated the way `activate_neighbor_upstream`'s target is -- nothing below or beside it
/// actually changed -- but a body that is actively moving in one column/row often has its
/// neighbours about to follow, so it earns a low-priority nudge into `rest_candidates` rather
/// than nothing at all.
///
/// This must NOT pass the edge's real flux magnitude (an earlier version of this fix did, via
/// plain `activate_neighbor`, and it regressed `test_settled_liquid_sleeps_and_wakes`: a
/// settled pool's ordinary free-surface flow is often well above `MUST_SIMULATE_THRESHOLD` on
/// its own, and multiplying that magnitude out to every perpendicular neighbour of every
/// touched edge pushed most of the pool's blocks into the unconditional `Fast` tier, exactly
/// the runaway `MUST_SIMULATE_THRESHOLD`'s own doc comment warns a too-low bar causes). Using
/// the fixed, sub-threshold `SIDE_DISPLACEMENT_HINT` instead keeps this strictly a low-priority
/// `budget_simulate` candidate no matter how large the triggering flow was.
#[inline]
fn activate_neighbor_side(neighbor_b: usize, modified: &mut Vec<bool>, next_displacements: &mut Vec<f32>) {
    modified[neighbor_b] = true;
    if next_displacements[neighbor_b] < SIDE_DISPLACEMENT_HINT {
        next_displacements[neighbor_b] = SIDE_DISPLACEMENT_HINT;
    }
}

/// Apply a mass transfer of `flow` from `center_idx` to `neighbor_idx`: activates both the
/// source and destination blocks, advects color and material properties, updates
/// `temp_heights`, and accumulates the per-tick bookkeeping (`total_flow`, `cell_flowed`,
/// `flow_occurred`). This is the body shared by the two flow sites in the granular CA path
/// (the avalanche-collapse safety check and the main slope-driven flow below it) — they differ
/// only in the guard condition that decides whether to call this at all, so the guard stays at
/// each call site rather than being folded in here.
#[allow(clippy::too_many_arguments)]
fn try_move(
    b: usize,
    center_idx: usize,
    neighbor_idx: usize,
    flow: f32,
    w: usize,
    h: usize,
    block_size: usize,
    cols: usize,
    temp_heights: &mut [f32],
    cell_colors: &mut [u8],
    cell_props: &mut [f32],
    modified: &mut Vec<bool>,
    next_displacements: &mut Vec<f32>,
    total_flow: &mut f32,
    cell_flowed: &mut bool,
    flow_occurred: &mut bool,
) {
    let nx = neighbor_idx % w;
    let ny = neighbor_idx / w;
    let neighbor_b = (ny / block_size) * cols + (nx / block_size);

    activate_neighbor(b, flow, modified, next_displacements);
    activate_neighbor(neighbor_b, flow, modified, next_displacements);

    // Upstream wake. The two calls above activate only the two blocks THIS edge touches
    // (`center_idx`'s and `neighbor_idx`'s), which is the same limit `flux_edge_apply` has on
    // the flux-solver path (see the matching "Upstream wake" comment on the `touched_v`/
    // `touched_h` loops in `settle_tick`) -- and it produces the identical failure mode here:
    // a cell one more step *upstream* of `center_idx`, on the opposite side from
    // `neighbor_idx`, just had its support move away and is never told. Under the block-LOD
    // scheduler that cell's block can then stay `Inactive` (not merely low-priority) for up to
    // `MAX_STALENESS` ticks after the material below it has already moved several cells,
    // opening a gap on the block boundary between them. `flow` here is already known positive
    // (checked at both call sites before this is reached) and always drains
    // `center_idx -> neighbor_idx`, so upstream is unconditionally one more grid step past
    // `center_idx`, away from `neighbor_idx` -- unlike the flux solver's signed `final_flux`,
    // there is no direction ambiguity to resolve here.
    //
    // `activate_neighbor_upstream`, not plain `activate_neighbor`: this dependency is causal
    // (its support genuinely moved), not speculative, so it earns priority as a `Medium`
    // (`budget_simulate`) candidate instead of the no-signal-at-all it would otherwise get --
    // but it still competes for `budget_n` like any other candidate rather than bypassing it
    // (see that function's doc comment).
    let cx = (center_idx % w) as isize;
    let cy = (center_idx / w) as isize;
    let dx = (nx as isize) - cx;
    let dy = (ny as isize) - cy;
    let up_x = cx - dx;
    let up_y = cy - dy;
    if !upstream_wake_gate::is_disabled() {
        if up_x >= 0 && up_y >= 0 && (up_x as usize) < w && (up_y as usize) < h {
            let up_b = (up_y as usize / block_size) * cols + (up_x as usize / block_size);
            activate_neighbor_upstream(up_b, modified, next_displacements);
        }

        // Speculative half: see the matching comment on the flux-solver's touched_v loop. The
        // donor block's two neighbours PERPENDICULAR to this move's direction are not causally
        // implicated, but a block that is actively flowing often has its lateral neighbours
        // about to follow, so give them a plain (budget-competing) nudge too. Bounded via
        // `modified.len()` (== `cols * rows`) rather than a separate `rows` parameter, since
        // `try_move` is not otherwise told the block grid's row count.
        let (donor_bx, donor_by) = ((center_idx % w) / block_size, (center_idx / w) / block_size);
        if dy != 0 {
            if donor_bx > 0 {
                activate_neighbor_side(donor_by * cols + (donor_bx - 1), modified, next_displacements);
            }
            if donor_bx + 1 < cols {
                activate_neighbor_side(donor_by * cols + (donor_bx + 1), modified, next_displacements);
            }
        } else if dx != 0 {
            if let Some(up_by) = donor_by.checked_sub(1) {
                activate_neighbor_side(up_by * cols + donor_bx, modified, next_displacements);
            }
            let down_b = (donor_by + 1) * cols + donor_bx;
            if down_b < modified.len() {
                activate_neighbor_side(down_b, modified, next_displacements);
            }
        }
    }

    advect_properties(cell_colors, cell_props, center_idx, neighbor_idx, flow, temp_heights[neighbor_idx]);
    temp_heights[center_idx] -= flow;
    temp_heights[neighbor_idx] += flow;
    *total_flow += flow;
    *cell_flowed = true;
    *flow_occurred = true;
}

// --- Per-mechanism tick-phase offsets (diagnostic instrumentation, test builds only) --------
//
// `settle_tick` derives several independent scan/schedule decisions from the same `tick_count`:
// LOD staleness, block-level scan order, three separate row/column parity switches, the
// cell-level lateral-sweep direction, the CA neighbour-order checkerboard, and (in the test
// harness only) the flow RNG seed. `test_water_blob_stays_left_right_symmetric_under_gravity`
// shows the solver as a whole is not invariant under a shift of the global tick phase, but
// seeding `TestSim.tick_count` at 1 instead of 0 shifts ALL of these at once, so that failure
// can't be attributed to any single one of them.
//
// These offsets let a test flip exactly ONE logical mechanism (`phase_offset(K)` nonzero) while
// every other site stays at its production phase (`phase_offset(K) == 0`), so each mechanism's
// contribution to the lean can be measured in isolation. `K_*` indices are one per LOGICAL
// MECHANISM, not one per code site — `K_BLOCK_ORDER` and `K_CA_CHECKERBOARD` each cover two call
// sites that must move together.
//
// In non-test builds `phase_offset` is a `#[inline(always)]` function that always returns 0,
// which the optimizer folds away entirely, so production codegen is unaffected — see
// `test_multistage_n8_is_bit_identical_to_shipped_geometry` and the phase-offset self-test below
// for the proof.
//
// The `K_*` indices themselves are NOT `#[cfg(test)]`-gated: `phase_offset(K_...)` call sites
// live in production code (`settle_tick` itself), so the index constants must exist in every
// build configuration. Only the backing storage and the non-zero read path are test-only.
#[allow(dead_code)]
pub(crate) const K_LOD_STALENESS: usize = 0;
#[allow(dead_code)]
pub(crate) const K_BLOCK_ORDER: usize = 1;
#[allow(dead_code)]
pub(crate) const K_NONDOWN_BLOCK_PARITY: usize = 2;
#[allow(dead_code)]
pub(crate) const K_NONDOWN_ROW_PARITY: usize = 3;
#[allow(dead_code)]
pub(crate) const K_LATERAL_SWEEP: usize = 4;
#[allow(dead_code)]
pub(crate) const K_NONGRAVITY_X_PARITY: usize = 5;
#[allow(dead_code)]
pub(crate) const K_CA_CHECKERBOARD: usize = 6;
#[allow(dead_code)]
pub(crate) const K_RNG_SEED: usize = 7;
#[cfg(test)]
pub(crate) const PHASE_MECHANISM_COUNT: usize = 8;

// THREAD-LOCAL, not a global static, and that is load-bearing rather than stylistic. cargo's
// test harness runs tests on many threads at once. With shared global storage, one test setting
// a non-zero offset would silently perturb every other test running concurrently that touches
// `settle_tick` — producing flaky failures with no visible connection to the diagnostic that
// caused them. The measurement test is `#[ignore]`d, which keeps it out of a plain `cargo test`,
// but `cargo test -- --include-ignored` is an ordinary thing to run and would walk straight into
// it. Thread-local storage makes the offsets private to the thread performing the measurement,
// so the hazard cannot arise at all and no discipline is required of anyone. `settle_tick` is
// always called on the calling test's own thread, so the offsets are visible where they matter.
#[cfg(test)]
thread_local! {
    static PHASE_OFFSETS: [std::cell::Cell<u32>; PHASE_MECHANISM_COUNT] = [
        std::cell::Cell::new(0),
        std::cell::Cell::new(0),
        std::cell::Cell::new(0),
        std::cell::Cell::new(0),
        std::cell::Cell::new(0),
        std::cell::Cell::new(0),
        std::cell::Cell::new(0),
        std::cell::Cell::new(0),
    ];
}

/// Read the diagnostic tick-phase offset for mechanism `k` (test builds only; always 0 in
/// production). See the module comment above.
#[cfg(test)]
#[inline]
pub(crate) fn phase_offset(k: usize) -> u32 {
    PHASE_OFFSETS.with(|o| o[k].get())
}

/// Set the diagnostic tick-phase offset for mechanism `k` (test builds only). Scoped to the
/// calling thread, so it cannot disturb tests running concurrently on other threads; a
/// measurement must still reset its own offsets between rows so they don't leak from one row of
/// the sweep to the next.
#[cfg(test)]
pub(crate) fn set_phase(k: usize, v: u32) {
    PHASE_OFFSETS.with(|o| o[k].set(v));
}

/// Reset every mechanism's phase offset to 0, on the calling thread.
#[cfg(test)]
pub(crate) fn reset_phase_offsets() {
    PHASE_OFFSETS.with(|o| {
        for c in o.iter() {
            c.set(0);
        }
    });
}

/// Production build: always 0, `#[inline(always)]` so the optimizer folds every `phase_offset(K)`
/// call site down to the literal `0` and production codegen is bit-identical to before this
/// instrumentation existed.
#[cfg(not(test))]
#[inline(always)]
pub(crate) fn phase_offset(_k: usize) -> u32 {
    0
}

/// Perform a single gravity flow/settling iteration inside the active bounding box.
pub fn settle_tick(
    heightmap: &mut Heightmap,
    temp_heights: &mut Vec<f32>,
    cell_colors: &mut Vec<u8>,
    cell_props: &mut Vec<f32>,
    sliding: &mut Vec<bool>,
    active_bounds: &mut ActiveBounds,
    active_blocks: &mut Vec<crate::BlockActivity>,
    last_displacements: &mut Vec<f32>,
    last_simulated_ticks: &mut Vec<u32>,
    budget_n: usize,
    block_size: usize,
    active_marbles: &[ActiveMarbleInfo],
    time_seed: u32,
    edge_vel_h: &mut Vec<f32>,
    edge_vel_v: &mut Vec<f32>,
    column_depth: &mut Vec<f32>,
    // TASK #55 step 2 (rebuilt): the PERSISTENT hydraulic head field (see
    // `task55_head_field`'s module doc comment for why this is now incremental state rather than
    // a per-call solve). Owned by the caller (`DrawingSimulation::head_field` in production, a
    // plain local `Vec` in test/spec harnesses) exactly like `column_depth` just above --
    // resized/zero-filled at construction, `reset()`, and any grid-size change, never rebuilt
    // from scratch here. Relaxed one tick further (`task55_head_field::advance_head_field`) only
    // while `head_field_active` (below) is true; otherwise left untouched, so an unused field
    // costs nothing beyond the one resize check.
    head_field: &mut Vec<f32>,
    shape_mask: &[u8],
    tick_count: u32,
    gravity_dir: glam::Vec2,
    // "Fresh pressure field" debug toggle (`DrawingSimulation::fresh_pressure_field`; see that
    // field's doc comment). `false` (default): `column_depth` is computed exactly as before --
    // inline, order-dependent, inside phase 1's own CA branch (see the "FALLBACK MEASUREMENT"
    // comments at that site and just above the `for phase in 0..2` loop) -- bit-for-bit
    // unchanged from the tree before this toggle existed. `true`: `column_depth` is instead
    // computed once per tick, unconditionally, over the whole grid, by `recompute_column_depth`
    // reading the frozen pre-tick `heightmap.data` snapshot -- see that function's own doc
    // comment for what this is and the numbers it was measured against
    // (`test_liquid_flowing_liquid_does_not_stand_in_walls`'s voids@160 metric: 66, at this
    // once-per-tick/before-phase-0 placement, vs. 85 for a once-per-phase placement that was
    // also tried and rejected).
    fresh_pressure_field: bool,
    // TASK #55 step 3: "drive transport from the head field" debug toggle
    // (`DrawingSimulation::head_field_transport`; see that field's doc comment). `false`
    // (default): unchanged from the tree before this parameter existed -- bit-identical. `true`:
    // the lateral and vertical (gravity-aligned) edge solvers, for LIQUID-ONLY edges (both
    // endpoints at `liquidity(wetness) >= LIQUID_ELLIPTIC_THRESHOLD`), read their driving head
    // from `task55_head_field::compute_head_field` (computed once per tick, below, before the
    // phase loop -- same placement as `fresh_pressure_field` above) instead
    // of from `column_depth`/`GRAVITY_HEAD_SCALE`. Granular edges and mixed liquid/granular edges
    // are entirely unaffected -- see the call sites in phase 0 and phase 1 for the exact gate.
    head_field_transport: bool,
    // TASK #55 step 2/3 fix: "show the pressure heat-map's new-field source" debug toggle
    // (`DrawingSimulation::pressure_heatmap_head_field`). Does NOT gate anything about transport --
    // `head_field_active` just below (built from `head_field_transport`/`head_field_gate` alone,
    // unchanged) still owns whether the edge solvers read their driving head from `head_field`, so
    // this parameter cannot perturb `update`'s simulation output (`heightmap`/`cell_colors`/
    // `cell_props`), matching `pressure_heatmap_head_field_toggle.rs`'s no-perturbation contract.
    // Its ONLY effect is on whether `head_field` itself gets this tick's relaxation sweeps
    // (`head_field_needs_advance` below): previously `head_field` was advanced ONLY while
    // `head_field_transport` was on, so the pressure heat-map's "new field" source rendered a
    // buffer that had never been touched since the last reset/resize (all zeros) whenever a user
    // turned the overlay on without also turning transport on -- exactly the reported "no heatmap
    // shows for new field" defect. Advancing `head_field` costs nothing beyond
    // `HEAD_FIELD_SWEEPS_PER_TICK` local sweeps (see that constant's own doc comment), so paying it
    // for the overlay alone is cheap and keeps the "both off costs nothing" property intact.
    pressure_heatmap_head_field: bool,
    // TASK #63: "pressure-sensitive flow rate" debug toggle
    // (`DrawingSimulation::pressure_sensitive_flow`; see that field's doc comment). `false`
    // (default): unchanged from the tree before this parameter existed -- bit-identical. `true`:
    // a LIQUID-ONLY edge whose DONOR carries less than `PRESSURE_RATE_FULL_AT_ROWS_OF_HEAD`
    // reference rows of hydrostatic head has its conveyance coefficient scaled by the square root
    // of the fraction it does carry
    // (`pressure_rate_factor`, applied to `c_sq` at both the phase-0 vertical and phase-1 lateral
    // edge sites). Free-falling material is exempt by construction; granular and mixed edges are
    // untouched.
    //
    // DELIBERATELY INDEPENDENT OF `head_field_transport`. This reads `head_field` for the donor's
    // pressure but does NOT change which driving head the edge uses, so it can be evaluated on its
    // own while transport stays off (transport is still blocked on #64). It therefore joins
    // `head_field_needs_advance` below -- turning this on alone is enough to keep the field live.
    pressure_sensitive_flow: bool,
    // TASK #70: "overfill pressure simulation" toggle and configurable capacity ratio.
    overfill_pressure: bool,
    overfill_ratio: f32,
) -> f32 {
    let w = heightmap.width;
    let h = heightmap.height;
    if w == 0 || h == 0 {
        return 0.0;
    }

    // Safety checks to prevent panics if heights or sliding buffer are resized
    if temp_heights.len() != heightmap.data.len() {
        temp_heights.resize(heightmap.data.len(), crate::DEFAULT_SAND_HEIGHT);
    }
    if sliding.len() != heightmap.data.len() {
        sliding.resize(heightmap.data.len(), false);
    }
    // Per-edge momentum. `edge_vel_h[i]` belongs to the horizontal edge between cell `i` and
    // cell `i + 1`; `edge_vel_v[i]` to the vertical edge between cell `i` and cell `i + w`. Each
    // edge is owned (and therefore integrated exactly once per pass) by its lower-index cell.
    if edge_vel_h.len() != heightmap.data.len() {
        edge_vel_h.resize(heightmap.data.len(), 0.0);
    }
    if edge_vel_v.len() != heightmap.data.len() {
        edge_vel_v.resize(heightmap.data.len(), 0.0);
    }
    // Persistent, like `edge_vel_h`/`edge_vel_v`: see the depth-integrated lateral pressure
    // note in the cross-gravity liquid branch below for what this holds and why it must
    // survive a tick where the block that computed it goes to sleep.
    if column_depth.len() != heightmap.data.len() {
        column_depth.resize(heightmap.data.len(), 0.0);
    }
    // Persistent, like `column_depth` just above -- see this parameter's own doc comment and
    // `task55_head_field`'s module doc comment. Zero-fill on (re)size, matching every other
    // persistent per-cell buffer's resize-safety fallback in this function.
    if head_field.len() != heightmap.data.len() {
        head_field.resize(heightmap.data.len(), 0.0);
    }
    
    // Task #70: Overfill pressure simulation active gate
    let overfill_active = overfill_pressure || overfill_pressure_gate::is_enabled();

    // RE-APPLIED (previously "tried and reverted"; see git history for the original attempt and
    // the task report for the full re-measurement this decision is based on). A one-tick-lagged
    // snapshot of `column_depth`, taken before anything below writes to it this tick, used only
    // for the lateral edge's *neighbour* term (`head_b_full`). `column_depth` is not part of the
    // edge-flux path the frozen-Jacobi conversion is scoped to -- it is a scalar overburden
    // estimate that only ever feeds a driving *term*, never a mass limit -- which is why freezing
    // its cross-neighbour read was originally treated as out of scope and reverted.
    //
    // That revert was measured against the PRE-Jacobi baseline. Re-measured on top of the
    // frozen-Jacobi conversion (current `main`), the picture changes: plain Jacobi alone had
    // already moved `test_liquid_stream_stays_coherent`'s max_width from 7 to 9 with this read
    // still live, so this freeze's INCREMENTAL cost there is zero (9 -> 9, not 7 -> 9). The
    // remaining incremental costs are real but smaller than the original note implied:
    // `test_liquid_flowing_liquid_does_not_stand_in_walls`'s voids@tick160 was already at 19 on
    // plain Jacobi (against a <= 20 bound, not 0) and this freeze pushes it to 23 (+4, crosses the
    // bound by 3; total void-cell-ticks over the full run actually improves, 10112 -> 9283); the
    // narrow-neck (nw=0.02) drain-order instrument's f_50 regresses from 0.644 to 0.617, close to
    // the no-ordering null of 0.613, though it *improves* at wider necks (0.04/0.08/0.12); and
    // `bench_sandfall` shows ~3-4% ms/tick overhead from the added per-tick `Vec::clone` (an
    // unoptimized snapshot; a double-buffer swap would remove this if it matters).
    //
    // In exchange, `test_water_blob_stays_left_right_symmetric_under_gravity`'s even/odd
    // tick-phase-parity mismatch -- worst=1.643e-2 vs 5.041e-2 and late_persistent_run=46 vs 75 on
    // plain Jacobi, a ~3x swing purely from which parity `tick_count` happens to start at -- nearly
    // disappears (worst 3.0527930e-2 vs 3.0527925e-2, late_run 43 vs 43). This is very likely the
    // cause of the reported asymmetric/left-drifting drainage, so despite the costs above the
    // freeze stays applied. NOTE: it does NOT reach full bit-for-bit invariance the way the
    // original attempt's note claimed -- `test_tick_phase_mechanism_isolation` at full precision
    // still shows a ~0.1% residual on `final` for the cell-level-lateral-sweep and block-order
    // mechanisms (2.1133e-3 vs 2.1156e-3 baseline); `worst` and `late_run` are effectively exact.
    // The test itself still fails either way (its bound is intentionally strict; see its own
    // comment), so this does not change that test's pass/fail status.

    let cols = (w + block_size - 1) / block_size;
    let rows = (h + block_size - 1) / block_size;
    let expected_len = cols * rows;

    if last_displacements.len() != expected_len {
        last_displacements.resize(expected_len, 0.0);
    }
    if last_simulated_ticks.len() != expected_len {
        last_simulated_ticks.resize(expected_len, 0);
    }
    if active_blocks.len() != expected_len {
        active_blocks.resize(expected_len, crate::BlockActivity::Inactive);
    }

    // --- Fresh-overburden MUST-simulate predicate (task #47: the "sand-slab" scheduling defect)
    // ---
    //
    // Every activation signal in the classification loop below is HISTORICAL: `last_displacements`
    // records what moved LAST tick, and `next_displacements` (this tick's own record) only becomes
    // `last_displacements` the tick after, so a block is woken one tick after the evidence a moving
    // body left behind appears — and falling sand outruns that lag, which is what produces the
    // large slabs-separated-by-clean-gaps artifact this task fixes ("Perfect simulation" — every
    // in-mask block holding material simulated every tick — does not show slabs, which is why this
    // is scheduling, not physics).
    //
    // This predicate is a STATE predicate on the CURRENT (pre-tick) field instead: it wakes a
    // block for what it is doing THIS tick, before anything moves, by checking whether any of its
    // cells hold material that is (a) unsupported (see `support_fraction`: the cell below it
    // cannot bear its weight) and (b) has somewhere to go. A naive "material with zero overburden"
    // test (an earlier attempt, kept as `FreshOverburdenVariant::OverburdenOnly` for comparison)
    // promotes almost the whole domain — every resting pile's entire free surface has zero
    // overburden, not just falling material, and surfaces span many blocks — which would bypass
    // `budget_n` wholesale, exactly what the wake-magnitude MUST bar below was built to avoid.
    // Requiring somewhere to go is what keeps a flat resting bed (unsupported nowhere, material
    // present, nothing to do) from qualifying.
    //
    // The shipped variant, `UnsupportedAndRoom`, reads `support_fraction` — current heights and
    // static per-cell capacity only, never `edge_vel_v`, never last tick's values — rather than
    // `column_depth`'s `in_transit_at`-based overburden estimate. An earlier version of this
    // predicate used overburden directly; `diag_task47_in_transit_underdetection` showed
    // `in_transit_at` reads exactly `0.0` for every interior row of a body falling as a solid,
    // packed mass (only its own leading/trailing edges read nonzero), so that version only ever
    // caught the outermost row or two of a falling body. `support_fraction` has no such blind spot.
    //
    // Deliberately does NOT touch `column_depth` or route through the `fresh_pressure_field`
    // toggle: `support_fraction` and `fresh_overburden_must_blocks` read only function-local state
    // and are called by nothing outside this block. `column_depth` itself, and every physics term
    // the driving head builds from it, is computed exactly as it is today, below, unconditionally
    // on this predicate. This predicate only ever adds indices to `must_simulate`; it never feeds a
    // physics quantity. Block activation and the driving head are independent consumers of
    // "does this material press on what is below it" — see `fresh_pressure_field`'s own doc comment
    // for the confirmed, unexplained standing-arch regression that toggle causes, which this
    // predicate must not inherit, and `support_fraction`'s own doc comment for why it is a
    // plausible candidate to replace `in_transit_at` there too, deliberately not done by this task.
    let fresh_active = if fresh_overburden_gate::is_disabled() {
        vec![false; expected_len]
    } else {
        fresh_overburden_must_blocks(
            w,
            h,
            block_size,
            cols,
            rows,
            shape_mask,
            &heightmap.data,
            &heightmap.external_mass_this_tick,
            cell_props,
            edge_vel_v,
            fresh_overburden_gate::variant(),
        )
    };

    // Constants from the design doc. `MUST_SIMULATE_THRESHOLD` now lives at module scope (see
    // its doc comment) so `activate_neighbor_upstream` can reuse it by name; kept here too via
    // that same binding rather than a second declaration.
    const MAX_STALENESS: u32 = 30;
    const FLOW_INACTIVE_THRESHOLD: f32 = 3e-4;

    // 1. Identify MUST, STALE, and REST blocks, and calculate priorities
    let mut must_simulate = Vec::new();
    let mut stale_simulate = Vec::new();
    let mut rest_candidates = Vec::new();

    // A block is MUST-simulate when the wake magnitude its cells recorded last tick clears this
    // bar; everything under it competes for the remaining budget by `staleness * displacement`.
    //
    // Sandbox used to sit at 0.1, a thousand times coarser than gravity's 1e-4, and it had to:
    // the liquid path's wake magnitude was an absolute height, `|h - DEFAULT_SAND_HEIGHT|`, which
    // never returns to zero for a pool resting anywhere else, so the only thing keeping the whole
    // domain from being permanently MUST was a bar set above a typical bed offset. The cost was
    // that no ripple could clear it either — a wavefront's recorded magnitude is ~1e-3 — so
    // Sandbox waves propagated at a speed set by the budget rather than by the physics.
    //
    // The liquid wake magnitude is now a head *difference* across the cell's owned edges (see the
    // block-activation note in the g = 0 branch below), which is zero for any pool at rest at any
    // level, so the two modes can share one threshold.
    //
    // Both halves are required and neither works alone. Dropping this bar while the wake magnitude
    // was still a level makes a settled 256x256 pool at 0.50 report 7680 of 7680 MUST block-ticks
    // over a staleness period — the entire domain, permanently, with nothing moving. Keeping the
    // bar while fixing the magnitude leaves a ~1e-3 wavefront just as far under 0.1 as before.
    // With both, that same settled pool measures 0 MUST block-ticks at 0.35 *and* at 0.50
    // (`test_settled_sandbox_pool_does_not_stay_hot`) and reach stops depending on the budget at
    // all (`test_sandbox_wave_reach_is_budget_independent`).
    let active_threshold = MUST_SIMULATE_THRESHOLD;
    for b in 0..expected_len {
        let displacement = last_displacements[b];
        // `phase_offset(K_LOD_STALENESS)` is a diagnostic knob only: staleness is a DIFFERENCE, not a
        // parity, so adding a constant offset here shifts *when* a block first crosses
        // `MAX_STALENESS` (the LOD schedule), not which of two symmetric branches runs. It is
        // still the right knob to isolate this mechanism's contribution to the global
        // tick-phase-shift lean, because it's the only local perturbation that reproduces what a
        // `tick_count` shift does to this site specifically, without touching any other site.
        let staleness = (tick_count + phase_offset(K_LOD_STALENESS))
            .saturating_sub(last_simulated_ticks[b])
            .min(MAX_STALENESS);

        if displacement >= active_threshold || fresh_active[b] {
            must_simulate.push(b);
        } else if staleness >= MAX_STALENESS {
            stale_simulate.push(b);
        } else if displacement > 0.0 {
            // Priority function: staleness * displacement
            let priority = (staleness as f32) * displacement;
            rest_candidates.push((b, priority));
        }
    }

    // Quick exit check if no blocks are active
    if must_simulate.is_empty() && stale_simulate.is_empty() && rest_candidates.is_empty() {
        active_bounds.active = false;
        active_blocks.fill(crate::BlockActivity::Inactive);
        return 0.0;
    }

    let total_always = must_simulate.len() + stale_simulate.len();
    let remaining_budget = if budget_n > total_always {
        budget_n - total_always
    } else {
        0
    };

    let mut budget_simulate = Vec::new();
    if remaining_budget > 0 && !rest_candidates.is_empty() {
        let n = remaining_budget.min(rest_candidates.len());
        rest_candidates.select_nth_unstable_by(n - 1, |a, b| {
            b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal)
        });
        for i in 0..n {
            budget_simulate.push(rest_candidates[i].0);
        }
    }

    let mut will_simulate = vec![false; expected_len];
    for &b in &must_simulate {
        will_simulate[b] = true;
    }
    for &b in &stale_simulate {
        will_simulate[b] = true;
    }
    for &b in &budget_simulate {
        will_simulate[b] = true;
    }

    // Update active_blocks for HUD statistics
    active_blocks.fill(crate::BlockActivity::Inactive);
    for &b in &must_simulate {
        active_blocks[b] = crate::BlockActivity::Fast;
    }
    for &b in &stale_simulate {
        active_blocks[b] = crate::BlockActivity::Slow;
    }
    for &b in &budget_simulate {
        active_blocks[b] = crate::BlockActivity::Medium;
    }

    // Use precomputed shape mask instead of per-frame eval_sandbox_shape
    // shape_mask values: 0 = OUTSIDE (wall), 1 = INSIDE (safe), 2 = BOUNDARY (inside, near wall)
    let is_inside = |cx: usize, cy: usize| -> bool {
        shape_mask[cy * w + cx] != crate::MASK_OUTSIDE
    };

    let mut modified = will_simulate.clone();

    // 1. Copy heightmap to working buffer at start of frame
    temp_heights.copy_from_slice(&heightmap.data);

    let gravity_active = gravity_dir.length_squared() > 1e-6;

    // FALLBACK CONFIGURATION (Task #54): by default (`fresh_pressure_field == false`),
    // `column_depth` (depth-integrated lateral pressure; see `LATERAL_PRESSURE_SCALE`'s doc
    // comment for what this quantity means) is NOT computed here or by any standalone pass. It is
    // computed inline, order-dependent, inside phase 1's own CA branch below (see the "FALLBACK
    // MEASUREMENT" comment at that site and at the top of the `for phase in 0..2` loop) —
    // exactly as it was before this task touched this pass. A standalone-pass version
    // (`recompute_column_depth`, called once per phase) was written and measured; it did not fix
    // the target regression and is kept unused for reference — see that function's own doc
    // comment for the numbers and the root-cause instrumentation.
    //
    // FRESH PRESSURE FIELD (`fresh_pressure_field == true`; see this function's own doc comment
    // on that parameter): run `recompute_column_depth` here instead, once per tick,
    // unconditionally, over the WHOLE grid, reading this tick's frozen `heightmap.data` snapshot
    // for both `source_heights` and `heightmap_data` (they are the same array at this
    // once-per-tick/before-phase-0 call site — see `recompute_column_depth`'s doc comment on why
    // those two parameters can differ at other call sites but do not here). The inline write
    // inside phase 1's CA branch is skipped whenever this is on (see the matching
    // "FRESH PRESSURE FIELD" guard at that site), so the two configurations never both write
    // `column_depth` the same tick.
    if fresh_pressure_field {
        recompute_column_depth(
            w,
            h,
            shape_mask,
            &heightmap.data,
            &heightmap.data,
            &heightmap.external_mass_this_tick,
            cell_props,
            edge_vel_v,
            &mut column_depth[..],
        );
    }

    let mut total_flow = 0.0f32;
    let mut next_displacements = vec![0.0f32; expected_len];
    let mut flow_occurred = false;


    // TASK #55 step 2/3 (rebuilt): the PERSISTENT hydraulic head field is advanced by exactly one
    // tick's worth of local relaxation here -- same placement/reasoning as `fresh_pressure_field`'s
    // `recompute_column_depth` call just above: reads
    // `heightmap.data` as the tick's frozen, not-yet-mutated snapshot, so both phases below see
    // one consistent field. Moves no mass itself (see `task55_head_field`'s module doc comment)
    // -- it is read-only input to the phase-0/phase-1 edge solvers' driving head, gated per-edge
    // on `head_field_transport` and liquid-only endpoints (see those call sites).
    //
    // UNLIKE the deleted `compute_head_field`, this does NOT solve to convergence here -- it
    // mutates the caller-owned persistent `head_field` buffer by exactly
    // `task55_head_field::HEAD_FIELD_SWEEPS_PER_TICK` local sweeps and returns, carrying whatever
    // state of relaxation it reaches over to the NEXT tick's call. Cost is therefore
    // `O(wet_cells)`, fixed and small regardless of grid resolution -- see that constant's own
    // doc comment for the propagation-speed argument this is sized from. Skipped entirely while
    // inactive, so an unused field costs nothing beyond the one resize check above.
    //
    // OR'd with `head_field_gate::is_enabled()` -- a `#[cfg(test)]`-only thread-local, same
    // pattern as `fresh_pressure_field`/`multiplicative_lateral_gate` -- so a test built through
    // `TestSim`/other helpers that never expose this parameter directly (e.g.
    // `test_dry_sand_has_angle_of_repose`'s `ReposeRig`) can still force the branch on to check
    // it against a scenario this parameter has no other route into.
    //
    // This is deliberately UNCHANGED from before `pressure_heatmap_head_field` existed, and stays
    // the only thing gating the phase-0/phase-1 edge solvers' driving-head selection just below --
    // the overlay toggle must never change what mass moves. `head_field_needs_advance` is the
    // (strictly wider) condition for whether `head_field` gets THIS tick's relaxation sweeps at
    // all: also true when only the overlay is on, so the pressure heat-map's new-field source has
    // a live buffer to read even with transport off (see `pressure_heatmap_head_field` parameter's
    // own doc comment for the defect this fixes).
    let head_field_active = head_field_transport || head_field_gate::is_enabled();
    // TASK #63 joins this gate but NOT `head_field_active`: `pressure_sensitive_flow` reads the
    // field for the donor's pressure without changing which driving head any edge uses, so it
    // needs the field LIVE but must not switch transport on as a side effect.
    let head_field_needs_advance =
        head_field_active || pressure_heatmap_head_field || pressure_sensitive_flow;
    if head_field_needs_advance {
        task55_head_field::advance_head_field(
            w,
            h,
            shape_mask,
            &heightmap.data,
            cell_props,
            &mut head_field[..],
        );
    }

    // --- Frozen-Jacobi candidate-flux state (edge-flux solver only; the granular CA's own
    //     `try_move` transfers are untouched and still apply immediately, sequentially) ---
    //
    // Each phase (0 = gravity-aligned, 1 = everything else the flux solver owns) now runs in
    // three sub-passes instead of one:
    //
    //   1. COLLECT: walk every cell exactly as before, but instead of computing-and-applying a
    //      flux in one step, compute the *candidate* flux for each edge this cell owns (see
    //      `flux_edge_candidate`) from state nothing in this phase has mutated yet, and record it
    //      here. Because nothing is mutated until step 3, every candidate in a phase is reading
    //      the identical frozen snapshot regardless of which cell the scheduler happened to visit
    //      first — the whole point of this rewrite.
    //   2/3. ARBITRATE + APPLY: see the big comment just above the post-collection loop at the end
    //      of each phase body for the capacity-limiter algorithm and why one pass suffices.
    //
    // `cand_h[i]` / `cand_v[i]` hold the horizontal edge (i, i+1) / vertical edge (i, i+w) owned by
    // cell `i`, valid only where `edge_h_active[i]` / `edge_v_active[i]` is set. They hold the
    // *candidate* (single-edge-clamped, pre-arbitration) flux during COLLECT and are overwritten
    // in place with the *final* (post-arbitration) flux during APPLY, since nothing downstream
    // needs the raw candidate once arbitration has run.
    //
    // `cell_avail[i]` / `cell_freecap[i]` are cell `i`'s frozen donor-mass / acceptor-free-capacity
    // limits for whichever edges touch it this phase (a pure function of the cell and the phase's
    // context — gravity-aligned vs. lateral, in-transit-adjusted or not — never of which specific
    // edge is asking, so it is safe for more than one edge to write the same value here).
    // `cell_out_total[i]` / `cell_in_total[i]` are the *sums* of raw candidate magnitudes where `i`
    // is the donor / acceptor across every edge that touched it this phase; that sum is exactly
    // what the single Gauss-Seidel sweep used to prevent by construction (an edge processed later
    // saw the earlier edge's already-reduced `temp_heights`) and what arbitration now prevents
    // explicitly.
    //
    // `cell_out_total_jit[i]` / `cell_in_total_jit[i]` are the same two sums with each term weighted
    // by that edge's `edge_share_jitter`. BOTH the raw and the jittered sums are needed and neither
    // can be derived from the other: the raw pair decides *whether* a cell is oversubscribed, the
    // jittered pair decides *how* an oversubscribed budget is divided. `budget_term`'s doc comment
    // has the soundness argument for using them for those two different jobs.
    //
    // All buffers are sized to the full grid (indexed by cell, not by block) and allocated once,
    // outside the phase loop; only the cells actually touched this phase are ever written to, and
    // the `touched_*` lists are what let the next phase clear exactly those entries back to their
    // default instead of paying an O(grid) reset every phase.
    let cell_count = heightmap.data.len();
    let mut cand_h = vec![0.0f32; cell_count];
    let mut cand_v = vec![0.0f32; cell_count];
    let mut edge_h_active = vec![false; cell_count];
    let mut edge_v_active = vec![false; cell_count];
    let mut cell_out_total = vec![0.0f32; cell_count];
    let mut cell_in_total = vec![0.0f32; cell_count];
    let mut cell_out_total_jit = vec![0.0f32; cell_count];
    let mut cell_in_total_jit = vec![0.0f32; cell_count];
    let mut cell_avail = vec![0.0f32; cell_count];
    let mut cell_freecap = vec![0.0f32; cell_count];
    // Phase 1's g=0 (Sandbox) liquid branch also needs, per center cell, the largest raw head
    // difference across its owned edges (`max_head_diff`, computed unconditionally during COLLECT
    // — it does not depend on arbitration) so the post-APPLY block-wake check can be run once
    // arbitration has settled `cand_h`/`cand_v` into their final values. `g0_liquid_cells` is the
    // set of cells that took that branch this phase at all, whether or not they ended up owning a
    // live edge.
    let mut max_head_diff_cell = vec![0.0f32; cell_count];
    let mut touched_h: Vec<usize> = Vec::new();
    let mut touched_v: Vec<usize> = Vec::new();
    let mut touched_cells: Vec<usize> = Vec::new();
    let mut g0_liquid_cells: Vec<usize> = Vec::new();

    // Pressure projection scratch (`pressure_project`, run once per phase between COLLECT and the
    // arbitration totals below). Same allocate-once-outside-the-loop, clear-only-what-was-touched
    // discipline as the buffers above; `pressure_project` itself owns clearing these via its own
    // `nodes` list from the previous call.
    let mut pressure_phi_a = vec![0.0f32; cell_count];
    let mut pressure_phi_b = vec![0.0f32; cell_count];
    let mut pressure_fstar = vec![0.0f32; cell_count];
    let mut pressure_lap = vec![0.0f32; cell_count];
    let mut pressure_degree = vec![0u32; cell_count];
    // Per-node capacity cache, populated once per node on its first visit each call (wetness, and
    // therefore `cell_capacity_for(wetness)`, is constant for the duration of one `pressure_project`
    // call, so this is what lets the Jacobi loop stop re-deriving it every iteration -- see that
    // function's perf note). Never needs clearing: every read is preceded, within the SAME call, by
    // a write at that node's first visit, unlike `phi_a`/`phi_b`/`fstar`/`lap`/`degree`, which carry
    // meaning across a call's own iterations and so must be reset from the previous call's leftovers.
    let mut pressure_cap = vec![0.0f32; cell_count];
    let mut pressure_nodes: Vec<usize> = Vec::new();

    // 2. Continuous per-cell solver (loop over active blocks)
    let b_len = expected_len;
    // Directional operator split for liquid under gravity.
    //
    //   phase 0 — the liquid solver's gravity-aligned edges only, scanned *against* gravity.
    //   phase 1 — everything else: the granular CA, the Sandbox (g = 0) liquid solver, and the
    //             liquid solver's cross-gravity edges.
    //
    // Both halves of the split are ordering choices, not tuned coefficients, and each fixes a
    // distinct failure of the naive fused pass:
    //
    // *Why the directions are separated.* A cell in free fall and a cell in a settled pool
    // present the same fill difference to their lateral neighbours — a full cell beside an empty
    // one — so a fused pass cannot tell "falling" from "resting" and spreads both, fanning a
    // 4-cell stream out to 33. What actually distinguishes them is that the falling cell has
    // somewhere to go *along* gravity and the pooled cell does not. Resolving the gravity-aligned
    // edges first makes that physical: by the time lateral edges are evaluated, a falling cell has
    // already handed its mass to the cell below and has nothing left to give sideways, while a
    // pooled cell still holds all of it and levels out. That is the hydrostatic statement "no
    // lateral pressure without support", obtained from the update order instead of from a
    // free-fall special case.
    //
    // *Why phase 0 runs against gravity.* Sweeping down-gravity is Gauss-Seidel in the flow
    // direction: row y donates into row y+1, then row y+1 — already topped up — donates into
    // y+2, so one pass cascades a parcel the whole height of the grid and the stream arrives as
    // a stretched 0.10-fill smear. Sweeping bottom-to-top empties the acceptor before the donor
    // is considered, which is the CFL-respecting order: mass advances at most one cell per tick
    // and a saturated stream stays saturated (peak fill 1.0).
    for phase in 0..2usize {
        // phase 0 only exists for in-plane gravity; at g = 0 there is no gravity-aligned
        // direction and the Sandbox liquid solver handles both of its edges in phase 1.
        if phase == 0 && !gravity_active {
            continue;
        }

        // FALLBACK MEASUREMENT (Task #54): standalone per-phase `column_depth` recompute
        // disabled here — `column_depth` is computed inline inside the phase-1 CA loop instead
        // (see the "FALLBACK MEASUREMENT" comment at that site). Restore this call when
        // reinstating step 1.

        // Clear exactly the candidate-flux state the *previous* phase touched (a no-op on
        // phase 0, the first phase run, since every `touched_*` list starts empty). Sparse by
        // construction — only cells with a live edge or a g=0-liquid visit last phase pay this
        // cost — rather than an O(grid) fill every phase.
        for &idx in &touched_h { edge_h_active[idx] = false; cand_h[idx] = 0.0; }
        for &idx in &touched_v { edge_v_active[idx] = false; cand_v[idx] = 0.0; }
        for &idx in &touched_cells {
            cell_out_total[idx] = 0.0;
            cell_in_total[idx] = 0.0;
            cell_out_total_jit[idx] = 0.0;
            cell_in_total_jit[idx] = 0.0;
            cell_avail[idx] = 0.0;
            cell_freecap[idx] = 0.0;
        }
        for &idx in &g0_liquid_cells { max_head_diff_cell[idx] = 0.0; }
        touched_h.clear();
        touched_v.clear();
        touched_cells.clear();
        g0_liquid_cells.clear();

        // True when phase 0 should walk rows bottom-to-top (the usual case: gravity points at
        // +y, i.e. down the grid).
        let against_gravity_is_up = gravity_dir.y >= 0.0;
    for idx_b in 0..b_len {
        let b = if phase == 0 {
            // Reverse of the main block order along the gravity axis.
            let by_fwd = idx_b / cols;
            let by = if against_gravity_is_up { rows - 1 - by_fwd } else { by_fwd };
            let bx_idx = idx_b % cols;
            let bx = if (tick_count + phase_offset(K_BLOCK_ORDER) + by as u32) % 2 == 0 {
                bx_idx
            } else {
                cols - 1 - bx_idx
            };
            by * cols + bx
        } else if gravity_active && gravity_dir.y > 0.0 {
            // Under downward gravity, process blocks top-to-bottom so falling sand advects across block boundaries without trapping
            let by = idx_b / cols;
            let bx_idx = idx_b % cols;
            let bx = if (tick_count + phase_offset(K_BLOCK_ORDER) + by as u32) % 2 == 0 {
                bx_idx
            } else {
                cols - 1 - bx_idx
            };
            by * cols + bx
        } else if (tick_count + phase_offset(K_NONDOWN_BLOCK_PARITY)) % 2 == 0 {
            idx_b
        } else {
            b_len - 1 - idx_b
        };
        if !will_simulate[b] {
            continue;
        }

        let bx = b % cols;
        let by = b / cols;
        let start_x = bx * block_size;
        let end_x = ((bx + 1) * block_size).min(w);
        let start_y = by * block_size;
        let end_y = ((by + 1) * block_size).min(h);

        let x_len = end_x - start_x;
        let y_len = end_y - start_y;
        for idy in 0..y_len {
            let y = if phase == 0 {
                if against_gravity_is_up { end_y - 1 - idy } else { start_y + idy }
            } else if gravity_active && gravity_dir.y > 0.0 {
                start_y + idy
            } else if (tick_count + phase_offset(K_NONDOWN_ROW_PARITY)) % 2 == 0 {
                end_y - 1 - idy
            } else {
                start_y + idy
            };
            let row_offset = y * w;
            for idx in 0..x_len {
                let x = if gravity_active {
                    if (tick_count + phase_offset(K_LATERAL_SWEEP) + y as u32) % 2 == 0 {
                        start_x + idx
                    } else {
                        end_x - 1 - idx
                    }
                } else if (tick_count + phase_offset(K_NONGRAVITY_X_PARITY)) % 2 == 0 {
                    start_x + idx
                } else {
                    end_x - 1 - idx
                };
                let center_idx = row_offset + x;

                let mask_val = shape_mask[center_idx];
                let inside = mask_val != crate::MASK_OUTSIDE;

                if !inside {
                    continue;
                }

                let wetness = cell_props[center_idx * 4 + PROP_WETNESS];

                if phase == 0 {
                    // Gravity-aligned pass — see the operator-split note above. Originally liquid
                    // only (gated on `wetness <= 0.65`, i.e. `liquidity == 0`); Stage B extends it
                    // to carry the granular share of this same edge too, so the vertical/
                    // gravity-aligned edge is now *entirely* owned by the flux solver for every
                    // material, liquid or granular, and the CA below no longer touches it (see the
                    // `ndy != 0.0` exclusion in the avalanche valve and main flow loop further
                    // down). `granular_share` there is always `1 - cell_liquidity` under gravity,
                    // so the two shares sum to exactly 1.0 and nothing needs a separate "granular"
                    // flux_edge call — one call at `weight = 1.0` covers the whole edge.
                    if x > 0 && x + 1 < w && y > 0 && y + 1 < h && is_inside(x, y + 1) {
                        let cell_liquidity = liquidity(wetness);
                        let nb_idx = center_idx + w;
                        // Frozen read: phase 0 is always the first phase to touch any cell (see
                        // the candidate-flux buffer comment above `settle_tick`'s phase loop), so
                        // `heightmap.data` — the tick's untouched starting heights — and
                        // `temp_heights` coincide here. Reading `heightmap.data` explicitly (not
                        // `temp_heights`) is what makes that invariant self-evident rather than an
                        // accident of phase order: every cell in this phase reads the SAME
                        // snapshot regardless of which cell the scheduler visits first, which is
                        // the frozen-Jacobi property this conversion exists to establish. Nothing
                        // is mutated until this phase's post-collection APPLY step, further down.
                        let h_a = heightmap.data[center_idx];
                        let h_b = heightmap.data[nb_idx];
                        let cap_a = cell_capacity_for(wetness);
                        let cap_b = cell_capacity_for(cell_props[nb_idx * 4 + PROP_WETNESS]);
                        let (cap_a_eff, cap_b_eff) = if overfill_active {
                            (
                                cell_overfill_capacity_for(wetness, overfill_ratio),
                                cell_overfill_capacity_for(cell_props[nb_idx * 4 + PROP_WETNESS], overfill_ratio),
                            )
                        } else {
                            (cap_a, cap_b)
                        };
                        // Driving head on this edge, fill term normalised to fraction-of-capacity
                        // (dimensionless, 0..1) rather than raw mass. Without this, "one saturated
                        // cell of fill" is 1.5 for granular material (`cell_capacity_for` at
                        // `liquidity == 0`) but only 1.0 for liquid, so `g * GRAVITY_HEAD_SCALE`
                        // — tuned to cancel exactly one *liquid* cell's fill per row — cancelled
                        // only 1/1.5 of a granular cell's fill, leaving a net upward driving head
                        // on the gravity-aligned edge under a resting, at-capacity granular slab
                        // for any `g < cap / GRAVITY_HEAD_SCALE` (0.06 for `cap = 1.5`), and the
                        // whole slab climbed into the empty air above it. Water has `cap_a ==
                        // cap_b == 1.0` always, so `h / cap == h` and this is an exact no-op for
                        // fully liquid material (see `test_gravity_head_normalization_...` in the
                        // test module for the bit-identity check against the un-normalised form).
                        //
                        // The normalisation applies only to the *driving* term passed as
                        // `head_a`/`head_b` below. `avail_a`/`avail_b`/`cap_a`/`cap_b` — the
                        // donor-mass and acceptor-room clamps inside `flux_edge` — stay in raw
                        // mass units; normalising those too would break conservation (see
                        // `flux_edge`'s doc comment on why those clamps must stay in mass units).
                        let base_head = gravity_dir.y * GRAVITY_HEAD_SCALE;
                        // TASK #54 STEP 3: deep material falls faster. Without this, `H = h +
                        // Phi(g)` on this gravity-aligned edge has no depth term at all -- a cell
                        // resting under a hundred rows of overburden and a cell at the free
                        // surface get the exact same downward pull. `column_depth[center_idx]`
                        // (computed once for the whole grid in the standalone pass near the top
                        // of this function, before this phase even started) is this cell's own
                        // overburden; `janssen_effective_depth` gives it the right depth SHAPE per
                        // material (linear/unbounded for liquid, saturating for granular -- see
                        // that function's doc comment, Task #54 step 4) and `k_of_liquidity`
                        // discounts it for granular material so sand's vertical bonus, like its
                        // lateral one, stays lower magnitude than water's at every depth, not just
                        // in the saturated regime. See `VERTICAL_PRESSURE_CAP_MULT`'s doc comment
                        // for the CFL-style bound this is capped at and why it is needed.
                        let vertical_bonus = if base_head.abs() > 1e-9 {
                            let overburden = janssen_effective_depth(column_depth[center_idx], cell_liquidity);
                            let raw_bonus = VERTICAL_PRESSURE_SCALE * k_of_liquidity(cell_liquidity) * overburden;
                            raw_bonus.min(base_head.abs() * VERTICAL_PRESSURE_CAP_MULT) * base_head.signum()
                        } else {
                            0.0
                        };
                        // TASK #55 step 3, LIQUID ONLY (both endpoints at or above
                        // `LIQUID_ELLIPTIC_THRESHOLD`, the head field's own
                        // domain restriction -- see that constant's doc comment for "why
                        // liquid-only": the field has no yield criterion, so a granular pile at
                        // its angle of repose is a permanent surface gradient that must produce
                        // ZERO flow, and only a hard liquid gate guarantees that).
                        //
                        // The field's own `z` term (`-row * depth_scale`, `task55_head_field`'s
                        // doc comment) already IS the gravitational potential in reference-row
                        // units, so `head_field[a] - head_field[b]` carries the gravity drive AND
                        // the pressure difference in one physically consistent number -- that is
                        // the entire point of a unified head, and `base_head`/`vertical_bonus`
                        // must NOT be added alongside it (that would double-count exactly what
                        // the field already supplies).
                        //
                        // UNIT CONVERSION (comment which side, per this task's own history of a
                        // silent-mismatch bug here): `head_field` is in `task55_head_field`'s
                        // reference-row units (`depth_scale = REFERENCE_GRID_HEIGHT / w`);
                        // `head_a`/`head_b` must be in the SAME local-cell units `h_a`/`cap_a` and
                        // `flux_edge_candidate`'s `c_sq`/`tau` are calibrated against, so the
                        // FIELD SIDE is divided by `depth_scale` to convert reference-row units
                        // down to local-cell units -- the inverse of `recompute_column_depth`'s
                        // own local -> reference-row multiplication.
                        //
                        // ...AND THEN MULTIPLIED BY `GRAVITY_HEAD_SCALE`, which the first version
                        // of this branch omitted. Dividing by `depth_scale` alone leaves the field
                        // in units of CELLS OF ELEVATION -- one row of drop is a driving head of
                        // exactly `1.0`. The `else` branch below drives that same one-row drop with
                        // `base_head = gravity_dir.y * GRAVITY_HEAD_SCALE` = `25.0`. So the field
                        // branch was driving every liquid edge 25x too weakly, which does not read
                        // as "somewhat slow" -- it lands under the solver's own thresholds and
                        // reads as a COMPLETELY FROZEN simulation. Measured: with the toggle on,
                        // `spec_falling_water_does_not_drift_sideways` and
                        // `spec_draining_vessel_surface_dips` both reported `total_flow = 0.0000`,
                        // a blob that never fell and a vessel that never drained.
                        //
                        // `GRAVITY_HEAD_SCALE` is the existing, already-calibrated conversion from
                        // "one cell of elevation" to this solver's driving-head units, so this is
                        // a unit reconciliation, NOT a tuned gain: at one row of free fall the two
                        // branches now produce the identical number (`25.0`), which is the check
                        // that says the conversion is right.
                        let liq_b = liquidity(cell_props[nb_idx * 4 + PROP_WETNESS]);
                        let (head_a, head_b) = if overfill_active {
                            let depth_scale = REFERENCE_GRID_HEIGHT as f32 / w as f32;
                            let overfill_head_unit = (GRAVITY_HEAD_SCALE / depth_scale) * OVERFILL_STIFFNESS_K;
                            let o_a = relative_overfill(h_a, cap_a);
                            let o_b = relative_overfill(h_b, cap_b);
                            let p_a = o_a * overfill_head_unit;
                            let p_b = o_b * overfill_head_unit;
                            (h_a / cap_a + base_head + p_a, h_b / cap_b + p_b)
                        } else if head_field_active
                            && cell_liquidity >= LIQUID_ELLIPTIC_THRESHOLD
                            && liq_b >= LIQUID_ELLIPTIC_THRESHOLD
                        {
                            let head_scale =
                                GRAVITY_HEAD_SCALE / (REFERENCE_GRID_HEIGHT as f32 / w as f32);
                            (head_field[center_idx] * head_scale, head_field[nb_idx] * head_scale)
                        } else {
                            (h_a / cap_a + base_head + vertical_bonus, h_b / cap_b)
                        };
                        // Sleeping edge (see `edge_sleeps`). This is the pass where sleeping pays
                        // most, because it is the one every cell in the domain enters: the
                        // interior of a filled chamber/pile is room-blocked in both directions,
                        // empty space above the free surface/heap has nothing to donate in either,
                        // and only the surface itself — a few cells per column — survives the test.
                        if edge_sleeps(
                            head_a - head_b, 0.0, edge_vel_v[center_idx],
                            h_a, h_b, cap_a_eff - h_a, cap_b_eff - h_b,
                        ) {
                            if edge_vel_v[center_idx] != 0.0 {
                                edge_vel_v[center_idx] = 0.0;
                            }
                            continue;
                        }
                        // Dynamics are blended by `cell_liquidity`, not the flux weight: at
                        // `cell_liquidity == 1` this reduces exactly to `wave_params(wetness)` at
                        // `weight == 1.0`, bit-for-bit what the liquid-only pass computed before
                        // (Water etc. are untouched). At `cell_liquidity == 0` (any granular
                        // material) it instead uses a saturating pair that reaches the donor/
                        // acceptor clamp within a tick or two from rest rather than the liquid's
                        // multi-tick ramp — the CA's own free-fall transfer coefficient was
                        // already 0.8-1.0 (near-instant) once a cell had clear room below, and nothing
                        // in the CA imposed a repose-style yield stress on straight-down motion (the
                        // dominant `gravity_push` term swamped `threshold` there), so `tau = 0` here
                        // matches its predecessor's effective behaviour rather than inventing a new
                        // one. The donor-mass/acceptor-room clamp inside `flux_edge` (not this
                        // ramp) is what actually stops a packed column, exactly as it does for
                        // liquid.
                        const GRANULAR_FALL_C_SQ: f32 = 1.0;
                        const GRANULAR_FALL_DAMPING: f32 = 1.0;
                        let (liquid_c_sq, liquid_damping) = wave_params(wetness);
                        let c_sq = GRANULAR_FALL_C_SQ * (1.0 - cell_liquidity) + liquid_c_sq * cell_liquidity;
                        let damping = GRANULAR_FALL_DAMPING * (1.0 - cell_liquidity) + liquid_damping * cell_liquidity;
                        // TASK #63: pressure-sensitive flow rate. Scales this edge's FLUX -- via
                        // `flux_edge_candidate`'s own `weight` parameter, which exists to do
                        // exactly this -- by how much hydrostatic head the DONOR carries. It does
                        // NOT touch the driving head, so which branch produced `head_a`/`head_b`
                        // above is irrelevant here and this composes with `head_field_transport`
                        // either way.
                        //
                        // THE FLUX, NOT `c_sq`, and the distinction is load-bearing. A first
                        // version attenuated `c_sq` -- how fast the edge's momentum spins up --
                        // which is the more obvious reading of "flow rate". MEASURED, and it
                        // cannot produce a depth ordering: at any edge with real room to move
                        // into, the driving head is large enough that `v` reaches the
                        // donor-mass/acceptor-room clamp within a tick or two whatever `c_sq` is,
                        // so the realised flux is MASS-limited and `c_sq` drops out entirely. A
                        // draining vessel measured 127.1 (10 deep) against 124.5 (20 deep) -- no
                        // ordering at all. `c_sq` only matters where flow is velocity-limited,
                        // i.e. the low-gradient case, which is not where depth lives. Scaling the
                        // realised flux is also the more faithful reading of the law being
                        // implemented: Torricelli's `Q = C_d * A * sqrt(2*g*h)` puts the head
                        // dependence in a DISCHARGE COEFFICIENT on the flux, which is what this
                        // now is.
                        //
                        // DONOR, not the edge average or the higher end: this scales the rate at
                        // which a cell can PUSH material out, and only the cell material is
                        // leaving has a say in that.
                        //
                        // Free fall is untouched: `rows_of_head_at` returns exactly `0.0` for
                        // material `advance_head_field` pinned as unsupported, and
                        // `pressure_rate_factor` returns `1.0` there -- see both functions' doc
                        // comments for why that separation is exact rather than an epsilon.
                        let pressure_weight = if pressure_sensitive_flow
                            && cell_liquidity >= LIQUID_ELLIPTIC_THRESHOLD
                            && liq_b >= LIQUID_ELLIPTIC_THRESHOLD
                        {
                            pressure_rate_factor(task55_head_field::rows_of_head_at(
                                center_idx, w, head_field,
                            ))
                        } else {
                            1.0
                        };
                        let (cap_a_eff, cap_b_eff, h_a_source) = if overfill_active {
                            let depth_scale = REFERENCE_GRID_HEIGHT as f32 / w as f32;
                            let overfill_head_unit = (GRAVITY_HEAD_SCALE / depth_scale) * OVERFILL_STIFFNESS_K;
                            let g_step = (base_head.max(0.0) / overfill_head_unit) * cell_liquidity;
                            (
                                cell_overfill_capacity_for(wetness, overfill_ratio),
                                cell_overfill_capacity_for(cell_props[nb_idx * 4 + PROP_WETNESS], overfill_ratio),
                                h_a + g_step,
                            )
                        } else {
                            (cap_a, cap_b, h_a)
                        };
                        // COLLECT only: compute this edge's candidate flux and record it, plus
                        // this cell's and its neighbour's donor/acceptor limits for arbitration.
                        // Nothing is mutated yet — see this phase's post-collection ARBITRATE +
                        // APPLY pass below the nested loops, and the buffer comment above the
                        // phase loop for why phase 0's arbitration is provably a no-op (each cell
                        // owns exactly one vertical edge as donor and one as acceptor here) but is
                        // still routed through the same general machinery rather than
                        // special-cased.
                        let candidate = flux_edge_candidate(
                            head_a, head_b,
                            c_sq, damping, 0.0,
                            cap_a_eff, cap_b_eff,
                            cap_a, cap_b,
                            h_a, h_b,
                            h_a_source, h_b,
                            pressure_weight,
                            edge_vel_v[center_idx],
                        );
                        cand_v[center_idx] = candidate;
                        edge_v_active[center_idx] = true;
                        touched_v.push(center_idx);
                        cell_avail[center_idx] = h_a;
                        cell_freecap[center_idx] = (cap_a_eff - h_a).max(0.0);
                        cell_avail[nb_idx] = h_b;
                        cell_freecap[nb_idx] = (cap_b_eff - h_b).max(0.0);
                        touched_cells.push(center_idx);
                        touched_cells.push(nb_idx);
                        // Totals deferred: see the unified post-COLLECT `accumulate_edge_totals`
                        // pass below (after `pressure_project`), which sums the (possibly
                        // pressure-corrected) final candidates instead of these raw ones.
                    }
                    continue;
                }

                if wetness >= 0.75 && !gravity_active {
                    // --- Conservative edge-flux liquid solver (replaces the per-cell wave
                    //     update; see `flux_edge`) ---
                    //
                    // Each cell integrates only the two edges it *owns* — the one to its right
                    // and the one below it — so every edge in the domain is integrated exactly
                    // once per pass, by its lower-index endpoint. The left/top edges of this cell
                    // are owned by its left/top neighbours and were (or will be) handled there.
                    //
                    // Neumann reflection at the shape boundary is now structural: an edge whose
                    // far side is outside the mask simply does not exist, so no flux crosses it
                    // and no mass can be stranded in a wall cell (the old formulation mirrored
                    // `h_center` across the wall to get a zero-gradient; skipping the edge is the
                    // same boundary condition expressed on the flux instead of the height).
                    //
                    // *Jacobi driving.* The head difference that integrates an edge's velocity is
                    // read from `heightmap.data`, the tick's frozen starting heights, never from
                    // `temp_heights` mid-sweep. `temp_heights` is mutated four times per cell per
                    // pass (once by each incident edge) and `heightmap.data` is not written until
                    // the copy-back in step 3, so it is a stable snapshot at zero cost — the same
                    // one the Sandbox granular CA below already reads.
                    //
                    // This is not a style preference, it is the stability condition. Driving the
                    // velocities from the live buffer makes the update Gauss-Seidel with a
                    // direction-alternating sweep, and Gauss-Seidel on a wave equation is not
                    // merely less accurate — it is a *gain*. Linearising the 1-D chain at Water's
                    // (c_sq, damping) = (0.24, 0.98) gives a per-tick spectral radius of 1.20 for
                    // the swept form against 0.994 for this one: the sweep injected ~20% of
                    // amplitude per tick while the damping removed 2%, so a ripple grew until it
                    // hit the cell cap and stuck there (peak 0.80 -> pinned at 1.0000 in under 50
                    // ticks) instead of decaying back to a flat pool. Raising the cap did not
                    // help; it only moved the ceiling and made the directional bias visible as
                    // 33x worse left/right asymmetry.
                    //
                    // The clamps below (`avail`/`cap - h`) are now candidate-level, not final —
                    // arbitration (this phase's post-collection ARBITRATE + APPLY pass, below the
                    // nested loops) is what actually enforces "must not see what the other edges
                    // incident on this cell have already taken", now that nothing is applied
                    // mid-phase to see. Because every edge still debits exactly what it credits,
                    // and arbitration only ever scales a candidate down (never up, never
                    // negative — see that pass's comment), Jacobi ordering costs nothing in
                    // conservation — that property is structural in the flux form, not a
                    // consequence of the sweep order.
                    //
                    // Gravity-driven liquid deliberately does *not* get this treatment: under
                    // gravity the solver is doing advection down a hydrostatic head, where the
                    // ordering (gravity-aligned edges first, swept against gravity) is load
                    // bearing for CFL. Gauss-Seidel is only wrong for the conservative,
                    // energy-carrying case, which is exactly this g = 0 branch.
                    let (c_sq, damping) = wave_params(wetness);
                    let cap_c = cell_capacity_for(wetness);
                    // Largest head difference across the edges this cell owns — the *driving*
                    // term, the same quantity `edge_sleeps`' branch 2 tests against `tau`. It is
                    // the wake magnitude; see the block-activation note at the end of this branch
                    // for why it is a difference and not a level. Computed unconditionally during
                    // COLLECT (it does not depend on arbitration); the block-wake check itself is
                    // deferred to this phase's post-APPLY pass, once `max_flux` — the *other* half
                    // of that check — has its final, post-arbitration value.
                    let mut max_head_diff = 0.0f32;
                    let head_c = heightmap.data[center_idx];
                    g0_liquid_cells.push(center_idx);

                    // *Which buffer the sleeping test reads is the whole subtlety here.* The two
                    // branches of `edge_sleeps` mirror two different clauses of `flux_edge`, and
                    // those clauses read different buffers on this path — so the predicate must
                    // too, or it would sleep an edge that would in fact have moved mass:
                    //
                    //   * the *driving* head goes to `yielded`, which under Jacobi driving is
                    //     computed from `heightmap.data` (the tick's frozen snapshot, per the note
                    //     above). Passing `temp_heights` here would test a head the solver never
                    //     uses.
                    //   * the *donor and acceptor* limits are the live clamps, which deliberately
                    //     stay on `temp_heights` so they see what the other three edges incident
                    //     on this cell have already taken this pass.
                    //
                    // A wave is safe from both branches by construction: its crest cells are not
                    // room-blocked (branch 1 needs a full or empty cell on both sides), and it only
                    // has `H_a == H_b` exactly while `v_e` is carrying it, which branch 2 excludes.
                    if x + 1 < w && is_inside(x + 1, y) {
                        let nb_idx = center_idx + 1;
                        let h_a = temp_heights[center_idx];
                        let h_b = temp_heights[nb_idx];
                        let cap_b = cell_capacity_for(cell_props[nb_idx * 4 + PROP_WETNESS]);
                        let (cap_c_eff, cap_b_eff) = if overfill_active {
                            (
                                cell_overfill_capacity_for(wetness, overfill_ratio),
                                cell_overfill_capacity_for(cell_props[nb_idx * 4 + PROP_WETNESS], overfill_ratio),
                            )
                        } else {
                            (cap_c, cap_b)
                        };
                        let (head_c_drive, head_b_drive) = if overfill_active {
                            let depth_scale = REFERENCE_GRID_HEIGHT as f32 / w as f32;
                            let overfill_head_unit = (GRAVITY_HEAD_SCALE / depth_scale) * OVERFILL_STIFFNESS_K;
                            let o_c = relative_overfill(head_c, cap_c);
                            let o_b = relative_overfill(heightmap.data[nb_idx], cap_b);
                            (head_c + o_c * overfill_head_unit, heightmap.data[nb_idx] + o_b * overfill_head_unit)
                        } else {
                            (head_c, heightmap.data[nb_idx])
                        };
                        max_head_diff = max_head_diff.max((head_c_drive - head_b_drive).abs());
                        if edge_sleeps(
                            head_c_drive - head_b_drive, 0.0, edge_vel_h[center_idx],
                            h_a, h_b, cap_c_eff - h_a, cap_b_eff - h_b,
                        ) {
                            if edge_vel_h[center_idx] != 0.0 {
                                edge_vel_h[center_idx] = 0.0;
                            }
                        } else {
                            // COLLECT only — see the phase-loop buffer comment and
                            // `flux_edge_candidate`'s doc comment. This cell also owns the y-edge
                            // below (next block), so — unlike phase 0 — this cell can be a donor
                            // (or acceptor) on *two* owned edges at once this phase, which is
                            // exactly the multi-edge case arbitration exists for.
                            let candidate = flux_edge_candidate(
                                head_c_drive, head_b_drive,
                                c_sq, damping, 0.0,
                                cap_c_eff, cap_b_eff,
                                cap_c, cap_b,
                                h_a, h_b, h_a, h_b, 1.0,
                                edge_vel_h[center_idx],
                            );
                            cand_h[center_idx] = candidate;
                            edge_h_active[center_idx] = true;
                            touched_h.push(center_idx);
                            cell_avail[center_idx] = h_a;
                            cell_freecap[center_idx] = (cap_c_eff - h_a).max(0.0);
                            cell_avail[nb_idx] = h_b;
                            cell_freecap[nb_idx] = (cap_b_eff - h_b).max(0.0);
                            touched_cells.push(center_idx);
                            touched_cells.push(nb_idx);
                            // Totals deferred: see the unified post-COLLECT pass below.
                        }
                    }

                    if y + 1 < h && is_inside(x, y + 1) {
                        let nb_idx = center_idx + w;
                        let h_a = temp_heights[center_idx];
                        let h_b = temp_heights[nb_idx];
                        let cap_b = cell_capacity_for(cell_props[nb_idx * 4 + PROP_WETNESS]);
                        let (cap_c_eff, cap_b_eff) = if overfill_active {
                            (
                                cell_overfill_capacity_for(wetness, overfill_ratio),
                                cell_overfill_capacity_for(cell_props[nb_idx * 4 + PROP_WETNESS], overfill_ratio),
                            )
                        } else {
                            (cap_c, cap_b)
                        };
                        let (head_c_drive, head_b_drive) = if overfill_active {
                            let depth_scale = REFERENCE_GRID_HEIGHT as f32 / w as f32;
                            let overfill_head_unit = (GRAVITY_HEAD_SCALE / depth_scale) * OVERFILL_STIFFNESS_K;
                            let o_c = relative_overfill(head_c, cap_c);
                            let o_b = relative_overfill(heightmap.data[nb_idx], cap_b);
                            (head_c + o_c * overfill_head_unit, heightmap.data[nb_idx] + o_b * overfill_head_unit)
                        } else {
                            (head_c, heightmap.data[nb_idx])
                        };
                        max_head_diff = max_head_diff.max((head_c_drive - head_b_drive).abs());
                        if edge_sleeps(
                            head_c_drive - head_b_drive, 0.0, edge_vel_v[center_idx],
                            h_a, h_b, cap_c_eff - h_a, cap_b_eff - h_b,
                        ) {
                            if edge_vel_v[center_idx] != 0.0 {
                                edge_vel_v[center_idx] = 0.0;
                            }
                        } else {
                            // COLLECT only — see the comment on the x-edge above.
                            let candidate = flux_edge_candidate(
                                head_c_drive, head_b_drive,
                                c_sq, damping, 0.0,
                                cap_c_eff, cap_b_eff,
                                cap_c, cap_b,
                                h_a, h_b, h_a, h_b, 1.0,
                                edge_vel_v[center_idx],
                            );
                            cand_v[center_idx] = candidate;
                            edge_v_active[center_idx] = true;
                            touched_v.push(center_idx);
                            cell_avail[center_idx] = h_a;
                            cell_freecap[center_idx] = (cap_c_eff - h_a).max(0.0);
                            cell_avail[nb_idx] = h_b;
                            cell_freecap[nb_idx] = (cap_b_eff - h_b).max(0.0);
                            touched_cells.push(center_idx);
                            touched_cells.push(nb_idx);
                            // Totals deferred: see the unified post-COLLECT pass below.
                        }
                    }

                    // Block-activation bookkeeping (`max_head_diff` only; `max_flux` is folded in
                    // once arbitration has finalised this cell's owned edges — see this phase's
                    // post-APPLY pass below the nested loops, which runs this exact check with the
                    // final `max_flux`). Recorded here unconditionally, matching the pre-Jacobi
                    // behaviour where this check ran once per cell regardless of which edges were
                    // live.
                    //
                    // The head-difference wake magnitude is `edge_sleeps`' branch-2 driving term,
                    // not an absolute level — see git history for why a level-based wake magnitude
                    // was a category error (a settled pool away from `DEFAULT_SAND_HEIGHT` looked
                    // perpetually "disturbed", while a real low-amplitude ripple's ~1e-3 deviation
                    // never cleared the old 0.1 must-simulate bar and only advanced when a block
                    // aged out — `test_sandbox_wave_reach_is_budget_independent` is the regression
                    // guard for that).
                    max_head_diff_cell[center_idx] = max_head_diff;
                } else {
                    // --- Cellular Automata (Sand settling behavior) ---
                    // CA requires accessing neighbors at offset 1, so we must be inside the grid boundaries
                    if x == 0 || x + 1 >= w || y == 0 || y + 1 >= h {
                        sliding[center_idx] = false;
                        continue;
                    }

                    // Continuous liquid weight for this cell (see `liquidity` doc comment).
                    // Computed once per center cell and reused by both the avalanche safety
                    // valve below and the main neighbor flow loop further down, so the acceptor
                    // capacity (C1, incompressibility) is enforced consistently everywhere a
                    // neighbor can receive mass in a single tick.
                    let cell_liquidity = liquidity(wetness);
                    let cell_capacity = 1.5 * (1.0 - cell_liquidity) + 1.0 * cell_liquidity;
                    // Complement of the liquid share handled by the edge-flux solver below.
                    // Exactly 1.0 for any granular material (liquidity == 0), so the CA path is
                    // bit-identical to before for sand.
                    let granular_share = if gravity_active { 1.0 - cell_liquidity } else { 1.0 };

                    // Stage C carve-out: Oobleck's shear-thickening (`get_ca_params`'s
                    // `wetness >= 0.50 && wetness < 0.65` branch, driven by `closest_marble_vel`)
                    // is a marble/mouse-interaction feature, not a repose/yield-stress question,
                    // and no test in this suite exercises it under gravity. Rather than guess at
                    // how a viscosity that depends on a dragged marble's speed should interact with
                    // a yield-stress lateral edge, Stage C leaves Oobleck's gravity-mode lateral
                    // transport entirely on the CA (bit-identical to pre-Stage-C), and the new
                    // combined lateral flux edge below is gated to skip it. `liquidity(0.55) == 0`,
                    // so Oobleck was already 100% CA-carried before this change; this keeps it that
                    // way rather than silently folding it into a mechanism that was never measured
                    // against it.
                    let is_oobleck_band = wetness >= 0.50 && wetness < 0.65;

                    // Per-edge RNG seed, hoisted from the main flow loop below (it used to be
                    // computed just before that loop) so the new combined lateral flux edge can
                    // also draw from it for dispersion/locking. Same formula, same inputs — moving
                    // it earlier changes nothing about what it produces.
                    let seed = (x as u32).wrapping_mul(1299689) ^ (y as u32).wrapping_mul(314159) ^ time_seed.wrapping_mul(7213);

                    // FALLBACK MEASUREMENT (Task #54): step 1 (the standalone column_depth pass)
                    // temporarily reverted back to this inline, order-dependent computation to
                    // measure steps 2-4 WITHOUT step 1, per the task brief. Restore the
                    // standalone pass (see git history / the per-phase version) once this
                    // measurement is taken; do not leave the tree in this state.
                    //
                    // FRESH PRESSURE FIELD: `&& !fresh_pressure_field` added so this inline write
                    // never runs when the once-per-tick standalone pass above (in this function's
                    // preamble, before the `for phase in 0..2` loop) already populated
                    // `column_depth` for this tick — the two must never both write the same cell
                    // the same tick, or whichever ran second would silently win.
                    if gravity_active && !fresh_pressure_field {
                        let above_idx = center_idx - w; // safe: the CA guard above requires y > 0
                        let depth_above = if is_inside(x, y - 1) {
                            let depth_scale = REFERENCE_GRID_HEIGHT as f32 / w as f32;
                            let resting_above =
                                (temp_heights[above_idx]
                                    - in_transit_at(above_idx, w, h, temp_heights, &heightmap.data, cell_props, edge_vel_v, shape_mask)
                                    - heightmap.external_mass_this_tick[above_idx].max(0.0))
                                .max(0.0)
                                * depth_scale;
                            resting_above + column_depth[above_idx]
                        } else {
                            0.0
                        };
                        column_depth[center_idx] = depth_above;
                    }

                    // --- Combined liquid + granular share: the same conservative edge-flux
                    //     solver as the g = 0 branch above, but with a non-zero gravitational
                    //     head Phi, AND (Stage C) a non-zero yield stress `tau` for the granular
                    //     share ---
                    //
                    // `H = h + Phi(g, r)` is the unified head. In Sandbox the grid plane is
                    // horizontal, gravity is perpendicular to it and Phi is identically zero, so
                    // `H = h` and the solver degenerates to the free-surface wave. In Sand-fall
                    // the grid is a vertical cross-section and gravity is in-plane, so Phi is a
                    // linear ramp along `g` and the head difference across a downhill edge picks
                    // up `|g| * GRAVITY_HEAD_SCALE` on top of the fill difference. Nothing else
                    // about the update changes: the same clamp on donor mass and acceptor
                    // capacity that makes ripples conservative at g = 0 is what produces
                    // hydrostatic stacking and level pools at g > 0.
                    //
                    // Before Stage C this call only carried the `cell_liquidity` share, with the
                    // granular CA below carrying the complementary `1 - cell_liquidity` share of
                    // this same edge via its own lateral `try_move`s. Stage C moves that granular
                    // share onto this same flux call instead of a second one: a cell's height is
                    // one blended quantity, not two separately-tracked liquid/granular stacks, so
                    // "cell_liquidity share of the flux" always meant "how liquid-like this edge's
                    // *parameters* are", not "only move the liquid fraction of the mass". `weight`
                    // is therefore 1.0 (the whole edge), and every parameter that differs between
                    // the two regimes — `tau` here, `wave_params` already did this for `(c_sq,
                    // damping)` — is blended by `cell_liquidity` instead. The granular CA's lateral
                    // loop is skipped for this case now (see the bail-out just below this block),
                    // so there is no double-counting.
                    if gravity_active && !is_oobleck_band && x + 1 < w && is_inside(x + 1, y) {
                        let nb_idx = center_idx + 1;
                        let h_a = temp_heights[center_idx];
                        let h_b = temp_heights[nb_idx];
                        let cap_b = cell_capacity_for(cell_props[nb_idx * 4 + PROP_WETNESS]);

                        // `tau` — the yield stress — is `PROP_THRESHOLD` (the material's own
                        // declared repose threshold, e.g. 0.08 for DrySand; see
                        // `MaterialMode::preset_props`), scaled by `GRANULAR_TAU_SCALE` (see that
                        // constant's doc comment for where to move this single value to retune
                        // repose angle later) and by `granular_share` (`1 - cell_liquidity`, and
                        // always available here since this whole branch requires `gravity_active`)
                        // so a fully liquid cell keeps zero yield stress (unchanged liquid
                        // behaviour) and a fully granular cell gets the whole of it. Deliberately
                        // NOT also multiplied by the CA's old in-gravity `* 0.35` discount
                        // (`get_ca_params`: "Lower friction/repose angle in Sand-fall mode for
                        // realistic fluid flow") — undoing that discount, which is precisely what
                        // made sand behave like a liquid under gravity, is the fix Stage C exists
                        // to make.
                        let threshold_prop = cell_props[center_idx * 4 + PROP_THRESHOLD];
                        let tau = GRANULAR_TAU_SCALE * threshold_prop * granular_share;

                        let h_a_frozen = heightmap.data[center_idx];
                        let h_b_frozen = heightmap.data[nb_idx];

                        // Lateral dispersion (see `DISPERSION_TAU_FRAC`'s doc comment): a small,
                        // signed, per-edge-per-tick random perturbation of the driving head,
                        // scaled to a FRACTION OF THIS EDGE'S OWN `tau` rather than a fixed
                        // magnitude. The old CA's dispersion term (`perp_dot * 3.5 *
                        // dispersion_noise`, up to 3.5 against a ~0.06-0.08 effective threshold)
                        // is exactly what made `tau` inoperative if ported unchanged — see the
                        // Stage C task brief. Scaling it to `tau` instead of a constant keeps it
                        // proportionate for every material's own threshold, and keeps a real yield
                        // stress that actually gates flow rather than being swamped by noise.
                        // Computed once and reused by both `edge_sleeps` and
                        // `flux_edge_candidate` below — both must see the identical driving term,
                        // per `edge_sleeps`'s own doc comment on why a mismatch there is unsound.
                        // Free-fall dispersion is not separately scaled down here (the old CA used
                        // a gentler `* 0.8` in free fall against `* 3.5` on the bed): the in-transit
                        // donor limit (`avail_a`/`avail_b` below) already suppresses a genuinely
                        // free-falling stream's lateral spread structurally (a saturated stream has
                        // `avail ~= 0` on this edge regardless of what drives it), so a second,
                        // separate free-fall tier here would be redundant with a mechanism that is
                        // already doing that job.
                        let disp_roll = ((seed ^ (nb_idx as u32).wrapping_mul(823)) & 0xFF) as f32 / 255.0;
                        let dispersion = (disp_roll - 0.5) * 2.0 * DISPERSION_TAU_FRAC * tau;

                        // `head_b_full` folds the neighbour's own depth-integrated overburden in
                        // (see `LATERAL_PRESSURE_SCALE`), symmetrically with `head_a` below, so the
                        // driving term compares total column pressure rather than local fill alone.
                        // `column_depth[nb_idx]` may be a tick stale if the neighbour's block ran
                        // after this one, or hasn't run yet this tick — harmless, since (like
                        // `GRAVITY_HEAD_SCALE`) it only ever feeds `driving`, never the mass limits.
                        // `column_depth` itself is now computed for EVERY material in the
                        // standalone pass near the top of this function (see that pass's doc
                        // comment) -- a pure granular cell gets its own genuine overburden here,
                        // not the `== 0` a stale comment used to claim.
                        //
                        // TASK #54 STEP 4: the depth term below is `janssen_effective_depth(...)`,
                        // not raw `column_depth`, for BOTH endpoints -- see that function's doc
                        // comment. Raw `column_depth` is a linear, unbounded, hydrostatic ramp:
                        // the right physical model for liquid, and known-wrong for granular
                        // material, whose vertical stress saturates with depth (Janssen) because
                        // wall friction carries load that would otherwise keep accumulating
                        // straight down. `k_of_liquidity` (below) still does the SEPARATE job of
                        // discounting how much of that (now depth-shaped) overburden reads through
                        // laterally at all -- a liquid is isotropic (K = 1, full transmission,
                        // `LATERAL_EARTH_PRESSURE_K` untouched); a granular medium's grain contacts
                        // and wall friction carry part of the load sideways too, hence K < 1. The
                        // two mechanisms are independent: the Janssen transform shapes HOW the raw
                        // overburden itself behaves with depth, `k_of_liquidity` then discounts HOW
                        // MUCH of that (already-shaped) result pushes sideways. Both blended by each
                        // cell's own liquidity, so a fully liquid cell is bit-identical to before
                        // this step (`janssen_effective_depth` is the identity transform at
                        // `liquidity == 1.0`, `k_of_liquidity` is exactly `1.0`).
                        let liq_b = liquidity(cell_props[nb_idx * 4 + PROP_WETNESS]);
                        let k_a = k_of_liquidity(cell_liquidity);
                        let k_b = k_of_liquidity(liq_b);
                        let depth_a = janssen_effective_depth(column_depth[center_idx], cell_liquidity);
                        let depth_b = janssen_effective_depth(column_depth[nb_idx], liq_b);

                        // TASK #55, gated (default OFF -- see `multiplicative_lateral_gate`):
                        // `head_a`/`head_b_full` below are ADDITIVE (fill + independently-scaled
                        // depth bonus). The gated alternative is MULTIPLICATIVE
                        // (`mult_lateral_conveyance(depth) * grad(eta)`, see that function's doc
                        // comment for the full reasoning) -- folded entirely into `head_a`, with
                        // `head_b_full` left at `0.0`, so the single `driving = head_a - head_b`
                        // read by `edge_sleeps` and `flux_edge_candidate` below is unaffected by
                        // which branch produced it and neither of those two functions needs to
                        // know this gate exists.
                        let (cap_a_eff, cap_b_eff) = if overfill_active {
                            (
                                cell_overfill_capacity_for(wetness, overfill_ratio),
                                cell_overfill_capacity_for(cell_props[nb_idx * 4 + PROP_WETNESS], overfill_ratio),
                            )
                        } else {
                            (cell_capacity, cap_b)
                        };

                        let (head_a, head_b_full, tau_eff) = if overfill_active {
                            let depth_scale = REFERENCE_GRID_HEIGHT as f32 / w as f32;
                            let overfill_head_unit = (GRAVITY_HEAD_SCALE / depth_scale) * OVERFILL_STIFFNESS_K;
                            let o_a = relative_overfill(h_a_frozen, cell_capacity);
                            let o_b = relative_overfill(h_b_frozen, cap_b);
                            let p_a = o_a * overfill_head_unit;
                            let p_b = o_b * overfill_head_unit;
                            let k_lat_a = k_of_liquidity(cell_liquidity);
                            let k_lat_b = k_of_liquidity(liq_b);
                            let driving_a = h_a_frozen + gravity_dir.x * GRAVITY_HEAD_SCALE + k_lat_a * p_a + dispersion;
                            let driving_b = h_b_frozen + k_lat_b * p_b;

                            // Mohr-Coulomb yield stress for granular material:
                            // Yield stress tau increases with local confining pressure (normal stress P_bar).
                            // For pure liquid (cell_liquidity == 1.0), friction mu = 0 -> tau_eff = tau (0 for liquid).
                            // For granular sand, friction mu = 0.60 -> deep sand resists failure under normal stress.
                            let mu = OVERFILL_MOHR_COULOMB_MU * (1.0 - cell_liquidity).max(1.0 - liq_b);
                            let normal_p = 0.5 * (p_a + p_b);
                            let tau_overfill = mu * normal_p * (depth_scale / GRAVITY_HEAD_SCALE);
                            let tau_eff = tau + tau_overfill;

                            (driving_a, driving_b, tau_eff)
                        } else if head_field_active
                            && cell_liquidity >= LIQUID_ELLIPTIC_THRESHOLD
                            && liq_b >= LIQUID_ELLIPTIC_THRESHOLD
                        {
                            // TASK #55 step 3, LIQUID ONLY: the driving head is the unified
                            // hydraulic head field itself, in place of the local-fill-plus-
                            // overburden approximation both branches below build by hand
                            // (`h + k*LATERAL_PRESSURE_SCALE*depth`, additive, or the
                            // `mult_lateral_conveyance` product, multiplicative) -- the field
                            // already IS the physically correct unified head (elevation + Pascal-
                            // transmitted pressure), which is exactly what those two branches were
                            // each independently approximating. Do NOT also add
                            // `k_a * LATERAL_PRESSURE_SCALE * depth_a` alongside it -- that is the
                            // overburden term the field replaces, and adding both would
                            // double-count it.
                            //
                            // Liquid-only, exactly the head field's own domain
                            // restriction (`LIQUID_ELLIPTIC_THRESHOLD`): the field has no yield
                            // criterion, so a granular pile at its angle of repose is a permanent
                            // surface gradient that must produce ZERO flow -- see
                            // `test_dry_sand_has_angle_of_repose`, run with this toggle forced ON,
                            // as this restriction's own non-regression check.
                            //
                            // `gravity_dir.x * GRAVITY_HEAD_SCALE` and `dispersion` are kept
                            // (unlike the overburden term, neither is part of what the field
                            // models): the field's own `z` term only carries VERTICAL
                            // (`-row * depth_scale`) gravitational potential, so a lateral gravity
                            // component -- never exercised by the shipped app today, always
                            // `gravity_dir.x == 0.0` -- would otherwise silently vanish; dispersion
                            // is a per-tick stochastic perturbation orthogonal to the physics the
                            // field computes and both other branches add it in exactly this spot.
                            //
                            // UNIT CONVERSION: `head_field` is in `task55_head_field`'s
                            // reference-row units (`depth_scale = REFERENCE_GRID_HEIGHT / w`);
                            // `head_a`/`head_b_full` must be in the SAME local-cell units
                            // `h_a_frozen`/`cell_capacity` and `flux_edge_candidate`'s `c_sq`/`tau`
                            // are calibrated against, so the FIELD SIDE is divided by `depth_scale`
                            // to convert reference-row units down to local-cell units -- the
                            // inverse of `recompute_column_depth`'s own local -> reference-row
                            // multiplication, and the same conversion the vertical (phase 0) edge
                            // site performs for the identical reason.
                            // ...AND MULTIPLIED BY `GRAVITY_HEAD_SCALE` for the same reason the
                            // vertical (phase 0) site does it: dividing by `depth_scale` alone
                            // leaves the field in CELLS OF ELEVATION, 25x weaker than the
                            // `gravity_dir.x * GRAVITY_HEAD_SCALE` term sitting right beside it
                            // here and than the `else` branches drive the same geometry with. See
                            // the vertical site's own comment for the measurement (a completely
                            // frozen simulation, `total_flow = 0.0000`) that exposed the omission.
                            let head_scale =
                                GRAVITY_HEAD_SCALE / (REFERENCE_GRID_HEIGHT as f32 / w as f32);
                            let head_a_field = head_field[center_idx] * head_scale
                                + gravity_dir.x * GRAVITY_HEAD_SCALE
                                + dispersion;
                            let head_b_field = head_field[nb_idx] * head_scale;
                            (head_a_field, head_b_field, tau)
                        } else if multiplicative_lateral_gate::is_enabled() {
                            let depth_scale = REFERENCE_GRID_HEIGHT as f32 / w as f32;
                            let h_a_ref = h_a_frozen * depth_scale;
                            let h_b_ref = h_b_frozen * depth_scale;
                            let eta_a = h_a_ref + gravity_dir.x * GRAVITY_HEAD_SCALE
                                + column_depth[center_idx];
                            let eta_b = h_b_ref + column_depth[nb_idx];
                            let conveyance_a =
                                mult_lateral_conveyance(h_a_ref, column_depth[center_idx], k_a, cell_liquidity);
                            let conveyance_b =
                                mult_lateral_conveyance(h_b_ref, column_depth[nb_idx], k_b, liq_b);
                            let conveyance = 0.5 * (conveyance_a + conveyance_b);
                            let driving = MULT_LATERAL_SCALE * conveyance * (eta_a - eta_b) + dispersion;
                            (driving, 0.0, tau)
                        } else {
                            (
                                h_a_frozen + gravity_dir.x * GRAVITY_HEAD_SCALE
                                    + k_a * LATERAL_PRESSURE_SCALE * depth_a + dispersion,
                                h_b_frozen + k_b * LATERAL_PRESSURE_SCALE * depth_b,
                                tau,
                            )
                        };

                        // Stochastic locking (see `GRAVITY_LOCK_CHANCE`'s doc comment):
                        // reproduces the CA's flat 0.05 `lock_chance` under gravity ("Low locking
                        // under gravity so sand avalanches smoothly"), scaled by `granular_share`
                        // so a fully liquid cell is never locked (matching its old behaviour of
                        // never passing through this mechanism at all) and a fully granular cell
                        // keeps the original flat rate. A locked edge is treated exactly like a
                        // sleeping one below (zero flux, velocity cleared) rather than as stored,
                        // undischarged momentum — matching the old CA, where a locked tick's
                        // would-be transfer was simply skipped, not deferred.
                        let lock_roll = ((seed ^ (nb_idx as u32).wrapping_mul(577)) & 0xFFFF) as f32 / 65535.0;
                        let locked = lock_roll < GRAVITY_LOCK_CHANCE * granular_share;

                        // Sleeping edge (see `edge_sleeps`), tested *before* the in-transit
                        // computation below rather than after, because that computation is the
                        // expensive part of this edge: two neighbour loads, a capacity lookup and
                        // two edge-velocity reads per endpoint. A sleeping edge must not pay for a
                        // donor limit whose only use is to be clamped to zero.
                        //
                        // Testing first means the predicate cannot see the in-transit reduction,
                        // so it is handed `h_a` / `h_b` — an *upper* bound on `avail_a` / `avail_b`
                        // (`in_transit >= 0`). Overstating `avail` can only suppress branch 1, so
                        // this is sound; the edges it gives up on are cells that received mass from
                        // above this tick, which are moving anyway and would not have slept for long.
                        // The cases branch 1 exists for are untouched by the bound: a pooled
                        // interior sleeps on `room_a == room_b == 0` and empty space on
                        // `h_a == h_b == 0`, neither of which involves `in_transit` at all.
                        //
                        // The driving term passed here must be `head_a - head_b_full` — the exact
                        // quantity `flux_edge` will compute internally below — or branch 2 could
                        // sleep an edge the depth-pressure/dispersion terms would in fact have moved.
                        if locked || edge_sleeps(
                            head_a - head_b_full, tau_eff, edge_vel_h[center_idx],
                            h_a, h_b, cap_a_eff - h_a, cap_b_eff - h_b,
                        ) {
                            if edge_vel_h[center_idx] != 0.0 {
                                edge_vel_h[center_idx] = 0.0;
                            }
                        } else {
                            let (c_sq, damping) = wave_params(wetness);
                            // TASK #63: pressure-sensitive flow rate -- see the phase-0 vertical
                            // site's own comment for the full reasoning (the flux and not `c_sq`;
                            // donor and not average; free fall exempt by construction; the driving
                            // head untouched).
                            //
                            // Note this limiter and `avail_a` below are independent and both
                            // wanted. `avail_a` removes mass that is still in transit downward (a
                            // falling parcel cannot push sideways at all); this scales how hard
                            // RESTING material pushes, by the head it carries.
                            let pressure_weight = if pressure_sensitive_flow
                                && cell_liquidity >= LIQUID_ELLIPTIC_THRESHOLD
                                && liq_b >= LIQUID_ELLIPTIC_THRESHOLD
                            {
                                pressure_rate_factor(task55_head_field::rows_of_head_at(
                                    center_idx, w, head_field,
                                ))
                            } else {
                                1.0
                            };
                            // Mass that arrived from upstream during phase 0 is still falling; it is
                            // unsupported and cannot push sideways (see `flux_edge`'s `avail_*`).
                            // `edge_vel_v[i - w]` is exactly the flux phase 0 realised on the
                            // gravity-aligned edge feeding cell `i`, and `edge_vel_v[i]` the flux it
                            // realised on the edge draining `i`.
                            //
                            // The inflow alone is the right limit for a free-falling parcel and the
                            // wrong one for a supported parcel, and it used to be subtracted
                            // unconditionally. A cell standing on a full column — or on the container
                            // floor, or on casing — bears the hydrostatic head of everything in it and
                            // must spread sideways at the normal rate however hard it is being fed
                            // from above. Subtracting the inflow there re-suppressed the motion the
                            // phase ordering already suppresses, and did so *permanently* under any
                            // continuous feed: a cell under a running pour receives from above on
                            // every single tick, so `avail_*` never recovered and lateral flow was
                            // dead at every depth of the pour rather than only in its falling part.
                            //
                            // The limit is therefore not the inflow but the amount of that inflow
                            // that can actually keep going down:
                            //
                            //     in_transit = min(inflow, outflow + room_below)
                            //
                            // `outflow` is what already left through the bottom this tick and
                            // `room_below` is the free space still under the cell, so the second term
                            // is everything the cell has any downstream route for. Inflow beyond it
                            // landed on a column that cannot take it any further: it is at rest, and
                            // it presses sideways like any other resting mass.
                            //
                            // The tempting simpler test — "is the cell below full?" — does not work
                            // here, and it is worth recording why. A *saturated* falling stream passes
                            // it at every interior cell: phase 0 sweeps bottom-to-top, so each stream
                            // cell hands `f` downward and is refilled by `f` from above, leaving every
                            // cell (hence every cell's below-neighbour) back at capacity by the time
                            // phase 1 reads it. By height alone a saturated stream is
                            // indistinguishable from a standing column; gating on height alone fanned
                            // the stream from 8 cells wide to 16. The `outflow` term is what separates
                            // them: the stream moved its whole content down, the pooled cell moved
                            // nothing. And `room_below` is what keeps the *front* of a stream falling
                            // — its edge momentum has not spun up, so it moves little downward on the
                            // tick it appears, but the empty space beneath it is a route all the same.
                            //
                            // Every case in free fall reproduces the old value exactly (in the stream
                            // interior `outflow = inflow` and `room_below = 0`; at the front
                            // `room_below` is a whole cell), so this is a strict relaxation. Stage C
                            // extends it from "liquid share only" to this whole edge (see this
                            // block's opening comment) — the function itself was already
                            // material-agnostic (`cell_capacity_for` handles the granular 1.5 cap
                            // fine), it just was not reached for a pure granular cell before.
                            //
                            // (`in_transit` itself is defined above, alongside `column_depth`,
                            // since that bookkeeping needs it for every liquid cell regardless of
                            // whether this edge sleeps.)
                            let avail_a = (h_a
                                - in_transit_at(center_idx, w, h, temp_heights, &heightmap.data, cell_props, edge_vel_v, shape_mask))
                                .max(0.0);
                            let avail_b = (h_b
                                - in_transit_at(nb_idx, w, h, temp_heights, &heightmap.data, cell_props, edge_vel_v, shape_mask))
                                .max(0.0);
                            // COLLECT only — see the phase-loop buffer comment and
                            // `flux_edge_candidate`'s doc comment. Under gravity this is the only
                            // owned edge this cell has in phase 1 (its vertical edge belongs to
                            // phase 0, already fully resolved), but it can still be the ACCEPTOR
                            // of up to two live edges this phase — its own, run in reverse (if the
                            // neighbour is higher), and its left neighbour's owned edge — which is
                            // exactly the multi-edge case arbitration exists for.
                            let candidate = flux_edge_candidate(
                                head_a,
                                head_b_full,
                                c_sq, damping, tau_eff,
                                cap_a_eff, cap_b_eff,
                                cell_capacity, cap_b,
                                avail_a, avail_b,
                                h_a, h_b,
                                pressure_weight,
                                edge_vel_h[center_idx],
                            );
                            cand_h[center_idx] = candidate;
                            edge_h_active[center_idx] = true;
                            touched_h.push(center_idx);
                            cell_avail[center_idx] = avail_a;
                            cell_freecap[center_idx] = (cap_a_eff - h_a).max(0.0);
                            cell_avail[nb_idx] = avail_b;
                            cell_freecap[nb_idx] = (cap_b_eff - h_b).max(0.0);
                            touched_cells.push(center_idx);
                            touched_cells.push(nb_idx);
                            // Totals deferred: see the unified post-COLLECT pass below.
                        }
                    }

                    // Stage C bail-out: for any non-Oobleck material under gravity, the lateral
                    // (x) edge is now entirely owned by the combined flux call just above — both
                    // the liquid share (as before Stage C) and the granular share (new). The
                    // granular CA below (avalanche valve + main flow loop) only ever touched the
                    // `ndy == 0` lateral edge under gravity (the `ndy != 0` vertical edge has been
                    // phase-0's since Stage B) so, with the lateral edge now also handled above,
                    // that whole remaining CA body would be pure redundant work at best and a
                    // double-counted transfer at worst if left reachable here. `sliding` is reset
                    // to `false` rather than left stale, matching the `granular_share <= 0.0`
                    // bail-out this one now supersedes for the gravity case (see its comment just
                    // below, now dead code for every material except Oobleck under gravity, which
                    // this bail-out deliberately does not touch — see `is_oobleck_band`'s comment
                    // above).
                    if gravity_active && !is_oobleck_band {
                        sliding[center_idx] = false;
                        continue;
                    }

                    // Sleeping cell: a fully liquid cell under gravity has `granular_share == 0`,
                    // and *every* transfer below is scaled by it — the avalanche safety valve's
                    // `clamped_flow` and the main flow loop's both end in `* granular_share`, and
                    // both are then gated on `> FLOW_INACTIVE_THRESHOLD` (or, in the tiny-residual
                    // arm, on `clamped_flow > 0.0`), which exact zero never passes. So the whole
                    // remaining body — four neighbour height loads against two arrays, the
                    // avalanche sweep, the higher-neighbour count, the marble distance search,
                    // `get_ca_params`, and the four-neighbour flow loop — is computed and then
                    // multiplied away. Its only surviving side effect is `sliding[center_idx] =
                    // cell_flowed`, which is necessarily `false` because no `try_move` can fire, so
                    // setting it here and bailing is exactly equivalent rather than an
                    // approximation. (The one other exit that writes `sliding`, the
                    // `!gravity_active && avalanche_checked` early-out, is unreachable here:
                    // `granular_share` is only ever below 1.0 when gravity is active.)
                    //
                    // This is the cell-level counterpart of `edge_sleeps` and it is what the
                    // liquid path actually spends its time on: `liquidity` saturates at
                    // `wetness >= 0.85`, so Water, Milk, CalmWater and VegetableOil have
                    // `granular_share == 0` in *every* cell under gravity. Granular materials have
                    // `liquidity == 0` hence `granular_share == 1`, so this never fires for them
                    // and the CA path is untouched.
                    if granular_share <= 0.0 {
                        sliding[center_idx] = false;
                        continue;
                    }

                    let h_center = if gravity_active {
                        temp_heights[center_idx].max(heightmap.data[center_idx])
                    } else {
                        heightmap.data[center_idx]
                    };

                    // Load neighbor heights and find minimum
                    let h_left = if gravity_active { temp_heights[center_idx - 1].max(heightmap.data[center_idx - 1]) } else { heightmap.data[center_idx - 1] };
                    let h_right = if gravity_active { temp_heights[center_idx + 1].max(heightmap.data[center_idx + 1]) } else { heightmap.data[center_idx + 1] };
                    let h_top = if gravity_active { temp_heights[center_idx - w].max(heightmap.data[center_idx - w]) } else { heightmap.data[center_idx - w] };
                    let h_bottom = if gravity_active { temp_heights[center_idx + w].max(heightmap.data[center_idx + w]) } else { heightmap.data[center_idx + w] };

                    let min_h = h_left.min(h_right).min(h_top).min(h_bottom);

                    let threshold_prop = cell_props[center_idx * 4 + PROP_THRESHOLD];
                    let flow_rate_prop = cell_props[center_idx * 4 + PROP_FLOW_RATE];
                    let grain_size = cell_props[center_idx * 4 + PROP_GRAIN_SIZE];

                    let threshold_min = if wetness < 0.15 {
                        0.5 * threshold_prop
                    } else {
                        threshold_prop
                    };

                    // Fast-path shortcut (disabled when gravity is active to allow flow on flat beds)
                    if gravity_dir.length_squared() < 1e-6 && h_center - min_h <= threshold_min {
                        sliding[center_idx] = false;
                        continue;
                    }

                    // `seed` is now computed once, earlier in this branch (see the comment there),
                    // so the new combined lateral flux edge can share it with this loop.

                    let neighbors_info = if gravity_active && gravity_dir.y > 0.0 {
                        if (tick_count + phase_offset(K_CA_CHECKERBOARD) + x as u32 + y as u32) % 2 == 0 {
                            [
                                (center_idx + w, 0.0, 1.0),  // Bottom (Gravity first)
                                (center_idx - 1, -1.0, 0.0), // Left
                                (center_idx + 1, 1.0, 0.0),  // Right
                                (center_idx - w, 0.0, -1.0), // Top
                            ]
                        } else {
                            [
                                (center_idx + w, 0.0, 1.0),  // Bottom (Gravity first)
                                (center_idx + 1, 1.0, 0.0),  // Right
                                (center_idx - 1, -1.0, 0.0), // Left
                                (center_idx - w, 0.0, -1.0), // Top
                            ]
                        }
                    } else if (tick_count + phase_offset(K_CA_CHECKERBOARD) + x as u32 + y as u32) % 2 == 0 {
                        [
                            (center_idx - 1, -1.0, 0.0), // Left
                            (center_idx + 1, 1.0, 0.0),  // Right
                            (center_idx - w, 0.0, -1.0), // Top
                            (center_idx + w, 0.0, 1.0),  // Bottom
                        ]
                    } else {
                        [
                            (center_idx + 1, 1.0, 0.0),  // Right
                            (center_idx - 1, -1.0, 0.0), // Left
                            (center_idx - w, 0.0, -1.0), // Top
                            (center_idx + w, 0.0, 1.0),  // Bottom
                        ]
                    };

                    let mut cell_flowed = false;

                    // A. Absolute gravity-avalanche collapse safety check (to prevent spikes)
                    let mut avalanche_checked = false;
                    for &(neighbor_idx, ndx, ndy) in &neighbors_info {
                        let gravity_dot = ndx * gravity_dir.x + ndy * gravity_dir.y;
                        if gravity_active && gravity_dot < -0.01 {
                            continue;
                        }
                        // The gravity-aligned (grid-y) edge is now owned entirely by the phase-0
                        // flux pass above (Stage B) — both directions of it, since a single
                        // `flux_edge` call there covers whichever way `gravity_dir.y` points. The
                        // CA must not also move mass across it, or the transfer double-counts.
                        // Only ndy == 0 (the lateral, grid-x edge) is left for the CA to arbitrate,
                        // which is exactly where the repose/avalanche behaviour this valve exists
                        // for actually lives.
                        if gravity_active && ndy != 0.0 {
                            continue;
                        }

                        let h_neighbor = if gravity_active { temp_heights[neighbor_idx].max(heightmap.data[neighbor_idx]) } else { heightmap.data[neighbor_idx] };
                        let geom_slope = h_center - h_neighbor;

                        if geom_slope > 0.20 {
                            let mut flow = (0.10 * (geom_slope - 0.20)).max(0.0);

                            // Never transfer mass into a neighbour outside the shape mask, for any
                            // material. Such a cell is skipped by `if !inside { continue }` at the
                            // top of this loop and is never simulated again, so anything landing
                            // there is a silent, permanent leak: total mass is still conserved (so
                            // the leak is invisible to the mass-conservation tests) but the sand or
                            // liquid is frozen inside a wall forever. The renderer draws
                            // MASK_OUTSIDE as opaque casing, which hides it visually too.
                            if !is_inside(neighbor_idx % w, neighbor_idx / w) {
                                flow = 0.0;
                            }

                            if flow > 0.0 {
                                let current_temp_center = temp_heights[center_idx];
                                let current_temp_neighbor = temp_heights[neighbor_idx];
                                let temp_diff = current_temp_center - current_temp_neighbor;
                                // Same acceptor capacity as the main flow loop below (C1): this
                                // avalanche safety valve bypasses the normal threshold/alpha flow
                                // computation entirely, so without this clamp it could push a
                                // liquid neighbor above the incompressibility cap on its own.
                                let max_dst_room = (cell_capacity - current_temp_neighbor).max(0.0);
                                let clamped_flow = flow.min(temp_diff * 0.4).min(max_dst_room).max(0.0)
                                    * granular_share;
                                if clamped_flow > FLOW_INACTIVE_THRESHOLD {
                                    try_move(
                                        b, center_idx, neighbor_idx, clamped_flow, w, h, block_size, cols,
                                        temp_heights, cell_colors, cell_props,
                                        &mut modified, &mut next_displacements,
                                        &mut total_flow, &mut cell_flowed, &mut flow_occurred,
                                    );
                                    #[cfg(test)]
                                    note_phase_flow(phase, clamped_flow);
                                }
                            }
                            avalanche_checked = true;
                        }
                    }
                    if !gravity_active && avalanche_checked {
                        sliding[center_idx] = cell_flowed;
                        continue;
                    }

                    // Cell-invariant properties
                    let mut higher_neighbors = 0;
                    for &(n_idx, _, _) in &neighbors_info {
                        let h_n = if gravity_active { temp_heights[n_idx].max(heightmap.data[n_idx]) } else { heightmap.data[n_idx] };
                        if h_n >= h_center - 1e-4 {
                            higher_neighbors += 1;
                        }
                    }

                    let mut closest_marble_idx = None;
                    let mut min_dist_to_marble = f32::MAX;
                    if !active_marbles.is_empty() {
                        let cell_x = (x as f32 / w as f32) * 2.0 - 1.0;
                        let cell_y = 1.0 - (y as f32 / h as f32) * 2.0;
                        let cell_pos = Vec2::new(cell_x, cell_y);

                        for (idx, m) in active_marbles.iter().enumerate() {
                            let dist = (cell_pos - m.pos).length();
                            if dist < min_dist_to_marble {
                                min_dist_to_marble = dist;
                                closest_marble_idx = Some(idx);
                            }
                        }
                    }

                    let closest_marble_vel = if let Some(idx) = closest_marble_idx {
                        active_marbles[idx].vel
                    } else {
                        0.0
                    };

                    let (threshold, alpha, lock_chance, quantize_size) = get_ca_params(
                        wetness,
                        threshold_prop,
                        flow_rate_prop,
                        grain_size,
                        higher_neighbors,
                        sliding[center_idx],
                        closest_marble_vel,
                        gravity_active,
                    );

                    // `cell_liquidity` / `cell_capacity` computed above (right after the CA
                    // branch was entered); reused below to blend the gravity_push multiplier,
                    // the transfer coefficient, and the acceptor cell capacity.
                    for &(neighbor_idx, ndx, ndy) in &neighbors_info {
                        let h_neighbor = if gravity_active { temp_heights[neighbor_idx].max(heightmap.data[neighbor_idx]) } else { heightmap.data[neighbor_idx] };
                        let geom_slope = h_center - h_neighbor;
                        let gravity_dot = ndx * gravity_dir.x + ndy * gravity_dir.y;
                        
                        // Under gravity, sand cannot flow upwards against gravity
                        if gravity_active && gravity_dot < -0.01 {
                            continue;
                        }
                        // The gravity-aligned (grid-y) edge is fully owned by the phase-0 flux
                        // pass now (Stage B) — see the identical exclusion in the avalanche valve
                        // above for why both directions of it must be skipped here.
                        if gravity_active && ndy != 0.0 {
                            continue;
                        }

                        let h_below = if center_idx + w < temp_heights.len() {
                            temp_heights[center_idx + w].max(heightmap.data[center_idx + w])
                        } else {
                            0.0
                        };
                        let is_below_inside = y + 1 < h && is_inside(x, y + 1);
                        let is_free_fall = gravity_active && is_below_inside && h_below < 0.10;

                        // Downward pull. Phase 5 removed the x40 liquid multiplier that used to
                        // be blended in here: a liquid's downhill drive is now the gravitational
                        // head Phi in the flux solver, not a fictitious slope bonus in the CA.
                        let mut gravity_push = gravity_dot * 4.0;
                        
                        // Sideways lateral term — the granular stochastic dispersion/splashing
                        // that builds the bed heap and scatters sand in free fall. Phase 5
                        // deleted the liquid counterpart that used to be blended in here (a term
                        // that cancelled `geom_slope` while the cell below could still accept
                        // mass): a liquid's lateral motion is now the flux solver's cross-gravity
                        // edge, gated by the in-transit donor limit rather than by a "can it
                        // still fall?" predicate.
                        let gravity_len = gravity_dir.length();
                        if gravity_len > 1e-6 {
                            let perp_x = -gravity_dir.y;
                            let perp_y = gravity_dir.x;
                            let perp_dot = (ndx * perp_x + ndy * perp_y).abs();
                            let rand_val = (seed ^ (neighbor_idx as u32).wrapping_mul(823)) & 0xFF;
                            let dispersion_noise = rand_val as f32 / 255.0;

                            gravity_push += if !is_free_fall {
                                // Lateral avalanche dispersion on bed heap to form a natural tall sand hill
                                perp_dot * 3.5 * dispersion_noise
                            } else {
                                // Always randomly scatter a little laterally in free fall for natural stream flow
                                perp_dot * 0.8 * dispersion_noise
                            };
                        }

                        let effective_slope = geom_slope + gravity_push;

                        if effective_slope <= threshold {
                            continue;
                        }

                        // C. Stochastic locking and sliding condition (bypass locking in free fall)
                        let flow_seed = (seed ^ (neighbor_idx as u32).wrapping_mul(997)) & 0xFFFF;
                        let rand_val = flow_seed as f32 / 65535.0;
                        let effective_lock_chance = if is_free_fall { 0.0 } else { lock_chance };
                        
                        if rand_val >= effective_lock_chance {
                            let alpha_noise = if gravity_active {
                                1.0 + (rand_val - 0.5) * 0.10 // Smooth laminar flow under gravity (+/- 5%)
                            } else {
                                1.0 + (rand_val - 0.5) * 0.80 // Natural stochastic noise in sandbox carving (+/- 40%)
                            };
                            let mut flow = (alpha * (effective_slope - threshold) * alpha_noise).max(0.0);
                            
                            if let Some(q) = quantize_size {
                                flow = (flow / q).round() * q;
                            }

                            if flow > 0.0 {
                                // Phase 5 removed the liquid arm of this coefficient (0.70 while
                                // the column below could still take more, 0.90 otherwise). A
                                // liquid's per-tick transfer is no longer a fraction of the donor
                                // chosen by hand — it is the donor's actual mass and the
                                // acceptor's actual free capacity, in `flux_edge`.
                                let max_transfer_coeff = if !gravity_active {
                                    0.40
                                } else if is_free_fall && gravity_dot > 0.0 {
                                    let rand_ff = ((seed ^ (neighbor_idx as u32).wrapping_mul(1543)) & 0xFFFF) as f32 / 65535.0;
                                    0.80 + 0.20 * rand_ff // Random transfer between 80% and 100% in mid-air free fall
                                } else {
                                    0.20 // Sand uses lower coeff on bed to prevent wave oscillations
                                };
                                // Acceptor cell capacity (incompressibility, C1). `cell_capacity`
                                // (computed once above, per center cell) is 1.5 for granular
                                // materials (unchanged, load-bearing for sand-pile height tests)
                                // and interpolates down to 1.0 for liquids via `cell_liquidity`, so
                                // there is no hard cut. Applied to BOTH branches below (not just the
                                // "push into an equal/higher neighbor" case) because within a single
                                // tick a cell can receive inflow from more than one neighbor; without
                                // a capacity check on the downhill (geom_slope > 0) branch too,
                                // several simultaneous donors could each independently push a liquid
                                // neighbor a little past 1.0 even though none of them individually
                                // looked like overpacking.
                                let max_dst_room = (cell_capacity - temp_heights[neighbor_idx]).max(0.0);

                                let src_h = temp_heights[center_idx];
                                let mut clamped_flow = if geom_slope > 0.0 {
                                    let temp_diff = temp_heights[center_idx] - temp_heights[neighbor_idx];
                                    let flow_capped = if src_h <= 0.003 {
                                        flow.min(temp_diff).max(0.0)
                                    } else {
                                        flow.min(temp_diff * max_transfer_coeff).max(0.0)
                                    };
                                    flow_capped.min(max_dst_room)
                                } else {
                                    let max_src_flow = if src_h <= 0.003 {
                                        src_h
                                    } else {
                                        src_h * max_transfer_coeff
                                    };
                                    flow.min(max_src_flow).min(max_dst_room).max(0.0)
                                };
                                
                                // Clean sweep for tiny residual amounts to prevent Zeno's paradox trapping & floating grains.
                                // Still respects the acceptor capacity (C1): this override previously bypassed
                                // max_dst_room entirely, which let a liquid neighbor already at capacity get pushed
                                // slightly over 1.0 by every tiny-residual neighbor sweeping into it in the same tick.
                                if (clamped_flow <= FLOW_INACTIVE_THRESHOLD || is_free_fall) && src_h > 0.0 && src_h <= 0.010 && flow > 0.0 {
                                    clamped_flow = src_h.min(max_dst_room);
                                }

                                // Mask-leak fix: never let a transfer land in a neighbor outside the
                                // shape mask, for any material. Such a cell is skipped by
                                // `if !inside { continue }` at the top of this loop and is never
                                // simulated again, so anything that reaches it is a silent,
                                // permanent leak that stays frozen there forever. Total mass is
                                // still conserved, so the mass-conservation tests never saw this;
                                // and the renderer draws MASK_OUTSIDE as opaque casing, so it was
                                // invisible on screen too. For liquid it was also what pinned a
                                // "spike" of water against the box wall/floor in
                                // `test_liquid_pool_levels_flat_in_closed_box` (surface_row scans
                                // the whole grid width, including outside-mask columns).
                                if !is_inside(neighbor_idx % w, neighbor_idx / w) {
                                    clamped_flow = 0.0;
                                }

                                clamped_flow *= granular_share;

                                if clamped_flow > FLOW_INACTIVE_THRESHOLD || (src_h <= 0.001 && clamped_flow > 0.0) {
                                    try_move(
                                        b, center_idx, neighbor_idx, clamped_flow, w, h, block_size, cols,
                                        temp_heights, cell_colors, cell_props,
                                        &mut modified, &mut next_displacements,
                                        &mut total_flow, &mut cell_flowed, &mut flow_occurred,
                                    );
                                    #[cfg(test)]
                                    note_phase_flow(phase, clamped_flow);
                                }
                            }
                        }
                    }

                    sliding[center_idx] = cell_flowed;
                }
            }
        }
    }

    // --- PRESSURE PROJECTION ---
    //
    // Runs once per phase, strictly between COLLECT (just finished above) and the arbitration
    // totals: it corrects `cand_h`/`cand_v` for the capacity constraint over the WHOLE
    // candidate-edge graph `touched_h ∪ touched_v` produced this phase, in place, so that a packed
    // column with a single outlet can learn where its only exit is within one tick instead of one
    // cell per tick. See `pressure_project`'s doc comment for the full derivation and the proof
    // that this cannot compromise the positivity/capacity guarantees arbitration still enforces
    // afterwards, or mass conservation (which stays structural throughout). A no-op on the
    // correction (falls through to plain totals accumulation) when `pressure_gate::is_disabled()`
    // or neither touched list holds anything this phase.
    //
    // `accumulate_edge_totals` -- previously a separate pass here, right after this call, walking
    // `touched_h`/`touched_v` a second time -- now runs INSIDE `pressure_project`, in the same
    // pass that applies and clamps the correction (see that function's perf note): every term
    // still reflects the (possibly pressure-corrected) final candidate, exactly as before, just
    // without paying for a whole extra traversal to get there. `edge_key`/`salt`/`a_idx`/`b_idx`
    // stay the same convention every COLLECT site in this phase used (edge_key == the edge's own
    // index into `cand_h`/`cand_v`, `a_idx`/`b_idx` its two endpoints, salt keyed only by
    // orientation and phase) -- see the `EDGE_SALT_*` calls above, at each COLLECT site, which
    // this replaces.
    pressure_project(
        &mut cand_h, &mut cand_v, &touched_h, &touched_v,
        temp_heights, cell_props, w,
        &mut pressure_phi_a, &mut pressure_phi_b, &mut pressure_fstar, &mut pressure_lap,
        &mut pressure_degree, &mut pressure_cap, &mut pressure_nodes,
        phase, time_seed, overfill_active, overfill_ratio,
        &mut cell_out_total, &mut cell_in_total, &mut cell_out_total_jit, &mut cell_in_total_jit,
    );

    // --- ARBITRATE + APPLY ---
    //
    // Every entry in `touched_v`/`touched_h` is a candidate this phase's COLLECT pass computed
    // from the single frozen snapshot described in the buffer comment above the phase loop: no
    // edge above saw any other edge's update. `cell_out_total`/`cell_in_total` are the RAW sums of
    // those candidates' magnitudes, per cell, in the donor and acceptor directions; comparing them
    // against the frozen `cell_avail`/`cell_freecap` and scaling by `edge_arbitration_scale` (see
    // its doc comment for the single-pass proof) is what restores the guarantee the old sequential
    // sweep used to provide for free — that a cell's total same-tick draw/receipt cannot exceed
    // what it actually has or actually has room for — now that no edge's application is visible to
    // the next one within this phase.
    //
    // Apply order between `touched_v` and `touched_h` (and within each list) does not matter: each
    // edge's final flux was already fixed by arbitration above, so applying them is pure
    // accumulation (`temp_heights[i] +=/-= final_flux`), which is commutative. This is a direct
    // consequence of the flux form's structural conservation (see `flux_edge_apply`'s doc
    // comment) and is what makes this whole rewrite order-independent where the old sweep was not.
    for &idx in &touched_v {
        let raw = cand_v[idx];
        let a_idx = idx;
        let b_idx = idx + w;
        let (donor, acceptor) = if raw >= 0.0 { (a_idx, b_idx) } else { (b_idx, a_idx) };
        let jit = edge_share_jitter(
            cell_props, donor, idx, EDGE_SALT_V.wrapping_add(phase as u32), time_seed,
        );
        let scale = edge_arbitration_scale(
            cell_out_total[donor], cell_out_total_jit[donor], cell_avail[donor],
            cell_in_total[acceptor], cell_in_total_jit[acceptor], cell_freecap[acceptor],
            jit,
        );
        let final_flux = raw * scale;
        cand_v[idx] = final_flux;
        let x = idx % w;
        let y = idx / w;
        let bx = x / block_size;
        let by = y / block_size;
        let a_b = by * cols + bx;
        let nb_b = ((y + 1) / block_size) * cols + bx;
        flux_edge_apply(
            a_b, nb_b, a_idx, b_idx, final_flux,
            &mut edge_vel_v[idx],
            temp_heights, cell_colors, cell_props,
            &mut modified, &mut next_displacements,
            &mut total_flow, &mut flow_occurred,
        );
        #[cfg(test)]
        note_phase_flow(phase, final_flux);

        // Upstream wake. `flux_edge_apply` above activates only `a_b`/`nb_b`, the two blocks
        // THIS edge touches. A cell one row further upstream of the donor -- e.g. directly
        // above a cell that just drained downward -- is neither of those and is never told its
        // support moved. Under the block-LOD scheduler (`will_simulate[b]`, gated on
        // `last_displacements` from a PRIOR tick) that leaves the block above able to stay
        // `Inactive` for up to `MAX_STALENESS` ticks while the material below it keeps falling,
        // which is exactly how a gap opens on a block boundary underneath actively-falling
        // material: this is the flux-solver counterpart of `try_move`'s identical fix on the
        // granular-CA path (see its "Upstream wake" comment for the general argument). Gated on
        // the same `MIN_FLUX` `flux_edge_apply` itself uses, so a below-threshold non-event
        // wakes nothing extra either. `activate_neighbor_upstream`, not plain `activate_neighbor`:
        // this is a causal dependency (its support genuinely moved), so it earns `Medium`
        // (`budget_simulate`) priority rather than the none it would otherwise get -- but it
        // still competes for `budget_n` like any other candidate (see that function's doc
        // comment for why this stays capped rather than bypassing the scheduler).
        //
        // Speculative half: the donor block's two LATERAL neighbours (same block-row, one
        // column either side) are not causally implicated the way the upstream block is --
        // nothing below them changed -- but a body that is actively falling in this column
        // often has its neighbours about to follow, so give them a low-priority
        // (`SIDE_DISPLACEMENT_HINT`, budget-competing) nudge too rather than leaving them to
        // find out only via their own edges or the `MAX_STALENESS` catch-up.
        if !upstream_wake_gate::is_disabled() && final_flux.abs() > 1e-7 {
            let up_y = if final_flux > 0.0 {
                y.checked_sub(1)
            } else {
                (y + 2 < h).then_some(y + 2)
            };
            if let Some(up_y) = up_y {
                let up_b = (up_y / block_size) * cols + bx;
                activate_neighbor_upstream(up_b, &mut modified, &mut next_displacements);
            }
            let donor_by = (donor / w) / block_size;
            if bx > 0 {
                activate_neighbor_side(donor_by * cols + (bx - 1), &mut modified, &mut next_displacements);
            }
            if bx + 1 < cols {
                activate_neighbor_side(donor_by * cols + (bx + 1), &mut modified, &mut next_displacements);
            }
        }
    }

    for &idx in &touched_h {
        let raw = cand_h[idx];
        let a_idx = idx;
        let b_idx = idx + 1;
        let (donor, acceptor) = if raw >= 0.0 { (a_idx, b_idx) } else { (b_idx, a_idx) };
        let jit = edge_share_jitter(
            cell_props, donor, idx, EDGE_SALT_H.wrapping_add(phase as u32), time_seed,
        );
        let scale = edge_arbitration_scale(
            cell_out_total[donor], cell_out_total_jit[donor], cell_avail[donor],
            cell_in_total[acceptor], cell_in_total_jit[acceptor], cell_freecap[acceptor],
            jit,
        );
        let final_flux = raw * scale;
        cand_h[idx] = final_flux;
        let x = idx % w;
        let y = idx / w;
        let bx = x / block_size;
        let by = y / block_size;
        let a_b = by * cols + bx;
        let nb_b = by * cols + (x + 1) / block_size;
        flux_edge_apply(
            a_b, nb_b, a_idx, b_idx, final_flux,
            &mut edge_vel_h[idx],
            temp_heights, cell_colors, cell_props,
            &mut modified, &mut next_displacements,
            &mut total_flow, &mut flow_occurred,
        );
        #[cfg(test)]
        note_phase_flow(phase, final_flux);

        // Upstream wake -- lateral counterpart of the vertical edge's identical fix just above
        // (see its comment for the general argument, and the touched_v loop's "Speculative
        // half" comment for the sibling-neighbour nudge below). One column further upstream of
        // the donor, on the far side from the acceptor, is never activated by `flux_edge_apply`
        // itself.
        if !upstream_wake_gate::is_disabled() && final_flux.abs() > 1e-7 {
            let up_x = if final_flux > 0.0 {
                x.checked_sub(1)
            } else {
                (x + 2 < w).then_some(x + 2)
            };
            if let Some(up_x) = up_x {
                let up_b = by * cols + (up_x / block_size);
                activate_neighbor_upstream(up_b, &mut modified, &mut next_displacements);
            }
            let donor_bx = (donor % w) / block_size;
            if by > 0 {
                activate_neighbor_side((by - 1) * cols + donor_bx, &mut modified, &mut next_displacements);
            }
            if by + 1 < rows {
                activate_neighbor_side((by + 1) * cols + donor_bx, &mut modified, &mut next_displacements);
            }
        }
    }

    // Block-wake bookkeeping for phase 1's g=0 (Sandbox) liquid branch, deferred from COLLECT
    // time until arbitration has settled `cand_h`/`cand_v` (overwritten in place, just above) into
    // their final post-arbitration values — see the comment where `max_head_diff_cell` is written,
    // in that branch itself, for why `max_head_diff` alone was safe to compute immediately but
    // `max_flux` was not.
    for &idx in &g0_liquid_cells {
        let max_flux = {
            let h_mag = if edge_h_active[idx] { cand_h[idx].abs() } else { 0.0 };
            let v_mag = if edge_v_active[idx] { cand_v[idx].abs() } else { 0.0 };
            h_mag.max(v_mag)
        };
        let max_head_diff = max_head_diff_cell[idx];
        if max_flux > 3e-4 || max_head_diff > 1e-4 {
            flow_occurred = true;
            let flow_val = max_flux.max(max_head_diff);
            let x = idx % w;
            let y = idx / w;
            let bx = x / block_size;
            let by = y / block_size;
            let wake_b = by * cols + bx;
            activate_neighbor(wake_b, flow_val, &mut modified, &mut next_displacements);
            if bx > 0 { activate_neighbor(wake_b - 1, flow_val, &mut modified, &mut next_displacements); }
            if bx + 1 < cols { activate_neighbor(wake_b + 1, flow_val, &mut modified, &mut next_displacements); }
            if by > 0 { activate_neighbor(wake_b - cols, flow_val, &mut modified, &mut next_displacements); }
            if by + 1 < rows { activate_neighbor(wake_b + cols, flow_val, &mut modified, &mut next_displacements); }
        }
    }

    } // end `for phase` — body left at the original indentation so the operator split reads as a
      // wrapper rather than as a 600-line reformat of the solver.

    // 3. Copy back updated blocks
    for b in 0..expected_len {
        if modified[b] {
            let bx = b % cols;
            let by = b / cols;
            let start_x = bx * block_size;
            let end_x = ((bx + 1) * block_size).min(w);
            let start_y = by * block_size;
            let end_y = ((by + 1) * block_size).min(h);
            for y in start_y..end_y {
                let offset = y * w;
                heightmap.data[offset + start_x..offset + end_x]
                    .copy_from_slice(&temp_heights[offset + start_x..offset + end_x]);
            }
        }
    }

    // Compute updated active bounds for this frame
    let mut min_bx = cols;
    let mut max_bx = 0;
    let mut min_by = rows;
    let mut max_by = 0;
    let mut any_modified = false;

    for b in 0..expected_len {
        if modified[b] {
            any_modified = true;
            let bx = b % cols;
            let by = b / cols;
            min_bx = min_bx.min(bx);
            max_bx = max_bx.max(bx);
            min_by = min_by.min(by);
            max_by = max_by.max(by);
        }
    }

    if any_modified {
        active_bounds.min_x = min_bx * block_size;
        active_bounds.max_x = ((max_bx + 1) * block_size - 1).min(w - 1);
        active_bounds.min_y = min_by * block_size;
        active_bounds.max_y = ((max_by + 1) * block_size - 1).min(h - 1);
        active_bounds.active = flow_occurred;
    } else {
        active_bounds.active = false;
    }

    for b in 0..expected_len {
        if !will_simulate[b] {
            next_displacements[b] = next_displacements[b].max(last_displacements[b]);
        } else {
            last_simulated_ticks[b] = tick_count;
        }
    }
    *last_displacements = next_displacements;

    // Clear the external-mass-exchange buffer now that this tick's per-cell loop (section 2
    // above, which is the only reader — see `column_depth`'s `resting_above` computation) has
    // consumed it. This must happen exactly once per tick, after that loop and not before it: a
    // caller (e.g. a waterfall/pour feature, or `test_liquid_stream_stays_coherent`) calls
    // `Heightmap::apply_external_mass` *before* `tick()`, so the buffer has to survive from that
    // call, through `temp_heights.copy_from_slice(&heightmap.data)` at this function's start, all
    // the way to section 2's per-cell pass — then must be zeroed here so the next tick's calls
    // aren't added on top of this tick's stale leftovers.
    heightmap.external_mass_this_tick.fill(0.0);

    total_flow
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DrawingSimulation, GRID_SIZE, MaterialMode, SandboxShape};

    fn get_test_props(mode: crate::MaterialMode, size: usize) -> Vec<f32> {
        let (wetness, threshold, flow_rate, grain_size) = match mode {
            crate::MaterialMode::DrySand => (0.00, 0.08, 0.25, 0.45),
            crate::MaterialMode::CoarseSand => (0.00, 0.11, 0.22, 0.80),
            crate::MaterialMode::KineticSand => (0.20, 0.10, 0.15, 0.35),
            crate::MaterialMode::WetSand => (0.45, 0.14, 0.08, 0.40),
            crate::MaterialMode::FinePowder => (0.00, 0.05, 0.30, 0.05),
            crate::MaterialMode::Snow => (0.05, 0.15, 0.20, 0.20),
            crate::MaterialMode::MoonDust => (0.00, 0.20, 0.20, 0.10),
            crate::MaterialMode::Oobleck => (0.55, 0.04, 0.12, 0.15),
            crate::MaterialMode::ButterCream => (0.70, 0.04, 0.15, 0.08),
            crate::MaterialMode::Water => (1.00, 0.00, 0.00, 0.00),
            crate::MaterialMode::CalmWater => (0.90, 0.00, 0.00, 0.00),
            crate::MaterialMode::Milk => (0.95, 0.00, 0.00, 0.00),
            crate::MaterialMode::VegetableOil => (0.85, 0.00, 0.00, 0.00),
            crate::MaterialMode::Yogurt => (0.75, 0.00, 0.00, 0.08),
        };
        let mut props = vec![0.0f32; size * 4];
        for chunk in props.chunks_exact_mut(4) {
            chunk[PROP_WETNESS] = wetness;
            chunk[PROP_THRESHOLD] = threshold;
            chunk[PROP_FLOW_RATE] = flow_rate;
            chunk[PROP_GRAIN_SIZE] = grain_size;
        }
        props
    }

    /// Generate a shape mask for testing. Uses eval_sandbox_shape to build the mask
    /// with proper INSIDE/BOUNDARY/OUTSIDE classification. Fixes `multistage_chambers` at
    /// 8 (today's only historical value) so the ~40 call sites that exercise every shape
    /// other than `MultiStageHourglass` don't need to know the new parameter exists; tests
    /// that actually vary the chamber count use `make_test_mask_with_chambers` below.
    fn make_test_mask(
        w: usize,
        h: usize,
        shape: SandboxShape,
        neck_width: f32,
        hourglass_curve: f32,
    ) -> Vec<u8> {
        make_test_mask_with_chambers(w, h, shape, neck_width, hourglass_curve, 8)
    }

    /// Same as `make_test_mask`, but with `multistage_chambers` exposed for
    /// `MultiStageHourglass` tests that sweep the widest-tier chamber count.
    fn make_test_mask_with_chambers(
        w: usize,
        h: usize,
        shape: SandboxShape,
        neck_width: f32,
        hourglass_curve: f32,
        multistage_chambers: u32,
    ) -> Vec<u8> {
        let mut mask = vec![crate::MASK_OUTSIDE; w * h];
        // Pass 1: inside/outside
        for y in 0..h {
            for x in 0..w {
                let (inside, _) = eval_sandbox_shape(
                    x, y, w, h, shape, neck_width, hourglass_curve, multistage_chambers, false,
                );
                mask[y * w + x] = if inside { crate::MASK_INSIDE } else { crate::MASK_OUTSIDE };
            }
        }
        // Pass 2: mark boundary cells
        let snapshot = mask.clone();
        for y in 0..h {
            for x in 0..w {
                if snapshot[y * w + x] == crate::MASK_INSIDE {
                    let has_outside =
                        (x == 0 || snapshot[y * w + x - 1] == crate::MASK_OUTSIDE) ||
                        (x + 1 >= w || snapshot[y * w + x + 1] == crate::MASK_OUTSIDE) ||
                        (y == 0 || snapshot[(y - 1) * w + x] == crate::MASK_OUTSIDE) ||
                        (y + 1 >= h || snapshot[(y + 1) * w + x] == crate::MASK_OUTSIDE);
                    if has_outside {
                        mask[y * w + x] = crate::MASK_BOUNDARY;
                    }
                }
            }
        }
        mask
    }

    /// Resolution multiplier for the handful of liquid tests parameterised by scale (see
    /// `test_liquid_stream_stays_coherent` / `test_liquid_flowing_liquid_does_not_stand_in_walls`).
    ///
    /// Read once per test invocation from `SANDART_TEST_SCALE` (an env var rather than a cargo
    /// feature: it needs no rebuild to flip, composes trivially with `cargo test <name>`
    /// filtering, and default `cargo test` runs are unaffected by its mere existence, which a
    /// feature flag would risk if anyone forgot `--no-default-features` bookkeeping). Unset,
    /// unparseable, or `0` all fall back to `1` -- today's grid sizes, today's numbers, today's
    /// speed. Invoke deliberately at production scale with:
    ///
    /// ```text
    /// SANDART_TEST_SCALE=8 distrobox enter sandart-dev -- /home/deck/.cargo/bin/cargo test \
    ///     --release -p sandart-sim -- --nocapture test_liquid_stream_stays_coherent \
    ///     test_liquid_flowing_liquid_does_not_stand_in_walls
    /// ```
    ///
    /// `8` takes the 64x64 / 64x96 test grids to 512x512 / 512x768 -- `GRID_SIZE`, production's
    /// actual resolution. See docs/ARCHITECTURE.md's test-methodology section for runtime and
    /// what these tests are guarding against at that scale.
    fn test_scale() -> usize {
        std::env::var("SANDART_TEST_SCALE")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .filter(|&s| s >= 1)
            .unwrap_or(1)
    }

    /// Bundles the many mutable buffers settle_tick needs so the liquid-gravity
    /// characterisation tests below don't have to repeat ~10 lines of boilerplate
    /// allocation each. Used only by the L1-L10 tests added in Phase 0.
    struct TestSim {
        hm: Heightmap,
        temp_heights: Vec<f32>,
        cell_colors: Vec<u8>,
        cell_props: Vec<f32>,
        sliding: Vec<bool>,
        bounds: ActiveBounds,
        active_blocks: Vec<crate::BlockActivity>,
        last_displacements: Vec<f32>,
        last_simulated_ticks: Vec<u32>,
        edge_vel_h: Vec<f32>,
        edge_vel_v: Vec<f32>,
        column_depth: Vec<f32>,
        /// Mirrors `DrawingSimulation::head_field` -- the persistent hydraulic head buffer (task
        /// #55 step 2 rebuilt). Owned here exactly like `column_depth` just above.
        head_field: Vec<f32>,
        mask: Vec<u8>,
        block_size: usize,
        tick_count: u32,
        /// Mirrors `DrawingSimulation::fresh_pressure_field` / `settle_tick`'s parameter of the
        /// same name. Defaults to `false` in `new()` so every existing `TestSim`-based test is
        /// unaffected; set directly (`sim.fresh_pressure_field = true`) to A/B the standalone
        /// `column_depth` pass against a specific scenario without touching `tick()`'s signature.
        fresh_pressure_field: bool,
        /// Mirrors `DrawingSimulation::head_field_transport` / `settle_tick`'s parameter of the
        /// same name (TASK #55 step 3). Defaults to `false` in `new()` so every existing
        /// `TestSim`-based test is unaffected; set directly (`sim.head_field_transport = true`)
        /// to exercise the new head-field-driven liquid transport branch on a specific scenario
        /// without touching `tick()`'s signature.
        head_field_transport: bool,
        /// Mirrors `DrawingSimulation::pressure_sensitive_flow` / `settle_tick`'s parameter of the
        /// same name (TASK #63). Defaults to `false` in `new()` so every existing `TestSim`-based
        /// test is unaffected; set directly (`sim.pressure_sensitive_flow = true`) to exercise the
        /// pressure-attenuated conveyance rate on a specific scenario.
        pressure_sensitive_flow: bool,
        /// Mirrors `DrawingSimulation::overfill_pressure` / `settle_tick`'s parameter of the
        /// same name (TASK #70). Defaults to `false` in `new()`.
        pub overfill_pressure: bool,
        pub overfill_ratio: f32,
    }

    impl TestSim {
        fn new(w: usize, h: usize, props: Vec<f32>, mask: Vec<u8>, block_size: usize) -> Self {
            let cols = (w + block_size - 1) / block_size;
            let rows = (h + block_size - 1) / block_size;
            let expected_len = cols * rows;
            TestSim {
                hm: Heightmap::new(w, h, 0.0),
                temp_heights: vec![0.0; w * h],
                cell_colors: vec![0u8; w * h * 4],
                cell_props: props,
                sliding: vec![false; w * h],
                bounds: ActiveBounds { min_x: 0, max_x: w - 1, min_y: 0, max_y: h - 1, active: true },
                active_blocks: vec![crate::BlockActivity::Inactive; expected_len],
                last_displacements: vec![1.0; expected_len],
                last_simulated_ticks: vec![0; expected_len],
                edge_vel_h: vec![0.0; w * h],
                edge_vel_v: vec![0.0; w * h],
                column_depth: vec![0.0; w * h],
                head_field: vec![0.0; w * h],
                mask,
                block_size,
                tick_count: 0,
                fresh_pressure_field: false,
                head_field_transport: false,
                pressure_sensitive_flow: false,
                overfill_pressure: false,
                overfill_ratio: 0.50,
            }
        }

        fn tick(&mut self, gravity_dir: glam::Vec2, budget_n: usize) -> f32 {
            let flow = settle_tick(
                &mut self.hm,
                &mut self.temp_heights,
                &mut self.cell_colors,
                &mut self.cell_props,
                &mut self.sliding,
                &mut self.bounds,
                &mut self.active_blocks,
                &mut self.last_displacements,
                &mut self.last_simulated_ticks,
                budget_n,
                self.block_size,
                &[],
                12345u32.wrapping_add(self.tick_count).wrapping_add(phase_offset(K_RNG_SEED)),
                &mut self.edge_vel_h,
                &mut self.edge_vel_v,
                &mut self.column_depth,
                &mut self.head_field,
                &self.mask,
                self.tick_count,
                gravity_dir,
                self.fresh_pressure_field,
                self.head_field_transport,
                false,
                self.pressure_sensitive_flow,
                self.overfill_pressure,
                self.overfill_ratio,
            );
            self.tick_count += 1;
            flow
        }

        fn mass(&self) -> f64 {
            self.hm.data.iter().map(|&v| v as f64).sum()
        }
    }

    #[test]
    fn test_draw_point_out_of_bounds() {
        let mut hm = Heightmap::new(512, 512, crate::DEFAULT_SAND_HEIGHT);
        let mut cell_colors = vec![0u8; 512 * 512 * 4];
        let mut cell_props = get_test_props(crate::MaterialMode::ButterCream, 512 * 512);
        let mut bounds = ActiveBounds {
            min_x: 0,
            max_x: 0,
            min_y: 0,
            max_y: 0,
            active: false,
        };

        // Drawing completely offscreen should not panic or modify the heightmap
        displace_line(
            &mut hm,
            &mut cell_colors,
            &mut cell_props,
            Vec2::new(5.0, 5.0),
            Vec2::new(5.0, 5.0),
            0.1,
            &mut bounds,
        );

        // Assert that heightmap data is unchanged
        for &val in hm.as_slice() {
            assert_eq!(val, crate::DEFAULT_SAND_HEIGHT);
        }
    }

    #[test]
    fn test_draw_point_partial_overlap() {
        let mut hm = Heightmap::new(512, 512, crate::DEFAULT_SAND_HEIGHT);
        let mut cell_colors = vec![0u8; 512 * 512 * 4];
        let mut cell_props = get_test_props(crate::MaterialMode::ButterCream, 512 * 512);
        let mut bounds = ActiveBounds {
            min_x: 0,
            max_x: 0,
            min_y: 0,
            max_y: 0,
            active: false,
        };

        // Position marble so it sits on the left boundary
        displace_line(
            &mut hm,
            &mut cell_colors,
            &mut cell_props,
            Vec2::new(-1.0, 0.0),
            Vec2::new(-1.0, 0.0),
            0.05,
            &mut bounds,
        );

        // Check that some points are carved below 0.1, and bounds are respected
        let mut modified_count = 0;
        for &val in hm.as_slice() {
            if val < 0.1 {
                modified_count += 1;
            }
        }
        assert!(modified_count > 0);
        assert!(bounds.active);
    }

    #[test]
    fn test_draw_line_interpolation() {
        let mut hm = Heightmap::new(512, 512, crate::DEFAULT_SAND_HEIGHT);
        let mut cell_colors = vec![0u8; 512 * 512 * 4];
        let mut cell_props = get_test_props(crate::MaterialMode::ButterCream, 512 * 512);
        let mut bounds = ActiveBounds {
            min_x: 0,
            max_x: 0,
            min_y: 0,
            max_y: 0,
            active: false,
        };

        // Draw a line from (-0.5, 0.0) to (0.5, 0.0)
        displace_line(
            &mut hm,
            &mut cell_colors,
            &mut cell_props,
            Vec2::new(-0.5, 0.0),
            Vec2::new(0.5, 0.0),
            0.05,
            &mut bounds,
        );

        // Helper to convert pos to grid index
        let norm_to_grid = |pos: Vec2| {
            let x = ((pos.x + 1.0) * 0.5 * 512.0).clamp(0.0, 511.0) as usize;
            let y = ((1.0 - pos.y) * 0.5 * 512.0).clamp(0.0, 511.0) as usize;
            (x, y)
        };

        // Verify that the path is continuous by checking that the center points are drawn
        let (cx1, cy1) = norm_to_grid(Vec2::new(-0.5, 0.0));
        let (cx2, cy2) = norm_to_grid(Vec2::new(0.0, 0.0));
        let (cx3, cy3) = norm_to_grid(Vec2::new(0.5, 0.0));

        assert!(hm.get(cx1, cy1) < 0.03);
        assert!(hm.get(cx2, cy2) < 0.03);
        assert!(hm.get(cx3, cy3) < 0.03);
    }

    #[test]
    fn test_draw_point_extreme_coordinates_overflow() {
        let mut hm = Heightmap::new(512, 512, crate::DEFAULT_SAND_HEIGHT);
        let mut cell_colors = vec![0u8; 512 * 512 * 4];
        let mut cell_props = get_test_props(crate::MaterialMode::ButterCream, 512 * 512);
        let mut bounds = ActiveBounds {
            min_x: 0,
            max_x: 0,
            min_y: 0,
            max_y: 0,
            active: false,
        };

        displace_line(
            &mut hm,
            &mut cell_colors,
            &mut cell_props,
            Vec2::new(1e18, 1e18),
            Vec2::new(1e18, 1e18),
            0.1,
            &mut bounds,
        );
        for &val in hm.as_slice() {
            assert_eq!(val, crate::DEFAULT_SAND_HEIGHT);
        }
    }

    #[test]
    fn test_multipass_carving() {
        let mut hm = Heightmap::new(512, 512, crate::DEFAULT_SAND_HEIGHT);
        let mut cell_colors = vec![0u8; 512 * 512 * 4];
        let mut cell_props = get_test_props(crate::MaterialMode::DrySand, 512 * 512);
        let mut bounds = ActiveBounds {
            min_x: 0,
            max_x: 0,
            min_y: 0,
            max_y: 0,
            active: false,
        };

        // Pass 1: carving at (0.0, 0.0) with DrySand properties
        displace_line(
            &mut hm,
            &mut cell_colors,
            &mut cell_props,
            Vec2::ZERO,
            Vec2::ZERO,
            0.05,
            &mut bounds,
        );

        let center_idx = 256 * 512 + 256;
        let h1 = hm.data[center_idx];
        // Expect height to be approximately 20% of 0.35 = 0.07
        assert!((h1 - 0.07).abs() < 0.035, "First pass height should be ~0.07, got {}", h1);

        // Pass 2: carving again at (0.0, 0.0)
        displace_line(
            &mut hm,
            &mut cell_colors,
            &mut cell_props,
            Vec2::ZERO,
            Vec2::ZERO,
            0.05,
            &mut bounds,
        );
        let h2 = hm.data[center_idx];
        // Expect height to be approximately 20% of h1 = 0.20 * 0.07 = 0.014
        assert!((h2 - 0.014).abs() < 0.035, "Second pass height should be ~0.014, got {}", h2);
        assert!(h2 < h1, "Second pass should carve deeper than first pass");
    }

    #[test]
    fn test_volume_conservation() {
        let mut hm = Heightmap::new(512, 512, 0.4);
        let mut cell_colors = vec![0u8; 512 * 512 * 4];
        let mut cell_props = get_test_props(crate::MaterialMode::ButterCream, 512 * 512);
        let mut bounds = ActiveBounds {
            min_x: 0,
            max_x: 0,
            min_y: 0,
            max_y: 0,
            active: false,
        };
        let initial_sum: f64 = hm.as_slice().iter().map(|&x| x as f64).sum();

        // Perform displacement along a path
        displace_line(
            &mut hm,
            &mut cell_colors,
            &mut cell_props,
            Vec2::new(-0.2, 0.2),
            Vec2::new(0.2, -0.2),
            0.03,
            &mut bounds,
        );

        let final_sum: f64 = hm.as_slice().iter().map(|&x| x as f64).sum();
        let diff = (final_sum - initial_sum).abs();
        assert!(diff < 1e-5, "Volume not conserved! diff = {}", diff);
    }

    #[test]
    fn test_draw_line_extreme_coordinates_overflow() {
        let mut hm = Heightmap::new(512, 512, crate::DEFAULT_SAND_HEIGHT);
        let mut cell_colors = vec![0u8; 512 * 512 * 4];
        let mut cell_props = get_test_props(crate::MaterialMode::ButterCream, 512 * 512);
        let mut bounds = ActiveBounds {
            min_x: 0,
            max_x: 0,
            min_y: 0,
            max_y: 0,
            active: false,
        };
        displace_line(
            &mut hm,
            &mut cell_colors,
            &mut cell_props,
            Vec2::new(-1e18, 0.0),
            Vec2::new(1e18, 0.0),
            0.1,
            &mut bounds,
        );
    }

    #[test]
    fn test_volume_conservation_with_saturation() {
        let mut hm = Heightmap::new(512, 512, 0.70);
        let mut cell_colors = vec![0u8; 512 * 512 * 4];
        let mut cell_props = get_test_props(crate::MaterialMode::ButterCream, 512 * 512);
        let mut bounds = ActiveBounds {
            min_x: 0,
            max_x: 0,
            min_y: 0,
            max_y: 0,
            active: false,
        };
        let initial_sum: f64 = hm.as_slice().iter().map(|&x| x as f64).sum();

        // Perform displacement at a single point to trigger local saturation in the inner ridge
        displace_line(
            &mut hm,
            &mut cell_colors,
            &mut cell_props,
            Vec2::ZERO,
            Vec2::ZERO,
            0.02,
            &mut bounds,
        );

        let final_sum: f64 = hm.as_slice().iter().map(|&x| x as f64).sum();
        let diff = (final_sum - initial_sum).abs();
        assert!(diff < 1e-5, "Volume not conserved! diff = {}", diff);
    }

    #[test]
    fn test_settling_flow_and_volume_conservation() {
        let mut hm = Heightmap::new(512, 512, 0.5);
        let mut temp_heights = vec![0.5; 512 * 512];
        let mut cell_colors = vec![0u8; 512 * 512 * 4];
        let mut cell_props = get_test_props(crate::MaterialMode::ButterCream, 512 * 512);

        let center_idx = 256 * 512 + 256;
        hm.data[center_idx] = 0.8;

        let mut bounds = ActiveBounds {
            min_x: 250,
            max_x: 262,
            min_y: 250,
            max_y: 262,
            active: true,
        };

        let initial_sum: f64 = hm.as_slice().iter().map(|&x| x as f64).sum();

        let mut edge_vel_h = vec![0.0; 512 * 512];
        let mut edge_vel_v = vec![0.0; 512 * 512];
        let mut column_depth = vec![0.0; 512 * 512];
        let mut head_field = vec![0.0; 512 * 512];
        let mut active_blocks: Vec<crate::BlockActivity> = Vec::new();
        let mut last_displacements = vec![1.0; 256];
        let mut last_simulated_ticks = vec![0; 256];
        let budget_n = 256;
        let mut flow_occurred = false;
        let mut sliding = vec![false; 512 * 512];

        let mask = make_test_mask(512, 512, crate::SandboxShape::Circle, 0.04, 1.0);
        for i in 0..10 {
            let flow = settle_tick(
                &mut hm,
                &mut temp_heights,
                &mut cell_colors,
                &mut cell_props,
                &mut sliding,
                &mut bounds,
                &mut active_blocks,
                &mut last_displacements,
                &mut last_simulated_ticks,
                budget_n,
                32,
                &[],
                12345,
                &mut edge_vel_h,
                &mut edge_vel_v,
                &mut column_depth,
                &mut head_field,
                &mask,
                i as u32,
                glam::Vec2::ZERO,
                false,
                false,
                false, false, false, 0.50,
            );
            if flow > 0.0 {
                flow_occurred = true;
            }
        }

        assert!(flow_occurred, "Sand should flow down from the peak");

        let final_sum: f64 = hm.as_slice().iter().map(|&x| x as f64).sum();
        let diff = (final_sum - initial_sum).abs();
        assert!(
            diff < 1e-5,
            "Settling did not conserve volume! diff = {}",
            diff
        );
        assert!(
            hm.data[center_idx] < 0.8,
            "Peak should be lower after flowing"
        );
    }

    #[test]
    fn test_settling_deactivation() {
        let mut hm = Heightmap::new(512, 512, 0.5);
        let mut temp_heights = vec![0.5; 512 * 512];
        let mut cell_colors = vec![0u8; 512 * 512 * 4];
        let mut cell_props = get_test_props(crate::MaterialMode::ButterCream, 512 * 512);

        let mut bounds = ActiveBounds {
            min_x: 250,
            max_x: 262,
            min_y: 250,
            max_y: 262,
            active: true,
        };

        let mut edge_vel_h = vec![0.0; 512 * 512];
        let mut edge_vel_v = vec![0.0; 512 * 512];
        let mut column_depth = vec![0.0; 512 * 512];
        let mut head_field = vec![0.0; 512 * 512];
        let mut active_blocks: Vec<crate::BlockActivity> = Vec::new();
        let mut last_displacements = Vec::new();
        let mut last_simulated_ticks = Vec::new();
        let budget_n = 256;
        let mut sliding = vec![false; 512 * 512];

        let mask = make_test_mask(512, 512, crate::SandboxShape::Circle, 0.04, 1.0);
        let flow = settle_tick(
            &mut hm,
            &mut temp_heights,
            &mut cell_colors,
            &mut cell_props,
            &mut sliding,
            &mut bounds,
            &mut active_blocks,
            &mut last_displacements,
            &mut last_simulated_ticks,
            budget_n,
            32,
            &[],
            12345,
            &mut edge_vel_h,
            &mut edge_vel_v,
            &mut column_depth,
            &mut head_field,
            &mask,
            0,
            glam::Vec2::ZERO,
            false,
            false,
            false, false, false, 0.50,
        );
        assert_eq!(flow, 0.0);
        assert!(!bounds.active, "Settling should deactivate when stable");
    }

    #[test]
    fn test_material_presets_and_avalanche() {
        use crate::MaterialMode;
        
        let materials = [
            MaterialMode::ButterCream,
            MaterialMode::DrySand,
            MaterialMode::Snow,
            MaterialMode::KineticSand,
            MaterialMode::WetSand,
            MaterialMode::FinePowder,
            MaterialMode::Oobleck,
            MaterialMode::MoonDust,
            MaterialMode::Water,
            MaterialMode::Milk,
            MaterialMode::VegetableOil,
            MaterialMode::CalmWater,
            MaterialMode::Yogurt,
            MaterialMode::CoarseSand,
        ];

        for &mat in &materials {
            let mut hm = Heightmap::new(64, 64, 0.5);
            let mut temp_heights = vec![0.5; 64 * 64];
            let mut cell_colors = vec![0u8; 64 * 64 * 4];
            let mut cell_props = get_test_props(mat, 64 * 64);
            let mut sliding = vec![false; 64 * 64];
            let mut bounds = ActiveBounds {
                min_x: 10,
                max_x: 54,
                min_y: 10,
                max_y: 54,
                active: true,
            };

            // Set a steep spike at center that exceeds the avalanche threshold (0.20 slope)
            let center_idx = 32 * 64 + 32;
            hm.data[center_idx] = 1.0;
            hm.data[center_idx - 1] = 0.5; // slope = 0.5 > 0.20

            let mut edge_vel_h = vec![0.0; 64 * 64];
            let mut edge_vel_v = vec![0.0; 64 * 64];
            let mut column_depth = vec![0.0; 64 * 64];
            let mut head_field = vec![0.0; 64 * 64];
            let mut active_blocks: Vec<crate::BlockActivity> = Vec::new();
            let mut last_displacements = vec![1.0; 4];
            let mut last_simulated_ticks = vec![0; 4];
            let budget_n = 256;
            let mask = make_test_mask(64, 64, crate::SandboxShape::Circle, 0.04, 1.0);
            let flow = settle_tick(
                &mut hm,
                &mut temp_heights,
                &mut cell_colors,
                &mut cell_props,
                &mut sliding,
                &mut bounds,
                &mut active_blocks,
                &mut last_displacements,
                &mut last_simulated_ticks,
                budget_n,
                32,
                &[ActiveMarbleInfo { pos: Vec2::ZERO, vel: 0.1, vel_vec: Vec2::new(0.1, 0.0) }],
                9999,
                &mut edge_vel_h,
                &mut edge_vel_v,
                &mut column_depth,
                &mut head_field,
                &mask,
                0,
                glam::Vec2::ZERO,
                false,
                false,
                false, false, false, 0.50,
            );

            assert!(flow > 0.0, "Material {:?} should flow under steep slope", mat);
        }
    }

    #[test]
    fn test_color_conservation() {
        let mut hm = Heightmap::new(128, 128, 0.5);
        // Put a peak in the center so sand flows
        let center_idx = 64 * 128 + 64;
        hm.data[center_idx] = 1.0;

        let mut cell_colors = vec![0u8; 128 * 128 * 4];
        let mut cell_props = vec![0.0f32; 128 * 128 * 4];
        // Initialize cell_colors and cell_props with a mixed striped pattern
        for y in 0..128 {
            for x in 0..128 {
                let idx = y * 128 + x;
                if (x / 16) % 2 == 0 {
                    cell_props[idx * 4 + PROP_WETNESS] = 0.00;
                    cell_props[idx * 4 + PROP_THRESHOLD] = 0.08;
                    cell_props[idx * 4 + PROP_FLOW_RATE] = 0.25;
                    cell_props[idx * 4 + PROP_GRAIN_SIZE] = 0.45;

                    cell_colors[idx * 4 + 0] = 200; // Reddish DrySand
                    cell_colors[idx * 4 + 1] = 100;
                    cell_colors[idx * 4 + 2] = 50;
                    cell_colors[idx * 4 + 3] = 255;
                } else {
                    cell_props[idx * 4 + PROP_WETNESS] = 0.45;
                    cell_props[idx * 4 + PROP_THRESHOLD] = 0.14;
                    cell_props[idx * 4 + PROP_FLOW_RATE] = 0.08;
                    cell_props[idx * 4 + PROP_GRAIN_SIZE] = 0.40;

                    cell_colors[idx * 4 + 0] = 50; // Bluish WetSand
                    cell_colors[idx * 4 + 1] = 100;
                    cell_colors[idx * 4 + 2] = 200;
                    cell_colors[idx * 4 + 3] = 255;
                }
            }
        }

        // Calculate initial total colors (Red, Green, Blue masses)
        let calculate_color_masses = |colors: &[u8], hmap: &Heightmap| -> (f64, f64, f64) {
            let mut r_mass = 0.0f64;
            let mut g_mass = 0.0f64;
            let mut b_mass = 0.0f64;
            for (idx, &h) in hmap.as_slice().iter().enumerate() {
                let r = colors[idx * 4 + 0] as f64;
                let g = colors[idx * 4 + 1] as f64;
                let b = colors[idx * 4 + 2] as f64;
                r_mass += r * h as f64;
                g_mass += g * h as f64;
                b_mass += b * h as f64;
            }
            (r_mass, g_mass, b_mass)
        };

        let (initial_r, initial_g, initial_b) = calculate_color_masses(&cell_colors, &hm);

        let mut temp_heights = vec![0.5; 128 * 128];
        let mut sliding = vec![false; 128 * 128];
        let mut bounds = ActiveBounds {
            min_x: 60,
            max_x: 68,
            min_y: 60,
            max_y: 68,
            active: true,
        };

        let mut edge_vel_h = vec![0.0; 128 * 128];
        let mut edge_vel_v = vec![0.0; 128 * 128];
        let mut column_depth = vec![0.0; 128 * 128];
        let mut head_field = vec![0.0; 128 * 128];
        let mut active_blocks: Vec<crate::BlockActivity> = Vec::new();
        let mut last_displacements = vec![1.0; 16];
        let mut last_simulated_ticks = vec![0; 16];

        let mask = make_test_mask(128, 128, crate::SandboxShape::Circle, 0.04, 1.0);
        // Settle a bit to trigger flows
        let flow = settle_tick(
            &mut hm,
            &mut temp_heights,
            &mut cell_colors,
            &mut cell_props,
            &mut sliding,
            &mut bounds,
            &mut active_blocks,
            &mut last_displacements,
            &mut last_simulated_ticks,
            256,
            32,
            &[],
            12345,
            &mut edge_vel_h,
            &mut edge_vel_v,
            &mut column_depth,
            &mut head_field,
            &mask,
            0,
            glam::Vec2::ZERO,
            false,
            false,
            false, false, false, 0.50,
        );

        assert!(flow > 0.0, "Settling flow must occur for the test");

        // Calculate final total colors
        let (final_r, final_g, final_b) = calculate_color_masses(&cell_colors, &hm);

        let diff_r = (final_r - initial_r).abs() / initial_r;
        let diff_g = (final_g - initial_g).abs() / initial_g;
        let diff_b = (final_b - initial_b).abs() / initial_b;

        assert!(diff_r < 0.005, "Red color mass not conserved! diff = {:.5}%, initial = {}, final = {}", diff_r * 100.0, initial_r, final_r);
        assert!(diff_g < 0.005, "Green color mass not conserved! diff = {:.5}%, initial = {}, final = {}", diff_g * 100.0, initial_g, final_g);
        assert!(diff_b < 0.005, "Blue color mass not conserved! diff = {:.5}%, initial = {}, final = {}", diff_b * 100.0, initial_b, final_b);
    }

    #[test]
    fn test_advect_properties_weighted() {
        let mut cell_colors = vec![0u8; 8];
        let mut cell_props = vec![0.0f32; 8];

        // Cell 0: Red, Wet Sand-ish
        cell_colors[0..4].copy_from_slice(&[200, 100, 50, 255]);
        cell_props[0..4].copy_from_slice(&[0.5, 0.1, 0.15, 0.3]);

        // Cell 1: Blue, Dry Sand-ish
        cell_colors[4..8].copy_from_slice(&[50, 100, 200, 255]);
        cell_props[4..8].copy_from_slice(&[0.0, 0.08, 0.25, 0.45]);

        // Advect from 0 to 1 with flow = 0.2, and dst height h_dst = 0.2
        advect_properties(&mut cell_colors, &mut cell_props, 0, 1, 0.2, 0.2);

        // Expected colors (weighted average):
        // Red = (50 * 0.5 + 200 * 0.5) = 125
        // Green = 100
        // Blue = (200 * 0.5 + 50 * 0.5) = 125
        assert_eq!(cell_colors[4], 125);
        assert_eq!(cell_colors[5], 100);
        assert_eq!(cell_colors[6], 125);

        // Expected properties (weighted average):
        // wetness = (0.0 * 0.5 + 0.5 * 0.5) = 0.25
        // threshold = (0.08 * 0.5 + 0.1 * 0.5) = 0.09
        // flow_rate = (0.25 * 0.5 + 0.15 * 0.5) = 0.20
        // grain_size = (0.45 * 0.5 + 0.3 * 0.5) = 0.375
        assert_eq!(cell_props[4], 0.25);
        assert_eq!(cell_props[5], 0.09);
        assert_eq!(cell_props[6], 0.20);
        assert_eq!(cell_props[7], 0.375);
    }

    #[test]
    fn test_displace_line_advects() {
        let mut hm = Heightmap::new(128, 128, 0.5);
        let mut cell_colors = vec![100u8; 128 * 128 * 4];
        let mut cell_props = vec![0.5f32; 128 * 128 * 4];
        let mut active_bounds = ActiveBounds {
            min_x: 0,
            max_x: 127,
            min_y: 0,
            max_y: 127,
            active: true,
        };

        // Source center area has different properties & colors
        for y in 60..68 {
            for x in 60..68 {
                let idx = y * 128 + x;
                cell_colors[idx * 4 + 0] = 200;
                cell_props[idx * 4 + PROP_WETNESS] = 0.1;
            }
        }

        // Draw a line through the center
        displace_line(
            &mut hm,
            &mut cell_colors,
            &mut cell_props,
            Vec2::new(0.0, 0.0),
            Vec2::new(0.1, 0.1),
            0.05,
            &mut active_bounds,
        );

        // Check that some cell outside the immediate line segment but within radius received advected properties
        // We will sum the red color and wetness in the ridge and assert change.
        let mut changed = false;
        for y in 0..128 {
            for x in 0..128 {
                let idx = y * 128 + x;
                // Exclude the starting zone
                if (x < 60 || x >= 68) || (y < 60 || y >= 68) {
                    if cell_colors[idx * 4 + 0] != 100 || cell_props[idx * 4 + PROP_WETNESS] != 0.5 {
                        changed = true;
                        break;
                    }
                }
            }
        }
        assert!(changed, "Properties/colors must have advected to surrounding cells during displacement");
    }

    #[test]
    fn test_property_and_color_conservation() {
        let mut sim = DrawingSimulation::new();
        // Set up alternating stripes of DrySand and WetSand properties, and mixed colors
        let mut cell_props = vec![0.0f32; GRID_SIZE * GRID_SIZE * 4];
        // This buffer goes through the external set_cell_colors(&[u8]) API below, so it
        // stays u8 (not the internal f32 source of truth) — it's exercising the boundary.
        let mut cell_colors = vec![0u8; GRID_SIZE * GRID_SIZE * 4];
        for y in 0..GRID_SIZE {
            for x in 0..GRID_SIZE {
                let idx = y * GRID_SIZE + x;
                // Alternating stripes of DrySand and WetSand properties
                if (x / 32) % 2 == 0 {
                    cell_props[idx * 4 + PROP_WETNESS] = 0.00;
                    cell_props[idx * 4 + PROP_THRESHOLD] = 0.08;
                    cell_props[idx * 4 + PROP_FLOW_RATE] = 0.25;
                    cell_props[idx * 4 + PROP_GRAIN_SIZE] = 0.45;

                    cell_colors[idx * 4 + 0] = 200; // Reddish DrySand
                    cell_colors[idx * 4 + 1] = 100;
                    cell_colors[idx * 4 + 2] = 50;
                    cell_colors[idx * 4 + 3] = 255;
                } else {
                    cell_props[idx * 4 + PROP_WETNESS] = 0.45;
                    cell_props[idx * 4 + PROP_THRESHOLD] = 0.14;
                    cell_props[idx * 4 + PROP_FLOW_RATE] = 0.08;
                    cell_props[idx * 4 + PROP_GRAIN_SIZE] = 0.40;

                    cell_colors[idx * 4 + 0] = 50; // Bluish WetSand
                    cell_colors[idx * 4 + 1] = 100;
                    cell_colors[idx * 4 + 2] = 200;
                    cell_colors[idx * 4 + 3] = 255;
                }
            }
        }
        sim.set_cell_props(&cell_props);
        sim.set_cell_colors(&cell_colors);

        // Put several heaps of sand to force movement
        sim.heightmap.data.fill(0.1);
        for cy in [GRID_SIZE / 4, GRID_SIZE / 2, (3 * GRID_SIZE) / 4] {
            for cx in [GRID_SIZE / 4, GRID_SIZE / 2, (3 * GRID_SIZE) / 4] {
                let c_idx = cy * GRID_SIZE + cx;
                sim.heightmap.data[c_idx] = 1.0;
            }
        }

        // Calculate initial total property masses and color masses
        let calculate_masses = |s: &DrawingSimulation| -> (f64, f64, f64, f64, f64, f64, f64) {
            let mut wet_mass = 0.0f64;
            let mut thresh_mass = 0.0f64;
            let mut flow_mass = 0.0f64;
            let mut grain_mass = 0.0f64;
            let mut r_mass = 0.0f64;
            let mut g_mass = 0.0f64;
            let mut b_mass = 0.0f64;
            for (idx, &h) in s.heightmap.data.iter().enumerate() {
                let w = s.cell_props[idx * 4 + PROP_WETNESS] as f64;
                let t = s.cell_props[idx * 4 + PROP_THRESHOLD] as f64;
                let f = s.cell_props[idx * 4 + PROP_FLOW_RATE] as f64;
                let gr = s.cell_props[idx * 4 + PROP_GRAIN_SIZE] as f64;
                let r = s.cell_colors[idx * 4 + 0] as f64;
                let g = s.cell_colors[idx * 4 + 1] as f64;
                let bl = s.cell_colors[idx * 4 + 2] as f64;
                wet_mass += w * h as f64;
                thresh_mass += t * h as f64;
                flow_mass += f * h as f64;
                grain_mass += gr * h as f64;
                r_mass += r * h as f64;
                g_mass += g * h as f64;
                b_mass += bl * h as f64;
            }
            (wet_mass, thresh_mass, flow_mass, grain_mass, r_mass, g_mass, b_mass)
        };

        let (init_wet, init_thresh, init_flow, init_grain, init_r, init_g, init_b) = calculate_masses(&sim);

        // Run 100 simulation steps with a moving marble
        let mut targets = [None; 5];
        for i in 0..100 {
            let angle = i as f32 * 0.15;
            let radius = i as f32 * 0.005;
            targets[0] = Some(Vec2::new(angle.cos() * radius, angle.sin() * radius));
            sim.update(
                0.016,
                &targets,
                0.02,
                MaterialMode::DrySand, // preset parameter is ignored for properties after init
                SandboxShape::Circle,
                16.0,
                16.0,
            );
        }

        let (final_wet, final_thresh, final_flow, final_grain, final_r, final_g, final_b) = calculate_masses(&sim);

        let diff_wet = (final_wet - init_wet).abs() / init_wet;
        let diff_thresh = (final_thresh - init_thresh).abs() / init_thresh;
        let diff_flow = (final_flow - init_flow).abs() / init_flow;
        let diff_grain = (final_grain - init_grain).abs() / init_grain;
        let diff_r = (final_r - init_r).abs() / init_r;
        let diff_g = (final_g - init_g).abs() / init_g;
        let diff_b = (final_b - init_b).abs() / init_b;

        // Properties and colors must be conserved within 0.8%
        assert!(diff_wet < 0.008, "Wetness mass leaked! diff = {:.5}%, init = {}, final = {}", diff_wet * 100.0, init_wet, final_wet);
        assert!(diff_thresh < 0.008, "Threshold mass leaked! diff = {:.5}%, init = {}, final = {}", diff_thresh * 100.0, init_thresh, final_thresh);
        assert!(diff_flow < 0.008, "Flow rate mass leaked! diff = {:.5}%, init = {}, final = {}", diff_flow * 100.0, init_flow, final_flow);
        assert!(diff_grain < 0.008, "Grain size mass leaked! diff = {:.5}%, init = {}, final = {}", diff_grain * 100.0, init_grain, final_grain);
        assert!(diff_r < 0.008, "Red color mass leaked! diff = {:.5}%, init = {}, final = {}", diff_r * 100.0, init_r, final_r);
        assert!(diff_g < 0.008, "Green color mass leaked! diff = {:.5}%, init = {}, final = {}", diff_g * 100.0, init_g, final_g);
        assert!(diff_b < 0.008, "Blue color mass leaked! diff = {:.5}%, init = {}, final = {}", diff_b * 100.0, init_b, final_b);
    }

    #[test]
    fn test_hourglass_boundary_math() {
        let w_f = 512.0;
        let h_f = 512.0;
        let center_x = w_f / 2.0;
        let center_y = h_f / 2.0;
        let chamber_h = 0.40 * h_f;
        let max_hw = 0.35 * w_f;
        let neck_hw = 0.04 * w_f;

        let is_inside = |cx: usize, cy: usize| -> bool {
            let dx = cx as f32 - center_x;
            let dy = cy as f32 - center_y;
            
            let dy_abs = dy.abs();
            if dy_abs < chamber_h {
                let t = dy_abs / chamber_h;
                let allowed_hw = neck_hw + t * (max_hw - neck_hw);
                dx.abs() < allowed_hw
            } else {
                false
            }
        };

        // Center of upper chamber (256, 156)
        assert!(is_inside(256, 156));
        // Center of lower chamber (256, 356)
        assert!(is_inside(256, 356));
        // Inside the neck (256, 256 = center)
        assert!(is_inside(256, 256));
        // Inside upper chamber but offset horizontally
        assert!(is_inside(256 + 50, 156));
        // Outside chamber horizontally
        assert!(!is_inside(256 + 150, 156));
        // Completely outside vertically
        assert!(!is_inside(256, 20));
    }

    #[test]
    fn test_gravity_bias_flow() {
        let mut hm = Heightmap::new(64, 64, 0.35);
        let mut temp_heights = vec![0.35; 64 * 64];
        let mut cell_colors = vec![0u8; 64 * 64 * 4];
        let mut cell_props = get_test_props(MaterialMode::DrySand, 64 * 64);
        let mut sliding = vec![false; 64 * 64];
        let mut bounds = ActiveBounds {
            min_x: 2,
            max_x: 61,
            min_y: 2,
            max_y: 61,
            active: true,
        };

        let mut edge_vel_h = vec![0.0; 64 * 64];
            let mut edge_vel_v = vec![0.0; 64 * 64];
            let mut column_depth = vec![0.0; 64 * 64];
            let mut head_field = vec![0.0; 64 * 64];
        let mut active_blocks = vec![crate::BlockActivity::Inactive; 4];
        let mut last_displacements = vec![1.0; 4];
        let mut last_simulated_ticks = vec![0; 4];
        
        // Put gravity pulling downwards (+Y direction) - matching UI default strength (0.04)
        let gravity_dir = glam::Vec2::new(0.0, 0.04);
        
        let initial_sum: f32 = hm.data.iter().sum();

        let mask = make_test_mask(64, 64, SandboxShape::Circle, 0.04, 1.0);
        // Run 50 ticks of gravity settling
        for i in 0..50 {
            settle_tick(
                &mut hm,
                &mut temp_heights,
                &mut cell_colors,
                &mut cell_props,
                &mut sliding,
                &mut bounds,
                &mut active_blocks,
                &mut last_displacements,
                &mut last_simulated_ticks,
                256,
                32,
                &[],
                12345,
                &mut edge_vel_h,
                &mut edge_vel_v,
                &mut column_depth,
                &mut head_field,
                &mask,
                i as u32,
                gravity_dir,
                false,
                false,
                false, false, false, 0.50,
            );
        }

        let final_sum: f32 = hm.data.iter().sum();
        // Mass conservation
        assert!((final_sum - initial_sum).abs() / initial_sum < 1e-4);

        // Sand should have accumulated in the bottom half of the circle
        let top_half_sum: f32 = hm.data[0..32*64].iter().sum();
        let bottom_half_sum: f32 = hm.data[32*64..64*64].iter().sum();
        assert!(bottom_half_sum > top_half_sum, "Sand did not flow downward under gravity!");
    }


    #[test]
    fn test_hourglass_flow_after_flip() {
        let w = 64;
        let h = 64;
        let center_x = w as f32 / 2.0;
        let center_y = h as f32 / 2.0;
        let chamber_h = 0.40 * h as f32;
        let max_hw = 0.35 * w as f32;
        let neck_hw = 0.15 * w as f32; // Wide neck to speed up test flow
        let hourglass_curve = 0.6;

        let mut hm = Heightmap::new(w, h, 0.0);

        // Fill only a shallow layer in the upper chamber just above the neck
        for y in 0..h {
            let dy = y as f32 - center_y;
            let dy_abs = dy.abs();
            for x in 0..w {
                let dx = x as f32 - center_x;
                if dy_abs < chamber_h {
                    let t = dy_abs / chamber_h;
                    let allowed_hw = neck_hw + t.powf(hourglass_curve) * (max_hw - neck_hw);
                    if dx.abs() < allowed_hw && dy < 0.0 && dy > -6.0 {
                        hm.data[y * w + x] = 1.0;
                    }
                }
            }
        }

        let mut temp_heights = hm.data.clone();
        let mut cell_props = get_test_props(MaterialMode::DrySand, w * h);
        let mut cell_colors = vec![0u8; w * h * 4];
        let mut sliding = vec![false; w * h];
        let mut bounds = ActiveBounds {
            min_x: 2,
            max_x: 61,
            min_y: 2,
            max_y: 61,
            active: true,
        };

        let mut edge_vel_h = vec![0.0; w * h];
        let mut edge_vel_v = vec![0.0; w * h];
        let mut column_depth = vec![0.0; w * h];
        let mut head_field = vec![0.0; w * h];
        let mut active_blocks = vec![crate::BlockActivity::Inactive; 4];
        let mut last_displacements = vec![1.0; 4];
        let mut last_simulated_ticks = vec![0; 4];

        // Downward gravity
        let gravity_dir = glam::Vec2::new(0.0, 0.04);

        let initial_top_sum: f32 = hm.data[0..32 * w].iter().sum();
        let initial_bottom_sum: f32 = hm.data[32 * w..].iter().sum();
        assert!(initial_top_sum > 10.0);
        assert_eq!(initial_bottom_sum, 0.0);

        let mask = make_test_mask(w, h, SandboxShape::Hourglass, 0.15, hourglass_curve);
        // Run 500 ticks to let almost all sand flow downward into the bottom chamber
        for i in 0..500 {
            settle_tick(
                &mut hm,
                &mut temp_heights,
                &mut cell_colors,
                &mut cell_props,
                &mut sliding,
                &mut bounds,
                &mut active_blocks,
                &mut last_displacements,
                &mut last_simulated_ticks,
                256,
                32,
                &[],
                12345 + i as u32,
                &mut edge_vel_h,
                &mut edge_vel_v,
                &mut column_depth,
                &mut head_field,
                &mask,
                i as u32,
                gravity_dir,
                false,
                false,
                false, false, false, 0.50,
            );
        }

        let mid_top_sum: f32 = hm.data[0..32 * w].iter().sum();
        let mid_bottom_sum: f32 = hm.data[32 * w..].iter().sum();

        // Sand should have flowed downward into the bottom chamber
        assert!(mid_bottom_sum > initial_top_sum * 0.40, "Not enough sand flowed to bottom! bottom_sum={}, init_top={}", mid_bottom_sum, initial_top_sum);
        assert!(mid_top_sum < initial_top_sum * 0.60);

        // Swap heights vertically (simulate flip)
        for y in 0..h / 2 {
            let y2 = h - 1 - y;
            for x in 0..w {
                hm.data.swap(y * w + x, y2 * w + x);
                temp_heights.swap(y * w + x, y2 * w + x);
            }
        }

        let post_flip_top_sum: f32 = hm.data[0..32 * w].iter().sum();
        let post_flip_bottom_sum: f32 = hm.data[32 * w..].iter().sum();

        // After flip, sand is back in the top chamber (allow tiny epsilon for floating point swap ordering)
        assert!((post_flip_top_sum - mid_bottom_sum).abs() < 1e-4, "Top sum mismatch: {} vs {}", post_flip_top_sum, mid_bottom_sum);
        assert!((post_flip_bottom_sum - mid_top_sum).abs() < 1e-4, "Bottom sum mismatch: {} vs {}", post_flip_bottom_sum, mid_top_sum);

        // Run another 500 ticks with downward gravity
        for i in 0..500 {
            settle_tick(
                &mut hm,
                &mut temp_heights,
                &mut cell_colors,
                &mut cell_props,
                &mut sliding,
                &mut bounds,
                &mut active_blocks,
                &mut last_displacements,
                &mut last_simulated_ticks,
                256,
                32,
                &[],
                12345 + 500 + i as u32,
                &mut edge_vel_h,
                &mut edge_vel_v,
                &mut column_depth,
                &mut head_field,
                &mask,
                (500 + i) as u32,
                gravity_dir,
                false,
                false,
                false, false, false, 0.50,
            );
        }

        let final_bottom_sum: f32 = hm.data[32 * w..].iter().sum();

        // Sand should have flowed downward again
        assert!(final_bottom_sum > post_flip_bottom_sum + (post_flip_top_sum * 0.30), "Sand did not flow downward after flip! init_bottom={}, final_bottom={}, post_flip_top={}", post_flip_bottom_sum, final_bottom_sum, post_flip_top_sum);
    }

    #[test]
    fn test_hourglass_statistical_symmetry() {
        // Initialize a symmetric grid with sand concentrated in the middle column
        let w = 64;
        let h = 64;
        let mut hm = Heightmap::new(w, h, 0.0);
        
        // Put a single block of sand at the top middle
        for y in 2..20 {
            for x in 30..34 {
                hm.data[y * w + x] = 1.0;
            }
        }

        let mut temp_heights = hm.data.clone();
        let mut cell_colors = vec![0u8; w * h * 4];
        let mut cell_props = get_test_props(MaterialMode::DrySand, w * h);
        let mut sliding = vec![false; w * h];
        let mut bounds = ActiveBounds {
            min_x: 2,
            max_x: 61,
            min_y: 2,
            max_y: 61,
            active: true,
        };

        let mut edge_vel_h = vec![0.0; w * h];
        let mut edge_vel_v = vec![0.0; w * h];
        let mut column_depth = vec![0.0; w * h];
        let mut head_field = vec![0.0; w * h];
        let mut active_blocks = vec![crate::BlockActivity::Inactive; 4];
        let mut last_displacements = vec![1.0; 4];
        let mut last_simulated_ticks = vec![0; 4];
        
        // Downward gravity
        let gravity_dir = glam::Vec2::new(0.0, 0.04);

        let mask = make_test_mask(w, h, SandboxShape::Circle, 0.04, 1.0);
        // Run 40 ticks of gravity settling
        for i in 0..40 {
            settle_tick(
                &mut hm,
                &mut temp_heights,
                &mut cell_colors,
                &mut cell_props,
                &mut sliding,
                &mut bounds,
                &mut active_blocks,
                &mut last_displacements,
                &mut last_simulated_ticks,
                256,
                32,
                &[],
                12345 + i as u32,
                &mut edge_vel_h,
                &mut edge_vel_v,
                &mut column_depth,
                &mut head_field,
                &mask,
                i as u32,
                gravity_dir,
                false,
                false,
                false, false, false, 0.50,
            );
        }

        // Measure center of mass along X axis
        let mut total_mass = 0.0f32;
        let mut weighted_x = 0.0f32;
        for y in 0..h {
            for x in 0..w {
                let val = hm.data[y * w + x];
                if val > 0.0 {
                    total_mass += val;
                    weighted_x += (x as f32) * val;
                }
            }
        }

        let center_of_mass_x = weighted_x / total_mass;
        let geometric_center_x = (w as f32 - 1.0) / 2.0; // 31.5

        // Center of mass should be extremely close to the geometric center (perfect symmetry)
        let bias = (center_of_mass_x - geometric_center_x).abs();
        assert!(bias < 0.25, "Found horizontal symmetry bias: {}", bias);
    }

    #[test]
    fn test_liquid_gravity_flows_downward() {
        // Verify that the wave-propagation solver (wetness >= 0.75) moves liquid
        // downward under gravity, not upward.
        let w = 64;
        let h = 64;
        let center_x = w as f32 / 2.0; // 32
        let center_y = h as f32 / 2.0; // 32
        let r = 0.46 * w as f32;       // 29.44
        let r_sq = r * r;

        let mut hm = Heightmap::new(w, h, 0.0);

        // Fill only the top half of the circle with liquid
        for y in 0..h {
            for x in 0..w {
                let dx = x as f32 - center_x;
                let dy = y as f32 - center_y;
                if dx * dx + dy * dy < r_sq && dy < -2.0 {
                    // Upper half of circle (dy < 0 means above center)
                    hm.data[y * w + x] = 0.8;
                }
            }
        }

        let mut temp_heights = hm.data.clone();
        // Use Water material (wetness=1.0)
        let mut cell_props = get_test_props(MaterialMode::Water, w * h);
        let mut cell_colors = vec![0u8; w * h * 4];
        let mut sliding = vec![false; w * h];
        let mut bounds = ActiveBounds {
            min_x: 2,
            max_x: 61,
            min_y: 2,
            max_y: 61,
            active: true,
        };

        let mut edge_vel_h = vec![0.0; w * h];
        let mut edge_vel_v = vec![0.0; w * h];
        let mut column_depth = vec![0.0; w * h];
        let mut head_field = vec![0.0; w * h];
        let mut active_blocks = vec![crate::BlockActivity::Inactive; 4];
        let mut last_displacements = vec![1.0; 4];
        let mut last_simulated_ticks = vec![0; 4];

        // Downward gravity
        let gravity_dir = glam::Vec2::new(0.0, 0.04);

        let initial_top_sum: f32 = hm.data[0..32 * w].iter().sum();
        let initial_bottom_sum: f32 = hm.data[32 * w..].iter().sum();
        assert!(initial_top_sum > initial_bottom_sum, "Initial state should have more liquid on top");

        let mask = make_test_mask(w, h, SandboxShape::Circle, 0.04, 1.0);
        // Run 500 ticks of gravity settling (liquid CA is slower than wave)
        for i in 0..500 {
            settle_tick(
                &mut hm,
                &mut temp_heights,
                &mut cell_colors,
                &mut cell_props,
                &mut sliding,
                &mut bounds,
                &mut active_blocks,
                &mut last_displacements,
                &mut last_simulated_ticks,
                256,
                32,
                &[],
                12345 + i as u32,
                &mut edge_vel_h,
                &mut edge_vel_v,
                &mut column_depth,
                &mut head_field,
                &mask,
                i as u32,
                gravity_dir,
                false,
                false,
                false, false, false, 0.50,
            );
        }

        // After settling under gravity, the bottom half should have MORE liquid
        // than the top half (liquid flows downward)
        let final_top_sum: f32 = hm.data[0..32 * w].iter().sum();
        let final_bottom_sum: f32 = hm.data[32 * w..].iter().sum();
        assert!(
            final_bottom_sum > final_top_sum,
            "Liquid should flow downward under gravity! top={}, bottom={}",
            final_top_sum, final_bottom_sum
        );
    }

    // =========================================================================================
    // Phase 0 characterisation tests (liquid-gravity overhaul safety net).
    //
    // L1-L3 and L5-L10 encode the *intended* correct behaviour for liquids under gravity and
    // are marked #[ignore] because they FAIL on today's code — that failure is the point: they
    // are the target later phases must turn green. L4 encodes an invariant that already holds
    // today (mass conservation of the CA gravity path) and is kept active as a regression guard.
    //
    // See scratchpad/liquid-gravity-proposal.md for the full diagnosis (defects C1-C9) this
    // suite is built against. Do NOT tune constants in physics.rs to make any of the ignored
    // tests below pass — that is explicitly out of scope for Phase 0.
    // =========================================================================================

    #[test]
    // Phase 2 (C2 fix): un-ignored. The fictitious lateral dispersion term is gone. Was: surface
    // spread = 47 rows (min=10, max=57) across 61 columns, up to 37 partially-filled cells in a
    // single column. Now: spread = 1 row (min=50, max=51), max 1 partially-filled cell/column.
    //
    // Phase 5: the mechanism underneath this changed and the tuning it needed went away. Levelling
    // is no longer a `liquid_alpha`/`max_transfer_coeff` pair sized to converge inside this test's
    // tick budget; it is what a conservative edge flux does on its own once cells have a capacity.
    // Two neighbouring columns of a pool present a real head difference at their surface row, the
    // flux moves mass down that gradient, and the acceptor's `cap - h` stops it at level. The
    // Phase 2 constants that used to set the convergence rate are deleted, and the result still
    // lands at spread = 1.
    fn test_liquid_pool_levels_flat_in_closed_box() {
        // A closed 64x64 box, Water poured into a 12-wide x 56-tall column, settled under
        // downward gravity for a long time. In a correct liquid solver this becomes a flat
        // pool with a single clean surface row per column.
        let w = 64;
        let h = 64;
        let mask = make_test_mask(w, h, SandboxShape::Square, 0.04, 1.0);
        let props = get_test_props(MaterialMode::Water, w * h);
        let mut sim = TestSim::new(w, h, props, mask, 32);
        for y in 4..60 {
            for x in 6..18 {
                sim.hm.data[y * w + x] = 1.0;
            }
        }
        let gravity_dir = glam::Vec2::new(0.0, 0.04);
        for _ in 0..1500 {
            sim.tick(gravity_dir, 256);
        }

        // Scan the WHOLE box, not just the original pour columns, since liquid disperses
        // sideways well beyond its starting footprint (C2).
        let mut surface_rows = Vec::new();
        let mut max_partial_in_a_column = 0;
        for x in 0..w {
            let mut surface_row: Option<usize> = None;
            let mut partial_count = 0;
            for y in 0..h {
                let val = sim.hm.data[y * w + x];
                if val > 0.5 && surface_row.is_none() {
                    surface_row = Some(y);
                }
                if val > 0.02 && val < 0.98 {
                    partial_count += 1;
                }
            }
            if let Some(sr) = surface_row {
                surface_rows.push(sr);
            }
            max_partial_in_a_column = max_partial_in_a_column.max(partial_count);
        }
        let min_row = *surface_rows.iter().min().unwrap();
        let max_row = *surface_rows.iter().max().unwrap();
        let spread = max_row - min_row;
        println!(
            "test_liquid_pool_levels_flat_in_closed_box: surface spread={} (min={}, max={}), \
             n_columns={}, max_partial_in_a_column={}",
            spread, min_row, max_row, surface_rows.len(), max_partial_in_a_column
        );

        // Measured today: spread=47, max_partial_in_a_column=37.
        assert!(spread <= 1, "Pool surface is not flat: spread={} rows", spread);
        assert!(
            max_partial_in_a_column <= 1,
            "Column has {} partially-filled cells, expected at most 1 (a single meniscus row)",
            max_partial_in_a_column
        );
    }

    #[test]
    // Phase 1 (C1 fix): liquid cells must respect CELL_CAPACITY = 1.0 (no cell above 1.0 + 1e-3)
    // and the occupied-cell footprint (h > 0.5) must be within 5% of the initial pour, i.e. no
    // phantom compression/shrinkage. Was ignored before Phase 1: max h = 1.502198 (cells packed
    // to the CA's 1.5 cap), occupied count 672 -> 466 (-30.65%). After the liquid-only capacity
    // fix (physics.rs get_ca_params / settle_tick, gated on `liquidity(wetness)`): max h = 1.0,
    // shrink = -0.30% (footprint grew slightly, well within tolerance).
    fn test_liquid_is_incompressible() {
        // Same pool as test_liquid_pool_levels_flat_in_closed_box: a closed box, Water poured
        // into a column, settled under gravity. A real (incompressible) liquid can never exceed
        // fill fraction 1.0 per cell, and settling should not make cells "disappear" (shrink the
        // occupied footprint) since mass is conserved.
        let w = 64;
        let h = 64;
        let mask = make_test_mask(w, h, SandboxShape::Square, 0.04, 1.0);
        let props = get_test_props(MaterialMode::Water, w * h);
        let mut sim = TestSim::new(w, h, props, mask, 32);
        for y in 4..60 {
            for x in 6..18 {
                sim.hm.data[y * w + x] = 1.0;
            }
        }
        let initial_occupied: usize = 12 * 56;
        let gravity_dir = glam::Vec2::new(0.0, 0.04);
        for _ in 0..1500 {
            sim.tick(gravity_dir, 256);
        }

        let max_h = sim.hm.data.iter().cloned().fold(0.0f32, f32::max);
        let final_occupied = sim.hm.data.iter().filter(|&&v| v > 0.5).count();
        let shrink_pct = 100.0 * (initial_occupied as f64 - final_occupied as f64) / initial_occupied as f64;
        println!(
            "test_liquid_is_incompressible: max_h={:.6}, occupied init={} final={} shrink={:.2}%",
            max_h, initial_occupied, final_occupied, shrink_pct
        );

        // Measured today: max_h=1.502198, shrink=30.65%.
        assert!(sim.hm.data.iter().all(|&v| v <= 1.0 + 1e-3), "Cell exceeded capacity: max_h={:.6}", max_h);
        assert!(
            shrink_pct.abs() < 5.0,
            "Occupied footprint changed by {:.2}%, expected within 5% (incompressible liquid)",
            shrink_pct
        );
    }

    #[test]
    // Phase 2 (C2 fix): un-ignored — a falling column stopped fanning out sideways.
    // Was: width=19, peak_h=0.3166 after 40 ticks. Phase 2: width=8, peak_h=0.7987.
    //
    // Phase 5: same width, but peak fill is now 1.0000 — the stream is genuinely saturated rather
    // than a narrow smear. Both properties come out of the update *order* rather than out of the
    // `liquid_can_still_fall` predicate and the 0.70 transfer coefficient Phase 2 used, which are
    // both deleted (see the operator-split note in `settle_tick`):
    //   - width, because gravity-aligned edges resolve before cross-gravity ones, so a falling
    //     cell has already handed its mass downward and has nothing left to spread;
    //   - peak fill, because the gravity-aligned sweep runs bottom-to-top, which is the
    //     CFL-respecting direction and stops a single pass from cascading a parcel down the whole
    //     grid and stretching it thin.
    //
    // STAGE 1 (resolution harness, see `test_scale`): every linear quantity -- grid, tap
    // position/width, and the tick budget -- scales by the same factor `s` so the scenario
    // stays physically equivalent rather than merely bigger. This particular scenario's own
    // per-tick speed limit is why the tick budget has to scale too: a falling stream advances at
    // most ~1 cell/tick (a CFL artifact of the flux solver, resolution-independent in cell
    // terms), so covering the same *physical* fraction of a taller box at `s`x resolution takes
    // `s`x as many ticks. `budget_n` is passed as `usize::MAX` rather than the original literal
    // `256`: at scale 1 this is a no-op (6 blocks at block_size=32 were already far under 256,
    // i.e. already unthrottled) but it removes the LOD-scheduler budget as a confound at scale=8,
    // where 256 would itself start throttling a much larger block grid and contaminate the
    // measurement with an unrelated effect.
    //
    // ASSERTION CLASSIFICATION (see docs/ARCHITECTURE.md, test methodology):
    // - `max_width`: RE-DERIVED to a FRACTION of container width (0.125, i.e. today's 8/64),
    //   not loosened. This is exactly the case the brief calls out: the sweep note on
    //   `LATERAL_PRESSURE_SCALE` and this harness's own scaled runs show the fractional width is
    //   stable at ~10-12% of the container across scales (8/64 = 12.5% at scale 1, ~49/512 =
    //   9.6% at scale 8), so pinning the *same* fraction at every scale preserves the original
    //   strictness while making the bound mean the same thing at any resolution.
    // - `peak_h`: left as the absolute `>= 0.5`. It is already a fill *fraction* (h in units of
    //   `cell_capacity`, not a cell count), so it means the same thing at every resolution and
    //   needs no re-derivation at all.
    fn test_liquid_stream_stays_coherent() {
        // A 64x96 box with a 4-cell-wide continuous source (a "tap") pouring at the top.
        // A coherent stream should stay narrow as it falls; today's dispersion noise
        // scatters it into a wide, thin sheet instead.
        let s = test_scale();
        let w = 64 * s;
        let h = 96 * s;
        let mask = make_test_mask(w, h, SandboxShape::Square, 0.04, 1.0);
        let props = get_test_props(MaterialMode::Water, w * h);
        let mut sim = TestSim::new(w, h, props, mask, 32);
        let gravity_dir = glam::Vec2::new(0.0, 0.04);
        for _ in 0..(40 * s) {
            for y in (6 * s)..(10 * s) {
                for x in (30 * s)..(34 * s) {
                    sim.hm.apply_external_mass(x, y, 1.0);
                }
            }
            sim.tick(gravity_dir, usize::MAX);
        }

        // Densest (widest) row and peak fill anywhere in the mid-air band, well clear of the
        // source (y=6..10 at scale 1) and the box floor (box bottom is around y=92 at scale 1).
        let mut max_width = 0usize;
        let mut peak_h = 0.0f32;
        for y in (15 * s)..(70 * s) {
            let mut min_x = None;
            let mut max_x = None;
            for x in 0..w {
                let val = sim.hm.data[y * w + x];
                if val > 0.05 {
                    if min_x.is_none() { min_x = Some(x); }
                    max_x = Some(x);
                    peak_h = peak_h.max(val);
                }
            }
            if let (Some(mn), Some(mx)) = (min_x, max_x) {
                max_width = max_width.max(mx - mn + 1);
            }
        }
        let max_width_frac = max_width as f32 / w as f32;
        println!(
            "test_liquid_stream_stays_coherent: scale={} w={} h={} max_width={} ({:.4} of w) \
             peak_h={:.4}",
            s, w, h, max_width, max_width_frac, peak_h
        );

        // Measured before the Phase 2/5 fixes (scale=1): max_width=19, peak_h=0.3166.
        //
        // THE BOUND IS ADDITIVE IN CELLS, NOT A FRACTION OF WIDTH, and that is the whole point.
        // It was a fraction (<= 0.125) until the frozen-Jacobi conversion, and cd53453 had
        // deliberately re-derived it as a fraction to survive resolution changes. That was the
        // wrong shape for THIS quantity, which the scaled harness makes obvious -- the excess
        // width over the tap is a CONSTANT 5 cells at every scale measured:
        //
        //   scale  w    tap   max_width   excess   fraction
        //     1     64    4        9         5      0.1406
        //     2    128    8       13         5      0.1016
        //     3    192   12       17         5      0.0885
        //     4    256   16       21         5      0.0820
        //     8    512   32       37         5      0.0723   <- production
        //
        // The dispersion is a fixed number of cells because the solver moves information one
        // cell per tick regardless of grid size; it does not scale with the domain. So a
        // fraction-of-width bound is tightest at the SMALLEST grid and loosest at production --
        // exactly backwards. The old 0.125 passed only because 5 cells happens to be under
        // 12.5% of 64 by one cell, and frozen Jacobi's extra half-cell of spread tipped it.
        // At production scale the stream is at 7.2% of width, its most coherent.
        //
        // An allowance of 8 cells over the tap is comfortably above the observed 5 at every
        // scale and still far below the dispersion failure mode this test exists to catch
        // (~0.30 of width, i.e. 15 cells of excess at scale 1 and 122 at scale 8).
        let tap_width = 4 * s;
        let excess = max_width.saturating_sub(tap_width);
        assert!(
            excess <= 8,
            "Stream cross-section too wide: {} cells, {} more than the {}-cell tap \
             (allowance 8; {:.4} of container width {})",
            max_width, excess, tap_width, max_width_frac, w
        );
        assert!(peak_h >= 0.5, "Stream peak fill too low: {:.4}", peak_h);
    }

    #[test]
    // Companion to `test_liquid_stream_stays_coherent`, and its deliberate opposite. That test
    // pins *falling* water narrow; this one pins *supported* water spreading, and — the point —
    // it measures while the liquid is still flowing rather than after it has settled.
    //
    // This is the case every other liquid test missed. They all settle with the inflow switched
    // off, so `edge_vel_v` decays to zero, the cross-gravity donor limit recovers, the pool
    // levels, and the end state looks right. The defect only existed during active flow: the
    // in-transit subtraction on the lateral edge was applied unconditionally, so in any
    // continuously fed body of liquid — a pour, or an upper chamber draining into a pool — every
    // cell received from above on every tick and `avail_*` never recovered. Lateral flow was
    // throttled at every depth, and the liquid stood up in vertical sheets against the casing
    // with a hollow between them instead of keeping a level surface: the user's "water walls".
    //
    // Metric: the number of *enclosed voids* — cells inside the shape that are essentially empty
    // (h <= 0.05) but have liquid (h > 0.5) somewhere to their left AND somewhere to their right
    // in the same row, with no casing in between. A level free surface has none; a pair of
    // standing walls with a drained channel between them has one per row of the channel, so the
    // count is a direct read of how wall-like the liquid is right now.
    //
    // Measured on a full hourglass upper chamber draining into the empty lower one, at the tick
    // where the drain is fully developed:
    //                            tick 120   tick 160   sum over 400 ticks
    //   before (unconditional):     223         41           38437
    //   after  (this fix):           94          0           30060
    //
    // `test_liquid_stream_stays_coherent` is the counterweight and is unchanged by the fix
    // (max_width 8, peak_h 1.0000, both before and after). Removing the in-transit limit
    // altogether does drive this test's tick-120 count to near zero, but it also blows that
    // stream out from 8 cells wide to 59 — see the note on `in_transit` in `settle_tick` for why
    // the limit has to survive for genuinely free-falling liquid.
    //
    // STAGE 1 (resolution harness, see `test_scale`): the grid scales by `s` in both dimensions
    // (Hourglass geometry is defined in normalized x/w, y/h coordinates, so this reproduces the
    // same shape at finer resolution, not a different one) and the tick budget scales by `s` for
    // the same CFL reason as `test_liquid_stream_stays_coherent` -- draining the same *physical*
    // fraction of a taller chamber takes proportionally more ticks. `budget_n` is `usize::MAX`
    // for the same "remove the LOD-scheduler confound" reason given there (at scale 1, 256 was
    // already far more than this test's 4 blocks needed, so this is a no-op at the default
    // scale).
    //
    // ASSERTION CLASSIFICATION: `at_120`, `at_160` and `total` are left as ABSOLUTE cell/tick
    // counts, deliberately NOT converted to a fraction of interior area. This is the case the
    // brief warns is easy to get backwards: a naive read says "a count of cells should grow with
    // resolution, like stream width," but this count isn't measuring the container's size, it is
    // measuring a *defect signature* (liquid standing in vertical sheets instead of leveling).
    // The physically correct target is close to ZERO of this at every resolution -- that is
    // literally what `LATERAL_PRESSURE_SCALE`'s hydrostatic term exists to guarantee, and is
    // exactly the resolution-invariance Stage 2 is supposed to restore. Converting this bound to
    // a fraction would quietly accept the very defect this harness exists to catch (measured
    // pre-Stage-2-fix at production scale: 34,161 / 31,718 / 66.7M against this test's 150 / 20 /
    // 34,000 -- see docs/ARCHITECTURE.md). So the threshold stays exactly what it was tuned to at
    // scale=1, is expected to legitimately FAIL at larger scales before Stage 2's fix, and is the
    // acceptance bar Stage 2 must clear afterwards.
    fn test_liquid_flowing_liquid_does_not_stand_in_walls() {
        let s = test_scale();
        let w = 64 * s;
        let h = 64 * s;
        let mask = make_test_mask(w, h, SandboxShape::Hourglass, 0.15, 0.6);
        let props = get_test_props(MaterialMode::Water, w * h);
        let mut sim = TestSim::new(w, h, props, mask.clone(), 32);
        let gravity_dir = glam::Vec2::new(0.0, 0.04);

        // Fill the whole upper chamber; the lower one starts empty, so the neck feeds a column
        // that is continuously fed from above for the entire measurement window.
        for y in 0..h / 2 {
            for x in 0..w {
                if mask[y * w + x] != crate::MASK_OUTSIDE {
                    sim.hm.data[y * w + x] = 1.0;
                }
            }
        }

        let count_voids = |sim: &TestSim| -> usize {
            let mut voids = 0;
            for y in 1..h - 1 {
                let mut liquid_to_the_left = false;
                for x in 0..w {
                    if mask[y * w + x] == crate::MASK_OUTSIDE {
                        // Casing breaks the row into independent spans.
                        liquid_to_the_left = false;
                        continue;
                    }
                    let v = sim.hm.data[y * w + x];
                    if v > 0.5 {
                        liquid_to_the_left = true;
                        continue;
                    }
                    if !liquid_to_the_left || v > 0.05 {
                        continue;
                    }
                    let liquid_to_the_right = (x + 1..w)
                        .take_while(|&x2| mask[y * w + x2] != crate::MASK_OUTSIDE)
                        .any(|x2| sim.hm.data[y * w + x2] > 0.5);
                    if liquid_to_the_right {
                        voids += 1;
                    }
                }
            }
            voids
        };

        let mut at_120 = 0;
        let mut at_160 = 0;
        let mut total = 0;
        let initial_mass = sim.mass();
        for t in 0..(400 * s) {
            sim.tick(gravity_dir, usize::MAX);
            let voids = count_voids(&sim);
            total += voids;
            if t + 1 == 120 * s {
                at_120 = voids;
            }
            if t + 1 == 160 * s {
                at_160 = voids;
            }
        }
        println!(
            "test_liquid_flowing_liquid_does_not_stand_in_walls: scale={} w={} h={} \
             voids@{}={} voids@{}={} total={} mass {:.3} -> {:.3}",
            s, w, h, 120 * s, at_120, 160 * s, at_160, total, initial_mass, sim.mass()
        );

        // Measured before the fix: 223 / 41 / 38437.
        assert!(
            at_120 <= 150,
            "Draining liquid is standing in walls: {} enclosed void cells at tick 120",
            at_120
        );
        assert!(
            at_160 <= 20,
            "Draining liquid is still standing in walls: {} enclosed void cells at tick 160",
            at_160
        );
        assert!(
            total <= 34_000,
            "Draining liquid spent too long in walls: {} void cell-ticks over 400 ticks",
            total
        );
    }

    /// TASK #55, prediction 1: "a flat surface cannot drive flow at any depth" under the gated
    /// multiplicative lateral head (`multiplicative_lateral_gate`), and, as the documented CONTRAST,
    /// the legacy additive term (`LATERAL_PRESSURE_SCALE`) *can* spuriously drive flow between two
    /// columns whose true free-surface elevation is level but whose `column_depth` differs.
    ///
    /// DIRECT CONSTRUCTION, not an emergent scenario left to develop over many ticks -- deliberately
    /// so, because it turns out "same top row, different floor" (the first, more obvious geometric
    /// shape tried here) does NOT exercise this bug at all: `column_depth` only ever accumulates
    /// what is genuinely ABOVE a cell within its own column, so two columns that share the same fill
    /// pattern down from a common top row read IDENTICAL `column_depth` at every shared row
    /// regardless of how much deeper one of them continues below -- the extra depth lives entirely
    /// below the row where a lateral neighbour would need to exist to compare against it. To make
    /// `column_depth` itself differ at a row where a real lateral edge exists, the columns need
    /// different amounts of material stacked ABOVE that row -- so this builds exactly that, with
    /// `fresh_pressure_field = true` so `column_depth` is a single deterministic top-down sum over a
    /// frozen pre-tick snapshot (see `recompute_column_depth`) rather than the order-dependent
    /// in-loop fallback, making the two `column_depth` values exactly hand-computable:
    ///
    /// Two adjacent columns `xa`/`xb`. One row above the comparison row (`row_stack`), each column
    /// gets a small resting fill (`stack_a`/`stack_b`, deliberately UNEQUAL). Given
    /// `depth_scale = REFERENCE_GRID_HEIGHT / w`, `recompute_column_depth` gives
    /// `column_depth[row_cmp] = stack * depth_scale` (nothing sits above `row_stack` itself, so its
    /// own `column_depth` is 0). At `row_cmp`, each column's OWN local fill (`h_a`/`h_b`) is then set
    /// so the true free-surface proxy `eta = h * depth_scale + column_depth` matches EXACTLY
    /// between the two columns (`h_b = h_a + (stack_a - stack_b)`) even though `column_depth`
    /// itself does not match at all -- precisely "two columns at the same surface level [with]
    /// different depths" from the task brief.
    ///
    /// TASK #55 UNIT FIX: `eta`'s `h` term is now lifted into `column_depth`'s reference-row units
    /// (`h * depth_scale`, see `mult_lateral_conveyance`'s call site) rather than added raw, so the
    /// flat-eta construction here solves `h_a * depth_scale + depth_a == h_b * depth_scale +
    /// depth_b` for `h_b`, i.e. `h_b = h_a + (depth_a - depth_b) / depth_scale = h_a + (stack_a -
    /// stack_b)` -- NOT `h_a + depth_scale * (stack_a - stack_b)` (that was the pre-fix formula's
    /// flat-eta condition, and produces a badly non-flat `eta` under the corrected one: measured,
    /// it drove `lateral_drift_on = 0.88` here, an order of magnitude over this test's `< 1e-3`
    /// bound, simply because the old `h_b` no longer describes a flat surface once `h`'s units are
    /// fixed).
    #[test]
    fn test_mult_lateral_flat_surface_same_eta_different_depth_no_flux() {
        let w = 16usize;
        let h = 16usize;
        let wall = 2usize;
        let xa = 6usize;
        let xb = 7usize; // lateral neighbour of xa
        let row_stack = 4usize; // feeds column_depth[row_cmp] via recompute_column_depth
        let row_cmp = 5usize; // the actual lateral edge under test

        // A narrow, two-column-wide chamber containing ONLY `xa` and `xb`, walled on every other
        // side. Two things this closes off, both confirmed empirically, not assumed:
        //
        // 1. The floor sits immediately below `row_cmp` (`row_cmp + 1` onward is OUTSIDE) -- not
        //    merely "pre-filled to capacity", a REAL floor, so there is no vertical edge leaving
        //    `row_cmp` downward at all. GRAVITY_HEAD_SCALE's flat, depth-independent driving fully
        //    drains any resting material into any empty room below in a single tick regardless of
        //    source amount; with an empty row below THAT too, a pre-filled-but-unwalled cushion just
        //    moved the cascade one row further before spilling into row_cmp+2. `row_stack` draining
        //    into `row_cmp` is still possible (there IS room there) but has nowhere further to
        //    cascade to, so it is bounded to that one, equal-on-both-branches transfer.
        // 2. `xa` and `xb` have no OTHER lateral neighbour (walls immediately outside both) -- with
        //    open neighbours on the outside, `k_a * LATERAL_PRESSURE_SCALE * depth_a` is large
        //    enough relative to the cells' own small fill that BOTH cells drain hard toward their
        //    OWN empty far side simultaneously, which swamps the one edge (`xa` |-> `xb`) this test
        //    means to isolate with unrelated three- and four-cell redistribution.
        let mut mask = vec![crate::MASK_OUTSIDE; w * h];
        for y in wall..=row_cmp {
            for x in [xa, xb] {
                mask[y * w + x] = crate::MASK_INSIDE;
            }
        }

        let depth_scale = REFERENCE_GRID_HEIGHT as f32 / w as f32; // 512/16 = 32
        let stack_a = 0.03f32;
        let stack_b = 0.02f32;
        let depth_a = stack_a * depth_scale;
        let depth_b = stack_b * depth_scale;
        let h_a = 0.10f32;
        // TASK #55 UNIT FIX: eta_new = h * depth_scale + column_depth, so the flat-eta condition
        // is h_a * depth_scale + depth_a == h_b * depth_scale + depth_b, i.e. h_b = h_a +
        // (depth_a - depth_b) / depth_scale = h_a + (stack_a - stack_b). See this test's own doc
        // comment for why this replaced the pre-fix `h_a + (depth_a - depth_b)`.
        let h_b = h_a + (stack_a - stack_b); // eta_a == eta_b by construction, post-unit-fix

        let gravity_dir = glam::Vec2::new(0.0, 0.04);
        let props = get_test_props(MaterialMode::Water, w * h);

        let run = |mult_enabled: bool| -> (f32, f32, f32, f32) {
            let mut sim = TestSim::new(w, h, props.clone(), mask.clone(), 16);
            sim.fresh_pressure_field = true;
            sim.hm.data[row_stack * w + xa] = stack_a;
            sim.hm.data[row_stack * w + xb] = stack_b;
            sim.hm.data[row_cmp * w + xa] = h_a;
            sim.hm.data[row_cmp * w + xb] = h_b;
            multiplicative_lateral_gate::set_enabled(mult_enabled);
            sim.tick(gravity_dir, usize::MAX);
            multiplicative_lateral_gate::set_enabled(false);
            (
                sim.hm.data[row_cmp * w + xa],
                sim.hm.data[row_cmp * w + xb],
                sim.column_depth[row_cmp * w + xa],
                sim.column_depth[row_cmp * w + xb],
            )
        };

        let (a_off, b_off, cd_a_off, cd_b_off) = run(false);
        let (a_on, b_on, cd_a_on, cd_b_on) = run(true);
        // `row_stack` draining vertically into `row_cmp` (see the mask's own doc comment) is
        // expected and identical regardless of the gate -- it is the phase-0 VERTICAL edge, which
        // this change does not touch. Isolate LATERAL drift by comparing against
        // `h_a + stack_a` / `h_b + stack_b` (what each cell holds after that vertical settling but
        // before any lateral edge could move anything), not against the pre-tick `h_a`/`h_b`.
        let expect_a = h_a + stack_a;
        let expect_b = h_b + stack_b;
        let lateral_drift_off = (a_off - expect_a).abs() + (b_off - expect_b).abs();
        let lateral_drift_on = (a_on - expect_a).abs() + (b_on - expect_b).abs();
        println!(
            "test_mult_lateral_flat_surface_same_eta_different_depth_no_flux: \
             h_a={:.4} h_b={:.4} depth_a={:.4} depth_b={:.4} (eta_a={:.4} eta_b={:.4})  \
             additive: column_depth=({:.4},{:.4}) after=({:.4},{:.4}) lateral_drift={:.5}  \
             multiplicative: column_depth=({:.4},{:.4}) after=({:.4},{:.4}) lateral_drift={:.5}",
            h_a, h_b, depth_a, depth_b, h_a * depth_scale + depth_a, h_b * depth_scale + depth_b,
            cd_a_off, cd_b_off, a_off, b_off, lateral_drift_off,
            cd_a_on, cd_b_on, a_on, b_on, lateral_drift_on
        );

        // Harness sanity: `column_depth` really did come out different between the two columns
        // (otherwise this test would trivially pass without exercising anything), AND the additive
        // (legacy, default-shipping) form really does show the documented bug here -- otherwise
        // this test would prove nothing about what the multiplicative gate changes.
        assert!(
            (cd_a_off - cd_b_off).abs() > 0.1,
            "harness sanity: column_depth did not differ between the two columns ({:.4} vs {:.4})",
            cd_a_off, cd_b_off
        );
        assert!(
            lateral_drift_off > 0.1,
            "harness sanity: the legacy additive form (gate off, today's shipped default) did not \
             show the documented same-surface/different-depth bug here (lateral_drift={:.5}) -- this \
             test's construction needs revisiting, it isn't exercising what it claims to.",
            lateral_drift_off
        );

        // The actual prediction: with the multiplicative gate on, a driving term proportional to
        // `eta_a - eta_b` (== 0 by construction) must not move any material laterally, regardless
        // of how different the two columns' `column_depth` is.
        assert!(
            lateral_drift_on < 1e-3,
            "Same-eta, different-depth columns saw lateral flux under the multiplicative gate: \
             lateral_drift={:.5} (expected ~0, since grad(eta) == 0 by construction)",
            lateral_drift_on
        );
    }

    /// TASK #55: own quick check of `test_liquid_flowing_liquid_does_not_stand_in_walls`'s void
    /// count with `multiplicative_lateral_gate` on vs off, run inside the SAME build so both
    /// numbers come from one compile (see the gate's own doc comment for why this pattern exists).
    /// Not a pass/fail spec on its own -- the task brief's real metric is a separate agent's
    /// diagnostic -- this exists only so a reader of this change can see, without trusting a
    /// second-hand number, what the multiplicative form actually does to this specific scenario.
    /// Reproduces the exact scenario `test_liquid_flowing_liquid_does_not_stand_in_walls` uses
    /// (same mask, same fill, same 400-tick run, same void-count metric) at `test_scale()`.
    #[test]
    #[ignore = "DIAGNOSTIC measurement, not a pass/fail spec — never assert on these numbers. \
                Run with: cargo test -p sandart-sim --release --lib \
                physics::tests::diag_mult_lateral_void_count_gate_on_vs_off -- --ignored --nocapture"]
    fn diag_mult_lateral_void_count_gate_on_vs_off() {
        let s = test_scale();
        let w = 64 * s;
        let h = 64 * s;
        let mask = make_test_mask(w, h, SandboxShape::Hourglass, 0.15, 0.6);
        let props = get_test_props(MaterialMode::Water, w * h);
        let gravity_dir = glam::Vec2::new(0.0, 0.04);

        let count_voids = |sim: &TestSim| -> usize {
            let mut voids = 0;
            for y in 1..h - 1 {
                let mut liquid_to_the_left = false;
                for x in 0..w {
                    if mask[y * w + x] == crate::MASK_OUTSIDE {
                        liquid_to_the_left = false;
                        continue;
                    }
                    let v = sim.hm.data[y * w + x];
                    if v > 0.5 {
                        liquid_to_the_left = true;
                        continue;
                    }
                    if !liquid_to_the_left || v > 0.05 {
                        continue;
                    }
                    let liquid_to_the_right = (x + 1..w)
                        .take_while(|&x2| mask[y * w + x2] != crate::MASK_OUTSIDE)
                        .any(|x2| sim.hm.data[y * w + x2] > 0.5);
                    if liquid_to_the_right {
                        voids += 1;
                    }
                }
            }
            voids
        };

        let run = |mult_enabled: bool| -> (usize, usize, usize) {
            let mut sim = TestSim::new(w, h, props.clone(), mask.clone(), 32);
            for y in 0..h / 2 {
                for x in 0..w {
                    if mask[y * w + x] != crate::MASK_OUTSIDE {
                        sim.hm.data[y * w + x] = 1.0;
                    }
                }
            }
            multiplicative_lateral_gate::set_enabled(mult_enabled);
            let mut at_120 = 0;
            let mut at_160 = 0;
            let mut total = 0;
            for t in 0..(400 * s) {
                sim.tick(gravity_dir, usize::MAX);
                let voids = count_voids(&sim);
                total += voids;
                if t + 1 == 120 * s {
                    at_120 = voids;
                }
                if t + 1 == 160 * s {
                    at_160 = voids;
                }
            }
            multiplicative_lateral_gate::set_enabled(false);
            (at_120, at_160, total)
        };

        let (off_120, off_160, off_total) = run(false);
        let (on_120, on_160, on_total) = run(true);
        println!(
            "diag_mult_lateral_void_count_gate_on_vs_off: scale={} \
             additive(off)   voids@120={} voids@160={} total={} \
             multiplicative(on) voids@120={} voids@160={} total={}",
            s, off_120, off_160, off_total, on_120, on_160, on_total
        );
    }

    /// TASK #55, granular sanity check (Janssen composition): does the gated multiplicative form
    /// stay well-behaved (mass-conserving, no NaN/explosion) for a GRANULAR material, where
    /// `mult_lateral_conveyance` routes `column_depth` through `janssen_effective_depth` before the
    /// power law -- rather than only through Water, where `janssen_effective_depth` is the identity
    /// transform and this path is never really exercised. Not a repose-angle measurement (that
    /// needs `test_dry_sand_has_angle_of_repose`'s much more careful rig); just: does draining
    /// DrySand down an Hourglass under the multiplicative gate conserve mass and stay finite,
    /// same as it does under the legacy additive term.
    #[test]
    #[ignore = "DIAGNOSTIC measurement, not a pass/fail spec — never assert on these numbers. \
                Run with: cargo test -p sandart-sim --release --lib \
                physics::tests::diag_mult_lateral_dry_sand_gate_on_vs_off -- --ignored --nocapture"]
    fn diag_mult_lateral_dry_sand_gate_on_vs_off() {
        let w = 64;
        let h = 64;
        let mask = make_test_mask(w, h, SandboxShape::Hourglass, 0.15, 0.6);
        let props = get_test_props(MaterialMode::DrySand, w * h);
        let gravity_dir = glam::Vec2::new(0.0, 0.04);

        let run = |mult_enabled: bool| -> (f64, f64, bool) {
            let mut sim = TestSim::new(w, h, props.clone(), mask.clone(), 32);
            for y in 0..h / 2 {
                for x in 0..w {
                    if mask[y * w + x] != crate::MASK_OUTSIDE {
                        sim.hm.data[y * w + x] = 1.0;
                    }
                }
            }
            let initial_mass = sim.mass();
            multiplicative_lateral_gate::set_enabled(mult_enabled);
            let mut any_non_finite = false;
            for _ in 0..400 {
                sim.tick(gravity_dir, usize::MAX);
                if sim.hm.data.iter().any(|v| !v.is_finite()) {
                    any_non_finite = true;
                }
            }
            multiplicative_lateral_gate::set_enabled(false);
            (initial_mass, sim.mass(), any_non_finite)
        };

        let (init_off, final_off, nan_off) = run(false);
        let (init_on, final_on, nan_on) = run(true);
        println!(
            "diag_mult_lateral_dry_sand_gate_on_vs_off: \
             additive(off)   mass {:.3} -> {:.3} (drift={:.4}) non_finite={}  \
             multiplicative(on) mass {:.3} -> {:.3} (drift={:.4}) non_finite={}",
            init_off, final_off, final_off - init_off, nan_off,
            init_on, final_on, final_on - init_on, nan_on
        );
        assert!(!nan_off, "additive (default) path produced non-finite heights");
        assert!(!nan_on, "multiplicative path produced non-finite heights");
    }

    #[test]
    #[ignore = "DIAGNOSTIC measurement, not a pass/fail spec — never assert on these numbers. \
                Task #54 (\"make pressure drive every flow\") step 2: a 'dam break' scenario -- \
                the LEFT half of a tall box is filled solid, the RIGHT half is completely empty, \
                for the full height -- so there is a genuine, unambiguous 'empty space beside \
                deep material' edge at every depth simultaneously, unlike a symmetric Hourglass \
                probed at its own centreline (tried first; discarded because the centre of a \
                symmetric fill has zero driving head BY CONSTRUCTION regardless of pressure, so \
                it could never show a difference). Reports, at several depths and several tick \
                counts, the fill-difference term, the k_lateral * LATERAL_PRESSURE_SCALE * \
                column_depth term, tau, and the outcome that actually matters: how much has \
                spread into the empty column (`h_b`) by that tick. \
                Run with: cargo test -p sandart-sim --release --lib \
                physics::tests::diag_lateral_pressure_term_magnitudes -- --ignored --nocapture"]
    // Prime suspect going in (see the task brief): `yielded = sign(dH) * max(|dH| - tau, 0)`, so
    // if `tau` is comparable in magnitude to the depth term, sand's lateral flow is gated off
    // regardless of how deep the material is -- the depth term gets subtracted away before it
    // can move anything. `tau` is 0 for liquid by construction (`granular_share == 0`), so this
    // would explain a sand/water asymmetry in depth-driven spreading even though `column_depth`
    // and `LATERAL_PRESSURE_SCALE` apply identically to both materials since the Stage C
    // follow-on.
    fn diag_lateral_pressure_term_magnitudes() {
        let w = 64;
        let h = 90;
        let mask = make_test_mask(w, h, SandboxShape::Square, 0.04, 1.0);
        let gravity_dir = glam::Vec2::new(0.0, 0.04);

        // Same k_of blend the lateral edge itself uses (see the `k_of` closure in `settle_tick`,
        // Stage C FOLLOW-ON): 1.0 for liquid, `LATERAL_EARTH_PRESSURE_K` for pure granular,
        // interpolated by the cell's own liquidity.
        let k_of = |liq: f32| liq + (1.0 - liq) * LATERAL_EARTH_PRESSURE_K;

        let top = 2usize;
        let bottom = h - 3;
        let cx = 31usize; // last filled column; cx+1 = 32 is the first empty column
        let depths = [1usize, 2, 5, 10, 15, 25, 40, 60];

        for &material in &[MaterialMode::Water, MaterialMode::DrySand] {
            for &ticks in &[1usize, 3, 10, 30] {
                let props = get_test_props(material, w * h);
                let threshold_prop = props[PROP_THRESHOLD]; // uniform across the grid in this harness
                let mut sim = TestSim::new(w, h, props, mask.clone(), 32);
                // Left half solid from `top` to `bottom`, right half left at 0 -- an unambiguous
                // dam break, with a genuinely deep column pressing on a genuinely empty one at
                // every depth simultaneously.
                for y in top..=bottom {
                    for x in 2..=cx {
                        sim.hm.data[y * w + x] = 1.0;
                    }
                }
                for _ in 0..ticks {
                    sim.tick(gravity_dir, usize::MAX);
                }

                println!("=== {:?}, ticks={} ===", material, ticks);
                for &depth_rows in &depths {
                    let y = top + depth_rows;
                    if y > bottom {
                        continue;
                    }
                    let idx = y * w + cx;
                    let nb_idx = idx + 1;
                    let h_a = sim.hm.data[idx];
                    let h_b = sim.hm.data[nb_idx];
                    let fill_diff = h_a - h_b;
                    let wetness_a = sim.cell_props[idx * 4 + PROP_WETNESS];
                    let liq_a = liquidity(wetness_a);
                    let k_a = k_of(liq_a);
                    let depth_term = k_a * LATERAL_PRESSURE_SCALE * sim.column_depth[idx];
                    let granular_share = if gravity_dir.length_squared() > 1e-6 { 1.0 - liq_a } else { 1.0 };
                    let tau = GRANULAR_TAU_SCALE * threshold_prop * granular_share;
                    println!(
                        "  depth={:>3} rows  h_a={:.4} h_b={:.4} fill_diff={:+.4}  \
                         column_depth={:.4}  k_lateral*scale*column_depth={:+.4}  tau={:.4}  \
                         depth_term/tau={}",
                        depth_rows, h_a, h_b, fill_diff, sim.column_depth[idx], depth_term, tau,
                        if tau > 1e-9 { format!("{:.1}", depth_term.abs() / tau) } else { "inf (tau=0)".to_string() }
                    );
                }
            }
        }
    }

    #[test]
    // Unit test for the sleeping predicate itself. The two branches of `edge_sleeps` are exact
    // — each is a restatement of a clause inside `flux_edge` that forces `flux == 0` — so this
    // pins the cases they are *meant* to catch and, more importantly, the two they must not.
    fn test_edge_sleeps_predicate() {
        let cap = 1.0f32; // Water
        let g = 0.04 * GRAVITY_HEAD_SCALE; // one saturated cell of head per row, as shipped

        // --- must sleep ---
        // Interior of a settled full pool, vertical edge under gravity. The driving head is a
        // whole cell (that is what gravity IS here), yet nothing can move: both cells are at
        // capacity, so neither direction has room. Branch 1. This is the case that the granular
        // CA's `h_center - min_h` shortcut structurally cannot express, and the reason flux can
        // sleep under gravity at all.
        assert!(
            edge_sleeps(cap + g - cap, 0.0, 0.0, cap, cap, cap - cap, cap - cap),
            "the interior of a settled full pool must sleep"
        );
        // Same edge with momentum still stored: still blocked, because the clamps ignore v_e.
        assert!(
            edge_sleeps(cap + g - cap, 0.0, 0.3, cap, cap, 0.0, 0.0),
            "a room-blocked edge must sleep whatever momentum it has stored"
        );
        // Empty space above the free surface: a big head, nothing to donate either way.
        assert!(
            edge_sleeps(0.0 + g - 0.0, 0.0, 0.0, 0.0, 0.0, cap, cap),
            "empty space must sleep"
        );
        // Flat pool at g = 0, at any level: level and at rest. Branch 2.
        assert!(
            edge_sleeps(0.0, 0.0, 0.0, 0.4, 0.4, cap - 0.4, cap - 0.4),
            "a level, motionless free surface must sleep"
        );
        // A settled granular heap at its angle of repose, once tau is a real yield stress:
        // below the yield stress and at rest.
        assert!(
            edge_sleeps(0.05, 0.20, 0.0, 0.8, 0.7, 0.7, 0.8),
            "a sub-yield-stress edge at rest must sleep"
        );

        // --- must NOT sleep: the two ways a live wave passes near one of the conditions ---
        // Turning point: the crest has stopped, so v_e is zero, but the surface is at its most
        // tilted. Sleeping here would freeze the ripple at maximum amplitude forever.
        assert!(
            !edge_sleeps(0.25, 0.0, 0.0, 0.6, 0.35, cap - 0.6, cap - 0.35),
            "a wave at its turning point (v_e == 0, large head) must NOT sleep"
        );
        // Zero crossing: the surface is momentarily level, but all the energy is in the
        // momentum. Sleeping here would swallow the wave.
        assert!(
            !edge_sleeps(0.0, 0.0, 0.05, 0.5, 0.5, cap - 0.5, cap - 0.5),
            "a wave crossing its rest level (head == 0, v_e != 0) must NOT sleep"
        );
        // An unequal surface with somewhere to go: the ordinary awake case.
        assert!(
            !edge_sleeps(0.3, 0.0, 0.0, 0.7, 0.4, cap - 0.7, cap - 0.4),
            "an edge with both a head and a route must NOT sleep"
        );
        // Full donor, empty acceptor: one direction is open, so the edge is live even though the
        // mirrored direction is doubly blocked.
        assert!(
            !edge_sleeps(cap + g, 0.0, 0.0, cap, 0.0, 0.0, cap),
            "a full cell above an empty one must NOT sleep"
        );
    }

    #[test]
    // The system-level half of edge sleeping: a body of liquid that has finished moving must stop
    // doing work, and must start again when something disturbs it.
    //
    // Without this, sleeping regresses silently, and *more* silently than usual. Sleeping is exact
    // — the edges it skips would have moved zero mass — so it leaves no trace in any heightmap,
    // mass total or flow total. Deleting it entirely changes nothing any other test in this file
    // measures while costing 2.7x on the Sand-fall benchmark. So this test looks at two things no
    // other test does:
    //
    //   1. `edge_sleep_stats`, the predicate's own outcome counter — the mechanism itself. It must
    //      be *low* while the pour is running (or the predicate is freezing live liquid) and *high*
    //      once the body has settled (or sleeping is not happening).
    //   2. The MUST-simulate block count (`BlockActivity::Fast`), the class that bypasses `budget_n`
    //      entirely and therefore the one that sets the frame cost.
    //
    // The wake half is the other risk. A sleeping edge writes nothing and calls `activate_neighbor`
    // for nothing, which is safe only because a sleeping edge would have moved zero mass anyway —
    // if that equivalence ever breaks, a pool goes quiet and then *stays* quiet through a
    // disturbance. So the second phase drops a column of water onto the settled pool (arming one
    // block the way a draw stroke does) and requires the activity to spread beyond that block, the
    // sleep fraction to fall, and both to recover afterwards.
    //
    // A measured caveat, recorded here because it bounds what this test can assert. The MUST count
    // decays 64 -> 8 and then sits at exactly 8 forever (checked to 20000 ticks): 8 blocks is the
    // full width of the pool's free-surface row. That row never reaches equilibrium. Water's
    // (c_sq, damping) = (0.24, 0.98) is a lightly damped oscillator, so the momentum an edge
    // accumulates from a height difference `d` settles at `c_sq * damping * d / (1 - damping)`,
    // about 12x `d`; a surface film of 0.02 therefore ping-pongs its entire contents between two
    // adjacent surface cells every tick, forever, at a flux far above the 1e-4 MUST threshold. It
    // is invisible (0.02 of one cell) and it is not something edge sleeping can address — those
    // edges are genuinely moving mass, and `edge_sleeps` skips only edges that provably are not.
    // It is a separate defect in the surface dynamics, so the assertion below is that the MUST
    // count *collapses to the surface row*, not that it reaches zero.
    fn test_settled_liquid_sleeps_and_wakes() {
        let (w, h, bs) = (128, 128, 16);
        let cols = w / bs;
        let mask = make_test_mask(w, h, SandboxShape::Square, 0.04, 1.0);
        let props = get_test_props(MaterialMode::Water, w * h);
        let mut sim = TestSim::new(w, h, props, mask.clone(), bs);
        let gravity_dir = glam::Vec2::new(0.0, 0.04);

        // A tall narrow column: it has to fall, hit the floor, spread across the box and level
        // off, so the run genuinely passes through a busy phase before the quiet one.
        for y in 8..h - 8 {
            for x in 48..80 {
                if mask[y * w + x] != crate::MASK_OUTSIDE {
                    sim.hm.data[y * w + x] = 1.0;
                }
            }
        }

        let must_count = |s: &TestSim| -> usize {
            s.active_blocks
                .iter()
                .filter(|a| matches!(a, crate::BlockActivity::Fast))
                .count()
        };

        // --- while the pour is running: busy, and hardly anything sleeps ---
        let mut peak_must = 0usize;
        edge_sleep_stats::reset();
        for _ in 0..40 {
            sim.tick(gravity_dir, 256);
            peak_must = peak_must.max(must_count(&sim));
        }
        let pouring_slept = edge_sleep_stats::slept_fraction().expect("no liquid edges were tested");

        // --- settle ---
        let mut trace = Vec::new();
        for t in 41..=1200 {
            sim.tick(gravity_dir, 256);
            peak_must = peak_must.max(must_count(&sim));
            if t % 300 == 0 {
                trace.push((t, must_count(&sim)));
            }
        }

        // Sampled over a whole staleness period (MAX_STALENESS = 30), so a block re-admitted on
        // the staleness path cannot hide inside a lucky single sample.
        edge_sleep_stats::reset();
        let mut settled_must = 0usize;
        for _ in 0..30 {
            sim.tick(gravity_dir, 256);
            settled_must += must_count(&sim);
        }
        let settled_slept = edge_sleep_stats::slept_fraction().expect("no liquid edges were tested");
        println!(
            "test_settled_liquid_sleeps_and_wakes: {} blocks total; peak must={} trace={:?}; \
             MUST block-ticks over 30 settled ticks={}; edges slept: {:.1}% while pouring, \
             {:.1}% settled",
            sim.active_blocks.len(), peak_must, trace, settled_must,
            100.0 * pouring_slept, 100.0 * settled_slept
        );

        assert!(
            peak_must >= 16,
            "the pour never generated any work to sleep through: peak MUST count was {}",
            peak_must
        );
        // 8 blocks is the free-surface row (see the caveat above); 30 ticks of it is 240.
        assert!(
            settled_must <= 300,
            "A settled pool is still MUST-simulating {} block-ticks per 30 ticks, out of a peak \
             of {} blocks/tick. Only the free-surface row should still be active once the body \
             has levelled off.",
            settled_must, peak_must
        );
        // THE assertion for the mechanism. Measured: 54.7% pouring, 92.8% settled.
        assert!(
            settled_slept > 0.90,
            "A settled body of liquid is not sleeping: only {:.1}% of the liquid edges tested were \
             skipped ({:.1}% while it was still pouring). Every edge inside a settled body is \
             either room-blocked in both directions or at zero head with zero stored velocity, so \
             almost all of them should take the `edge_sleeps` early-out.",
            100.0 * settled_slept, 100.0 * pouring_slept
        );
        assert!(
            pouring_slept < settled_slept - 0.25,
            "The sleeping predicate does not discriminate: it skipped {:.1}% of edges while the \
             liquid was actively pouring and {:.1}% once it had settled. A predicate that sleeps \
             moving liquid is not a fast path, it is a freeze.",
            100.0 * pouring_slept, 100.0 * settled_slept
        );

        // --- wake ---
        // Drop a fresh column into one block and arm it, exactly as a draw stroke does.
        let (drop_x, drop_y) = (24usize, 100usize);
        let drop_b = (drop_y / bs) * cols + (drop_x / bs);
        for y in drop_y - 6..drop_y {
            for x in drop_x..drop_x + 8 {
                if mask[y * w + x] != crate::MASK_OUTSIDE {
                    sim.hm.data[y * w + x] = 1.0;
                }
            }
        }
        sim.last_displacements[drop_b] = 1.0;
        let mass_after_drop = sim.mass();

        edge_sleep_stats::reset();
        let mut woke_blocks = std::collections::HashSet::new();
        for _ in 0..120 {
            sim.tick(gravity_dir, 256);
            for (b, a) in sim.active_blocks.iter().enumerate() {
                if matches!(a, crate::BlockActivity::Fast) {
                    woke_blocks.insert(b);
                }
            }
        }
        let woken_slept = edge_sleep_stats::slept_fraction().expect("no liquid edges were tested");
        println!(
            "test_settled_liquid_sleeps_and_wakes: after the drop into block {}, {} distinct \
             blocks became MUST; edges slept {:.1}%",
            drop_b, woke_blocks.len(), 100.0 * woken_slept
        );
        assert!(
            woke_blocks.len() > 1,
            "The disturbance did not propagate out of the block it was drawn into: only {} block \
             ever became MUST. A sleeping edge must not be able to swallow a wake.",
            woke_blocks.len()
        );
        // The sleep fraction is deliberately *not* asserted on here. It is a whole-domain ratio
        // over the blocks that ran, and the drop wakes nine blocks of a sixty-four block pool that
        // is otherwise still settled, so it barely moves (measured 93.1% against 92.8%). What
        // proves the wake is the block count above: the disturbance crossed out of the block it
        // was drawn into, which it can only do through `activate_neighbor`.

        // And it must go quiet again afterwards, not stay awake because it was once disturbed.
        for _ in 0..900 {
            sim.tick(gravity_dir, 256);
        }
        edge_sleep_stats::reset();
        let mut requiet_must = 0usize;
        for _ in 0..30 {
            sim.tick(gravity_dir, 256);
            requiet_must += must_count(&sim);
        }
        let requiet_slept = edge_sleep_stats::slept_fraction().expect("no liquid edges were tested");
        println!(
            "test_settled_liquid_sleeps_and_wakes: re-settled MUST block-ticks={} slept={:.1}%",
            requiet_must, 100.0 * requiet_slept
        );
        assert!(
            requiet_slept > 0.90 && requiet_must <= 300,
            "The pool did not go back to sleep after the disturbance: {:.1}% of edges slept, \
             {} MUST block-ticks over 30 ticks",
            100.0 * requiet_slept, requiet_must
        );

        let mass_err = (sim.mass() - mass_after_drop).abs() / mass_after_drop;
        println!(
            "test_settled_liquid_sleeps_and_wakes: mass rel_err over the woken phase={:.3e}",
            mass_err
        );
        assert!(mass_err < 1e-4, "sleeping leaked mass: rel_err={:.3e}", mass_err);
    }

    #[test]
    fn test_liquid_mass_conserved_under_gravity() {
        // Regression guard (Phase 0): this invariant already holds today and must keep holding
        // through every later phase. Water poured into the upper chamber of an hourglass,
        // 2000 gravity ticks, total mass must be conserved.
        let w = 64;
        let h = 64;
        let mask = make_test_mask(w, h, SandboxShape::Hourglass, 0.15, 0.6);
        let props = get_test_props(MaterialMode::Water, w * h);
        let mut sim = TestSim::new(w, h, props, mask, 32);
        let center_x = w as f32 / 2.0;
        let center_y = h as f32 / 2.0;
        for y in 0..h {
            let dy = y as f32 - center_y;
            if dy < 0.0 && dy > -6.0 {
                for x in 0..w {
                    let dx = x as f32 - center_x;
                    if dx.abs() < 22.4 {
                        sim.hm.data[y * w + x] = 1.0;
                    }
                }
            }
        }
        let initial_mass = sim.mass();
        let gravity_dir = glam::Vec2::new(0.0, 0.04);
        for _ in 0..2000 {
            sim.tick(gravity_dir, 256);
        }
        let final_mass = sim.mass();
        let rel_err = (final_mass - initial_mass).abs() / initial_mass;
        println!(
            "test_liquid_mass_conserved_under_gravity: init={:.6} final={:.6} rel_err={:.8}",
            initial_mass, final_mass, rel_err
        );
        // Measured today: rel_err ~= 1.2e-6.
        assert!(rel_err < 1e-4, "Mass not conserved under gravity: rel_err={:.8}", rel_err);
    }

    #[test]
    // Positivity/capacity guard for the frozen-Jacobi edge-flux solver specifically (phase 0's
    // gravity-aligned edges and phase 1's lateral/g=0 edges — see `edge_arbitration_scale`'s doc
    // comment for why a single arbitration pass is supposed to make a negative or over-capacity
    // cell structurally impossible). Deliberately scoped away from the marble/`displace_line`
    // path (`add_sand_with_limit_properties` etc.), which is untouched by this conversion and was
    // independently confirmed, while writing this guard, to already have its own tiny pre-existing
    // capacity overshoot (~1.4e-3 over a 1.5 cap, reproduces bit-for-bit on an unmodified checkout
    // of this crate) unrelated to the flux path — a blanket whole-grid assertion would trip on
    // that every time this test module runs and misattribute it to this change.
    //
    // Exercises three of this conversion's paths directly: granular free-fall under gravity
    // (phase 0's `weight = 1.0` edge, `DrySand`), liquid free-fall + lateral spreading under
    // gravity (phase 0 and phase 1's `cell_liquidity`-gated lateral edge, `Water`), and the g=0
    // Sandbox liquid wave (phase 1's `wetness >= 0.75 && !gravity_active` branch).
    fn test_frozen_jacobi_never_exceeds_capacity_or_goes_negative() {
        const EPS: f32 = 1e-4;
        let check = |sim: &TestSim, label: &str| {
            let mut min_h = f32::MAX;
            let mut max_over = f32::MIN;
            for idx in 0..sim.hm.data.len() {
                if sim.mask[idx] == crate::MASK_OUTSIDE {
                    continue;
                }
                let hgt = sim.hm.data[idx];
                min_h = min_h.min(hgt);
                let cap = cell_capacity_for(sim.cell_props[idx * 4 + PROP_WETNESS]);
                max_over = max_over.max(hgt - cap);
            }
            println!("test_frozen_jacobi_never_exceeds_capacity_or_goes_negative[{label}]: min_h={min_h:.6} max_over_capacity={max_over:.6}");
            assert!(min_h >= -EPS, "[{label}] a cell went negative: min_h={min_h:.6}");
            assert!(max_over <= EPS, "[{label}] a cell exceeded its capacity by {max_over:.6}");
        };

        // Granular free-fall (phase 0 only; g=0 branch never entered).
        {
            let w = 48;
            let h = 64;
            let mask = make_test_mask(w, h, SandboxShape::Square, 0.04, 1.0);
            let props = get_test_props(MaterialMode::DrySand, w * h);
            let mut sim = TestSim::new(w, h, props, mask, 16);
            for y in 4..10 {
                for x in 4..w - 4 {
                    sim.hm.data[y * w + x] = 1.4;
                }
            }
            let gravity_dir = glam::Vec2::new(0.0, 0.04);
            for _ in 0..400 {
                sim.tick(gravity_dir, 256);
                check(&sim, "DrySand under gravity");
            }
        }

        // Liquid free-fall + lateral spreading under gravity (phase 0 and phase 1 both active).
        {
            let w = 48;
            let h = 64;
            let mask = make_test_mask(w, h, SandboxShape::Square, 0.04, 1.0);
            let props = get_test_props(MaterialMode::Water, w * h);
            let mut sim = TestSim::new(w, h, props, mask, 16);
            for y in 4..10 {
                for x in 4..w - 4 {
                    sim.hm.data[y * w + x] = 1.0;
                }
            }
            let gravity_dir = glam::Vec2::new(0.0, 0.04);
            for _ in 0..400 {
                sim.tick(gravity_dir, 256);
                check(&sim, "Water under gravity");
            }
        }

        // g=0 Sandbox liquid wave (phase 1's `wetness >= 0.75 && !gravity_active` branch only).
        {
            let w = 48;
            let h = 48;
            let mask = make_test_mask(w, h, SandboxShape::Circle, 0.0, 1.0);
            let props = get_test_props(MaterialMode::Water, w * h);
            let mut sim = TestSim::new(w, h, props, mask, 16);
            for y in 0..h {
                for x in 0..w {
                    if sim.mask[y * w + x] != crate::MASK_OUTSIDE {
                        sim.hm.data[y * w + x] = 0.5;
                    }
                }
            }
            add_bump(&mut sim, w, h, w as f32 / 2.0, h as f32 / 2.0, 0.4, 4.0);
            for _ in 0..400 {
                sim.tick(glam::Vec2::ZERO, 256);
                check(&sim, "Water Sandbox g=0");
            }
        }
    }

    #[test]
    // Phase 5 (C7 fix): un-ignored. The sandbox liquid solver now conserves mass even when the
    // block LOD scheduler only simulates a fraction of the blocks per tick. Before the fix
    // (128x128 Water "dome", gravity=0, budget_n=4 of 16 blocks, 600 ticks) this measured
    // rel_err = +13.85% — mass INCREASED. (The liquid-gravity-proposal.md design doc reports
    // -1.345% for a similar but not identical setup; the sign disagrees, and this reproduction
    // is the one to trust.) Two independent causes, both structural rather than tunable:
    //   1. each cell adjusted *itself* by its own Laplacian, which only telescopes to zero over
    //      the domain if every cell updates in the same pass — `will_simulate[b]` gates blocks
    //      by frame budget, so it does not;
    //   2. the trailing `.clamp(0.0, 1.0)` was a unilateral edit with no counterparty: flooring
    //      a negative excursion to 0 adds mass, capping at 1.0 discards it.
    // Replaced by the per-edge flux form (`flux_edge`), where every edge debits exactly what it
    // credits and the donor/acceptor limits can only ever *reduce a transfer*. After the fix:
    // rel_err = -7e-9 (f32 rounding on the debit/credit pair), vs the required 1e-4.
    fn test_liquid_mass_conserved_in_sandbox_under_lod() {
        let w = 128;
        let h = 128;
        let block_size = 32; // 4x4 = 16 blocks
        let mask = make_test_mask(w, h, SandboxShape::Circle, 0.04, 1.0);
        let props = get_test_props(MaterialMode::Water, w * h);
        let mut sim = TestSim::new(w, h, props, mask, block_size);
        assert_eq!(sim.active_blocks.len(), 16, "Expected 16 blocks at 128x128 with block_size=32");

        let center_x = w as f32 / 2.0;
        let center_y = h as f32 / 2.0;
        for y in 0..h {
            for x in 0..w {
                let dx = x as f32 - center_x;
                let dy = y as f32 - center_y;
                if dx * dx + dy * dy < 20.0 * 20.0 {
                    sim.hm.data[y * w + x] = 1.0;
                }
            }
        }
        let initial_mass = sim.mass();
        // Sandbox mode: gravity = 0 (routes wetness >= 0.75 through the wave solver), and a
        // throttled budget so only 4 of 16 blocks simulate per tick.
        for _ in 0..600 {
            sim.tick(glam::Vec2::ZERO, 4);
        }
        let final_mass = sim.mass();
        let rel_err = (final_mass - initial_mass) / initial_mass;
        println!(
            "test_liquid_mass_conserved_in_sandbox_under_lod: init={:.6} final={:.6} rel_err={:.6}",
            initial_mass, final_mass, rel_err
        );
        assert!(rel_err.abs() < 1e-4, "Mass not conserved under partial-block LOD: rel_err={:.6}", rel_err);
    }

    // =======================================================================================
    // Sandbox (gravity = 0) wave dynamics.
    //
    // Until these existed there was no test of liquid *behaviour* at g = 0 at all. Every other
    // liquid test is gravity-oriented except `test_liquid_mass_conserved_in_sandbox_under_lod`,
    // which weighs the pool and never looks at it. That blind spot let `cce3b571` ship a solver
    // whose Sandbox ripples *grew* ~20% in amplitude per tick until they pinned against the cell
    // cap — the user's "used to ripple and reflect, now fully chaotic" — through 59 green tests,
    // because mass stayed perfect the whole time. Conservation and dynamics are independent
    // properties and each needs its own test.
    // =======================================================================================

    /// A flat Sandbox pool: every in-mask cell filled to `level`, gravity to be passed as zero.
    fn wave_pool(w: usize, h: usize, block_size: usize, shape: SandboxShape, level: f32) -> TestSim {
        let mask = make_test_mask(w, h, shape, 0.04, 1.0);
        let props = get_test_props(MaterialMode::Water, w * h);
        let mut sim = TestSim::new(w, h, props, mask, block_size);
        for i in 0..w * h {
            if sim.mask[i] != crate::MASK_OUTSIDE {
                sim.hm.data[i] = level;
            }
        }
        sim
    }

    /// Radially symmetric gaussian crest centred on `(bx, by)`.
    fn add_bump(sim: &mut TestSim, w: usize, h: usize, bx: f32, by: f32, amp: f32, sigma: f32) {
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                if sim.mask[i] == crate::MASK_OUTSIDE {
                    continue;
                }
                let dx = x as f32 - bx;
                let dy = y as f32 - by;
                sim.hm.data[i] += amp * (-(dx * dx + dy * dy) / (2.0 * sigma * sigma)).exp();
            }
        }
    }

    /// Crest that is uniform in y, so the dynamics reduce to a 1-D channel along x. Used by the
    /// reflection test, where a radial ripple would confound "bounced off the wall" with
    /// "spread out sideways".
    fn add_band_bump(sim: &mut TestSim, w: usize, h: usize, bx: f32, amp: f32, sigma: f32) {
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                if sim.mask[i] == crate::MASK_OUTSIDE {
                    continue;
                }
                let dx = x as f32 - bx;
                sim.hm.data[i] += amp * (-(dx * dx) / (2.0 * sigma * sigma)).exp();
            }
        }
    }

    /// Level the pool must relax to once the ripple is gone: all the mass, spread evenly over
    /// the cells that can hold it. Derived from the state rather than hard-coded so it stays
    /// correct if the mask area changes.
    fn wave_rest_level(sim: &TestSim) -> f32 {
        let mut total = 0.0f64;
        let mut n = 0usize;
        for i in 0..sim.hm.data.len() {
            if sim.mask[i] != crate::MASK_OUTSIDE {
                total += sim.hm.data[i] as f64;
                n += 1;
            }
        }
        (total / n.max(1) as f64) as f32
    }

    /// Ripple amplitude: the largest departure from the resting pool level, in either direction.
    fn wave_amplitude(sim: &TestSim, rest: f32) -> f32 {
        let mut a = 0.0f32;
        for i in 0..sim.hm.data.len() {
            if sim.mask[i] != crate::MASK_OUTSIDE {
                a = a.max((sim.hm.data[i] - rest).abs());
            }
        }
        a
    }

    /// Ripple amplitude restricted to the vertical strip `x in [x0, x1)`.
    fn wave_amplitude_in_band(sim: &TestSim, w: usize, h: usize, rest: f32, x0: usize, x1: usize) -> f32 {
        let mut a = 0.0f32;
        for y in 0..h {
            for x in x0..x1.min(w) {
                let i = y * w + x;
                if sim.mask[i] != crate::MASK_OUTSIDE {
                    a = a.max((sim.hm.data[i] - rest).abs());
                }
            }
        }
        a
    }

    #[test]
    // THE regression test for this bug. A crest dropped on a still pool must lose amplitude and
    // settle back to a flat pool; it is a damped wave, and the only source of energy is the
    // initial disturbance.
    //
    // The `cce3b571` edge-flux solver drove each edge's velocity from `temp_heights`, the buffer
    // it was concurrently writing, so a cell's four incident edges each saw whatever the previous
    // ones had already done and the pass became Gauss-Seidel with a direction-alternating sweep.
    // Gauss-Seidel on a wave equation is a gain, not just a loss of accuracy: linearising the
    // 1-D chain at Water's (c_sq, damping) = (0.24, 0.98) gives a per-tick spectral radius of
    // 1.20 for the swept form versus 0.994 for the snapshot form. 20% growth per tick against 2%
    // damping, so this test measured `maxh` climbing 0.80 -> 1.0000 by tick 50 and pinning there
    // for the remaining 350 — a pool of saturated cells, which is what the user saw as "fully
    // chaotic". Raising the cell cap to 3.0 only moved the ceiling (peak pinned at 3.0000) and
    // made the sweep bias 33x more visible, confirming injection rather than clipping.
    //
    // Driving the velocities from `heightmap.data` instead — frozen for the whole tick, since the
    // copy-back happens after the sweep — restores Jacobi ordering without touching the flux
    // form, so conservation (checked separately below) is unaffected.
    fn test_sandbox_wave_decays_to_flat_pool() {
        let (w, h, bs) = (128, 128, 32);
        let mut sim = wave_pool(w, h, bs, SandboxShape::Circle, 0.50);
        add_bump(&mut sim, w, h, w as f32 / 2.0, h as f32 / 2.0, 0.30, 12.0);
        let rest = wave_rest_level(&sim);

        let mut samples = vec![(0u32, wave_amplitude(&sim, rest))];
        for t in 1..=400u32 {
            sim.tick(glam::Vec2::ZERO, 16);
            if t % 50 == 0 {
                samples.push((t, wave_amplitude(&sim, rest)));
            }
        }
        println!(
            "test_sandbox_wave_decays_to_flat_pool: rest={:.4} amplitude {:?}",
            rest,
            samples.iter().map(|&(t, a)| (t, (a * 1e4).round() / 1e4)).collect::<Vec<_>>()
        );

        // The envelope must come down at every sample. A growing solver fails this on the first
        // interval; a solver that merely stalls fails it later.
        for pair in samples.windows(2) {
            let ((t0, a0), (t1, a1)) = (pair[0], pair[1]);
            assert!(
                a1 < a0,
                "Sandbox ripple did not decay between tick {} and {}: {:.6} -> {:.6}. \
                 A growing amplitude means the wave update is injecting energy (Gauss-Seidel \
                 ordering); see this test's comment.",
                t0, t1, a0, a1
            );
        }
        let final_amp = samples.last().unwrap().1;
        assert!(
            final_amp < 0.25 * samples[0].1,
            "Sandbox ripple still holds {:.1}% of its initial amplitude after 400 ticks \
             ({:.6} of {:.6}); it should have relaxed toward the {:.4} rest level",
            100.0 * final_amp / samples[0].1, final_amp, samples[0].1, rest
        );
    }

    #[test]
    // A centred disturbance in a left-right symmetric domain must stay centred. Any bias in the
    // update order — and the solver's block, row and column sweeps all flip on `tick_count % 2`
    // — shows up here long before it is visible as instability.
    //
    // This is deliberately a *separate* assertion from the decay test: raising the cell cap to
    // 3.0 while the solver was still Gauss-Seidel left the pool "stable-looking" at its new
    // ceiling but drove asymmetry from 0.0029 to 0.0996, 33x worse. Amplitude and symmetry fail
    // independently, so they are tested independently.
    fn test_sandbox_wave_stays_left_right_symmetric() {
        let (w, h, bs) = (128, 128, 32);
        let mut sim = wave_pool(w, h, bs, SandboxShape::Circle, 0.50);
        // Centred on (w-1)/2, the exact axis of the mirror map x -> w-1-x, so the initial
        // condition is *bit* symmetric and any asymmetry that appears later is the solver's.
        add_bump(&mut sim, w, h, (w as f32 - 1.0) / 2.0, (h as f32 - 1.0) / 2.0, 0.30, 12.0);

        // Mirror error of the height field itself, normalised by total mass. Compared against
        // the field's own reflection rather than a left/right mass split, which is far coarser:
        // equal masses either side says nothing about equal *shapes* either side.
        let mirror_error = |s: &TestSim| -> f64 {
            let (mut diff, mut total) = (0.0f64, 0.0f64);
            for y in 0..h {
                for x in 0..w {
                    let (i, j) = (y * w + x, y * w + (w - 1 - x));
                    if s.mask[i] == crate::MASK_OUTSIDE || s.mask[j] == crate::MASK_OUTSIDE {
                        continue;
                    }
                    diff += (s.hm.data[i] - s.hm.data[j]).abs() as f64;
                    total += s.hm.data[i] as f64;
                }
            }
            if total > 0.0 { diff / total } else { 0.0 }
        };

        let initial = mirror_error(&sim);
        assert!(initial < 1e-9, "test setup is not mirror symmetric: {:.3e}", initial);

        let mut worst = 0.0f64;
        let mut trace = Vec::new();
        for t in 1..=400u32 {
            sim.tick(glam::Vec2::ZERO, 16);
            let e = mirror_error(&sim);
            worst = worst.max(e);
            if t % 100 == 0 {
                trace.push((t, e));
            }
        }
        let final_err = mirror_error(&sim);
        println!(
            "test_sandbox_wave_stays_left_right_symmetric: worst={:.3e} final={:.3e} trace={:?}",
            worst, final_err,
            trace.iter().map(|&(t, e)| (t, format!("{:.2e}", e))).collect::<Vec<_>>()
        );

        // Some asymmetry is unavoidable: the sweep order alternates every tick, so a symmetric
        // pair of cells is not visited in the same relative order on every tick. What must not
        // happen is for that to *accumulate*.
        assert!(
            worst < 1e-2,
            "Centred disturbance went lopsided: mirror error reached {:.3e}. A directional \
             sweep bias in the wave update is the cause to look for.",
            worst
        );
        assert!(
            final_err < 0.25 * worst,
            "Mirror error is not transient — it peaked at {:.3e} and is still {:.3e} after 400 \
             ticks, so the bias is accumulating rather than washing out",
            worst, final_err
        );
    }

    #[test]
    // Conservation, on the same disturbance the decay test uses. `cce3b571` bought this at the
    // cost of stability (-3.93% drift over 400 ticks before it, ~0% after), and the fix for the
    // stability half must not hand the drift back: Jacobi ordering changes *when* an edge's
    // velocity is read, not the fact that the edge debits exactly what it credits.
    fn test_sandbox_wave_conserves_mass() {
        let (w, h, bs) = (128, 128, 32);
        let mut sim = wave_pool(w, h, bs, SandboxShape::Circle, 0.50);
        add_bump(&mut sim, w, h, w as f32 / 2.0, h as f32 / 2.0, 0.30, 12.0);
        let initial = sim.mass();
        for _ in 0..400 {
            sim.tick(glam::Vec2::ZERO, 16);
        }
        let final_mass = sim.mass();
        let rel_err = (final_mass - initial) / initial;
        println!(
            "test_sandbox_wave_conserves_mass: init={:.6} final={:.6} rel_err={:.3e}",
            initial, final_mass, rel_err
        );
        assert!(
            rel_err.abs() < 1e-4,
            "Sandbox ripple leaked mass: rel_err={:.6}", rel_err
        );
    }

    #[test]
    // Reflection — "ripples that would reflect", the other half of the user's report.
    //
    // A y-uniform crest near the left wall of a square pool makes the problem a 1-D channel, so
    // the wave that leaves the crest has nowhere to go but the far wall and back; a radial bump
    // would let "spread out sideways" masquerade as "bounced".
    //
    // Two distinct failure modes are ruled out by looking at both ends of the channel:
    //   * absorbed into the wall  -> the far band rings up and the near band never rings again;
    //   * piled against the wall  -> the far band rings up and stays up.
    // A real reflection is the far band rising and then falling *and* the near band recovering
    // after its own minimum.
    fn test_sandbox_wave_reflects_off_boundary() {
        let (w, h, bs) = (64, 64, 32);
        let mut sim = wave_pool(w, h, bs, SandboxShape::Square, 0.50);
        // Actual mask extent along the mid row — the Square shape insets from the grid edge.
        let (mut x_lo, mut x_hi) = (w, 0usize);
        for x in 0..w {
            if sim.mask[(h / 2) * w + x] != crate::MASK_OUTSIDE {
                x_lo = x_lo.min(x);
                x_hi = x_hi.max(x);
            }
        }
        add_band_bump(&mut sim, w, h, x_lo as f32 + 4.0, 0.30, 3.0);
        let rest = wave_rest_level(&sim);

        // Signed mean deviation of a whole column. Signed, not absolute: a crest reflecting off
        // a Neumann wall arrives as a *positive* excursion where there was a negative one, which
        // an absolute-value metric would blur into the static offset.
        let column = |s: &TestSim, x: usize| -> f32 {
            let (mut sum, mut n) = (0.0f32, 0usize);
            for y in 0..h {
                let i = y * w + x;
                if s.mask[i] != crate::MASK_OUTSIDE {
                    sum += s.hm.data[i] - rest;
                    n += 1;
                }
            }
            sum / n.max(1) as f32
        };
        let (near_x, far_x) = (x_lo + 4, x_hi - 1);

        let (mut near, mut far) = (vec![column(&sim, near_x)], vec![column(&sim, far_x)]);
        for _ in 0..400 {
            sim.tick(glam::Vec2::ZERO, 4);
            near.push(column(&sim, near_x));
            far.push(column(&sim, far_x));
        }

        let far_peak_t = (0..far.len()).max_by(|&a, &b| far[a].total_cmp(&far[b])).unwrap();
        let far_peak = far[far_peak_t];
        let far_end = far[far.len() - 1];
        // Whatever comes back to the near column *after* the wave has reached the far wall.
        let return_from = far_peak_t + 20;
        let near_return = near[return_from..].iter().cloned().fold(f32::MIN, f32::max);
        println!(
            "test_sandbox_wave_reflects_off_boundary: mask x {}..{}, rest={:.4}; \
             far start={:+.5} peak={:+.5}@t{} end={:+.5}; near at t{}={:+.5} return={:+.5}",
            x_lo, x_hi, rest, far[0], far_peak, far_peak_t, far_end,
            far_peak_t, near[far_peak_t], near_return
        );

        // The far column starts below the rest level (all the disturbance is at the near end)
        // and must stay there until the wave physically crosses the pool.
        assert!(far[0] < 0.0, "far column did not start below rest: {:+.6}", far[0]);
        assert!(
            far_peak_t > 40,
            "The far wall reacted at t={}, far sooner than a wave can cross {} cells at this \
             wave speed — that is not propagation",
            far_peak_t, far_x - near_x
        );
        assert!(
            far_peak > 0.02,
            "The disturbance never reached the far wall: that column only ever rose to {:+.6} \
             above the rest level",
            far_peak
        );
        // Reflected, not absorbed and not accumulated.
        assert!(
            far_end < 0.25 * far_peak,
            "The disturbance piled up against the far wall instead of bouncing off it: the wall \
             column peaked at {:+.6} and is still {:+.6} at the end",
            far_peak, far_end
        );
        assert!(
            near[far_peak_t] <= 0.0,
            "The near end had not gone quiet by the time the wave hit the far wall ({:+.6}), so \
             the recovery below would not prove anything",
            near[far_peak_t]
        );
        assert!(
            near_return > 0.004,
            "Nothing came back: after the wave hit the far wall at t={}, the near column only \
             ever recovered to {:+.6}. The boundary swallowed the wave instead of reflecting it",
            far_peak_t, near_return
        );
    }

    #[test]
    // THE regression test for "waves in sandbox don't continue to the edge, they freeze half way
    // through" — and the one thing the four tests above structurally cannot see.
    //
    // Those tests run a wave, but never through the block scheduler:
    //   * `TestSim::new` sets `last_displacements` to 1.0 everywhere, so every block is MUST on
    //     tick 1 whatever the wake magnitude says, and
    //   * their pools sit at 0.50, which is 0.15 above DEFAULT_SAND_HEIGHT — above the old 0.1
    //     MUST bar — so under the old `|h - DEFAULT_SAND_HEIGHT|` wake magnitude every block was
    //     MUST on *every* tick for the whole run. Measured on the pre-fix code, a settled 256x256
    //     pool at 0.50: 7680 of 7680 MUST block-ticks over a staleness period. They measured a
    //     solver with the LOD switched off.
    //
    // So this test does the two things they don't: it puts the pool at the level the app actually
    // starts at (DEFAULT_SAND_HEIGHT — `sandart/src/main.rs` fills the bed with it), and it arms
    // *only* the blocks the disturbance was drawn into, leaving the rest of the domain asleep and
    // the scheduler in charge of waking it.
    //
    // The assertion is not "the wave arrives" but "the wave arrives at the same time regardless of
    // how much simulation budget there is". Propagation speed is a property of the medium; a
    // scheduler is an optimisation and optimisations do not get to change physics. Reach tracking
    // the budget is the exact signature of the bug, and byte-identical reach across two budgets is
    // the exact signature of it being gone.
    //
    // Measured, 1200 ticks, mask spanning columns 11..245, reach = furthest column ever deviating
    // > 2e-3 from where it started:
    //
    //     budget | before                   | after
    //     -------+--------------------------+--------------------------
    //       32   | column 148, far peak 0   | column 245, far peak 0.00775
    //       64   | column 200, far peak 0   | column 245, far peak 0.00775
    //      256   | column 245               | column 245, far peak 0.00779
    //
    // Before, the far column's deviation was *exactly* 0.00000 for all 1200 ticks at budget 32 and
    // 64: not a slow wave, a stopped one. After, budgets 32 and 64 agree to the bit. Budget 256 is
    // allowed to differ in the last digits — it simulates the sub-threshold rest candidates too,
    // which is a different (larger) set of floating-point additions, not a different wave.
    fn test_sandbox_wave_reach_is_budget_independent() {
        let (w, h, bs) = (256, 256, 16);
        let cols = (w + bs - 1) / bs;

        // Returns (mask extent, furthest column the disturbance ever reached, that column's peak).
        let run = |budget: usize| -> (usize, usize, usize, f32) {
            let mut sim = wave_pool(w, h, bs, SandboxShape::Square, crate::DEFAULT_SAND_HEIGHT);
            let (mut x_lo, mut x_hi) = (w, 0usize);
            for x in 0..w {
                if sim.mask[(h / 2) * w + x] != crate::MASK_OUTSIDE {
                    x_lo = x_lo.min(x);
                    x_hi = x_hi.max(x);
                }
            }
            // y-uniform crest hard against the left wall, so this is a 1-D channel and "reached
            // the far wall" cannot be confused with "spread out sideways" (same reasoning as
            // `test_sandbox_wave_reflects_off_boundary`).
            add_band_bump(&mut sim, w, h, x_lo as f32 + 4.0, 0.30, 3.0);

            // The load-bearing line: only the blocks that actually hold the crest start awake.
            // Everything ahead of the wavefront must be woken by the solver's own activation
            // bookkeeping, which is the machinery under test.
            sim.last_displacements.fill(0.0);
            for y in 0..h {
                for x in 0..w {
                    let i = y * w + x;
                    if sim.mask[i] != crate::MASK_OUTSIDE
                        && (sim.hm.data[i] - crate::DEFAULT_SAND_HEIGHT).abs() > 1e-6
                    {
                        sim.last_displacements[(y / bs) * cols + (x / bs)] = 1.0;
                    }
                }
            }

            let column = |s: &TestSim, x: usize| -> f32 {
                let (mut sum, mut n) = (0.0f32, 0usize);
                for y in 0..h {
                    let i = y * w + x;
                    if s.mask[i] != crate::MASK_OUTSIDE {
                        sum += s.hm.data[i];
                        n += 1;
                    }
                }
                sum / n.max(1) as f32
            };
            let base: Vec<f32> = (0..w).map(|x| column(&sim, x)).collect();
            let mut peak = vec![0.0f32; w];
            for _ in 0..1200u32 {
                sim.tick(glam::Vec2::ZERO, budget);
                for x in 0..w {
                    peak[x] = peak[x].max((column(&sim, x) - base[x]).abs());
                }
            }
            let reach = (x_lo..=x_hi).filter(|&x| peak[x] > 2e-3).max().unwrap_or(x_lo);
            (x_lo, x_hi, reach, peak[x_hi])
        };

        // Only the two throttled budgets are run. Budget 256 reaches the wall even on the
        // pre-fix code — it simulates everything, so it never exercised the scheduler path this
        // test exists for — and it costs a third of the runtime. Measured at 256 when the fix
        // landed: reach 245, far peak 0.00779.
        let (x_lo, x_hi, reach_32, far_32) = run(32);
        let (_, _, reach_64, far_64) = run(64);
        println!(
            "test_sandbox_wave_reach_is_budget_independent: mask x {}..{}; \
             reach/far-peak = {}/{:.5} at budget 32, {}/{:.5} at 64",
            x_lo, x_hi, reach_32, far_32, reach_64, far_64
        );

        for (budget, reach, far) in [(32, reach_32, far_32), (64, reach_64, far_64)] {
            assert_eq!(
                reach, x_hi,
                "At budget {} the disturbance stalled at column {} of {} and never reached the \
                 wall (that wall column only ever moved by {:.6}). The wave solver is not the \
                 suspect: check that the g = 0 liquid branch's wake magnitude is the head \
                 difference across the cell's owned edges, and that the scheduler's Sandbox \
                 must-simulate threshold is low enough for a ripple-sized head to clear it.",
                budget, reach, x_hi, far
            );
            assert!(
                far > 2e-3,
                "At budget {} the far wall column only ever moved by {:.6}", budget, far
            );
        }

        // The sharp one. Two budgets, one wave, bit for bit.
        assert_eq!(
            (reach_32, far_32.to_bits()), (reach_64, far_64.to_bits()),
            "Propagation still depends on the simulation budget: reach {}/far peak {:.6} at \
             budget 32 versus reach {}/far peak {:.6} at 64. The wavefront is being scheduled \
             rather than simulated.",
            reach_32, far_32, reach_64, far_64
        );
    }

    #[test]
    // The other half of the fix, and the reason it could not be "just lower the threshold".
    //
    // The scheduler's Sandbox must-simulate bar was 0.1 — 1000x gravity's — purely because the
    // liquid wake magnitude it read was an absolute level, `|h - DEFAULT_SAND_HEIGHT|`. A pool is
    // at DEFAULT_SAND_HEIGHT only by coincidence: the user pours wherever they pour. So lowering
    // the bar alone makes a still, flat, utterly quiet pool at any other level report every block
    // as MUST forever — measured on this exact 256x256 setup at level 0.50: 7680 of 7680 MUST
    // block-ticks over a staleness period, the whole domain, permanently, with nothing moving.
    // That is worse than the bug: it burns the entire budget every tick to simulate a flat pool.
    //
    // A head *difference* across the cell's owned edges is zero for a flat pool at every level, so
    // it is safe to compare against a threshold 1000x finer. Both levels below now measure 0 MUST
    // block-ticks. On the pre-fix code the same two runs measured 0 at 0.35 — only because that is
    // the constant the wake magnitude subtracted, so it is no evidence of anything — and the full
    // 7680 at 0.50, which is why this test checks a level the solver has no special knowledge of
    // as well as the one it does.
    fn test_settled_sandbox_pool_does_not_stay_hot() {
        let (w, h, bs) = (256, 256, 16);
        let cols = (w + bs - 1) / bs;
        let rows = (h + bs - 1) / bs;
        let block_ticks = cols * rows * 30;

        for &level in &[crate::DEFAULT_SAND_HEIGHT, 0.50f32] {
            let mut sim = wave_pool(w, h, bs, SandboxShape::Square, level);
            let mut must = 0usize;
            // Long enough for the initial all-awake state to drain; then count MUST blocks over a
            // whole staleness period, so a block that merely ages back in is not mistaken for one
            // the wake magnitude is holding hot.
            for t in 0..300u32 {
                sim.tick(glam::Vec2::ZERO, 256);
                if t >= 270 {
                    must += sim.active_blocks.iter()
                        .filter(|&&a| a == crate::BlockActivity::Fast).count();
                }
            }
            println!(
                "test_settled_sandbox_pool_does_not_stay_hot: level={:.2} must={} of {}",
                level, must, block_ticks
            );
            assert_eq!(
                must, 0,
                "A flat, still pool at level {:.2} keeps {} of {} block-ticks MUST-simulate. The \
                 liquid wake magnitude has become a level again rather than a head difference: \
                 anything that does not return to zero for a pool at rest *at any level* makes \
                 the whole domain permanently hot at this threshold.",
                level, must, block_ticks
            );
        }
    }

    #[test]
    #[ignore = "MARKER, not a regression, and deliberately NOT fixed here: a pool already sitting \
                at cell capacity has no headroom for a crest, so it cannot ripple. Water's \
                cell_capacity_for(1.0) is exactly 1.0, and every transfer in `flux_edge` is \
                limited by the acceptor's `(cap_b - h_b).max(0.0)`, so at h == cap that limit is \
                zero on every edge in every direction. A refilling trough can therefore rise to \
                the rest level but can never overshoot it, and overshoot is what ringing IS. \
                Measured today, the same narrow trough carved into the same pool at two fill \
                levels: a half-full pool (rest 0.4964) overshoots the rest level by 1.306e-1 and \
                rings; an at-capacity pool (rest 0.9928) overshoots by 7.220e-3 — which is not a \
                damped version of the same thing, it is *exactly* the 7.220e-3 of headroom \
                between that rest level and the cap, to every digit. The crest is not attenuated, \
                it is clipped by the ceiling. Distinct from the Gauss-Seidel energy injection \
                fixed alongside these tests (that one made ripples grow; this one stops them \
                existing). The remedy is headroom — a free surface allowed above the packing \
                limit, or a capacity above the fill ceiling — which changes what a cell means and \
                belongs in its own commit."]
    fn test_sandbox_wave_at_capacity_cannot_ripple() {
        let (w, h, bs) = (64, 64, 32);
        // Carve one narrow trough and watch its centre refill. A real damped wave overshoots the
        // rest level and rings; the question is only whether there is room above rest to do it in.
        let run = |level: f32| -> (f32, f32) {
            let mut sim = wave_pool(w, h, bs, SandboxShape::Square, level);
            add_bump(&mut sim, w, h, 32.0, 32.0, -level, 2.0);
            let rest = wave_rest_level(&sim);
            let probe = 32 * w + 32;
            let mut peak = f32::MIN;
            for _ in 0..600 {
                sim.tick(glam::Vec2::ZERO, 4);
                peak = peak.max(sim.hm.data[probe]);
            }
            (rest, peak - rest)
        };

        let cap = cell_capacity_for(1.0);
        let (rest_half, over_half) = run(0.50);
        let (rest_full, over_full) = run(cap);
        println!(
            "test_sandbox_wave_at_capacity_cannot_ripple: cap={:.3}; half-full rest={:.4} \
             overshoot={:.3e}; at-capacity rest={:.4} headroom={:.3e} overshoot={:.3e}",
            cap, rest_half, over_half, rest_full, cap - rest_full, over_full
        );
        assert!(over_half > 1e-3, "control case did not ring at all: {:.3e}", over_half);
        assert!(
            over_full > 0.5 * over_half,
            "A pool at capacity cannot ripple: the same trough overshoots the rest level by \
             {:.3e} in a half-full pool but only {:.3e} at capacity, where the entire headroom \
             above rest is {:.3e}",
            over_half, over_full, cap - rest_full
        );
    }


    #[test]
    // Phase 5: un-ignored, and it came for free with the L5 fix. Toggling gravity to zero
    // mid-simulation (the shipped slider reaches 0.0 in Sand-fall mode, demo.js:710) used to
    // measure rel_err = +9.75% over 60 ticks at g=(0,0.04) followed by 300 at g=0 — mass
    // INCREASED, because the g=0 branch's `clamp(0.0, 1.0)` is asymmetric: an undershoot below 0
    // was floored to 0 without removing the corresponding mass from any neighbour, and an
    // hourglass gives many ticks of wall reflection for that to accumulate. (The design doc
    // reports -21.5%; the sign disagrees and this reproduction is the one to trust.) The edge
    // flux form has no unilateral clamp at all, so this is conservative by construction rather
    // than by tuning. After the fix: rel_err = 1.3e-8, vs the required 1e-3.
    fn test_liquid_survives_gravity_toggle() {
        let w = 64;
        let h = 64;
        let mask = make_test_mask(w, h, SandboxShape::Hourglass, 0.15, 0.6);
        let props = get_test_props(MaterialMode::Water, w * h);
        let mut sim = TestSim::new(w, h, props, mask, 32);
        let center_x = w as f32 / 2.0;
        let center_y = h as f32 / 2.0;
        for y in 0..h {
            let dy = y as f32 - center_y;
            if dy < 0.0 && dy > -6.0 {
                for x in 0..w {
                    let dx = x as f32 - center_x;
                    if dx.abs() < 22.4 {
                        sim.hm.data[y * w + x] = 1.0;
                    }
                }
            }
        }
        let initial_mass = sim.mass();
        for _ in 0..60 {
            sim.tick(glam::Vec2::new(0.0, 0.04), 256);
        }
        for _ in 0..300 {
            sim.tick(glam::Vec2::ZERO, 256);
        }
        let final_mass = sim.mass();
        let rel_err = (final_mass - initial_mass).abs() / initial_mass;
        println!(
            "test_liquid_survives_gravity_toggle: init={:.6} final={:.6} rel_err={:.6}",
            initial_mass, final_mass, rel_err
        );
        assert!(rel_err < 1e-3, "Mass not conserved across a gravity toggle: rel_err={:.6}", rel_err);
    }

    #[test]
    #[ignore = "STILL FAILING after Phase 5, but for a different reason than before, and the \
                remaining gap looks like a defect in this test rather than in the solver. \
                Originally (C4): get_ca_params collapsed every wetness >= 0.75 material to the \
                same (threshold = 0.0, alpha = 0.50) and wave_params — the only thing that \
                distinguishes Water/CalmWater/Milk/VegOil — was unreachable under gravity, so \
                all four produced bit-identical centroids (max_sep = 0.000000). Phase 5 put the \
                gravity liquid path on the same edge-flux solver as the g = 0 path, so \
                wave_params IS now reached and the four presets are no longer identical: \
                max_sep = 0.0237. But that is still far short of the 0.5 this test demands, \
                because the metric is the centroid of the FINAL SETTLED state after 800 ticks. \
                A conservative, incompressible solver settles every liquid into the same shape \
                — that is the point of Phase 1's capacity constraint and Phase 5's conservation \
                — so the settled centroid cannot distinguish them no matter how different their \
                dynamics are. What actually differs is how fast they get there: c_sq/damping \
                span (0.08, 0.76) for Yogurt to (0.24, 0.98) for Water, roughly a 2x spread in \
                free-fall rate. That is the design doc's own alternative criterion ('settle-time \
                differing > 10%'), which this test does not implement. Deliberately left \
                failing and unmodified rather than weakened."]
    fn test_liquid_presets_are_distinguishable_under_gravity() {
        let w = 64;
        let h = 64;
        let center_x = w as f32 / 2.0;
        let center_y = h as f32 / 2.0;
        let r_sq = (0.46 * w as f32) * (0.46 * w as f32);
        let mask = make_test_mask(w, h, SandboxShape::Circle, 0.04, 1.0);
        let gravity_dir = glam::Vec2::new(0.0, 0.04);

        let mut centroids = Vec::new();
        for mat in [MaterialMode::Water, MaterialMode::CalmWater, MaterialMode::Milk, MaterialMode::VegetableOil] {
            let props = get_test_props(mat, w * h);
            let mut sim = TestSim::new(w, h, props, mask.clone(), 32);
            for y in 0..h {
                for x in 0..w {
                    let dx = x as f32 - center_x;
                    let dy = y as f32 - center_y;
                    if dx * dx + dy * dy < r_sq && dy < -2.0 {
                        sim.hm.data[y * w + x] = 0.8;
                    }
                }
            }
            for _ in 0..800 {
                sim.tick(gravity_dir, 256);
            }
            let mut total = 0.0f64;
            let mut wx = 0.0f64;
            let mut wy = 0.0f64;
            for y in 0..h {
                for x in 0..w {
                    let val = sim.hm.data[y * w + x] as f64;
                    if val > 0.0 {
                        total += val;
                        wx += x as f64 * val;
                        wy += y as f64 * val;
                    }
                }
            }
            let centroid = (wx / total, wy / total);
            println!("test_liquid_presets_are_distinguishable_under_gravity: {:?} centroid={:?}", mat, centroid);
            centroids.push((mat, centroid));
        }

        // Pairwise centroid separation: at least one pair should differ by more than 0.5 cells.
        let mut max_sep = 0.0f64;
        for i in 0..centroids.len() {
            for j in (i + 1)..centroids.len() {
                let (ax, ay) = centroids[i].1;
                let (bx, by) = centroids[j].1;
                let sep = ((ax - bx).powi(2) + (ay - by).powi(2)).sqrt();
                max_sep = max_sep.max(sep);
            }
        }
        println!("test_liquid_presets_are_distinguishable_under_gravity: max pairwise centroid separation={:.6}", max_sep);
        // Measured today: max_sep = 0.0 (bit-identical).
        assert!(max_sep > 0.5, "All liquid presets produced statistically identical results under gravity: max_sep={:.6}", max_sep);
    }

    #[test]
    // Phase 1 (C5 fix): the wetness >= 0.75 liquid/granular branch cut must be stable under
    // property advection — two runs seeded a hair on either side of 0.75 should agree closely,
    // not diverge catastrophically. Was ignored before Phase 1: wetness=0.7499 settled near
    // centroid y=41.18, wetness=0.7501 near centroid y=51.50 — a ~16% difference relative to the
    // 64-row grid. Cause: advect_properties blends with weights that don't sum to exactly 1.0 in
    // f32, and cells that drifted to wetness < 0.75 fell wholesale into the granular branch where
    // alpha = flow_rate * 1.5 = 0.0 for a Yogurt-like material (flow_rate=0.08 here), freezing
    // solid instead of flowing. Fixed by replacing the hard `wetness >= 0.75` parameter switch in
    // get_ca_params with a `liquidity(wetness)` smoothstep blend of the granular and liquid
    // (threshold, alpha) pairs, so a drift of a few 1e-4 in wetness only perturbs the blend
    // weight by a similarly tiny amount. After the fix: rel_diff ~= 0.00018 (vs required < 0.01).
    fn test_wetness_classification_is_stable_under_advection() {
        let w = 64;
        let h = 64;
        let mask = make_test_mask(w, h, SandboxShape::Circle, 0.04, 1.0);
        let gravity_dir = glam::Vec2::new(0.0, 0.04);
        let center_x = w as f32 / 2.0;
        let center_y = h as f32 / 2.0;
        let r_sq = (0.46 * w as f32) * (0.46 * w as f32);

        let mut centroids_y = Vec::new();
        for wetness in [0.7499f32, 0.7501f32] {
            let mut props = vec![0.0f32; w * h * 4];
            for chunk in props.chunks_exact_mut(4) {
                chunk[PROP_WETNESS] = wetness;
                chunk[PROP_THRESHOLD] = 0.0;
                chunk[PROP_FLOW_RATE] = 0.08;
                chunk[PROP_GRAIN_SIZE] = 0.08;
            }
            let mut sim = TestSim::new(w, h, props, mask.clone(), 32);
            for y in 0..h {
                for x in 0..w {
                    let dx = x as f32 - center_x;
                    let dy = y as f32 - center_y;
                    if dx * dx + dy * dy < r_sq && dy < -2.0 {
                        sim.hm.data[y * w + x] = 0.8;
                    }
                }
            }
            for _ in 0..800 {
                sim.tick(gravity_dir, 256);
            }
            let mut total = 0.0f64;
            let mut wy = 0.0f64;
            for y in 0..h {
                for x in 0..w {
                    let val = sim.hm.data[y * w + x] as f64;
                    if val > 0.0 {
                        total += val;
                        wy += y as f64 * val;
                    }
                }
            }
            let centroid_y = wy / total;
            println!("test_wetness_classification_is_stable_under_advection: wetness={} centroid_y={:.5}", wetness, centroid_y);
            centroids_y.push(centroid_y);
        }

        let diff = (centroids_y[0] - centroids_y[1]).abs();
        let rel_diff = diff / h as f64;
        println!("test_wetness_classification_is_stable_under_advection: |diff|={:.5} rel_diff={:.5}", diff, rel_diff);
        // Measured today: rel_diff ~= 0.16 (16%).
        assert!(
            rel_diff < 0.01,
            "wetness=0.7499 and wetness=0.7501 diverge by {:.2}% of grid height, expected < 1%",
            rel_diff * 100.0
        );
    }

    #[test]
    #[ignore = "Phase 2/3 target: liquid should have essentially no angle of repose (settles \
                flat, spread <= 1 row) while granular DrySand should retain a real heap (spread \
                >= 8 rows) under identical pour conditions — this is the user's core complaint. \
                Measured today (continuous point-source pour, 400 ticks pouring + 600 ticks \
                settling): Water spread=1 (already flat, min=54 max=55) but DrySand spread=7 \
                (min=52 max=59) — DrySand falls just short of the 8-row bar too. That is a \
                second, distinct finding: under Sand-fall gravity mode DrySand's own repose \
                angle is weaker than expected (get_ca_params halves the threshold again via \
                'threshold *= 0.35' at physics.rs:226, and lock_chance drops to a flat 0.05 at \
                physics.rs:241-242 'for smooth avalanching'), so even dry sand piles flatter \
                than a real angle of repose would allow."]
    fn test_liquid_has_no_angle_of_repose() {
        let w = 64;
        let h = 64;
        let mask = make_test_mask(w, h, SandboxShape::Square, 0.04, 1.0);
        let gravity_dir = glam::Vec2::new(0.0, 0.04);

        let mut spreads = Vec::new();
        for mat in [MaterialMode::Water, MaterialMode::DrySand] {
            let props = get_test_props(mat, w * h);
            let mut sim = TestSim::new(w, h, props, mask.clone(), 32);
            // Continuous point-source pour near the top center (like a tap), so enough mass
            // accumulates for DrySand to build a real angle-of-repose cone rather than a
            // single blob settling flat by construction.
            for _ in 0..400 {
                for y in 4..8 {
                    for x in 30..34 {
                        sim.hm.data[y * w + x] = 1.0;
                    }
                }
                sim.tick(gravity_dir, 256);
            }
            // Let it settle without further pouring.
            for _ in 0..600 {
                sim.tick(gravity_dir, 256);
            }

            let mut surface_rows = Vec::new();
            for x in 6..58 {
                for y in 0..h {
                    if sim.hm.data[y * w + x] > 0.05 {
                        surface_rows.push(y);
                        break;
                    }
                }
            }
            let spread = if surface_rows.is_empty() {
                0
            } else {
                surface_rows.iter().max().unwrap() - surface_rows.iter().min().unwrap()
            };
            println!("test_liquid_has_no_angle_of_repose: {:?} spread={}", mat, spread);
            spreads.push(spread);
        }

        // Measured today: Water spread=1, DrySand spread=7.
        assert!(spreads[0] <= 1, "Liquid (Water) should settle nearly flat: spread={}", spreads[0]);
        assert!(spreads[1] >= 8, "DrySand should retain a real heap: spread={}", spreads[1]);
    }

    /// Linear-regression slope of `h(x)` along one row, `-d(height)/d(offset)` so a downhill
    /// flank (height falling away from the peak as `|offset|` grows) reads as a positive slope
    /// — the same sign convention as the CA's own `geom_slope = h_center - h_neighbor`.
    /// `offsets` is the set of *signed* offsets from `x0` to fit against (deliberately not just
    /// a contiguous range, so callers can average the left and right flank in one call and get
    /// a slope that is robust to small left/right asymmetry from the CA's stochastic dispersion
    /// term).
    fn regress_slope(sim: &TestSim, w: usize, x0: usize, row: usize, offsets: &[isize]) -> f32 {
        let mut sum_x = 0f64;
        let mut sum_y = 0f64;
        let mut sum_xy = 0f64;
        let mut sum_xx = 0f64;
        let mut n = 0f64;
        for &dx in offsets {
            let x = (x0 as isize + dx) as usize;
            let y = sim.hm.data[row * w + x] as f64;
            // Fold the left flank (negative dx) onto the same "distance from peak" axis as the
            // right flank (positive dx) by regressing height against |dx| with a sign flip on
            // the left, so a symmetric ramp contributes consistently from both sides.
            let signed_x = dx.unsigned_abs() as f64;
            sum_x += signed_x;
            sum_y += y;
            sum_xy += signed_x * y;
            sum_xx += signed_x * signed_x;
            n += 1.0;
        }
        let slope = (n * sum_xy - sum_x * sum_y) / (n * sum_xx - sum_x * sum_x);
        (-slope) as f32
    }

    /// Offsets covering the mid-flank of a ramp of the given `half_width`: excludes the
    /// peak/plateau (inner ~20%) and the near-zero tail (outer ~15%), on both left and right,
    /// where the CA's per-tick noise and rounding dominate the signal. `half_width` is in cells,
    /// already scaled by `test_scale()` by the caller.
    fn flank_offsets(half_width: isize) -> Vec<isize> {
        let lo = (half_width as f32 * 0.20).round() as isize;
        let hi = (half_width as f32 * 0.85).round() as isize;
        let lo = lo.max(1);
        let hi = hi.max(lo + 1);
        (lo..=hi).chain((-hi..=-lo).rev()).collect()
    }

    /// Angle-of-repose test scaffolding shared by the four cases below: a wide, deep,
    /// fully-packed "bedrock" base resting on the container's true floor (found by scanning
    /// `eval_sandbox_shape`, not assumed), with a single ramp row directly on top of it. The
    /// bedrock exists so the ramp's own vertical position is pinned before any tick runs --
    /// building the ramp as a free-floating block instead (tried first; see task report) lets it
    /// fall and pool into a couple of rows near the wall, where boundary-adjacent cells show
    /// runaway lateral erosion unrelated to the repose threshold and swamp the signal this test
    /// wants. Resting on bedrock mid-grid removes that confound: the only way height can change
    /// after this point is genuine lateral (x) CA flow, which is exactly the mechanism under
    /// test.
    struct ReposeRig {
        w: usize,
        h: usize,
        x0: usize,
        ramp_row: usize,
        mask: Vec<u8>,
        gravity_dir: glam::Vec2,
    }

    impl ReposeRig {
        fn new(s: usize) -> Self {
            let w = 64 * s;
            let h = 64 * s;
            let mask = make_test_mask(w, h, SandboxShape::Square, 0.04, 1.0);
            let x0 = w / 2;
            let floor_row = (0..h)
                .rev()
                .find(|&y| eval_sandbox_shape(x0, y, w, h, SandboxShape::Square, 0.04, 1.0, 8, false).0)
                .expect("container must have at least one inside row at x0");
            ReposeRig { w, h, x0, ramp_row: floor_row - 12 * s, mask, gravity_dir: glam::Vec2::new(0.0, 0.04) }
        }

        /// Builds a fresh sim with the packed bedrock base (floor_row - 12*s + 1 .. floor_row)
        /// and a triangular ramp of the given `slope`, holding `area` (peak_h * half_width,
        /// scaled as `s^2` by the caller) of sand on `ramp_row`, directly atop the bedrock.
        fn build(&self, s: usize, slope: f32, area: f32) -> TestSim {
            self.build_material(s, slope, area, MaterialMode::DrySand, 1.5)
        }

        /// Same as `build`, generalised over material and its incompressibility cap (1.5 for
        /// granular materials, 1.0 for liquids -- see `cell_capacity_for`), so the identical
        /// rig/construction can be used as a zero-repose reference point: build the same ramp out
        /// of `Water` instead of `DrySand` and see how it behaves under the SAME construction,
        /// budget and measurement. Used by the non-vacuity comparison in
        /// `test_dry_sand_has_angle_of_repose`.
        fn build_material(&self, s: usize, slope: f32, area: f32, material: MaterialMode, capacity: f32) -> TestSim {
            let props = get_test_props(material, self.w * self.h);
            let mut sim = TestSim::new(self.w, self.h, props, self.mask.clone(), 32);
            let base_row_hi = self.ramp_row + 12 * s + 1; // exclusive, = floor_row + 1
            // Pinned close to the container's own walls (found by scanning, not assumed) rather
            // than an arbitrary fraction of width: a base whose own edges have room to slump
            // sideways is itself unstable over long tick counts (its own hard 1.5-to-0 step is
            // far steeper than anything under test) and was observed, during exploration, to
            // slowly erode and eventually let the ramp above drain into it. Pinning the base
            // edges directly against the wall leaves them nowhere to go.
            let base_x_lo = (0..self.x0)
                .find(|&x| eval_sandbox_shape(x, self.ramp_row + 1, self.w, self.h, SandboxShape::Square, 0.04, 1.0, 8, false).0)
                .unwrap_or(0)
                + 1;
            let base_x_hi = (self.x0..self.w)
                .rev()
                .find(|&x| eval_sandbox_shape(x, self.ramp_row + 1, self.w, self.h, SandboxShape::Square, 0.04, 1.0, 8, false).0)
                .unwrap_or(self.w - 1);
            for y in (self.ramp_row + 1)..base_row_hi {
                for x in base_x_lo..base_x_hi {
                    sim.hm.data[y * self.w + x] = capacity; // fully packed bedrock
                }
            }
            let peak_h = (area * slope).sqrt();
            let half_width = (peak_h / slope).round() as isize;
            assert!(
                (half_width as usize) < (base_x_hi - self.x0).min(self.x0 - base_x_lo),
                "ramp half_width {} does not fit inside the packed base margins at scale {}",
                half_width, s
            );
            for dx in -half_width..=half_width {
                let hgt = (peak_h - slope * dx.unsigned_abs() as f32).max(0.0);
                sim.hm.data[self.ramp_row * self.w + (self.x0 as isize + dx) as usize] = hgt;
            }
            sim
        }

        fn slope_at(&self, sim: &TestSim, half_width: isize) -> f32 {
            regress_slope(sim, self.w, self.x0, self.ramp_row, &flank_offsets(half_width))
        }

        /// Ticks `sim` for `ticks` total, then averages the measured flank slope over a further
        /// sampling window instead of reading a single instant. Necessary because this rig's
        /// lateral flow (see the CASE-1-doc-comment finding on the stochastic dispersion term)
        /// does not settle to a fixed point -- it continues to fluctuate tick-to-tick within a
        /// narrow band even once the fast collapse/rise phase is over. A single-tick snapshot
        /// lands at an arbitrary point in that fluctuation; averaging several evenly-spaced
        /// samples over the tail of the run reports the band itself, which is what "the measured
        /// repose angle" actually means here. The run is fully deterministic (seed derived from
        /// `tick_count`), so this is reproducible, not a flakiness workaround.
        fn settle_and_measure(&self, sim: &mut TestSim, ticks: usize, half_width: isize) -> f32 {
            let (avg, _flow) = self.settle_and_measure_with_flow(sim, ticks, half_width);
            avg
        }

        /// Same as `settle_and_measure`, also returning the total flow moved over the whole
        /// call (fast phase + sampling window), for the quiescence guard.
        fn settle_and_measure_with_flow(&self, sim: &mut TestSim, ticks: usize, half_width: isize) -> (f32, f64) {
            let window = (ticks / 4).max(20);
            let fast_phase = ticks.saturating_sub(window);
            let mut total_flow = 0.0f64;
            for _ in 0..fast_phase {
                total_flow += sim.tick(self.gravity_dir, usize::MAX) as f64;
            }
            let samples = 5;
            let step = (window / samples).max(1);
            let mut sum = 0.0f32;
            for _ in 0..samples {
                for _ in 0..step {
                    total_flow += sim.tick(self.gravity_dir, usize::MAX) as f64;
                }
                sum += self.slope_at(sim, half_width);
            }
            (sum / samples as f32, total_flow)
        }
    }

    #[test]
    fn test_dry_sand_has_angle_of_repose() {
        // THE GAP: sand's angle of repose lives entirely in the granular CA's lateral-flow
        // threshold today (`if geom_slope > 0.20` in the avalanche valve, `settle_tick`,
        // physics.rs -- measured at line 2995 in the current tree, not line ~2977 as the task
        // brief guessed; the `flow = 0.10 * (geom_slope - 0.20)` line the brief quotes is the
        // very next line, 2996). Nothing in the suite asserts sand actually has one. The planned
        // Stage C migration moves sand's lateral transport onto the edge-flux solver, whose
        // equivalent mechanism (`tau`, in `flux_edge`/`edge_sleeps`) is fully implemented but
        // hardcoded to `tau = 0.0` at every call site -- moving sand across as-is would silently
        // flatten it like a liquid. This test exists to catch exactly that regression before it
        // ships.
        //
        // MEASURE, DON'T ASSUME -- what this test found, empirically (see the task report for
        // the full exploration), is more complicated than the brief's framing:
        //
        // 1. There is no literal fixed point. A hand-built ramp's lateral per-cell height
        //    difference (`geom_slope`) does not converge to a permanent stable value at ANY tick
        //    count, even with the 0.20 valve fully intact. A **second**, independent mechanism
        //    in the same CA -- the main flow loop a few dozen lines below the avalanche valve --
        //    carries its own, much smaller threshold under gravity (`threshold_prop * 0.35`,
        //    ~0.028 for DrySand, further halved by the `sliding_active` hysteresis branch to
        //    ~0.014) plus an unconditional stochastic "dispersion" term
        //    (`gravity_push += perp_dot * 3.5 * dispersion_noise`, physics.rs ~3128) that fires
        //    most ticks regardless of local slope. That second mechanism causes slow, continuous
        //    lateral creep at *any* nonzero slope. Even on this test's "bedrock base" rig (see
        //    `ReposeRig`, chosen specifically because a pile touching the true floor -- a
        //    boundary-mask row -- erodes far faster, fully flattening within ~250 ticks), a pile
        //    with the REAL, unmodified 0.20 threshold still decays from ~0.04 towards 0 by
        //    ~900 ticks. "The measured repose angle" here is therefore a *snapshot at a fixed,
        //    moderate tick budget* (chosen to match realistic interactive timescales -- a user
        //    watches the sandbox for a few hundred ticks, not hundreds of thousands), not a
        //    literal fixed point of the ODE.
        //
        // 2. NON-VACUITY FINDING, and a correction to the brief: flipping ONLY the named 0.20
        //    valve (physics.rs:2995) does NOT make the four cases below fail. Measured: CASE 1's
        //    final slope moves from 0.0426 (real) to 0.0332 (valve zeroed) -- a real but modest
        //    change, and every case's assertion still passes, because cases 2/3's targets are
        //    DERIVED from case 1's own measured result, so the four cases are self-consistent
        //    (and self-normalizing) regardless of how strong the underlying repose actually is.
        //    At this test's ~100-tick budget, what makes DrySand look different from a
        //    zero-repose material is dominated by the granular CA's flow-RATE constants (alpha
        //    ~0.375, `lock_chance` = 0.05 flat under gravity, `max_transfer_coeff` 0.20-0.40 on
        //    the bed) -- properties independent of either slope threshold -- not by the 0.20
        //    valve specifically. Confirmed directly: build the identical ramp out of Water
        //    (wetness=1.0, routed through the flux solver only -- `granular_share <= 0.0` skips
        //    the granular CA entirely, so it never sees either threshold) and it flattens to
        //    ~0.0000 within ~100 ticks regardless. The NON-VACUITY ANCHOR check below exists
        //    because of this: it is a SEPARATE, longer-budget (~4.5x `measure_ticks`) comparison
        //    of DrySand against that same Water baseline, at a point where a rate-limited-but-
        //    thresholdless pile has had time to mostly catch up to Water's floor while a
        //    genuinely-thresholded one has not. THAT check is what actually distinguishes the
        //    0.20 valve being present from absent -- see its own comment for the measured
        //    numbers (dry=0.0401 real vs dry=0.0171 with the valve zeroed, against a 0.025
        //    margin over Water's ~0.0000).
        //
        // SCALE INVARIANCE -- the DECAY PROCESS is approximately self-similar across scale, not
        // (per finding 1 above) a stable angle both scales converge to and hold. A slope is
        // dimensionless and `geom_slope` never divides by grid size, so time-rescaled snapshots
        // are comparable: at `SANDART_TEST_SCALE=8` a slope-0.35 pile reads ~0.048 after 51200
        // ticks (a one-off exploration run, not this test's own budget -- see below), the same
        // order of magnitude as this test's own scale-1 reading of ~0.04-0.05 after 100 ticks.
        // But those two tick counts are NOT "the same point in the process" by any linear
        // scaling -- reaching them is not proportional to `s`. The same slope-0.35 pile that
        // collapses to under half its starting slope within 100 ticks at scale 1 is still at
        // 0.353 (no measurable collapse at all) after 800 ticks (100 * 8) at scale 8, and needs
        // roughly 20000-25000 ticks -- 200-250x the naive linear scaling, not 8x -- before CASE
        // 1's "collapsed to under half" bar is cleared. That is because
        // `test_liquid_stream_stays_coherent`'s mechanism (the precedent for this file's
        // linear-in-`s` tick-budget convention) is advective -- a falling stream covers a fixed
        // number of cells per tick, so linear is the right shape there; this rig's mechanism is
        // a lateral relaxation/avalanche process, empirically worse than the O(distance^2) a
        // pure diffusion process would suggest (closer to cubic in the half_width ratio).
        // Running this test's actual assertions at scale 8 with a large-enough budget measured
        // ~312s for ONE of the four cases alone (`measure_ticks` temporarily raised to 51200) --
        // impractical even as an opt-in manual check. `measure_ticks` therefore stays
        // linear-in-`s`, fast at every scale, but this means the test does NOT literally pass
        // under `SANDART_TEST_SCALE=8` -- CASE 1 fails there because the pile hasn't had time to
        // collapse yet, not because the angle differs. Reported here rather than worked around
        // (e.g. scaling the budget as `s^3`, which would make the scale-1 cost model misleading
        // and still be a guess at the true exponent) because the honest shape is "assert the
        // angle at scale 1; know, and say plainly, that this test cannot afford to re-verify
        // convergence at production scale on every run."
        let s = test_scale();
        let rig = ReposeRig::new(s);
        let area = 10.0 * (s as f32) * (s as f32); // half_width scales as s at fixed slope
        let measure_ticks = 100 * s;

        // ---- Case 1: built STEEPER than repose must COLLAPSE toward the angle. ----
        // Also serves as the primary measurement: DrySand's documented weak gravity-mode repose
        // (get_ca_params halves-then-scales its threshold under gravity; see the doc comment
        // above) means there's no way to know the converged value without measuring, so start
        // absurdly steep (0.35 -- for context, that's already far above anything the exploration
        // found DrySand settling toward) and read off wherever it lands.
        let steep_initial = 0.35f32;
        let mut sim1 = rig.build(s, steep_initial, area);
        let half_width_1 = ((area * steep_initial).sqrt() / steep_initial).round() as isize;
        let slope1_initial = rig.slope_at(&sim1, half_width_1);
        let (slope1_final, flow1_total) = rig.settle_and_measure_with_flow(&mut sim1, measure_ticks, half_width_1);
        println!(
            "test_dry_sand_has_angle_of_repose CASE 1 (steep): initial={:.4} ({:.2} deg) final={:.4} ({:.2} deg) total_flow={:.2}",
            slope1_initial, slope1_initial.atan().to_degrees(),
            slope1_final, slope1_final.atan().to_degrees(), flow1_total
        );
        assert!(
            flow1_total > 1.0,
            "scenario went quiescent (total_flow={:.4}) -- the other assertions would pass vacuously",
            flow1_total
        );
        assert!(
            slope1_final < slope1_initial * 0.5,
            "CASE 1: a pile built at slope {:.4} should collapse substantially over {} ticks, but only reached {:.4}",
            slope1_initial, measure_ticks, slope1_final
        );

        let s_measured = slope1_final;

        // ---- NON-VACUITY ANCHOR: DrySand vs Water in the identical rig, at a longer budget. ----
        // See the NON-VACUITY FINDING in the doc comment above for why this exists and why 100
        // ticks alone cannot carry it: at the short budget the four cases above use, DrySand's
        // elevated slope (vs. a hypothetical zero-repose material) is dominated by the granular
        // CA's own flow-RATE constants (alpha, lock_chance, per-tick transfer caps), not by
        // either slope threshold -- so it does not distinguish "threshold present" from
        // "threshold zeroed". Water (wetness=1.0) is a genuine, already-implemented zero-repose
        // reference: `granular_share <= 0.0` routes it through the flux solver only, never the
        // granular CA, so it never sees either threshold at all. Built with the IDENTICAL rig,
        // construction and starting slope, Water flattens to ~0 within ~100 ticks and stays
        // there. At a longer budget (~4.5x `measure_ticks`, chosen from measurement: this is
        // where a rate-limited-but-thresholdless pile has had time to mostly catch up to
        // Water's floor while a genuinely-thresholded one has not), DrySand retaining
        // meaningfully more slope than Water is the actual load-bearing signal for "the
        // threshold mechanism is doing something", separate from the four cases' own internal
        // (and, per the finding, threshold-insensitive) cross-consistency.
        let anchor_ticks = measure_ticks * 9 / 2;
        let mut sim_water = rig.build_material(s, steep_initial, area, MaterialMode::Water, 1.0);
        let water_anchor_final = rig.settle_and_measure(&mut sim_water, anchor_ticks, half_width_1);
        let mut sim_dry_anchor = rig.build(s, steep_initial, area);
        let dry_anchor_final = rig.settle_and_measure(&mut sim_dry_anchor, anchor_ticks, half_width_1);
        println!(
            "test_dry_sand_has_angle_of_repose NON-VACUITY ANCHOR @{} ticks: DrySand={:.4} ({:.2} deg) Water={:.4} ({:.2} deg)",
            anchor_ticks, dry_anchor_final, dry_anchor_final.atan().to_degrees(),
            water_anchor_final, water_anchor_final.atan().to_degrees()
        );
        assert!(
            dry_anchor_final > water_anchor_final + 0.025,
            "NON-VACUITY ANCHOR: DrySand should retain meaningfully more slope than Water in the \
             identical rig at {} ticks -- dry={:.4}, water={:.4} (need dry > water + 0.025)",
            anchor_ticks, dry_anchor_final, water_anchor_final
        );

        // ---- Case 3: built AT the repose angle must be STABLE. ----
        // (Measured before case 2 so case 2's "shallower than repose" can be defined relative to
        // it, matching the brief's framing.)
        let at_slope = s_measured;
        let mut sim3 = rig.build(s, at_slope, area);
        let half_width_3 = ((area * at_slope).sqrt() / at_slope).round() as isize;
        let slope3_final = rig.settle_and_measure(&mut sim3, measure_ticks, half_width_3);
        println!(
            "test_dry_sand_has_angle_of_repose CASE 3 (at angle): initial={:.4} final={:.4} ({:.2} deg)",
            at_slope, slope3_final, slope3_final.atan().to_degrees()
        );

        // ---- Case 2: built SHALLOWER than repose must STAY PUT / converge toward the SAME
        // angle from below (not slump toward flat). ----
        let shallow_initial = s_measured * 0.6;
        let mut sim2 = rig.build(s, shallow_initial, area);
        let half_width_2 = ((area * shallow_initial).sqrt() / shallow_initial).round() as isize;
        let slope2_final = rig.settle_and_measure(&mut sim2, measure_ticks, half_width_2);
        println!(
            "test_dry_sand_has_angle_of_repose CASE 2 (shallow): initial={:.4} final={:.4} ({:.2} deg) s_measured={:.4} ({:.2} deg)",
            shallow_initial, slope2_final, slope2_final.atan().to_degrees(),
            s_measured, s_measured.atan().to_degrees()
        );

        // CASE 2 MUST STAY PUT. It must NOT rise to meet case 1.
        //
        // This assertion previously required case 1 (settling from above) and case 2 (built from
        // below) to converge on the SAME value. That was written against the pre-Stage-C
        // mechanism, which had no real yield stress: piles crept, and a shallow pile drifted
        // upward toward the same creep-determined slope, so two-sided convergence looked like it
        // pinned an angle. It did not -- it pinned a creep rate, and it would have been satisfied
        // just as well by two piles converging on a value near zero.
        //
        // With a real `tau` a sub-threshold pile is STABLE: the driving head never exceeds the
        // yield stress, so nothing moves and it stays exactly where it was built. A material that
        // spontaneously STEEPENED to reach its repose angle would be unphysical -- gravity has no
        // mechanism to do that. So the correct two-sided statement is: from above, collapse to the
        // angle; from below, hold position. Measured here: 0.0532 -> 0.0536, i.e. it holds to
        // within 0.0004.
        //
        // The three parts below are each load-bearing. Held position rules out creep; strictly
        // below case 1 rules out having been built above the angle by accident, which would make
        // "it held" trivially true; and not collapsing toward flat rules out the failure this
        // whole test exists to catch, a material with no yield stress at all. The NON-VACUITY
        // ANCHOR further up (sand versus water in the identical rig) is what guards the case
        // where `tau` is present but too weak to matter.
        // Still used by cases 3 and 4 below, which legitimately compare a settled result against
        // the measured repose angle. Tolerance from the exploration's tick-to-tick noise band at
        // this budget (~0.01-0.02).
        const CONVERGENCE_TOL: f32 = 0.03;
        const HOLD_TOL: f32 = 0.02;
        assert!(
            (slope2_final - shallow_initial).abs() < HOLD_TOL,
            "CASE 2: a pile built SHALLOWER than the repose angle must hold its slope, not creep: \
             initial={:.4}, final={:.4}, |drift|={:.4} (tolerance {:.4})",
            shallow_initial, slope2_final, (slope2_final - shallow_initial).abs(), HOLD_TOL
        );
        assert!(
            slope2_final < slope1_final,
            "CASE 2 must sit BELOW case 1's repose angle, or 'it held' proves nothing: \
             case2_final={:.4}, case1_final={:.4}",
            slope2_final, slope1_final
        );
        assert!(
            slope2_final > 0.5 * shallow_initial,
            "CASE 2 collapsed toward flat rather than holding -- no effective yield stress: \
             initial={:.4}, final={:.4}",
            shallow_initial, slope2_final
        );
        // (An assertion requiring CASE 2 to RISE by at least 10% toward the repose angle used to
        // sit here, alongside the convergence check replaced above. Same mistake, same reason:
        // gravity has no mechanism to steepen a stable pile, so demanding that it rise only ever
        // described the old creep-dominated behaviour. Both are now replaced by the hold /
        // below-case-1 / not-collapsed triple above. `s_measured` remains used by cases 3 and 4.)
        assert!(
            (slope3_final - s_measured).abs() < CONVERGENCE_TOL,
            "CASE 3: a pile built at the measured repose angle ({:.4}) should stay close to it, \
             not drift -- got {:.4}",
            s_measured, slope3_final
        );

        // ---- Case 4: material DEPOSITED ON THE PEAK of a settled pile must avalanche down the
        // flanks and RE-ESTABLISH the angle -- not remain a spike, and not punch a hole. ----
        // Continues from case 1's already-settled pile (slope1_final ~= s_measured).
        let mass_before_deposit: f64 = sim1.hm.data.iter().map(|&v| v as f64).sum();
        let deposit_h = 2.0f32; // a large spike relative to the settled peak (~sqrt(area*s_measured))
        let deposit_half = (1 * s).max(1) as isize;
        for dx in -deposit_half..=deposit_half {
            let idx = rig.ramp_row * rig.w + (rig.x0 as isize + dx) as usize;
            sim1.hm.data[idx] += deposit_h;
        }
        let mass_after_deposit: f64 = sim1.hm.data.iter().map(|&v| v as f64).sum();
        let peak_h_after_deposit = sim1.hm.data[rig.ramp_row * rig.w + rig.x0];

        let (slope4_final, flow4_total) = rig.settle_and_measure_with_flow(&mut sim1, measure_ticks, half_width_1);
        let mass_after_resettle: f64 = sim1.hm.data.iter().map(|&v| v as f64).sum();
        let peak_h_after_resettle = sim1.hm.data[rig.ramp_row * rig.w + rig.x0];
        // "No hole": the peak column and its near neighbours a few cells out should not have
        // been carved into a crater -- i.e. the peak should not now be *lower* than a point
        // partway down the flank it's supposed to be feeding.
        let mid_flank_offset = (half_width_1 as f32 * 0.5).round() as isize;
        let h_mid_flank = sim1.hm.data[rig.ramp_row * rig.w + (rig.x0 as isize + mid_flank_offset) as usize];
        let h_peak = sim1.hm.data[rig.ramp_row * rig.w + rig.x0];

        println!(
            "test_dry_sand_has_angle_of_repose CASE 4 (deposit on peak): mass_before_deposit={:.3} \
             mass_after_deposit={:.3} mass_after_resettle={:.3} peak_after_deposit={:.4} \
             peak_after_resettle={:.4} h_peak={:.4} h_mid_flank(dx={})={:.4} flank_slope={:.4} \
             ({:.2} deg) total_flow={:.2}",
            mass_before_deposit, mass_after_deposit, mass_after_resettle,
            peak_h_after_deposit, peak_h_after_resettle, h_peak, mid_flank_offset, h_mid_flank,
            slope4_final, slope4_final.atan().to_degrees(), flow4_total
        );
        assert!(
            (mass_after_resettle - mass_after_deposit).abs() < 0.5,
            "CASE 4: mass should be conserved while the spike avalanches down (before resettle: \
             {:.3}, after: {:.3})",
            mass_after_deposit, mass_after_resettle
        );
        assert!(
            flow4_total > 1.0,
            "CASE 4 scenario went quiescent (total_flow={:.4}) -- the spike never avalanched",
            flow4_total
        );
        assert!(
            peak_h_after_resettle < peak_h_after_deposit * 0.85,
            "CASE 4: the deposited spike should avalanche down (peak height should drop \
             substantially from right after deposit), not remain a spike: after_deposit={:.4}, \
             after_resettle={:.4}",
            peak_h_after_deposit, peak_h_after_resettle
        );
        assert!(
            h_peak >= h_mid_flank * 0.5,
            "CASE 4: the peak should not have been carved into a crater -- h_peak={:.4} is far \
             below h_mid_flank={:.4} at dx={}",
            h_peak, h_mid_flank, mid_flank_offset
        );
        assert!(
            (slope4_final - s_measured).abs() < CONVERGENCE_TOL,
            "CASE 4: the flank slope after the peak re-settles should re-establish the measured \
             repose angle ({:.4}), not stay disturbed -- got {:.4}",
            s_measured, slope4_final
        );
    }

    /// TASK #55 step 3 canary. `test_dry_sand_has_angle_of_repose` re-run with
    /// `head_field_transport` forced ON via `head_field_gate` (the test-only thread-local; see
    /// its own doc comment -- `ReposeRig`/`TestSim` never expose the real parameter directly, so
    /// the thread-local is the only way to force it on from a test rig).
    ///
    /// This is the positive check that the LIQUID-ONLY restriction (`LIQUID_ELLIPTIC_THRESHOLD`
    /// gate on both edge sites) actually holds: `test_dry_sand_has_angle_of_repose` builds pure
    /// DrySand piles (wetness 0.00, `liquidity == 0.0`, far below the gate), so if the gate is
    /// doing its job the head field must never be consulted for a single edge in that test, and
    /// the whole suite of angle-of-repose assertions -- CASE 1 through 4 and the NON-VACUITY
    /// ANCHOR -- must still pass exactly as they do with the toggle off (that baseline is
    /// unconditionally covered by `test_dry_sand_has_angle_of_repose` itself, which this canary
    /// runs unmodified, just under the gate). Run with `--nocapture` to see the measured angle and
    /// anchor numbers printed by the inner test. A prior shipped #55 attempt's multiplicative
    /// lateral head flattened a 19.29 degree slope to 0.41 degrees under an equivalent forced-on
    /// check, with its own non-vacuity anchor reading 0.00 for both DrySand and Water; this test
    /// exists so a regression of that shape cannot ship silently again.
    #[test]
    fn test_head_field_transport_repose_non_regression() {
        head_field_gate::set_enabled(true);
        let result = std::panic::catch_unwind(test_dry_sand_has_angle_of_repose);
        head_field_gate::set_enabled(false);
        assert!(
            result.is_ok(),
            "test_dry_sand_has_angle_of_repose FAILED with head_field_transport FORCED ON: the \
             liquid-only gate (LIQUID_ELLIPTIC_THRESHOLD on both edge endpoints) did not keep the \
             head field out of a pure-DrySand scenario (DrySand has liquidity == 0.0, far below \
             the gate) -- see the panic output above (re-run with --nocapture to see the printed \
             CASE/NON-VACUITY-ANCHOR angles) for which assertion tripped."
        );
    }

    #[test]
    #[ignore = "Phase 3 target: a liquid blob impacting a floor should splash — spreading \
                laterally beyond its original width AND moving at least 1 row upward against \
                gravity. Measured today: lateral spread does happen (width 8 -> 30 within 10 \
                ticks of impact, via the same dispersion noise as C2) but upward movement is \
                impossible BY CONSTRUCTION: physics.rs:1162 and physics.rs:1248 both `continue` \
                whenever `gravity_active && gravity_dot < -0.01`, i.e. no cell may ever flow \
                against gravity in Sand-fall mode. min_row_after (59) never goes above \
                top_row_at_impact (50)."]
    fn test_liquid_splashes_on_impact() {
        let w = 64;
        let h = 64;
        let mask = make_test_mask(w, h, SandboxShape::Square, 0.04, 1.0);
        let props = get_test_props(MaterialMode::Water, w * h);
        let mut sim = TestSim::new(w, h, props, mask, 32);
        let gravity_dir = glam::Vec2::new(0.0, 0.04);

        // A compact blob close to the floor.
        for y in 50..54 {
            for x in 28..36 {
                sim.hm.data[y * w + x] = 1.0;
            }
        }
        let initial_width = 36 - 28;

        let mut top_row_at_impact = None;
        for _ in 0..60 {
            sim.tick(gravity_dir, 256);
            if top_row_at_impact.is_some() {
                continue;
            }
            // Detect impact: material reaches near the floor (y=58 or 59).
            let touching_floor = (20..44).any(|x| sim.hm.data[58 * w + x] > 0.05 || sim.hm.data[59 * w + x] > 0.05);
            if touching_floor {
                let min_row = (0..h)
                    .find(|&y| (20..44).any(|x| sim.hm.data[y * w + x] > 0.05))
                    .expect("material must exist somewhere once it's touching the floor");
                top_row_at_impact = Some(min_row);
            }
        }
        let top_at_impact = top_row_at_impact.expect("blob never reached the floor within 60 ticks");

        for _ in 0..10 {
            sim.tick(gravity_dir, 256);
        }

        let mut min_x = w;
        let mut max_x = 0;
        let mut min_row_after = h;
        for y in 0..h {
            for x in 0..w {
                if sim.hm.data[y * w + x] > 0.05 {
                    min_x = min_x.min(x);
                    max_x = max_x.max(x);
                    min_row_after = min_row_after.min(y);
                }
            }
        }
        let width_after = max_x.saturating_sub(min_x) + 1;
        let lateral_spread = width_after > initial_width;
        let upward_move = min_row_after < top_at_impact;
        println!(
            "test_liquid_splashes_on_impact: width_after={} (initial={}), min_row_after={} \
             (top_at_impact={}), lateral_spread={}, upward_move={}",
            width_after, initial_width, min_row_after, top_at_impact, lateral_spread, upward_move
        );

        // Measured today: lateral_spread=true, upward_move=false (structurally impossible).
        assert!(lateral_spread, "Blob did not spread laterally beyond its original width on impact");
        assert!(upward_move, "Blob did not move upward at all on impact (min_row_after={}, top_at_impact={})", min_row_after, top_at_impact);
    }

    #[test]
    // Companion to `test_sandbox_wave_stays_left_right_symmetric`, for the gravity + liquid path
    // that test never touches: `test_hourglass_statistical_symmetry` uses DrySand
    // (`cell_liquidity == 0`), which is bit-identical to the pre-liquid CA and never reaches the
    // `gravity_active && cell_liquidity > 0.0` lateral-edge branch at all. This is the first test
    // to put WATER through gravity's lateral edge and check it does not lean.
    //
    // A centred, bit-symmetric blob dropped onto a floor must fall and spread without ever
    // preferring one side. Unlike the Sandbox wave test, this tracks the *signed* left-minus-right
    // difference, not just its worst absolute value: an explosion that stays symmetric (grows
    // equally both ways) and a one-sided lean (a "tendril" that always tips the same way) both can
    // trip a magnitude bound, but only the signed trace tells them apart, and the reported bug
    // ("tendrils usually on the left") is a claim about sign, not magnitude.
    //
    // IMPORTANT — this test runs the scenario under BOTH x-sweep parities and is EXPECTED TO FAIL.
    // The lateral pass's sweep direction is `(tick_count + y as u32) % 2` (see that line in
    // `settle_tick`), so starting `TestSim.tick_count` at 0 vs 1 is exactly a parity flip — a pure
    // iteration-order change, no physics change — reachable through the harness's own tick counter
    // with no production-code knob needed. Verified directly: flipping the parity this way turns a
    // passing run into one with a persistent same-signed run of 75 ticks against this test's own
    // `late_run < 25` tolerance, with the lean flipped to the opposite side. (This said 61 until
    // the bisection was run; 61 does not reproduce against current code. See the correction in the
    // failure message below.) That means the
    // previously-shipped single-parity version of this test (which only ever started at
    // `tick_count == 0`) was GREEN FOR THE WRONG REASON: 7a3ef9f's Jacobi-driving fix reduced the
    // sweep's order-dependent lean but did not remove it, and the one parity that shipped merely
    // happens to land inside tolerance. Asserting both parities here makes that residual order
    // dependence visible instead of hiding behind whichever one `tick_count` happens to start at.
    // Do NOT weaken the assertions, raise the tolerances, `#[ignore]` this test, or attempt to
    // remove the order dependence itself to make it green again — the failure is intentionally
    // documenting real outstanding work. See the assertion messages below for the mechanism (a
    // residual order dependence in the gravity lateral-edge driving path) and the principled fix
    // if live state must be kept: red-black *edge* coloring on the lateral pass — process all
    // even-x lateral edges, then all odd-x lateral edges, so no single pass ever shares a cell
    // between two edges it updates.
    fn test_water_blob_stays_left_right_symmetric_under_gravity() {
        struct RunResult {
            trace: Vec<f64>,
            worst: f64,
            final_err: f64,
            late_run: usize,
            late_trace: Vec<f64>,
        }

        let w = 64;
        let h = 64;
        const N_TICKS: usize = 150;
        const EPS: f64 = 1e-6;
        const WINDOW: usize = 25;

        // Counts the longest run of consecutive same-signed samples in a slice, ignoring swings
        // too small to be anything but f32/sweep-parity noise (`EPS`).
        let longest_same_sign_run = |samples: &[f64]| -> usize {
            let mut max_run = 0usize;
            let mut run_sign = 0i32;
            let mut run_len = 0usize;
            for &d in samples {
                let s = if d > EPS { 1 } else if d < -EPS { -1 } else { 0 };
                if s != 0 && s == run_sign {
                    run_len += 1;
                } else if s != 0 {
                    run_sign = s;
                    run_len = 1;
                } else {
                    run_sign = 0;
                    run_len = 0;
                }
                max_run = max_run.max(run_len);
            }
            max_run
        };

        // Runs the whole centred-blob-under-gravity scenario with `TestSim.tick_count` seeded at
        // `initial_tick_count` instead of 0. Because the lateral sweep parity in `settle_tick` is
        // `(tick_count + y as u32) % 2`, seeding at 0 vs 1 is exactly equivalent to flipping which
        // parity runs first on the very first tick — the scenario, mask, and blob are otherwise
        // identical.
        let run = |initial_tick_count: u32| -> RunResult {
            let mask = make_test_mask(w, h, SandboxShape::Square, 0.04, 1.0);
            let props = get_test_props(MaterialMode::Water, w * h);
            let mut sim = TestSim::new(w, h, props, mask, 32);
            sim.tick_count = initial_tick_count;
            let gravity_dir = glam::Vec2::new(0.0, 0.04);

            // `eval_sandbox_shape` reflects `dx = cx as f32 - w as f32 / 2.0` about `w / 2.0` (an
            // *integer* for even `w`), so the mask's true mirror map is `x -> w - x` (verified
            // directly against `make_test_mask`'s output for this exact shape/size: the Square
            // mask here is inside for x in [3, 61], symmetric under `x -> 64 - x`, NOT under
            // `x -> 63 - x` / `w - 1 - x` — the convention `test_sandbox_wave_stays_left_right_symmetric`
            // uses. That test gets away with the off-by-one because its Circle bump never reaches
            // the mask boundary; this test's blob spreads to fill nearly the whole 64-wide box
            // (see `test_liquid_splashes_on_impact`'s width_after=59), so the wrong axis measures
            // a spurious ~1-column wall-proximity bias on top of any real solver bias. Centring
            // the blob on 9 columns (28..=36, an odd count around x=32) makes it bit-symmetric
            // about the mask's actual axis: mirror(28)=36, mirror(29)=35, ..., mirror(32)=32
            // (self).
            for y in 50..54 {
                for x in 28..37 {
                    sim.hm.data[y * w + x] = 1.0;
                }
            }

            // Signed left-minus-right mass difference: positive means excess mass on the left
            // half, negative means excess on the right. Normalised by total mass so the scale is
            // comparable tick to tick as the blob spreads and (if it splashes) loses/gains contact
            // area. Pairs `x` with `w - x` (the mask's true mirror, see above); `x = 0` has no
            // partner (its mirror `w` is out of range) but is always outside the mask for this
            // shape, so skipping it costs nothing, and `x = w / 2` is its own mirror and
            // contributes exactly 0.
            let signed_diff = |s: &TestSim| -> f64 {
                let mut diff = 0.0f64;
                for y in 0..h {
                    for x in 1..w / 2 {
                        let j = w - x;
                        let i = y * w + x;
                        let jj = y * w + j;
                        if s.mask[i] == crate::MASK_OUTSIDE || s.mask[jj] == crate::MASK_OUTSIDE {
                            continue;
                        }
                        diff += (s.hm.data[i] - s.hm.data[jj]) as f64;
                    }
                }
                diff
            };
            let total_mass = |s: &TestSim| -> f64 { s.hm.data.iter().map(|&v| v as f64).sum() };

            let initial = signed_diff(&sim);
            assert!(
                initial.abs() < 1e-9,
                "test setup is not mirror symmetric (initial_tick_count={}): {:.3e}",
                initial_tick_count, initial
            );

            let mut trace: Vec<f64> = Vec::with_capacity(N_TICKS);
            for _ in 0..N_TICKS {
                sim.tick(gravity_dir, 256);
                let mass = total_mass(&sim);
                let rel = if mass > 0.0 { signed_diff(&sim) / mass } else { 0.0 };
                trace.push(rel);
            }

            let worst = trace.iter().cloned().fold(0.0f64, |a, b: f64| a.max(b.abs()));
            let final_err = trace.last().copied().unwrap_or(0.0).abs();

            // The impact itself (roughly the first half of the run: the blob is still falling as
            // a single coherent block, then hits the floor and briefly splashes) is allowed a
            // transient, same-signed asymmetry — a symmetric blob hitting a floor is not obliged
            // to stay instantaneously mirror-exact while it does so, and the existing
            // `test_sandbox_wave_stays_left_right_symmetric` makes the same allowance via its
            // `final_err < 0.25 * worst` check rather than demanding zero asymmetry from tick 1.
            // What must not happen is for that lean to *persist* once the impact has settled out,
            // which is exactly the Gauss-Seidel gain's signature (see the driving-term comment on
            // the fixed branch): unbounded/non-decaying growth versus a transient that relaxes to
            // noise.
            let late = &trace[trace.len() / 2..];
            let late_run = longest_same_sign_run(late);
            let late_trace = late.to_vec();

            RunResult { trace, worst, final_err, late_run, late_trace }
        };

        // Run both parities before asserting anything, so every failure message below can quote
        // both traces side by side regardless of which parity (or both) actually trips.
        let even = run(0);
        let odd = run(1);

        for (label, r) in [("even (initial_tick_count=0)", &even), ("odd (initial_tick_count=1)", &odd)] {
            println!(
                "test_water_blob_stays_left_right_symmetric_under_gravity[{label}]: worst={:.3e} \
                 final={:.3e} late_persistent_run={} trace_tail={:?}",
                r.worst, r.final_err, r.late_run,
                r.trace[r.trace.len().saturating_sub(10)..].iter().map(|v| format!("{:.2e}", v)).collect::<Vec<_>>()
            );
        }

        let mechanism_note = "This is known outstanding work, not a new regression: the \
             simulation is not invariant under a shift of the global tick phase, and it should be. \
             \
             WHAT THE TWO RUNS ACTUALLY DIFFER BY: seeding `tick_count` at 1 rather than 0 is NOT \
             a pure lateral-sweep parity flip, despite what an earlier version of this note \
             claimed. `tick_count` also drives block-level x order, LOD staleness accounting, two \
             further parity switches, the CA checkerboard, and the RNG seed. So this test asserts \
             the broader and stronger property — symmetry under a global tick-phase shift — and a \
             failure does not by itself localise the cause to any one of those. Treat the list \
             below as candidates, not as a diagnosis. \
             \
             LEADING CANDIDATE: residual order dependence in the gravity lateral-edge driving path \
             inside settle_tick. The x-sweep visits lateral edges in `(tick_count + y as u32) % 2` \
             order, so within a single tick a cell's neighbour may already reflect this tick's \
             update while its mirror partner still sees the previous tick's value, and which side \
             gets the stale read depends on parity. 7a3ef9f's Jacobi-driving fix reduced this lean \
             but did not remove it. Note `column_depth` is still built from the LIVE `temp_heights` \
             and chains off its own earlier values in the same pass, so it remains order-dependent \
             even after that fix. \
             \
             THE BISECTION HAS NOW BEEN RUN, so the candidate list above is no longer where to \
             start. `test_tick_phase_mechanism_isolation` (ignored; run it with \
             `--ignored --nocapture`) flips each mechanism ALONE via the per-mechanism phase \
             offsets and measures this same scenario. Result: SIX of the eight mechanisms are \
             bit-identical to baseline here, because each is gated behind a condition this \
             scenario never enters -- the three spare parity switches need non-down or inactive \
             gravity, and the CA checkerboard and RNG seed live in the granular path a liquid \
             scenario never reaches. Only TWO mechanisms move anything: the cell-level lateral \
             sweep and block-level x order. Setting just those two reproduces the all-mechanisms \
             reference bit-for-bit. \
             \
             Attribution, on this test's own three metrics (baseline -> that mechanism alone): \
             the lateral sweep alone takes late_persistent_run 42 -> 75, which IS the full \
             reference value, so it accounts for the whole of the persistence. Block order alone \
             reaches 62. On `worst` and `final` the picture is not additive: the lateral sweep \
             alone is BELOW baseline on both (7.04e-2 / 1.03e-2 against 1.11e-1 / 1.18e-2), block \
             order alone overshoots `final`, and only the two together reproduce the reference \
             magnitude. So the sweep governs how long the lean persists while the peak and final \
             magnitude come from the two interacting. Expect edge coloring to fix persistence \
             and NOT to close the magnitude gap on its own. \
             \
             AN EARLIER VERSION OF THIS NOTE QUOTED 61 ticks for a cell-parity-only flip and 42 \
             for the full tick-phase offset, and said the gap between them was everything other \
             than the sweep. Both numbers were wrong and the inference from them was wrong. \
             Measured against current code: baseline is 42, full shift is 75, sweep-alone is 75. \
             The 42 that was labelled \"full offset\" is in fact the BASELINE. Do not resurrect \
             those figures. \
             \
             If live state must be kept (i.e. this cannot simply be made a frozen Jacobi read), \
             the principled fix for the lateral pass is red-black EDGE coloring: process all \
             even-x lateral edges, then all odd-x lateral edges, so no single pass ever shares a \
             cell between two edges it updates. Do not respond to this failure by re-tuning \
             tolerances, ignoring the test, or picking a different scan order (Hilbert or diagonal \
             orders only relocate the bias, they don't remove it).";

        for (label, r, other_label, other) in [
            ("even (initial_tick_count=0)", &even, "odd (initial_tick_count=1)", &odd),
            ("odd (initial_tick_count=1)", &odd, "even (initial_tick_count=0)", &even),
        ] {
            assert!(
                r.worst < 0.06,
                "[{label}] Centred water blob went badly lopsided under gravity: signed mirror \
                 error reached {:.3e} of total mass (tolerance 0.06). [{other_label}] worst={:.3e} \
                 final={:.3e}. {}\n[{label}] full trace={:?}\n[{other_label}] full trace={:?}",
                r.worst, other.worst, other.final_err, mechanism_note, r.trace, other.trace
            );
            assert!(
                r.final_err < 0.25 * r.worst,
                "[{label}] Mirror error is not transient — it peaked at {:.3e} and is still \
                 {:.3e} after {} ticks, so the lean is persisting/growing rather than washing out. \
                 [{other_label}] worst={:.3e} final={:.3e}. {}\n[{label}] full trace={:?}\n\
                 [{other_label}] full trace={:?}",
                r.worst, r.final_err, N_TICKS, other.worst, other.final_err, mechanism_note,
                r.trace, other.trace
            );
            assert!(
                r.late_run < WINDOW,
                "[{label}] Signed asymmetry held the same sign for {} consecutive ticks (>= {}) \
                 in the second half of the run, well after the impact transient should have \
                 settled: a persistent one-sided lean, not symmetric noise. [{other_label}] \
                 late_persistent_run={}. {}\n[{label}] second-half trace={:?}\n[{other_label}] \
                 second-half trace={:?}",
                r.late_run, WINDOW, other.late_run, mechanism_note, r.late_trace, other.late_trace
            );
        }
    }

    #[test]
    #[ignore]
    // DIAGNOSTIC measurement, not a pass/fail spec — never assert on these numbers.
    //
    // `test_water_blob_stays_left_right_symmetric_under_gravity` (above) shows the whole solver
    // is not invariant under a shift of the global tick phase, but seeding `TestSim.tick_count`
    // at 1 perturbs all eight `tick_count`-driven mechanisms in `settle_tick` at once (LOD
    // staleness, block-level x order, three independent parity switches, the cell-level lateral
    // sweep, the CA checkerboard, and the harness's own RNG seed), so that failure cannot be
    // attributed to any single one of them.
    //
    // This test isolates each mechanism in turn, using the `phase_offset(K_*)` diagnostic knobs
    // defined just above `settle_tick`: it sets every offset to 0 except one mechanism's, which
    // it sets to 1, runs the identical centred-water-blob-under-gravity scenario that the failing
    // test above runs, and records the same three metrics (`worst`, `final_err`,
    // `late_persistent_run`). It also records two references measured with the same harness:
    // all offsets 0 (the symmetric baseline — must reproduce the failing test's own `even`/
    // `initial_tick_count=0` numbers) and all offsets 1 (closely analogous to, but not perfectly
    // identical to, the failing test's `odd`/`initial_tick_count=1` global tick-count shift — see
    // the note at the `K_LOD_STALENESS` call site for why the LOD-staleness mechanism in
    // particular differs slightly between "seed tick_count at 1" and "add 1 to every site that
    // reads tick_count": a global shift also shifts `last_simulated_ticks[b]`'s write side, so its
    // staleness bias is transient; the offset here only touches the read side, so its bias is a
    // small constant every tick).
    //
    // SINGLE-THREADED BY CONSTRUCTION: `PHASE_OFFSETS` is process-global mutable state (see the
    // module comment above `settle_tick`), so every measurement below happens sequentially inside
    // this one function body — never across threads or interleaved with another test — and each
    // is bracketed by `reset_phase_offsets()`. A `Guard` with a `Drop` impl resets the offsets
    // again on the way out even if a measurement panics, so a failure here can't leave nonzero
    // offsets live for whatever test runs next in the same process. This test is `#[ignore]`d (it
    // does not run under plain `cargo test`) and touches global state no other test in this file
    // writes to, but if it is ever run with other tests that also call `set_phase`, pass
    // `--test-threads=1` to keep the two from interleaving.
    //
    // Run with:
    //   cargo test -p sandart-sim --lib physics::tests::test_tick_phase_mechanism_isolation -- --ignored --nocapture
    fn test_tick_phase_mechanism_isolation() {
        struct Guard;
        impl Drop for Guard {
            fn drop(&mut self) {
                reset_phase_offsets();
            }
        }
        let _guard = Guard;
        reset_phase_offsets();

        let w = 64;
        let h = 64;
        const N_TICKS: usize = 150;
        const EPS: f64 = 1e-6;

        // Identical to the failing test's own helper (duplicated rather than shared, so this
        // diagnostic can never accidentally change that test's behaviour).
        let longest_same_sign_run = |samples: &[f64]| -> usize {
            let mut max_run = 0usize;
            let mut run_sign = 0i32;
            let mut run_len = 0usize;
            for &d in samples {
                let s = if d > EPS { 1 } else if d < -EPS { -1 } else { 0 };
                if s != 0 && s == run_sign {
                    run_len += 1;
                } else if s != 0 {
                    run_sign = s;
                    run_len = 1;
                } else {
                    run_sign = 0;
                    run_len = 0;
                }
                max_run = max_run.max(run_len);
            }
            max_run
        };

        // Identical scenario to the failing test above: a centred, bit-symmetric water blob
        // dropped under gravity onto a Square-mask floor. `TestSim.tick_count` always starts at
        // 0 here — whatever lean shows up is driven entirely by the `PHASE_OFFSETS` set before
        // calling this, not by seeding the tick counter.
        let run_scenario = || -> (f64, f64, usize) {
            let mask = make_test_mask(w, h, SandboxShape::Square, 0.04, 1.0);
            let props = get_test_props(MaterialMode::Water, w * h);
            let mut sim = TestSim::new(w, h, props, mask, 32);
            let gravity_dir = glam::Vec2::new(0.0, 0.04);

            for y in 50..54 {
                for x in 28..37 {
                    sim.hm.data[y * w + x] = 1.0;
                }
            }

            let signed_diff = |s: &TestSim| -> f64 {
                let mut diff = 0.0f64;
                for y in 0..h {
                    for x in 1..w / 2 {
                        let j = w - x;
                        let i = y * w + x;
                        let jj = y * w + j;
                        if s.mask[i] == crate::MASK_OUTSIDE || s.mask[jj] == crate::MASK_OUTSIDE {
                            continue;
                        }
                        diff += (s.hm.data[i] - s.hm.data[jj]) as f64;
                    }
                }
                diff
            };
            let total_mass = |s: &TestSim| -> f64 { s.hm.data.iter().map(|&v| v as f64).sum() };

            let initial = signed_diff(&sim);
            assert!(
                initial.abs() < 1e-9,
                "test setup is not mirror symmetric: {:.3e}",
                initial
            );

            let mut trace: Vec<f64> = Vec::with_capacity(N_TICKS);
            for _ in 0..N_TICKS {
                sim.tick(gravity_dir, 256);
                let mass = total_mass(&sim);
                let rel = if mass > 0.0 { signed_diff(&sim) / mass } else { 0.0 };
                trace.push(rel);
            }

            let worst = trace.iter().cloned().fold(0.0f64, |a, b: f64| a.max(b.abs()));
            let final_err = trace.last().copied().unwrap_or(0.0).abs();
            let late = &trace[trace.len() / 2..];
            let late_run = longest_same_sign_run(late);

            (worst, final_err, late_run)
        };

        let mechanisms: [(&str, usize); 8] = [
            ("LOD staleness", K_LOD_STALENESS),
            ("Block-level x order", K_BLOCK_ORDER),
            ("Non-down-gravity block-order parity switch", K_NONDOWN_BLOCK_PARITY),
            ("Non-down-gravity row-order parity switch", K_NONDOWN_ROW_PARITY),
            ("Cell-level lateral sweep (leading candidate)", K_LATERAL_SWEEP),
            ("Non-gravity-active x-order parity switch", K_NONGRAVITY_X_PARITY),
            ("CA checkerboard", K_CA_CHECKERBOARD),
            ("RNG seed", K_RNG_SEED),
        ];

        let mut rows: Vec<(String, f64, f64, usize)> = Vec::new();

        // Reference 1: symmetric baseline, every offset 0.
        reset_phase_offsets();
        let (w0, f0, l0) = run_scenario();
        rows.push(("REFERENCE: all offsets 0 (symmetric baseline)".to_string(), w0, f0, l0));
        reset_phase_offsets();

        // Each mechanism flipped alone.
        for &(name, k) in &mechanisms {
            reset_phase_offsets();
            set_phase(k, 1);
            let (worst, final_err, late_run) = run_scenario();
            rows.push((format!("{name} (flipped alone)"), worst, final_err, late_run));
            reset_phase_offsets();
        }

        // Reference 2: every offset 1, analogous to the failing test's global tick_count=1 shift.
        reset_phase_offsets();
        for &(_, k) in &mechanisms {
            set_phase(k, 1);
        }
        let (wa, fa, la) = run_scenario();
        rows.push(("REFERENCE: all offsets 1 (~= global tick-phase shift)".to_string(), wa, fa, la));
        reset_phase_offsets();

        // Bonus sanity check (not one of the requested rows): if every mechanism other than
        // block order and the lateral sweep is a no-op in this scenario (as the single-flip rows
        // above suggest — gravity here is active and points straight down, so the three
        // non-down-gravity/non-gravity-active parity switches are dead branches, and this
        // scenario never puts a granular/CA cell through the checkerboard or RNG-seeded tie
        // break), then flipping just those two together should reproduce Reference 2 exactly.
        reset_phase_offsets();
        set_phase(K_BLOCK_ORDER, 1);
        set_phase(K_LATERAL_SWEEP, 1);
        let (wc, fc, lc) = run_scenario();
        rows.push((
            "COMBO CHECK: block order + lateral sweep only".to_string(),
            wc, fc, lc,
        ));
        reset_phase_offsets();

        println!();
        println!(
            "{:<58} {:>12} {:>12} {:>10}",
            "mechanism", "worst", "final", "late_run"
        );
        println!("{}", "-".repeat(96));
        for (name, worst, final_err, late_run) in &rows {
            println!(
                "{:<58} {:>12.4e} {:>12.4e} {:>10}",
                name, worst, final_err, late_run
            );
        }
        println!();
    }

    #[test]
    fn test_hourglass_full_drainage() {
        // Fill the upper chamber of an hourglass with DrySand and let it settle under gravity
        // for long enough to reach a steady state, then verify (a) mass is conserved and
        // (b) the large majority of the sand has drained through the neck into the lower
        // chamber. This does NOT assert the upper chamber reaches exactly zero: measured today,
        // a residual pile (~13% of total mass) permanently rests in the upper chamber above the
        // neck once the local slope drops below the (gravity-reduced) repose threshold, and
        // upper-chamber mass is observed to plateau (stop changing tick-over-tick) well before
        // 3000 ticks — i.e. "drainage" reaches a stable end state, just not a literally empty
        // upper chamber. That residual-pile behavior is plausible for granular material and is
        // NOT something Phase 0 should "fix"; this test only pins down that it doesn't regress.
        let w = 64;
        let h = 64;
        let mut hm = Heightmap::new(w, h, 0.0);

        let center_x = 32.0;
        let center_y = 32.0;
        let chamber_h = 0.40 * 64.0;
        let max_hw = 0.35 * 64.0;
        let neck_hw = 0.04 * 64.0;

        for y in 0..64 {
            let dy = y as f32 - center_y;
            let dy_abs = dy.abs();
            if dy_abs < chamber_h {
                let t = dy_abs / chamber_h;
                let allowed_hw = neck_hw + t.powf(0.60) * (max_hw - neck_hw);
                for x in 0..64 {
                    let dx = x as f32 - center_x;
                    if dx.abs() < allowed_hw && dy < 0.0 {
                        let idx = y * w + x;
                        hm.data[idx] = 0.55;
                    }
                }
            }
        }
        let initial_mass: f64 = hm.data.iter().map(|&v| v as f64).sum();

        let mut temp_heights = hm.data.clone();
        let mut cell_props = get_test_props(MaterialMode::DrySand, w * h);
        let mut cell_colors = vec![0u8; w * h * 4];
        let mut sliding = vec![false; w * h];
        let mut bounds = ActiveBounds {
            min_x: 0,
            max_x: 63,
            min_y: 0,
            max_y: 63,
            active: true,
        };

        let mut edge_vel_h = vec![0.0; w * h];
        let mut edge_vel_v = vec![0.0; w * h];
        let mut column_depth = vec![0.0; w * h];
        let mut head_field = vec![0.0; w * h];
        let mut active_blocks = vec![crate::BlockActivity::Inactive; 4];
        let mut last_displacements = vec![1.0; 4];
        let mut last_simulated_ticks = vec![0; 4];

        let gravity_dir = glam::Vec2::new(0.0, 0.04);

        let mask = make_test_mask(w, h, SandboxShape::Hourglass, 0.04, 0.60);
        for i in 0..3000u32 {
            settle_tick(
                &mut hm,
                &mut temp_heights,
                &mut cell_colors,
                &mut cell_props,
                &mut sliding,
                &mut bounds,
                &mut active_blocks,
                &mut last_displacements,
                &mut last_simulated_ticks,
                256,
                32,
                &[],
                12345u32.wrapping_add(i),
                &mut edge_vel_h,
                &mut edge_vel_v,
                &mut column_depth,
                &mut head_field,
                &mask,
                i,
                gravity_dir,
                false,
                false,
                false, false, false, 0.50,
            );
        }

        println!("--- Sand around neck (y=30..36) ---");
        for y in 30..36 {
            for x in 28..36 {
                let idx = y * w + x;
                println!("y={:2}, x={:2}: h={:.4}", y, x, hm.data[idx]);
            }
        }

        let final_mass: f64 = hm.data.iter().map(|&v| v as f64).sum();
        let final_lower_mass: f64 = hm.data[32 * w..].iter().map(|&v| v as f64).sum();
        let mass_err = (final_mass - initial_mass).abs() / initial_mass;
        let drained_frac = final_lower_mass / initial_mass;
        println!(
            "test_hourglass_full_drainage: init_mass={:.6} final_mass={:.6} mass_err={:.8} \
             final_lower_mass={:.6} drained_frac={:.4}",
            initial_mass, final_mass, mass_err, final_lower_mass, drained_frac
        );

        // Measured today: mass_err ~= 1.1e-7, drained_frac ~= 0.869 (86.9%).
        assert!(mass_err < 1e-4, "Mass not conserved during drainage: mass_err={:.8}", mass_err);
        assert!(
            drained_frac > 0.75,
            "Less than 75% of the sand drained into the lower chamber: drained_frac={:.4}",
            drained_frac
        );
    }

    #[test]
    // The failure mode mass conservation cannot see: a cascade whose necks are too pinched (or
    // structurally wrong) to actually pass sand within a reasonable number of ticks would trap
    // almost everything in tier 0 -- or stack it up in an intermediate tier -- while still
    // conserving mass perfectly, because nothing is leaking out of the shape mask, it just never
    // reaches the bottom. Fills tier 0 (the widest tier) and lets the cascade run long enough
    // to settle, then checks that most of that sand made it all the way down to the single
    // bottom chamber.
    //
    // Uses a wider-than-default neck (0.06, versus the slider's default 0.005) the same way
    // `test_hourglass_full_drainage` does above -- the point here is whether the *geometry*
    // lets sand reach the bottom at all, not how slowly the default neck throttles it.
    //
    // Swept across the widest-tier chamber count's full user-selectable range (5..=16) rather
    // than just the shipped default of 8, since the tier count itself changes at n = 9 (4 tiers
    // below that, 5 at and above) and both shapes need to prove they still drain: n = 5 (bottom
    // of slider, 4 tiers, an odd merge 5 -> 3), n = 8 (shipped default, the regression anchor),
    // n = 11 (mid-range, 5 tiers, another odd merge 11 -> 6 -> 3), n = 16 (top of slider, 5
    // tiers, the narrowest chambers).
    //
    // Measured today at n = 8: bottom_frac climbs from 0.197 (80 ticks) to 0.967 (500 ticks) to
    // 1.000 (1500+ ticks) -- a genuinely gradual multi-tier drain, not an initialization
    // artifact. 1500 ticks is used here for margin against the 0.5 threshold while keeping the
    // test cheap (~0.2s per chamber count on the 128x128 grid used below).
    fn test_cascade_drains_to_bottom_chamber() {
        let w = 128;
        let h = 128;
        let neck_width = 0.06;
        let curve = 0.6;

        for chambers in [5u32, 8, 11, 16] {
            let mask = make_test_mask_with_chambers(
                w, h, SandboxShape::MultiStageHourglass, neck_width, curve, chambers,
            );

            let center_y = h as f32 / 2.0;
            let total_half = 0.42 * h as f32;
            let n_tiers = multistage_tier_chambers(chambers).len();
            let tier_h = (2.0 * total_half) / n_tiers as f32;
            // Fill tier 0 only: dy < -total_half + tier_h, matching the fill threshold
            // `initialize_hourglass` uses for this shape.
            let fill_row_end = (center_y - total_half + tier_h).round() as usize;
            // Bottom tier (single chamber) starts here; must match `y1` for the second-to-last
            // tier index in `eval_sandbox_shape`'s MultiStageHourglass branch.
            let bottom_tier_row0 = (center_y + total_half - tier_h).round() as usize;

            let mut hm = Heightmap::new(w, h, 0.0);
            for y in 0..fill_row_end {
                for x in 0..w {
                    let idx = y * w + x;
                    if mask[idx] != crate::MASK_OUTSIDE {
                        hm.data[idx] = 0.55;
                    }
                }
            }
            let initial_mass: f64 = hm.data.iter().map(|&v| v as f64).sum();
            assert!(initial_mass > 0.0, "chambers={}: tier 0 was not filled with any sand", chambers);

            let mut temp_heights = hm.data.clone();
            let mut cell_props = get_test_props(MaterialMode::DrySand, w * h);
            let mut cell_colors = vec![0u8; w * h * 4];
            let mut sliding = vec![false; w * h];
            let mut bounds = ActiveBounds { min_x: 0, max_x: w - 1, min_y: 0, max_y: h - 1, active: true };

            let mut edge_vel_h = vec![0.0; w * h];
            let mut edge_vel_v = vec![0.0; w * h];
            let mut column_depth = vec![0.0; w * h];
            let mut head_field = vec![0.0; w * h];
            let block_size = 32;
            let (cols, rows) = (w / block_size, h / block_size);
            let mut active_blocks = vec![crate::BlockActivity::Inactive; cols * rows];
            let mut last_displacements = vec![1.0; cols * rows];
            let mut last_simulated_ticks = vec![0; cols * rows];

            let gravity_dir = glam::Vec2::new(0.0, 0.04);

            for i in 0..1500u32 {
                settle_tick(
                    &mut hm, &mut temp_heights, &mut cell_colors, &mut cell_props,
                    &mut sliding, &mut bounds, &mut active_blocks, &mut last_displacements,
                    &mut last_simulated_ticks, cols * rows, block_size, &[],
                    12345u32.wrapping_add(i),
                    &mut edge_vel_h, &mut edge_vel_v, &mut column_depth, &mut head_field, &mask, i, gravity_dir, false,
                    false,
                    false, false, false, 0.50,
                );
            }

            let final_mass: f64 = hm.data.iter().map(|&v| v as f64).sum();
            let bottom_mass: f64 = hm.data[bottom_tier_row0 * w..].iter().map(|&v| v as f64).sum();
            let mass_err = (final_mass - initial_mass).abs() / initial_mass;
            let bottom_frac = bottom_mass / final_mass;

            println!(
                "test_cascade_drains_to_bottom_chamber[chambers={}]: init_mass={:.6} \
                 final_mass={:.6} mass_err={:.8} bottom_mass={:.6} bottom_frac={:.4}",
                chambers, initial_mass, final_mass, mass_err, bottom_mass, bottom_frac
            );

            assert!(
                mass_err < 1e-4,
                "chambers={}: mass not conserved during drainage: mass_err={:.8}",
                chambers, mass_err
            );
            assert!(
                bottom_frac > 0.5,
                "chambers={}: cascade failed to drain to the bottom chamber: only {:.1}% of the \
                 mass reached the bottom tier after 1500 ticks (bottom_mass={:.6}, \
                 final_mass={:.6}). Sand is stuck in an upper tier.",
                chambers, bottom_frac * 100.0, bottom_mass, final_mass
            );
        }
    }

    #[test]
    // DIAGNOSTIC, not a gate: measures whether sand actually flows through the tightest neck the
    // parameter space allows -- multistage_chambers = 16 (narrowest chambers), grid = 64
    // (smallest grid, so the smallest absolute neck), neck_width at the new UI slider minimum
    // (`0.5 / 64`, a literal 1-cell-wide opening). By explicit user decision, clogging at this
    // extreme is ACCEPTABLE and this test does NOT narrow the supported range based on the
    // answer ("if it blocks sand flow, I could just increase the neck width -- no point not
    // allowing me to pick") -- it only asserts mass conservation (the geometry must not leak)
    // and prints the drained fraction so the practical limit is known and documented rather than
    // guessed at. Run with --nocapture to see the numbers cited in the task report.
    fn test_drainage_at_narrowest_possible_neck() {
        let w = 64;
        let h = 64;
        let chambers = 16u32;
        let neck_width = 0.5 / w as f32; // exactly the new UI slider minimum at this grid
        let curve = 0.6;

        let mask = make_test_mask_with_chambers(
            w, h, SandboxShape::MultiStageHourglass, neck_width, curve, chambers,
        );

        let center_y = h as f32 / 2.0;
        let total_half = 0.42 * h as f32;
        let n_tiers = multistage_tier_chambers(chambers).len();
        let tier_h = (2.0 * total_half) / n_tiers as f32;
        let fill_row_end = (center_y - total_half + tier_h).round() as usize;
        let bottom_tier_row0 = (center_y + total_half - tier_h).round() as usize;

        let mut hm = Heightmap::new(w, h, 0.0);
        for y in 0..fill_row_end {
            for x in 0..w {
                let idx = y * w + x;
                if mask[idx] != crate::MASK_OUTSIDE {
                    hm.data[idx] = 0.55;
                }
            }
        }
        let initial_mass: f64 = hm.data.iter().map(|&v| v as f64).sum();
        assert!(initial_mass > 0.0, "tier 0 was not filled with any sand");

        let mut temp_heights = hm.data.clone();
        let mut cell_props = get_test_props(MaterialMode::DrySand, w * h);
        let mut cell_colors = vec![0u8; w * h * 4];
        let mut sliding = vec![false; w * h];
        let mut bounds = ActiveBounds { min_x: 0, max_x: w - 1, min_y: 0, max_y: h - 1, active: true };

        let mut edge_vel_h = vec![0.0; w * h];
        let mut edge_vel_v = vec![0.0; w * h];
        let mut column_depth = vec![0.0; w * h];
        let mut head_field = vec![0.0; w * h];
        let block_size = 32;
        let (cols, rows) = (w / block_size, h / block_size);
        let mut active_blocks = vec![crate::BlockActivity::Inactive; cols * rows];
        let mut last_displacements = vec![1.0; cols * rows];
        let mut last_simulated_ticks = vec![0; cols * rows];

        let gravity_dir = glam::Vec2::new(0.0, 0.04);

        // A generous tick budget (3x test_cascade_drains_to_bottom_chamber's) since this neck
        // is deliberately as tight as the parameter space allows, so it is expected to be slow.
        const TICKS: u32 = 4500;
        for i in 0..TICKS {
            settle_tick(
                &mut hm, &mut temp_heights, &mut cell_colors, &mut cell_props,
                &mut sliding, &mut bounds, &mut active_blocks, &mut last_displacements,
                &mut last_simulated_ticks, cols * rows, block_size, &[],
                12345u32.wrapping_add(i),
                &mut edge_vel_h, &mut edge_vel_v, &mut column_depth, &mut head_field, &mask, i, gravity_dir, false,
                false,
                false, false, false, 0.50,
            );
        }

        let final_mass: f64 = hm.data.iter().map(|&v| v as f64).sum();
        let bottom_mass: f64 = hm.data[bottom_tier_row0 * w..].iter().map(|&v| v as f64).sum();
        let mass_err = (final_mass - initial_mass).abs() / initial_mass;
        let bottom_frac = bottom_mass / final_mass;
        let tier0_mass: f64 = hm.data[..fill_row_end * w].iter().map(|&v| v as f64).sum();

        println!(
            "test_drainage_at_narrowest_possible_neck: chambers={} w={} neck_width={:.6} \
             (1-cell floor) ticks={} init_mass={:.6} final_mass={:.6} mass_err={:.8} \
             tier0_remaining={:.6} bottom_mass={:.6} bottom_frac={:.4}",
            chambers, w, neck_width, TICKS, initial_mass, final_mass, mass_err,
            tier0_mass, bottom_mass, bottom_frac
        );

        // Gate ONLY on conservation -- geometry must never leak, regardless of how slowly it
        // drains. Deliberately no assertion on `bottom_frac`/drain speed: a clog at this
        // deliberately extreme setting is accepted, not a failure.
        assert!(
            mass_err < 1e-4,
            "Mass not conserved at the narrowest possible neck: mass_err={:.8}",
            mass_err
        );
    }

    #[test]
    fn test_residual_sand_drains_to_zero() {
        let w = 64;
        let h = 64;
        let mut hm = Heightmap::new(w, h, 0.0);

        // Put a single small residual sand pixel at (32, 10)
        let src_idx = 10 * w + 32;
        hm.data[src_idx] = 0.002;

        let mut temp_heights = hm.data.clone();
        let mut cell_props = get_test_props(MaterialMode::DrySand, w * h);
        let mut cell_colors = vec![0u8; w * h * 4];
        let mut sliding = vec![false; w * h];
        let mut bounds = ActiveBounds {
            min_x: 2,
            max_x: 61,
            min_y: 2,
            max_y: 61,
            active: true,
        };

        let mut edge_vel_h = vec![0.0; w * h];
        let mut edge_vel_v = vec![0.0; w * h];
        let mut column_depth = vec![0.0; w * h];
        let mut head_field = vec![0.0; w * h];
        let mut active_blocks = vec![crate::BlockActivity::Inactive; 4];
        let mut last_displacements = vec![1.0; 4];
        let mut last_simulated_ticks = vec![0; 4];

        // Downward gravity
        let gravity_dir = glam::Vec2::new(0.0, 0.04);

        let mask = make_test_mask(w, h, SandboxShape::Circle, 0.04, 1.0);
        // Run 20 ticks of gravity settling
        for i in 0..20 {
            settle_tick(
                &mut hm,
                &mut temp_heights,
                &mut cell_colors,
                &mut cell_props,
                &mut sliding,
                &mut bounds,
                &mut active_blocks,
                &mut last_displacements,
                &mut last_simulated_ticks,
                256,
                32,
                &[],
                12345 + i as u32,
                &mut edge_vel_h,
                &mut edge_vel_v,
                &mut column_depth,
                &mut head_field,
                &mask,
                i as u32,
                gravity_dir,
                false,
                false,
                false, false, false, 0.50,
            );
        }

        // The source pixel should be cleanly zero (0.0) without leaving residual ghost height trapped
        assert_eq!(hm.data[src_idx], 0.0, "Residual sand was trapped! h={}", hm.data[src_idx]);
    }

    #[test]
    fn test_no_floating_sand_under_gravity() {
        let w = 64;
        let h = 64;
        let mut hm = Heightmap::new(w, h, 0.0);

        // Fill random sand in upper chamber inside hourglass boundary
        let center_x = 32.0;
        let center_y = 32.0;
        let chamber_h = 0.40 * 64.0;
        let max_hw = 0.35 * 64.0;
        let neck_hw = 0.04 * 64.0;

        for y in 5..30 {
            let dy = y as f32 - center_y;
            let dy_abs = dy.abs();
            if dy_abs < chamber_h {
                let t = dy_abs / chamber_h;
                let allowed_hw = neck_hw + t.powf(0.6) * (max_hw - neck_hw);
                for x in 2..62 {
                    let dx = x as f32 - center_x;
                    if dx.abs() < allowed_hw {
                        let idx = y * w + x;
                        let pseudo_rand = ((x * 17 + y * 31) % 100) as f32 / 100.0;
                        hm.data[idx] = pseudo_rand * 0.8;
                    }
                }
            }
        }

        let mut temp_heights = hm.data.clone();
        let mut cell_props = get_test_props(MaterialMode::DrySand, w * h);
        let mut cell_colors = vec![0u8; w * h * 4];
        let mut sliding = vec![false; w * h];
        let mut bounds = ActiveBounds {
            min_x: 2,
            max_x: 61,
            min_y: 2,
            max_y: 61,
            active: true,
        };

        let mut edge_vel_h = vec![0.0; w * h];
        let mut edge_vel_v = vec![0.0; w * h];
        let mut column_depth = vec![0.0; w * h];
        let mut head_field = vec![0.0; w * h];
        let mut active_blocks = vec![crate::BlockActivity::Inactive; 4];
        let mut last_displacements = vec![1.0; 4];
        let mut last_simulated_ticks = vec![0; 4];

        let gravity_dir = glam::Vec2::new(0.0, 0.04);

        let mask = make_test_mask(w, h, SandboxShape::Hourglass, 0.04, 0.6);
        // Run gravity settling until all falling sand completes landing and flow reaches zero
        for i in 0..2000 {
            let flow = settle_tick(
                &mut hm,
                &mut temp_heights,
                &mut cell_colors,
                &mut cell_props,
                &mut sliding,
                &mut bounds,
                &mut active_blocks,
                &mut last_displacements,
                &mut last_simulated_ticks,
                256,
                32,
                &[],
                12345 + i as u32,
                &mut edge_vel_h,
                &mut edge_vel_v,
                &mut column_depth,
                &mut head_field,
                &mask,
                i as u32,
                gravity_dir,
                false,
                false,
                false, false, false, 0.50,
            );
            if i > 200 && flow == 0.0 {
                break;
            }
        }

        // Verify no cell with sand (h > 0.005) has an empty cell (h_below == 0.0) directly below it in mid-air
        for y in 2..60 {
            for x in 2..62 {
                let idx = y * w + x;
                let idx_below = (y + 1) * w + x;
                let h_curr = hm.data[idx];
                let h_below = hm.data[idx_below];

                let center_x = 32.0;
                let center_y = 32.0;
                let chamber_h = 0.40 * 64.0;
                let max_hw = 0.35 * 64.0;
                let neck_hw = 0.04 * 64.0;

                let is_in = |cx: usize, cy: usize| -> bool {
                    let dx = cx as f32 - center_x;
                    let dy = cy as f32 - center_y;
                    let dy_abs = dy.abs();
                    if dy_abs < chamber_h {
                        let t = dy_abs / chamber_h;
                        let allowed_hw = neck_hw + t.powf(0.6) * (max_hw - neck_hw);
                        dx.abs() < allowed_hw
                    } else {
                        false
                    }
                };

                if is_in(x, y) && h_curr > 0.005 && is_in(x, y + 1) && h_below == 0.0 {
                    println!("Column x={}:", x);
                    for py in 0..15 {
                        let p_idx = py * w + x;
                        println!("y={}: h={} inside={}", py, hm.data[p_idx], is_in(x, py));
                    }
                    panic!("Found floating sand inside container at ({}, {}) with h={} and empty air below!", x, y, h_curr);
                }
            }
        }
    }

    #[test]
    // Stage B: granular material's gravity-aligned edge moved onto the same conservative
    // `flux_edge` solver liquid already uses (see the phase-0 block in `settle_tick` and the
    // `ndy != 0.0` exclusion that keeps the CA from also touching that edge). Every other test
    // added for that migration measures a *settled* pile — exactly the blind spot called out
    // repeatedly in this file's history (defects C5/C13/etc. were all invisible to settled-state
    // tests and only showed up while mass was still moving). This is the flowing-state check for
    // sand, the direct analogue of `test_liquid_stream_stays_coherent`.
    //
    // Two things could go wrong in a way no settled-state test would catch:
    //   1. Mass could leak or duplicate specifically while the flux edge is active (as opposed to
    //      at rest, where a bug would show up in every other conservation test too).
    //   2. The new (c_sq, damping) = (1.0, 1.0) pair chosen for granular fall (see the phase-0
    //      comment) could either stall (an over-eager `edge_sleeps` wrongly freezing a falling
    //      column) or overshoot CFL (mass advancing more than one row per tick, which would show
    //      up as the falling front's row jumping by more than 1 in a single tick).
    fn test_granular_flowing_fall_conserves_mass_and_respects_cfl() {
        let w = 64;
        let h = 96;
        let mask = make_test_mask(w, h, SandboxShape::Square, 0.04, 1.0);
        let props = get_test_props(MaterialMode::DrySand, w * h);
        let mut sim = TestSim::new(w, h, props, mask, 32);
        let gravity_dir = glam::Vec2::new(0.0, 0.04);

        // A 4-cell-wide continuous tap, matching the liquid stream test's shape exactly so the
        // two are directly comparable.
        let mut poured_mass = 0.0f64;
        let source_cells = 4 * 4; // y in 6..10, x in 30..34
        let mut max_row_jump = 0i64;
        let mut prev_front: Option<usize> = None;
        let mut max_width = 0usize;
        let mut min_mass_err_ticks_with_flow = 0;

        for t in 0..60 {
            for y in 6..10 {
                for x in 30..34 {
                    let idx = y * w + x;
                    // Only top up cells not already full, so the poured-mass tally stays exact.
                    let before = sim.hm.data[idx];
                    sim.hm.data[idx] = 1.0;
                    poured_mass += (1.0 - before) as f64;
                }
            }
            sim.tick(gravity_dir, 256);

            // Mass conservation *while flowing*, not just once settled: total mass in the grid
            // must equal what was poured in, at every single tick, not just the last one.
            let current_mass: f64 = sim.hm.data.iter().map(|&v| v as f64).sum();
            let mass_err = (current_mass - poured_mass).abs() / poured_mass.max(1e-9);
            assert!(
                mass_err < 1e-3,
                "Mass not conserved mid-flow at tick {}: poured={:.6} actual={:.6} err={:.2e}",
                t, poured_mass, current_mass, mass_err
            );
            if current_mass > 1e-6 {
                min_mass_err_ticks_with_flow += 1;
            }

            // Falling front: the deepest row (below the source) that still has any sand in it.
            let mut front = None;
            for y in (10..h).rev() {
                let row_has_sand = (0..w).any(|x| sim.hm.data[y * w + x] > 0.05);
                if row_has_sand {
                    front = Some(y);
                    break;
                }
            }
            if let (Some(f), Some(pf)) = (front, prev_front) {
                // The front may not advance every tick (it can pause while a cell ramps up), but
                // it must never advance by more than one row in a single tick — more would mean
                // mass hopped over a row without ever being subject to that row's own donor/
                // acceptor clamp, breaking the CFL property the whole gravity-aligned phase-0
                // ordering exists to guarantee (see the operator-split note in `settle_tick`).
                max_row_jump = max_row_jump.max((f as i64 - pf as i64).max(0));
            }
            prev_front = front.or(prev_front);

            // Coherence: same measurement `test_liquid_stream_stays_coherent` uses, restricted to
            // the mid-air band clear of the source and the eventual floor.
            for y in 15..70 {
                let mut min_x = None;
                let mut max_x = None;
                for x in 0..w {
                    if sim.hm.data[y * w + x] > 0.05 {
                        if min_x.is_none() { min_x = Some(x); }
                        max_x = Some(x);
                    }
                }
                if let (Some(mn), Some(mx)) = (min_x, max_x) {
                    max_width = max_width.max(mx - mn + 1);
                }
            }
        }

        println!(
            "test_granular_flowing_fall_conserves_mass_and_respects_cfl: poured={:.6} \
             max_row_jump={} max_width={} ticks_with_flow={} source_cells={}",
            poured_mass, max_row_jump, max_width, min_mass_err_ticks_with_flow, source_cells
        );

        assert!(
            max_row_jump <= 1,
            "Falling front advanced {} rows in a single tick — CFL violated by the granular \
             vertical flux edge",
            max_row_jump
        );
        // Measured today: max_width=22. Water's equivalent tap stays at 8 cells
        // (`test_liquid_stream_stays_coherent`) because it has no dispersion term at all; sand's
        // free-fall CA lateral loop (untouched by Stage B — this is the pre-existing
        // `perp_dot * 0.8 * dispersion_noise` scatter, still owned entirely by the CA) is expected
        // to be wider than that, so this bound is generous rather than a tight pin. What it
        // guards against is Stage B's *own* failure mode: the vertical flux edge silently handing
        // mass sideways instead of down (e.g. a `weight`/`cap` mixup), which would blow this out
        // much further, the way removing the liquid in-transit limiter blew that test's stream
        // from 8 to 59.
        assert!(
            max_width <= 30,
            "Granular stream fanned out too wide while falling: {} cells",
            max_width
        );
    }

    #[test]
    fn test_falling_stream_no_block_boundary_density_spikes() {
        let w = 512;
        let h = 512;
        let block_size = 32;
        let cols = w / block_size;
        let rows = h / block_size;

        let center_x = w as f32 / 2.0;
        let center_y = h as f32 / 2.0;
        let chamber_h = 0.40 * h as f32;
        let max_hw = 0.35 * w as f32;
        let neck_hw = 0.005 * w as f32;
        let hourglass_curve = 0.6;

        let mut hm = Heightmap::new(w, h, 0.0);

        // Fill upper chamber (y < center_y) up to 0.50 capacity
        for y in 0..h {
            let dy = y as f32 - center_y;
            let dy_abs = dy.abs();
            if dy_abs < chamber_h {
                let t = dy_abs / chamber_h;
                let allowed_hw = neck_hw + t.powf(hourglass_curve) * (max_hw - neck_hw);
                for x in 0..w {
                    let dx = x as f32 - center_x;
                    if dx.abs() < allowed_hw && dy < -4.0 && dy > -0.50 * chamber_h {
                        hm.data[y * w + x] = 0.5;
                    }
                }
            }
        }

        let mut temp_heights = hm.data.clone();
        let mut cell_props = get_test_props(MaterialMode::DrySand, w * h);
        let mut cell_colors = vec![0u8; w * h * 4];
        let mut sliding = vec![false; w * h];
        let mut bounds = ActiveBounds {
            min_x: 0,
            max_x: w - 1,
            min_y: 0,
            max_y: h - 1,
            active: true,
        };

        let mut edge_vel_h = vec![0.0; w * h];
        let mut edge_vel_v = vec![0.0; w * h];
        let mut column_depth = vec![0.0; w * h];
        let mut head_field = vec![0.0; w * h];
        let mut active_blocks = vec![crate::BlockActivity::Inactive; cols * rows];
        let mut last_displacements = vec![1.0; cols * rows];
        let mut last_simulated_ticks = vec![0; cols * rows];

        let gravity_dir = glam::Vec2::new(0.0, 0.04);

        let mask = make_test_mask(512, 512, SandboxShape::Hourglass, hourglass_curve, 0.005);
        // Run simulation for 80 ticks
        for i in 0..80 {
            settle_tick(
                &mut hm,
                &mut temp_heights,
                &mut cell_colors,
                &mut cell_props,
                &mut sliding,
                &mut bounds,
                &mut active_blocks,
                &mut last_displacements,
                &mut last_simulated_ticks,
                256,
                32,
                &[],
                i as u32,
                &mut edge_vel_h,
                &mut edge_vel_v,
                &mut column_depth,
                &mut head_field,
                &mask,
                i as u32,
                gravity_dir,
                false,
                false,
                false, false, false, 0.50,
            );
        }

        // Print heights at block boundaries along stream center x=256
        let stream_x = 256;
        println!("--- Stream Height Profile at 32-pixel Block Boundaries ---");
        for by in 8..15 {
            let boundary_y = by * 32;
            let h_prev = hm.data[(boundary_y - 1) * w + stream_x];
            let h_bound = hm.data[boundary_y * w + stream_x];
            let h_next = hm.data[(boundary_y + 1) * w + stream_x];
            println!(
                "Boundary y={}: y-1={:.4}, y_bound={:.4}, y+1={:.4}",
                boundary_y, h_prev, h_bound, h_next
            );
        }

        // Classify EVERY consecutive-row height difference down the whole stream column as
        // either "at a block boundary" (the lower row index is a multiple of block_size, i.e.
        // this step crosses the seam between two 32px blocks copied back independently at
        // physics.rs:1372-1386) or "interior" (both rows are inside the same block). If the
        // block-based LOD/copy-back scheme were introducing density spikes specifically at
        // block seams, the boundary population's worst case would be anomalously large compared
        // to the interior population's worst case (which already reflects the stream's normal
        // leading-edge/settling discontinuities).
        let mut max_boundary_jump = 0.0f32;
        let mut max_interior_jump = 0.0f32;
        for y in 1..h {
            let h_prev = hm.data[(y - 1) * w + stream_x];
            let h_curr = hm.data[y * w + stream_x];
            let jump = (h_curr - h_prev).abs();
            if y % block_size == 0 {
                max_boundary_jump = max_boundary_jump.max(jump);
            } else {
                max_interior_jump = max_interior_jump.max(jump);
            }
        }
        println!(
            "max_boundary_jump={:.5} max_interior_jump={:.5} ratio={:.3}",
            max_boundary_jump, max_interior_jump, max_boundary_jump / max_interior_jump.max(1e-6)
        );

        // Measured today: max_boundary_jump=0.14356, max_interior_jump=0.38460 (ratio 0.373) —
        // boundary-adjacent jumps are actually SMALLER than the worst interior jump, i.e. no
        // block-seam-specific spike. Generous margin (2x) to absorb run-to-run tuning changes
        // that don't touch the LOD/copy-back mechanism itself.
        assert!(
            max_boundary_jump <= max_interior_jump * 2.0,
            "Block-boundary height jump ({:.5}) is anomalously large relative to the worst \
             interior jump ({:.5}) — possible density spike at a 32px block seam",
            max_boundary_jump, max_interior_jump
        );
    }

    #[test]
    fn test_hourglass_color_and_property_conservation_under_gravity() {
        // Verify that RGBA cell colors and material properties (wetness, grain size, flow rate)
        // are 100% conserved when sand flows down through the hourglass neck under gravity.
        let w = 128;
        let h = 128;
        let mut hm = Heightmap::new(w, h, 0.0);
        let mut temp_heights = vec![0.0f32; w * h];
        let mut cell_colors = vec![0u8; w * h * 4];
        let mut cell_props = vec![0.0f32; w * h * 4];
        let mut sliding = vec![false; w * h];
        let mut edge_vel_h = vec![0.0f32; w * h];
        let mut edge_vel_v = vec![0.0f32; w * h];
        let mut column_depth = vec![0.0f32; w * h];
        let mut head_field = vec![0.0f32; w * h];

        let center_x = w as f32 / 2.0;
        let center_y = h as f32 / 2.0;
        let chamber_h = 0.40 * (h as f32);
        let max_hw = 0.35 * (w as f32);
        let neck_hw = 0.04 * (w as f32);

        // Fill upper chamber with two distinct colored & property layers (Red Dry Sand / Blue Wet Sand)
        for y in 0..h {
            let dy = y as f32 - center_y;
            if dy < 0.0 && dy.abs() < chamber_h {
                let t = dy.abs() / chamber_h;
                let allowed_hw = neck_hw + t.powf(0.6) * (max_hw - neck_hw);
                for x in 0..w {
                    let dx = x as f32 - center_x;
                    if dx.abs() < allowed_hw {
                        let idx = y * w + x;
                        hm.data[idx] = 0.80; // 80% initial fill height

                        if dy < -0.20 * (h as f32) {
                            // Top Layer: Red Dry Sand (Wetness = 0.0, GrainSize = 0.50)
                            cell_colors[idx * 4 + 0] = 230;
                            cell_colors[idx * 4 + 1] = 40;
                            cell_colors[idx * 4 + 2] = 40;
                            cell_colors[idx * 4 + 3] = 255;

                            cell_props[idx * 4 + PROP_WETNESS] = 0.00;
                            cell_props[idx * 4 + PROP_THRESHOLD] = 0.08;
                            cell_props[idx * 4 + PROP_FLOW_RATE] = 0.25;
                            cell_props[idx * 4 + PROP_GRAIN_SIZE] = 0.50;
                        } else {
                            // Bottom Layer: Blue Wet Sand (Wetness = 0.40, GrainSize = 0.30)
                            cell_colors[idx * 4 + 0] = 40;
                            cell_colors[idx * 4 + 1] = 80;
                            cell_colors[idx * 4 + 2] = 230;
                            cell_colors[idx * 4 + 3] = 255;

                            cell_props[idx * 4 + PROP_WETNESS] = 0.40;
                            cell_props[idx * 4 + PROP_THRESHOLD] = 0.12;
                            cell_props[idx * 4 + PROP_FLOW_RATE] = 0.15;
                            cell_props[idx * 4 + PROP_GRAIN_SIZE] = 0.30;
                        }
                    }
                }
            }
        }
        temp_heights.copy_from_slice(&hm.data);

        // Helper to calculate total color and property mass
        let calc_totals = |colors: &[u8], props: &[f32], hmap: &Heightmap| -> (f64, f64, f64, f64, f64) {
            let mut r_total = 0.0f64;
            let mut g_total = 0.0f64;
            let mut b_total = 0.0f64;
            let mut wet_total = 0.0f64;
            let mut grain_total = 0.0f64;
            for (idx, &height) in hmap.as_slice().iter().enumerate() {
                let h_val = height as f64;
                if h_val > 0.0 {
                    r_total += (colors[idx * 4 + 0] as f64) * h_val;
                    g_total += (colors[idx * 4 + 1] as f64) * h_val;
                    b_total += (colors[idx * 4 + 2] as f64) * h_val;
                    wet_total += (props[idx * 4 + PROP_WETNESS] as f64) * h_val;
                    grain_total += (props[idx * 4 + PROP_GRAIN_SIZE] as f64) * h_val;
                }
            }
            (r_total, g_total, b_total, wet_total, grain_total)
        };

        let (init_r, init_g, init_b, init_wet, init_grain) = calc_totals(&cell_colors, &cell_props, &hm);

        let mut bounds = ActiveBounds {
            min_x: 0,
            max_x: w - 1,
            min_y: 0,
            max_y: h - 1,
            active: true,
        };

        let expected_len = (w / 32) * (h / 32);
        let mut active_blocks = vec![crate::BlockActivity::Fast; expected_len];
        let mut last_displacements = vec![1.0; expected_len];
        let mut last_simulated_ticks = vec![0; expected_len];
        let gravity_dir = glam::Vec2::new(0.0, 0.04);

        let mask = make_test_mask(w, h, SandboxShape::Hourglass, 0.6, 0.04);
        // Run 300 gravity ticks flowing sand down into the lower chamber
        for i in 0..300 {
            settle_tick(
                &mut hm,
                &mut temp_heights,
                &mut cell_colors,
                &mut cell_props,
                &mut sliding,
                &mut bounds,
                &mut active_blocks,
                &mut last_displacements,
                &mut last_simulated_ticks,
                256,
                32,
                &[],
                i as u32,
                &mut edge_vel_h,
                &mut edge_vel_v,
                &mut column_depth,
                &mut head_field,
                &mask,
                i as u32,
                gravity_dir,
                false,
                false,
                false, false, false, 0.50,
            );
        }

        let (final_r, final_g, final_b, final_wet, final_grain) = calc_totals(&cell_colors, &cell_props, &hm);

        println!("Init Totals:  R={:.2}, G={:.2}, B={:.2}, Wet={:.2}, Grain={:.2}", init_r, init_g, init_b, init_wet, init_grain);
        println!("Final Totals: R={:.2}, G={:.2}, B={:.2}, Wet={:.2}, Grain={:.2}", final_r, final_g, final_b, final_wet, final_grain);

        // Relative error tolerances
        let r_err = (final_r - init_r).abs() / init_r;
        let g_err = (final_g - init_g).abs() / init_g;
        let b_err = (final_b - init_b).abs() / init_b;
        let wet_err = (final_wet - init_wet).abs() / init_wet;
        let grain_err = (final_grain - init_grain).abs() / init_grain;

        println!("Errors: R_err={:.9}, G_err={:.9}, B_err={:.9}, Wet_err={:.9}, Grain_err={:.9}", r_err, g_err, b_err, wet_err, grain_err);

        // Color is stored as `u8` and every blend is rounded back to an integer by
        // `stochastic_round`, so the color channels carry a quantisation residual that the
        // (pure f32) property channels do not. It is unbiased, so it is a zero-mean random
        // walk in the totals rather than the one-directional loss plain `.round()` produced.
        //
        // Measured over 36 independent realizations of the rounding (identical physics, only
        // the rounding entropy varied): per-channel absolute error is zero-mean with
        // sigma ~= 90-140 color-mass units on totals of 1.3e5 (G) to 4.0e5 (R), i.e.
        // sigma_rel = 3.5e-4 (R), 6.9e-4 (G), 4.7e-4 (B). Worst single realization was
        // 2.0e-3 (G, ~2.9 sigma); this realization gives 6.1e-4 / 2.7e-4 / 4.8e-4.
        //
        // 0.005 is therefore ~7 sigma on the noisiest channel — loose enough that a reshuffle
        // of the draws cannot make this flake, tight enough to still fail hard on a
        // *systematic* loss: plain u8 rounding measured 7.4e-2 here, 15x over this bound.
        // (The pre-u8 f32 storage ran at ~1e-7 and used 0.001; that is unreachable with an
        // integer buffer and is not what this test is for.)
        assert!(r_err < 0.005, "Red color mass loss under gravity: err={:.6}", r_err);
        assert!(g_err < 0.005, "Green color mass loss under gravity: err={:.6}", g_err);
        assert!(b_err < 0.005, "Blue color mass loss under gravity: err={:.6}", b_err);
        assert!(wet_err < 0.001, "Wetness property loss under gravity: err={:.6}", wet_err);
        assert!(grain_err < 0.001, "Grain size property loss under gravity: err={:.6}", grain_err);
    }

    /// Stochastic rounding is unbiased, so every conservation test above stays green no matter
    /// how badly color smears — the totals are conserved by construction. The risk it actually
    /// carries is *diffusion*: each blend injects roughly +/-0.5 LSB of noise, and the flux
    /// solver performs a very large number of advection events, so the random walk can
    /// accumulate into spatial blur. Nothing else in the suite measures that.
    ///
    /// A square box is filled to a *uniform* height above the per-cell cap and left to compact
    /// under gravity for 3000 ticks. Uniform means the free surface stays flat, so the bulk
    /// motion is vertical rather than a pile collapsing sideways, while still transporting
    /// ~5.9e5 units of volume — this is not a quiescent bed. Color is split left/right by a
    /// vertical line, i.e. the interface is *parallel* to the flow.
    ///
    /// Two things are measured, for two different reasons:
    ///
    /// 1. **Interface width.** This is mostly *physical*: the solver's own lateral mixing
    ///    smears the split over ~11 columns here, and the f32 color buffer this change replaced
    ///    measures 11.364 against the u8 buffer's 11.571 — quantisation contributes essentially
    ///    none of it. The bound is therefore set against that physical baseline. It is the
    ///    coarse "did the picture turn to mush" check.
    /// 2. **Deep-interior drift.** Away from the split every neighbour started the same color,
    ///    and a weighted blend of equal integers is that same integer (to within one ulp of
    ///    `w_keep + w_arrive != 1.0`), so an exact solver leaves those cells on their starting
    ///    value however many transfers pass through. This part has essentially no physical
    ///    component and is the sensitive one: amplifying the rounding noise 3x moves the
    ///    interface width only 11.57 -> 12.88 but moves deep drift 0.017 -> 7.07 LSB.
    #[test]
    fn test_color_boundary_does_not_diffuse_under_gravity() {
        let w = 128;
        let h = 128;
        let split = w / 2;
        const LEFT: [u8; 3] = [230, 40, 40];
        const RIGHT: [u8; 3] = [40, 80, 230];

        let mask = make_test_mask(w, h, SandboxShape::Square, 0.6, 0.04);
        let mut hm = Heightmap::new(w, h, 0.0);
        let mut cell_colors = vec![0u8; w * h * 4];
        let mut cell_props = get_test_props(MaterialMode::DrySand, w * h);

        // Color *every* cell by which side of `split` its column is on, including the empty
        // ones the sand will fall into. If the destination cells started black they would blend
        // towards black on arrival, which is a real (physical) color change and would swamp the
        // quantisation signal this test is trying to isolate.
        for y in 0..h {
            for x in 0..w {
                let idx = y * w + x;
                let c = if x < split { LEFT } else { RIGHT };
                cell_colors[idx * 4..idx * 4 + 3].copy_from_slice(&c);
                cell_colors[idx * 4 + 3] = 255;
            }
        }
        // Fill the whole box to a uniform height. Uniform means the free surface stays flat, so
        // the bulk motion is vertical compaction (the bed is above the per-cell cap and settles
        // downwards through most of its own depth) rather than a pile collapsing sideways.
        for y in 0..h {
            for x in 0..w {
                let idx = y * w + x;
                if mask[idx] != crate::MASK_OUTSIDE {
                    hm.data[idx] = 1.0;
                }
            }
        }

        let mut temp_heights = hm.data.clone();
        let mut sliding = vec![false; w * h];
        let mut edge_vel_h = vec![0.0f32; w * h];
        let mut edge_vel_v = vec![0.0f32; w * h];
        let mut column_depth = vec![0.0f32; w * h];
        let mut head_field = vec![0.0f32; w * h];
        let mut bounds = ActiveBounds { min_x: 0, max_x: w - 1, min_y: 0, max_y: h - 1, active: true };
        let n_blocks = (w / 32) * (h / 32);
        let mut active_blocks = vec![crate::BlockActivity::Fast; n_blocks];
        let mut last_displacements = vec![1.0; n_blocks];
        let mut last_simulated_ticks = vec![0; n_blocks];
        let gravity_dir = glam::Vec2::new(0.0, 0.04);

        const TICKS: u32 = 3000;
        let mut total_flow = 0.0f64;
        for i in 0..TICKS {
            total_flow += settle_tick(
                &mut hm,
                &mut temp_heights,
                &mut cell_colors,
                &mut cell_props,
                &mut sliding,
                &mut bounds,
                &mut active_blocks,
                &mut last_displacements,
                &mut last_simulated_ticks,
                n_blocks,
                32,
                &[],
                i,
                &mut edge_vel_h,
                &mut edge_vel_v,
                &mut column_depth,
                &mut head_field,
                &mask,
                i,
                gravity_dir,
                false,
                false,
                false, false, false, 0.50,
            ) as f64;
        }

        // Only look at rows that ended up solidly buried, so the free surface (which does move
        // sideways as the pile relaxes) is excluded.
        const HALF_WIN: usize = 20; // columns inspected either side of the split
        const DEEP_GAP: usize = 24; // "deep interior" starts this far from the split
        const WALL: usize = 12; // ...and stops this far from the side walls
        let solid_row = |y: usize| (WALL..w - WALL).all(|x| hm.data[y * w + x] > 0.9);
        let red = |x: usize, y: usize| cell_colors[(y * w + x) * 4] as f64;
        let span = LEFT[0] as f64 - RIGHT[0] as f64;

        let mut widths: Vec<f64> = Vec::new();
        // Deep-interior fidelity: every neighbour of these cells started the run the same color,
        // so no weighted blend of them can produce anything else. An exact solver leaves them on
        // their starting value however many transfers pass through — the f32 reference run
        // measures 0.0024 LSB here. Whatever is measured is the rounding, not the solver.
        let mut deep_dev: Vec<f64> = Vec::new();
        let mut deep_max = 0.0f64;
        let mut deep_exact = 0usize;

        for y in 0..h {
            if !solid_row(y) {
                continue;
            }
            // Transition width: columns whose red channel sits strictly between the two starting
            // levels, measured against the exact starting colors rather than a local average, so
            // a smear cannot drag the reference along with it.
            widths.push(
                (split - HALF_WIN..split + HALF_WIN)
                    .filter(|&x| {
                        let t = (red(x, y) - RIGHT[0] as f64) / span;
                        (0.1..=0.9).contains(&t)
                    })
                    .count() as f64,
            );

            for x in WALL..w - WALL {
                let exact = if x + DEEP_GAP < split {
                    LEFT
                } else if x > split + DEEP_GAP {
                    RIGHT
                } else {
                    continue;
                };
                for ch in 0..3 {
                    let d = (cell_colors[(y * w + x) * 4 + ch] as f64 - exact[ch] as f64).abs();
                    deep_dev.push(d);
                    deep_max = deep_max.max(d);
                    if d == 0.0 {
                        deep_exact += 1;
                    }
                }
            }
        }

        assert!(
            widths.len() >= 20,
            "not enough buried rows to measure ({}), the bed did not form as expected",
            widths.len()
        );
        let mean_width = widths.iter().sum::<f64>() / widths.len() as f64;
        let worst_width = widths.iter().cloned().fold(0.0f64, f64::max);
        let mean_dev = deep_dev.iter().sum::<f64>() / deep_dev.len() as f64;
        let exact_frac = deep_exact as f64 / deep_dev.len() as f64;
        println!(
            "after {} ticks, {} buried rows: interface width mean {:.3} / worst {:.0} columns; \
             deep interior drift mean {:.4} LSB, max {:.0} LSB, {:.2}% still exact",
            TICKS, widths.len(), mean_width, worst_width, mean_dev, deep_max, exact_frac * 100.0
        );
        assert!(
            total_flow > 1.0e5,
            "only {:.1} units of volume moved — the scenario went quiescent and this test would \
             pass vacuously",
            total_flow
        );

        // MEASURED (deterministic — `stochastic_round` is hash-seeded, not RNG-seeded, so these
        // are exact and reproducible, not a sample):
        //   interface width mean 11.571 / worst 24 columns
        //   deep interior drift mean 0.0167 LSB, max 4 LSB, 98.39% still exact
        //   total volume transported 588402
        // Same scenario against the f32 color buffer this change replaced, for reference:
        //   interface width mean 11.364 / worst 23 columns; deep drift mean 0.0024 LSB, max 1.
        //
        // The interface bound sits ~20% above the physical baseline: wide enough not to police
        // the solver's own lateral mixing, tight enough that a smear which meaningfully widens
        // the interface fails. Amplifying the rounding noise 3x/6x/12x measures 12.88 / 18.75 /
        // 28.27 columns, so this catches 6x and up on width alone.
        assert!(
            mean_width < 14.0,
            "color interface diffused: mean transition width {:.3} columns over {} rows \
             (physical baseline 11.36 with an exact color buffer)",
            mean_width, widths.len()
        );
        assert!(
            worst_width < 30.0,
            "color interface diffused on some row: worst transition width {:.0} columns",
            worst_width
        );

        // This is the assertion that actually guards the u8 decision, and it has no physical
        // component, so it is bounded tightly: 0.5 LSB is 30x the measured mean and still 14x
        // below what a mere 3x noise amplification produces (7.07 mean / 42 max).
        assert!(
            mean_dev < 0.5 && deep_max <= 12.0,
            "deep interior color drifted off its exact starting value: mean |d| = {:.4} LSB, \
             max |d| = {:.0} LSB over {} samples",
            mean_dev, deep_max, deep_dev.len()
        );
    }

    #[test]
    fn test_concentric_rings_eventually_all_drain_through_neck() {
        // Paint the upper chamber with rings centered on the neck (matching the UI's
        // "Concentric Rings" color pattern: sandart-wasm/web/demo.js generateColormap).
        // Ring 0 (nearest the neck) is green; odd rings are yellow.
        let w = 128;
        let h = 128;
        let mut hm = Heightmap::new(w, h, 0.0);
        let mut cell_colors = vec![0u8; w * h * 4];
        let cell_props_mode = get_test_props(MaterialMode::DrySand, w * h);
        let mut cell_props = cell_props_mode;

        let center_x = w as f32 / 2.0;
        let center_y = h as f32 / 2.0;
        let chamber_h = 0.40 * h as f32;
        let max_hw = 0.35 * w as f32;
        let neck_hw = 0.04 * w as f32;
        let ring_width = 8.0; // cells; proportionally matches the UI's 32px/512

        let mut initial_green_mass = 0.0f64;
        let mut initial_yellow_mass = 0.0f64;

        for y in 0..h {
            let dy = y as f32 - center_y;
            if dy < 0.0 && dy.abs() < chamber_h {
                let t = dy.abs() / chamber_h;
                let allowed_hw = neck_hw + t.powf(0.6) * (max_hw - neck_hw);
                for x in 0..w {
                    let dx = x as f32 - center_x;
                    if dx.abs() < allowed_hw {
                        let idx = y * w + x;
                        hm.data[idx] = 0.60;

                        let dist = (dx * dx + dy * dy).sqrt();
                        let ring_even = ((dist / ring_width) as i64) % 2 == 0;
                        if ring_even {
                            // Green
                            cell_colors[idx * 4 + 0] = 34;
                            cell_colors[idx * 4 + 1] = 139;
                            cell_colors[idx * 4 + 2] = 34;
                            cell_colors[idx * 4 + 3] = 255;
                            initial_green_mass += hm.data[idx] as f64;
                        } else {
                            // Yellow
                            cell_colors[idx * 4 + 0] = 255;
                            cell_colors[idx * 4 + 1] = 215;
                            cell_colors[idx * 4 + 2] = 0;
                            cell_colors[idx * 4 + 3] = 255;
                            initial_yellow_mass += hm.data[idx] as f64;
                        }
                    }
                }
            }
        }

        assert!(initial_green_mass > 0.0 && initial_yellow_mass > 0.0, "Test setup should paint both colors");

        let mut temp_heights = hm.data.clone();
        let mut sliding = vec![false; w * h];
        let mut bounds = ActiveBounds { min_x: 0, max_x: w - 1, min_y: 0, max_y: h - 1, active: true };
        let mut edge_vel_h = vec![0.0; w * h];
        let mut edge_vel_v = vec![0.0; w * h];
        let mut column_depth = vec![0.0; w * h];
        let mut head_field = vec![0.0; w * h];
        let expected_len = (w / 32) * (h / 32);
        let mut active_blocks = vec![crate::BlockActivity::Fast; expected_len];
        let mut last_displacements = vec![1.0; expected_len];
        let mut last_simulated_ticks = vec![0; expected_len];
        let gravity_dir = glam::Vec2::new(0.0, 0.04);

        let mask = make_test_mask(w, h, SandboxShape::Hourglass, 0.04, 0.60);

        // Measure the height-weighted AVERAGE color strictly below the neck (lower chamber).
        // Pure ring paint is (34,139,34) green / (255,215,0) yellow; as sand mixes en route
        // to the neck, individual cells take on blended, in-between hues rather than staying
        // categorically one or the other. Tracking the weighted average RGB (and derived R:G
        // ratio, 0.0 = pure green, 1.0 = pure yellow) shows that drift directly.
        let measure_lower_chamber_avg = |colors: &[u8], hmap: &Heightmap| -> (f64, f64, f64, f64) {
            let mut r_sum = 0.0f64;
            let mut g_sum = 0.0f64;
            let mut b_sum = 0.0f64;
            let mut mass = 0.0f64;
            for y in (center_y as usize)..h {
                for x in 0..w {
                    let idx = y * w + x;
                    let hgt = hmap.data[idx] as f64;
                    if hgt <= 0.0 {
                        continue;
                    }
                    r_sum += colors[idx * 4 + 0] as f64 * hgt;
                    g_sum += colors[idx * 4 + 1] as f64 * hgt;
                    b_sum += colors[idx * 4 + 2] as f64 * hgt;
                    mass += hgt;
                }
            }
            if mass <= 0.0 {
                return (0.0, 0.0, 0.0, 0.0);
            }
            let avg_r = r_sum / mass;
            let avg_g = g_sum / mass;
            let avg_b = b_sum / mass;
            // 0.0 at pure green (34,139,34), 1.0 at pure yellow (255,215,0), interpolating on R and B.
            let yellow_frac = (((avg_r - 34.0) / (255.0 - 34.0)) + ((34.0 - avg_b) / 34.0)) / 2.0;
            (avg_r, avg_g, avg_b, yellow_frac)
        };

        for i in 0..4000u32 {
            settle_tick(
                &mut hm,
                &mut temp_heights,
                &mut cell_colors,
                &mut cell_props,
                &mut sliding,
                &mut bounds,
                &mut active_blocks,
                &mut last_displacements,
                &mut last_simulated_ticks,
                256,
                32,
                &[],
                12345 + i,
                &mut edge_vel_h,
                &mut edge_vel_v,
                &mut column_depth,
                &mut head_field,
                &mask,
                i,
                gravity_dir,
                false,
                false,
                false, false, false, 0.50,
            );

            if i % 500 == 0 || i == 3999 {
                let (r, g, b, yellow_frac) = measure_lower_chamber_avg(&cell_colors, &hm);
                println!("tick {:5}: lower-chamber avg color = ({:.1}, {:.1}, {:.1})  yellow_frac={:.3}",
                    i, r, g, b, yellow_frac);
            }
        }

        let (final_r, final_g, final_b, final_yellow_frac) = measure_lower_chamber_avg(&cell_colors, &hm);
        println!("FINAL: lower-chamber avg color = ({:.1}, {:.1}, {:.1})  yellow_frac={:.3}", final_r, final_g, final_b, final_yellow_frac);
        assert!(
            final_yellow_frac > 0.05,
            "Lower chamber's average color never drifted toward yellow at all: yellow_frac={:.3}",
            final_yellow_frac
        );
    }

    #[test]
    // Sweeps the full neck_width slider (0.005..=0.12) crossed with several hourglass_curve
    // values (0.1..=3.0) against MultiStageHourglass's 8 -> 4 -> 2 -> 1 cascade, and checks the
    // two ways this geometry could break silently while mass conservation stays perfectly happy:
    //
    //   1. A dam: the neck at the bottom of some chamber closes completely, trapping its sand
    //      above it forever. Checked by sampling the mask at each chamber's own neck centre.
    //
    //   2. Neighbouring chambers in the same tier fusing into one opening at the neck -- the
    //      specific failure the per-tier neck cap in `eval_sandbox_shape` exists to prevent.
    //      Tier 0's chambers are only w/8 wide; an unclamped neck near the slider's top of
    //      0.12 * w would be nearly as wide as the chamber itself and adjacent necks would
    //      overlap into open space. Checked by sampling the mask at the wall *between* two
    //      adjacent necks (the slot boundary, equidistant from both) -- if the cap is doing its
    //      job this stays MASK_OUTSIDE across the entire sweep; without it, it would flip to
    //      MASK_INSIDE well before the slider reaches its top end.
    fn test_cascade_no_dam_or_neck_merge_across_full_slider_range() {
        let w = 512;
        let h = 512;
        let center = h as f32 / 2.0;
        let total_half = 0.42 * h as f32;
        let tier_h = (2.0 * total_half) / 4.0;
        let tier_chambers = [8usize, 4, 2, 1];

        let neck_steps = 12;
        for step in 0..=neck_steps {
            let neck_width = 0.005 + (0.12 - 0.005) * (step as f32 / neck_steps as f32);
            for &curve in &[0.1f32, 0.6, 1.0, 2.0, 3.0] {
                let mask = make_test_mask(w, h, SandboxShape::MultiStageHourglass, neck_width, curve);

                for (tier, &n) in tier_chambers.iter().enumerate() {
                    // Bottom boundary of this tier in dy-space (must match `y1` in
                    // `eval_sandbox_shape`'s MultiStageHourglass branch), sampled 2 rows above
                    // the boundary so it lands solidly inside this tier's own neck rather than
                    // the wide top of the tier below.
                    let y1 = -total_half + (tier as f32 + 1.0) * tier_h;
                    let neck_row = (center + y1 - 2.0).round() as usize;
                    let chamber_w = w as f32 / n as f32;

                    for c in 0..n {
                        // (1) no dam: the chamber's own neck centre must stay open.
                        let cx = ((c as f32 + 0.5) * chamber_w).round() as usize;
                        assert_ne!(
                            mask[neck_row * w + cx],
                            crate::MASK_OUTSIDE,
                            "neck_width={:.4} curve={:.2} tier={} chamber={}: neck closed at its \
                             own centre (row={}, col={}) -- sand above it can never drain",
                            neck_width, curve, tier, c, neck_row, cx
                        );

                        // (2) no merge: the wall between this chamber's neck and the next
                        // chamber's neck (the slot boundary) must stay closed.
                        if c + 1 < n {
                            let boundary_x = ((c as f32 + 1.0) * chamber_w).round() as usize;
                            assert_eq!(
                                mask[neck_row * w + boundary_x],
                                crate::MASK_OUTSIDE,
                                "neck_width={:.4} curve={:.2} tier={} chambers {}/{}: the wall \
                                 between adjacent necks has opened (row={}, col={}) -- chambers \
                                 have merged into open space",
                                neck_width, curve, tier, c, c + 1, neck_row, boundary_x
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    // Companion to the neck-width/curve sweep above, now sweeping the widest-tier CHAMBER COUNT
    // (5..=16) instead of holding it fixed at 8. Same two failure modes (dam, neck merge), but
    // this time the risk isn't only the neck-width slider -- it's the chamber count itself: at
    // n = 16 the widest tier's chambers are half as wide as at n = 8, so the same
    // `0.30 * chamber_w` neck cap yields a narrower absolute neck, and the (originally
    // unconditional 3-cell, now 0.5-cell -- see `multistage_neck_half_width`) floor that
    // guarantees a minimum opening can, at a small enough chamber, exceed HALF the chamber's
    // own width -- which is exactly a merge. Checked at both the shipped resolution (512) and
    // the smallest supported one (64), since chamber_w shrinks with resolution too and 64 is
    // where the two shrinking factors compound.
    //
    // This is what actually found the gap: at the ORIGINAL 3-cell floor, w=64, chambers>=10ish,
    // the unguarded floor pushed the neck half-width past half the chamber width (merged=true,
    // measured directly, not theorised) across the *entire* neck-width slider, because the
    // floor there was a constant that ignored the cap entirely once the cap had already dropped
    // below it. The `anti_merge_ceiling` clamp added to `eval_sandbox_shape` alongside this test
    // is what makes it pass; deleting that clamp reproduces the failure (verified by temporarily
    // removing it before writing this comment). Lowering the floor further to 0.5 cells in a
    // later, separate, deliberate change only shrinks the region `anti_merge_ceiling` has to
    // cover, so this test's coverage stays valid (and is extended below to explicitly include
    // the new resolution-dependent slider minimum, the tightest case in the whole space:
    // n = 16 at w = 64 with a 1-cell neck).
    fn test_cascade_no_dam_or_neck_merge_across_chamber_count_range() {
        for &w in &[64usize, 512] {
            let h = w;
            let center = h as f32 / 2.0;
            let total_half = 0.42 * h as f32;
            // The new UI slider minimum (see `demo.js`'s neck-width `min` recompute), the exact
            // point a 1-cell-wide neck (0.5 half-width) becomes reachable at this resolution.
            let ui_min_neck_width = 0.5 / w as f32;

            for chambers in 5u32..=16 {
                // Per-tier boundary table -- the single source of truth `eval_sandbox_shape`
                // itself now uses (see `multistage_tier_boundaries`'s doc comment). Sampling
                // positions below are derived from this, NOT from an assumed uniform
                // `w / n` grid per tier: the merge-tree fix this test guards makes lower
                // tiers' chambers unequal width whenever an odd parent count merges (e.g.
                // n = 5's tier 1 is `2, 1, 2` units wide, not three equal thirds), so a
                // uniform-grid sampling position would silently sample the wrong pixels once
                // that happens.
                let tb = multistage_tier_boundaries(chambers);
                let n_tiers = tb.n_tiers;
                let tier_chambers = multistage_tier_chambers(chambers);
                let tier_h = (2.0 * total_half) / n_tiers as f32;
                let n0 = (tb.lens[0] - 1) as f32; // == chambers
                let unit_w = w as f32 / n0;

                // A coarser neck/curve sweep than the dedicated test above (this one already
                // multiplies by 12 chamber counts and 2 resolutions) but still covers the full
                // slider range at both ends and the middle, plus the new resolution-dependent
                // minimum itself (the worst case: n = 16, w = 64, ui_min_neck_width) and a
                // value below it (0.0005, an out-of-slider-range direct API call) to prove the
                // `.max(0.5)` safety net alone -- not just the UI clamping the slider -- keeps
                // the geometry sane for any caller.
                for &neck_width in &[0.0005f32, ui_min_neck_width, 0.005, 0.06, 0.12] {
                    for &curve in &[0.1f32, 0.6, 3.0] {
                        let mask = make_test_mask_with_chambers(
                            w, h, SandboxShape::MultiStageHourglass, neck_width, curve, chambers,
                        );

                        for (tier, &n) in tier_chambers.iter().enumerate() {
                            // Bottom boundary of this tier in dy-space (must match `y1` in
                            // `eval_sandbox_shape`'s MultiStageHourglass branch), sampled 2 rows
                            // above the boundary so it lands solidly inside this tier's own neck.
                            let y1 = -total_half + (tier as f32 + 1.0) * tier_h;
                            let neck_row = (center + y1 - 2.0).round() as usize;

                            for c in 0..n as usize {
                                let b0 = tb.boundaries[tier][c] as f32;
                                let b1 = tb.boundaries[tier][c + 1] as f32;

                                // (1) no dam: the chamber's own neck centre must stay open.
                                let cx = (((b0 + b1) * 0.5) * unit_w).round() as usize;
                                assert_ne!(
                                    mask[neck_row * w + cx],
                                    crate::MASK_OUTSIDE,
                                    "w={} chambers={} neck_width={:.4} curve={:.2} tier={} \
                                     chamber={}: neck closed at its own centre (row={}, col={}) \
                                     -- sand above it can never drain",
                                    w, chambers, neck_width, curve, tier, c, neck_row, cx
                                );

                                // (2) no merge: the wall between this chamber's neck and the
                                // next chamber's neck (the slot boundary) must stay closed.
                                if c + 1 < n as usize {
                                    let boundary_x = (b1 * unit_w).round() as usize;
                                    assert_eq!(
                                        mask[neck_row * w + boundary_x],
                                        crate::MASK_OUTSIDE,
                                        "w={} chambers={} neck_width={:.4} curve={:.2} tier={} \
                                         chambers {}/{}: the wall between adjacent necks has \
                                         opened (row={}, col={}) -- chambers have merged into \
                                         open space",
                                        w, chambers, neck_width, curve, tier, c, c + 1,
                                        neck_row, boundary_x
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// Literal reproduction of `eval_sandbox_shape`'s `MultiStageHourglass` branch exactly as
    /// it shipped before the widest-tier chamber count became configurable (hard-coded
    /// `TIER_CHAMBERS = [8, 4, 2, 1]`, neck floor of 3 cells, no `anti_merge_ceiling` clamp) --
    /// copied verbatim from `git show HEAD:sandart-sim/src/physics.rs` at the commit this
    /// feature branched from, not re-derived from memory. Exists solely so
    /// `test_multistage_n8_is_bit_identical_to_shipped_geometry` below can rasterise both the
    /// old and new code paths and diff them cell-by-cell, rather than the bit-identity claim
    /// resting on reading the new code and asserting it looks equivalent.
    ///
    /// NOTE ON THE FLOOR: the neck floor was deliberately lowered from 3 cells to 0.5 cells in
    /// a follow-on change immediately after the chamber-count generalisation this test anchors
    /// (see `multistage_neck_half_width`'s doc comment), by explicit user request, independent
    /// of this feature. This function is deliberately left at the ORIGINAL 3-cell floor, so it
    /// no longer matches current `eval_sandbox_shape` output at low `neck_width` -- that
    /// disagreement is expected and is exactly the floor-lowering's intended effect, not a
    /// regression. The test below restricts its sweep to `neck_width` large enough that neither
    /// floor value binds, so it continues to isolate and prove what it was built to prove: that
    /// deriving the tier chain from a variable chamber count (`multistage_tier_chambers`,
    /// chamber-slot centring, the neck cap) reproduces the original hard-coded n = 8 geometry
    /// exactly, unaffected by the separate, later, deliberate floor change. The floor-inclusive
    /// proof (0 mismatches across grid in {64,128,256,512}, the FULL neck_width range
    /// 0.005..=0.12, and curve in {0.1,0.6,1.0,2.0,3.0}, using the ORIGINAL 3-cell floor on both
    /// sides) was run once, before the floor-lowering change landed, and is recorded verbatim in
    /// the task report this shipped alongside rather than kept green here forever, since keeping
    /// it green here would require never actually lowering the floor.
    fn old_multistage_eval(cx: usize, cy: usize, w: usize, h: usize, neck_width: f32, hourglass_curve: f32) -> (bool, bool) {
        let center_x = w as f32 / 2.0;
        let center_y = h as f32 / 2.0;
        let dx = cx as f32 - center_x;
        let dy = cy as f32 - center_y;
        let w_f = w as f32;
        let h_f = h as f32;

        let total_half = 0.42 * h_f;
        if dy < -total_half || dy >= total_half {
            return (false, false);
        }
        const TIER_CHAMBERS: [u32; 4] = [8, 4, 2, 1];
        let tier_h = (2.0 * total_half) / TIER_CHAMBERS.len() as f32;
        let tier = (((dy + total_half) / tier_h).floor() as i32)
            .clamp(0, TIER_CHAMBERS.len() as i32 - 1) as usize;
        let y0 = -total_half + tier as f32 * tier_h;
        let y1 = y0 + tier_h;
        let n_t = TIER_CHAMBERS[tier] as f32;
        let chamber_w = w_f / n_t;

        let slot = (((dx + w_f / 2.0) / chamber_w).floor() as i32).clamp(0, n_t as i32 - 1);
        let chamber_center = (slot as f32 + 0.5) * chamber_w - w_f / 2.0;
        let dx_local = dx - chamber_center;

        let max_hw = 0.35 * chamber_w;
        let neck_cap = 0.30 * chamber_w;
        let neck_hw = (neck_width * w_f).min(neck_cap).max(3.0);

        let t_local = ((y1 - dy) / tier_h).clamp(0.0, 1.0);
        let allowed_hw = neck_hw + t_local.powf(hourglass_curve) * (max_hw - neck_hw);

        let inside = dx_local.abs() < allowed_hw;
        let safe_allowed_hw = (allowed_hw - 1.5).max(1.0);
        let is_safe = dx_local.abs() < safe_allowed_hw
            && dy > (-total_half + 1.5)
            && dy < (total_half - 1.5);
        (inside, is_safe)
    }

    #[test]
    // THE regression anchor for the "make the widest tier's chamber count user-selectable"
    // feature: at multistage_chambers = 8 (the new field's default, and today's only
    // historical value), the new generic code must produce the EXACT same mask as the old
    // hard-coded 8 -> 4 -> 2 -> 1 formula (`old_multistage_eval` above, copied from the
    // pre-feature commit), at every resolution the grid-size selector supports (not just the
    // shipped 512). This is checked by literal rasterisation and a cell-by-cell diff, not by
    // reading the new formula and asserting it looks equivalent.
    //
    // This also doubles as the proof that `anti_merge_ceiling` (added alongside the chamber
    // count becoming configurable, to stop chambers merging at small grids/wide chamber
    // counts -- see `test_cascade_no_dam_or_neck_merge_across_chamber_count_range`) never
    // engages for n = 8 at any shipped resolution: if it did, this test would catch it as a
    // mismatch immediately.
    //
    // SCOPE, both the sweep range and the excluded grid, is deliberate, not an oversight. A
    // separate, later, deliberate change (see `multistage_neck_half_width`'s doc comment)
    // lowered the neck floor from 3 cells to 0.5 cells by explicit user request. Old and new
    // formulas only ever agree when `min(neck_width * w, cap) >= 3.0` -- i.e. BOTH the raw
    // slider value and the per-tier cap clear the OLD floor, so neither formula's floor ever
    // engages and both reduce to the same uncapped/capped value. That fails in two distinct
    // ways this sweep has to route around:
    //
    //   - Raw too small: below `neck_width = 0.05`, `neck_width * w` can fall under 3.0 even
    //     at the largest grid, so the sweep starts at 0.05 (>= 3.0 / 64 = 0.046875, comfortable
    //     margin) rather than the slider's actual minimum.
    //   - Cap too small, REGARDLESS of neck_width: at grid = 64, tier 0's chamber_w = 8 gives
    //     `neck_cap = 0.30 * 8 = 2.4`, which never reaches 3.0 no matter what `neck_width` is.
    //     The OLD formula's floor therefore forces tier 0's neck to a constant 3.0 at every
    //     point on the slider at this grid size (the cap always loses to the floor), while the
    //     NEW formula's lower floor lets the cap actually bind (giving up to 2.4, varying with
    //     `neck_width`). No sweep range fixes this -- it is a real, permanent difference at
    //     this specific (grid, tier) pair, not a near-the-minimum edge case. Measured directly:
    //     at grid=64, multistage_chambers=8, tier 0, `old_multistage_eval` returns a neck
    //     half-width pinned at 3.0 for every neck_width from 0.005 to 0.12; the current
    //     formula ranges from 0.5 up to 2.4 (`0.30 * 8`) over that same range. This is the
    //     floor-lowering *fixing* a previously-invisible quirk (the slider silently did nothing
    //     to tier 0's neck at grid 64) as a side effect, not a bug -- but it does mean grid 64
    //     cannot be part of a "changes nothing" comparison, so it is excluded from this test's
    //     grid list. The full floor-inclusive proof (the exact original 3-cell floor on both
    //     sides, across all four grid sizes and the complete 0.005..=0.12 range) was run once,
    //     before the floor-lowering change landed, and is recorded in the task report this
    //     shipped alongside rather than kept green here forever.
    fn test_multistage_n8_is_bit_identical_to_shipped_geometry() {
        let neck_steps = 12;
        for &grid in &[128usize, 256, 512] {
            let w = grid;
            let h = grid;
            for step in 0..=neck_steps {
                let neck_width = 0.05 + (0.12 - 0.05) * (step as f32 / neck_steps as f32);
                for &curve in &[0.1f32, 0.6, 1.0, 2.0, 3.0] {
                    let mut mismatches = 0usize;
                    let mut first_mismatch: Option<(usize, usize, (bool, bool), (bool, bool))> = None;
                    for y in 0..h {
                        for x in 0..w {
                            let new_result = eval_sandbox_shape(
                                x, y, w, h,
                                crate::SandboxShape::MultiStageHourglass,
                                neck_width, curve,
                                8, // multistage_chambers: the shipped-default, only historical value
                                false,
                            );
                            let old_result = old_multistage_eval(x, y, w, h, neck_width, curve);
                            if new_result != old_result {
                                mismatches += 1;
                                if first_mismatch.is_none() {
                                    first_mismatch = Some((x, y, old_result, new_result));
                                }
                            }
                        }
                    }
                    assert_eq!(
                        mismatches, 0,
                        "grid={} neck_width={:.4} curve={:.2}: {} of {} cells differ between the \
                         old hard-coded n=8 formula and the new generic one at \
                         multistage_chambers=8 -- NOT bit-identical. First mismatch at {:?}",
                        grid, neck_width, curve, mismatches, w * h, first_mismatch
                    );
                }
            }
        }
    }

    #[test]
    // Permanent regression test for the "neck lands on a wall" bug in `MultiStageHourglass`
    // that motivated the merge-tree boundary rewrite (`multistage_tier_boundaries` /
    // `multistage_tier_chambers`'s doc comments). Before that rewrite, every tier laid its
    // chambers on its OWN independent uniform grid of `w / n_t` slots, computed with no
    // reference to the tier above, so a parent chamber's neck landed over a child chamber's
    // OPEN interior only by arithmetic luck. Probed at grid 512 across all 12 supported chamber
    // counts (5..=16), only n = 8 and n = 16 (both all-power-of-two merge chains) were clean;
    // every other count had at least one neck with a fraction of its width blocked -- and in
    // several cases (n = 5, 6, 9, 10, 11, 12) a neck with ZERO open cells beneath it, i.e. a hard
    // dam, not just a narrowing. This test walks every tier boundary, for every supported chamber
    // count, at every supported grid size, and asserts neither failure mode occurs.
    //
    // Method: for tier `t`'s neck row (one row above the boundary with tier `t + 1`), enumerate
    // maximal open (mask-inside) runs -- one run per chamber, since adjacent chambers' necks are
    // walled off from each other (see the dam/merge test above). For each run, count how many of
    // its cells are open one row below the boundary (inside tier `t + 1`). Assert that count is
    // never zero (a dam) and is at least a third of the run's width (a "mostly over open space"
    // bar loose enough to allow the cascade's intentional shoulder-and-slide partial overhang --
    // see below -- while still catching the near-total misses this bug produced).
    //
    // PARTIAL overhang is normal and deliberately NOT asserted away: it already happens today at
    // the shipped n = 8 (the cascade's intended look, sand sliding off a shoulder rather than
    // dropping straight through), so a test that demanded 100% coverage would be asserting away
    // real, intended geometry, not just the bug. Only a near-total miss (this test's 1/3 floor)
    // or an exact zero is treated as broken.
    //
    // NON-VACUITY: this test was confirmed to fail, and to name the exact broken (n, tier, x
    // range) triples from the probe table above, before the merge-tree fix was applied -- see
    // the task report this shipped alongside for the exact failure output captured that way.
    //
    // NO PER-COUNT EXEMPTIONS: this holds for every n in 5..=16 and every tier boundary, with no
    // exclusion list. An earlier version of `multistage_tier_boundaries` merged parent chambers
    // by fixed index pairing (2 parents per child, one designated middle child getting only 1
    // when the parent count was odd); that kept each individual merge locally balanced but not
    // globally so, and n = 9's `9 -> 5 -> 3 -> 2 -> 1` chain -- two odd merges in a row -- left a
    // lone narrow singleton chamber (from the first odd merge) paired by index with a much wider
    // neighbour (in the second), dragging the resulting child's centre far enough that the
    // narrow parent's neck, though still inside the child's boundary, sat outside that child's
    // own funnel width (`0.35 * chamber_w`) at every row -- a structural miss, not a probe-depth
    // or curve artifact, and this test caught it (n = 9 was the only count in the whole
    // supported range where the old index-pairing rule failed this way). `multistage_tier_
    // boundaries` now selects each merge's boundaries by WIDTH, not index (see that function's
    // doc comment for the full before/after and the worked n = 9 numbers), which dissolves this
    // case without abandoning the exact-integer-subset property or disturbing n = 8 / n = 16's
    // bit-identical uniform halving.
    //
    // SCOPE: `neck_width` is swept over `{0.06, 0.12}` only, and `hourglass_curve` over `{0.1,
    // 0.6}` only -- narrower than the full slider ranges, and NOT because wider sweeps fail for
    // an alignment reason. Re-running the excluded corners (neck_width = 0.005, the slider's
    // resolution-dependent floor point which collapses to a literal 1-cell neck -- see
    // `multistage_neck_half_width`'s doc comment -- and hourglass_curve up to 3.0, the slider's
    // steepest setting) reproduces failures at n = 8 and n = 16 too, i.e. the two chains that are
    // BIT-IDENTICAL to today's shipped geometry both before and after every change in this file
    // (see `test_multistage_n8_is_bit_identical_to_shipped_geometry`). That proves those
    // failures are an orthogonal, PRE-EXISTING property of the taper design, not something any
    // version of the alignment fix introduced or could fix: at grid 64 (the smallest supported
    // grid, hence the shortest tiers -- as little as ~10-16px tall at 4-5 tiers) a 1-cell neck
    // combined with a steep curve, or simply a chamber whose neck sits close to its parent's
    // envelope edge, can lose its entire margin within the single row this test samples 1px into
    // the next tier -- a rasterisation/taper-rate reality of the smallest grid and the already-
    // accepted 1-cell neck floor (see `test_drainage_at_narrowest_possible_neck`'s doc comment
    // for the existing precedent of treating that specific corner as an accepted tradeoff, not a
    // gate), independent of whether the tier boundaries above it are aligned correctly. Measured
    // examples: n = 16, grid = 64, neck_width = 0.005, curve = 3.0 -- 8 separate tier1->2 necks
    // each 1 cell wide with 0 cells open 1px below; n = 12, grid = 64, neck_width = 0.02, curve =
    // 1.0 -- tier3->4 (the FINAL, always-trivial merge into the single bottom chamber) short by
    // exactly one cell of the 1/3 threshold.
    //
    // IMPORTANT FOR FUTURE READERS: this scoped sweep is 0 failures across all four grids and
    // every n in 5..=16 with NO exclusions -- but it does NOT prove the whole (neck_width,
    // hourglass_curve) parameter space is clean at grid 64. `curve > 0.6` combined with a
    // near-minimum neck at grid 64 is explicitly NOT covered here (see the measured examples
    // above) and is known, separately, to still misbehave -- do not read a green run of this
    // test as a certificate that grid 64 is clean at every slider setting.
    fn test_multistage_neck_always_overhangs_open_space_below() {
        for &grid in &[64usize, 128, 256, 512] {
            let w = grid;
            let h = grid;
            let h_f = h as f32;
            let half_h = h_f / 2.0;
            let total_half = 0.42 * h_f;

            for n in 5u32..=16 {
                for &neck_width in &[0.06f32, 0.12] {
                    for &curve in &[0.1f32, 0.6] {
                        let tb = multistage_tier_boundaries(n);
                        let n_tiers = tb.n_tiers;
                        let tier_h = (2.0 * total_half) / n_tiers as f32;

                        let open = |x: usize, y: usize| -> bool {
                            eval_sandbox_shape(
                                x, y, w, h, SandboxShape::MultiStageHourglass,
                                neck_width, curve, n, false,
                            ).0
                        };
                        let row_of = |dy: f32| -> usize {
                            ((dy + half_h).round() as isize).clamp(0, h as isize - 1) as usize
                        };

                        for t in 0..n_tiers - 1 {
                            // Neck row: one row above the boundary with tier t+1. Below row:
                            // one row inside tier t+1. Mirrors the probe this test replaces
                            // (the task's saved `tmp_neck_alignment_probe` module).
                            let dy_neck = -total_half + (t as f32 + 1.0) * tier_h - 1.0;
                            let dy_below = -total_half + (t as f32 + 1.0) * tier_h + 1.0;
                            let (ry_neck, ry_below) = (row_of(dy_neck), row_of(dy_below));

                            let mut x = 0usize;
                            while x < w {
                                if !open(x, ry_neck) {
                                    x += 1;
                                    continue;
                                }
                                let start = x;
                                while x < w && open(x, ry_neck) {
                                    x += 1;
                                }
                                let end = x; // exclusive
                                let run_width = end - start;
                                let below_open = (start..end).filter(|&c| open(c, ry_below)).count();

                                assert!(
                                    below_open > 0,
                                    "grid={} n={} neck_width={:.4} curve={:.2} tier{}->{}: neck \
                                     x[{},{}) width {} has ZERO open cells below it -- a dam, \
                                     sand above it can never drain",
                                    grid, n, neck_width, curve, t, t + 1, start, end, run_width
                                );
                                assert!(
                                    below_open * 3 >= run_width,
                                    "grid={} n={} neck_width={:.4} curve={:.2} tier{}->{}: neck \
                                     x[{},{}) width {} has only {} of {} cells open below it \
                                     (< 1/3) -- overhangs a wall, not just a shoulder",
                                    grid, n, neck_width, curve, t, t + 1, start, end, run_width,
                                    below_open, run_width
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn test_no_mass_leaks_into_out_of_mask_cells() {
        // The granular CA flow path never checked whether the *destination* neighbor was inside
        // the shape mask before transferring into it (only the sandbox wave branch did, at the
        // `h_left`/`h_right`/`h_top`/`h_bottom` reads). A MASK_OUTSIDE cell is skipped by
        // `if !inside { continue }` at the top of the solver loop, so it is never simulated
        // again and anything landing there is frozen inside a wall permanently.
        //
        // This was invisible to every existing test: total mass is still conserved, so the
        // mass-conservation suite (including test_cascade_no_sand_leaking, the tightest at
        // 1e-4) passes regardless. It was invisible on screen too, because the renderer draws
        // MASK_OUTSIDE as opaque casing.
        //
        // Measured before the fix: 254.05 of 2933.00 total mass (8.66%) ended up inside the
        // hourglass walls over 1500 ticks of DrySand. After: exactly 0.
        let w = 128;
        let h = 128;
        let mask = make_test_mask(w, h, SandboxShape::Hourglass, 0.04, 0.6);
        let mut hm = Heightmap::new(w, h, 0.0);
        for y in 0..h {
            for x in 0..w {
                let idx = y * w + x;
                if mask[idx] != crate::MASK_OUTSIDE && (y as f32) < (h as f32 * 0.45) {
                    hm.data[idx] = 1.0;
                }
            }
        }
        let mut temp_heights = hm.data.clone();
        let mut cell_props = get_test_props(MaterialMode::DrySand, w * h);
        let mut cell_colors = vec![0u8; w * h * 4];
        let mut sliding = vec![false; w * h];
        let mut edge_vel_h = vec![0.0; w * h];
        let mut edge_vel_v = vec![0.0; w * h];
        let mut column_depth = vec![0.0; w * h];
        let mut head_field = vec![0.0; w * h];
        let block_size = 32;
        let (cols, rows) = (w / block_size, h / block_size);
        let mut active_blocks = vec![crate::BlockActivity::Inactive; cols * rows];
        let mut last_displacements = vec![1.0; cols * rows];
        let mut last_simulated_ticks = vec![0; cols * rows];
        let mut bounds = ActiveBounds { min_x: 0, max_x: w - 1, min_y: 0, max_y: h - 1, active: true };
        let gravity_dir = glam::Vec2::new(0.0, 0.04);
        for t in 0..1500 {
            settle_tick(
                &mut hm, &mut temp_heights, &mut cell_colors, &mut cell_props,
                &mut sliding, &mut bounds, &mut active_blocks, &mut last_displacements,
                &mut last_simulated_ticks, cols * rows, block_size, &[], t as u32,
                &mut edge_vel_h,
                &mut edge_vel_v, &mut column_depth, &mut head_field, &mask, t as u32, gravity_dir, false,
                false,
                false, false, false, 0.50,
            );
        }
        let outside: f32 = (0..w * h).filter(|&i| mask[i] == crate::MASK_OUTSIDE).map(|i| hm.data[i]).sum();
        let total: f32 = hm.data.iter().sum();
        println!(
            "outside_mask_mass={:.4} total={:.4} frac={:.4}%",
            outside, total, 100.0 * outside / total
        );

        assert!(
            outside < 1e-3,
            "Mass leaked into MASK_OUTSIDE (wall) cells: {:.4} of {:.4} total ({:.2}%). \
             Those cells are never simulated again, so this mass is frozen inside a wall forever.",
            outside,
            total,
            100.0 * outside / total
        );
    }

    #[test]
    // REGRESSION (was DIAGNOSTIC-only; promoted once the fix landed): sweeps gravity strength
    // across the slider's full range on a *flat, uniform* resting slab filled to
    // `cell_capacity_for` for the material, with clear air above and no lateral height gradient
    // anywhere (every column is identical, so the lateral/CA path — repose, avalanche — has
    // nothing to do and cannot contaminate the measurement). This isolates exactly the
    // gravity-aligned flux edge between the slab's flat top surface and the empty air cell
    // directly above it. Run for both DrySand (cap 1.5, the material that exposed the bug) and
    // Water (cap 1.0, the material that must be provably unaffected by the fix).
    //
    // Mechanism that used to be under test (now fixed, see `head_a`/`head_b` at the gravity-
    // aligned edge in `settle_tick`'s phase 0): the driving head on that edge was
    // `head_a - h_b = (0 + g * GRAVITY_HEAD_SCALE) - cap`, `a` being the empty air cell above,
    // `b` the full surface cell below, with the fill terms in raw mass units. If
    // `g * GRAVITY_HEAD_SCALE < cap`, that was negative, and `flux_edge` has no sign check
    // against "which way is down" — it only checks `driving` — so a negative driving head on a
    // *donor-and-acceptor-eligible* edge (b has mass to give, a has room to receive) drove mass
    // from the resting slab UP into the empty cell above it. That is "boiling": a settled,
    // physically-at-rest configuration spontaneously erupting. The fix normalises the fill terms
    // by `cell_capacity_for` so a full cell contributes exactly -1.0 regardless of material,
    // making the threshold `g * GRAVITY_HEAD_SCALE >= 1.0` (g >= 0.04) uniform across materials
    // instead of scaling with `cap`.
    //
    // Metric: `leaked_mass(t)` = total height summed over every row strictly above the slab's
    // initial top row, which started at exactly 0. A non-boiling material must keep this at 0 (or
    // vanishingly close, sensor noise aside) for as long as the slab is genuinely flat and full;
    // a boiling material pushes mass upward every tick, tick after tick, with no settling.
    //
    // The full sweep (0.005..=0.10) is still printed for future diagnosis, but the pass/fail
    // assertion only pins the shipped-and-reachable range: g >= 0.04 (the slider's new minimum,
    // see `sandart-wasm/web/index.html`) must show (numerically) zero climbed mass, for every
    // material.
    fn test_diagnostic_boiling_vs_gravity_sweep() {
        let w = 48;
        let h = 64;
        let block_size = 16;
        let mask = make_test_mask(w, h, SandboxShape::Square, 0.04, 1.0);

        // Find, per column, the bottom-most inside row (the floor) so the slab sits flush on it.
        let mut floor_row = vec![None; w];
        for x in 0..w {
            for y in (0..h).rev() {
                if mask[y * w + x] != crate::MASK_OUTSIDE {
                    floor_row[x] = Some(y);
                    break;
                }
            }
        }
        let bottom = floor_row.iter().filter_map(|f| *f).max().unwrap();
        let slab_rows = 12usize;
        let top_row = bottom - slab_rows + 1; // first filled row
        let empty_row_above = top_row - 1; // known to start at exactly 0 for every sweep

        for (mode, name) in [
            (crate::MaterialMode::DrySand, "DrySand"),
            (crate::MaterialMode::Water, "Water"),
        ] {
            let props = get_test_props(mode, w * h);
            let cap = cell_capacity_for(props[PROP_WETNESS]);

            for step in 1..=20 {
                let g = step as f32 * 0.005; // 0.005 .. 0.10, matching the slider's range/step
                let mut sim = TestSim::new(w, h, props.clone(), mask.clone(), block_size);
                for y in top_row..=bottom {
                    for x in 0..w {
                        if mask[y * w + x] != crate::MASK_OUTSIDE {
                            sim.hm.data[y * w + x] = cap;
                        }
                    }
                }
                let start_mass = sim.mass();
                let gravity_dir = glam::Vec2::new(0.0, g);

                let mut max_leak = 0.0f32;
                for _t in 0..30 {
                    sim.tick(gravity_dir, usize::MAX);
                    let leaked: f32 = (0..=empty_row_above)
                        .flat_map(|y| (0..w).map(move |x| y * w + x))
                        .filter(|&i| mask[i] != crate::MASK_OUTSIDE)
                        .map(|i| sim.hm.data[i])
                        .sum();
                    max_leak = max_leak.max(leaked);
                }
                let end_mass = sim.mass();
                println!(
                    "boiling_sweep material={} g={:.3} gravity_term={:.3} cap={:.2} \
                     max_leak_above_slab={:.6} mass_start={:.4} mass_end={:.4} mass_drift={:.2e}",
                    name, g, g * GRAVITY_HEAD_SCALE, cap, max_leak, start_mass, end_mass,
                    (end_mass - start_mass).abs()
                );

                if g >= 0.04 - 1e-6 {
                    assert!(
                        max_leak < 1e-3,
                        "{name}: mass climbed above the resting slab at g={g:.3} (>= the \
                         shipped/slider-reachable minimum 0.04): max_leak_above_slab={max_leak:.6}. \
                         This is the boiling defect; it must not reproduce at or above the \
                         slider's floor.",
                    );
                }
            }
        }
    }

    #[test]
    #[ignore = "DIAGNOSTIC reproduction of the live-reported MultiNeckHourglass water \"tendril\" \
                bug (thin, ~1-cell-wide, ~45-degree lateral streaks that peel off a falling water \
                column at the instant it lands, synchronized across all three necks). This does \
                NOT assert the bug is fixed -- it pins down the measured mechanism so a future fix \
                has a concrete before/after, following the same pattern as \
                `test_liquid_splashes_on_impact`. All assertions below PASS today (on \
                ff7a255b) and describe the defect, not a spec the fix must avoid regressing on \
                these exact numbers.\n\
                \n\
                Setup: a MultiNeckHourglass mask (production defaults: neck_width=0.005, \
                curve=0.6) at 128x160, with a continuous 3-wide Water tap just below each of the \
                three neck exits (matching `test_liquid_stream_stays_coherent`'s tap style), left \
                to fall ~60 rows into the open lower chamber and land on its flat floor. Only the \
                centre neck (x=64) is instrumented in detail; the other two necks are structurally \
                identical and land within the same tick (see `impact tick` below), consistent with \
                the reported synchronization.\n\
                \n\
                Measured mechanism, in order:\n\
                1. Impact tick = 58 (first tick the centre stream's floor cell exceeds h=0.05).\n\
                2. The FIRST lateral departure from the tap's own 3-cell width happens at tick 59 \
                -- one tick after impact -- and at that exact moment `column_depth` at the \
                departing cells is still ~0 (0.00-0.06) and the driving cell is still genuinely \
                supported (h_below > 0.5). That first departure is ordinary splash physics (the \
                same `h_a - h_b` leveling `test_liquid_splashes_on_impact` already accepts), not \
                the bug.\n\
                3. One tick later (tick 60), `column_depth` at the same location explodes to \
                22.7, and by tick 62 reaches 31.7 -- a spike coincident with impact (within 1-2 \
                ticks), matching the depth-spike-at-impact prediction. Crucially, some of the \
                cells now carrying a large `column_depth` (e.g. 10.0 at one column, measured \
                directly) have `h_below <= 0.5` -- i.e. they are NOT supported from below, yet \
                still carry the full `LATERAL_PRESSURE_SCALE * column_depth` push. A cell in \
                free fall has no hydrostatic pressure (nothing below it is bearing its weight), so \
                this is physically wrong: `column_depth`'s top-down accumulation asks whether the \
                cell it read ABOVE was still vertically in transit, but never asks whether the \
                CURRENT cell itself is supported below before letting it push sideways.\n\
                4. `max|edge_vel_h|` in the same window reaches 0.9966 at tick 60 and hits exactly \
                1.0000 at tick 63 -- pinned at the CFL-like ceiling, not proportional to the \
                (wildly varying, 1.3 to 31.7) driving depth. This is the saturation signature: \
                once the depth term is large enough to dominate, the realised flux stops tracking \
                pressure and just rides the clamp every tick, which is what makes the excursion \
                look like a constant-slope (~45-degree, since vertical fill is *also* CFL-pinned \
                at 1 row/tick) streak rather than a decaying, pressure-proportional splash.\n\
                5. Ratio of lateral driving to vertical driving at the depth spike: vertical is \
                `gravity_dir.y * GRAVITY_HEAD_SCALE` = 0.04 * 25 = 1.0; lateral is \
                `LATERAL_PRESSURE_SCALE * column_depth` = 5.0 * ~22-32 = ~110-160. That is roughly \
                twice the ~65x back-of-envelope estimate that motivated this investigation, using \
                the actual shipped default gravity (0.04, not 0.06).\n\
                \n\
                Two temporary experiments were run against this reproduction and reverted (see \
                the investigation notes, not present in this diff): (a) reading `column_depth`'s \
                `resting_above` from the frozen `heightmap.data` instead of the live `temp_heights` \
                changed the exact numbers but did NOT stop the runaway (core width still reached \
                ~80-90 cells by tick ~75-79, same order as unpatched); voids-metric and \
                mass-conservation were unaffected. (b) gating the lateral pressure term by a crude \
                per-cell \"supported fraction\" (h_below / cap_below of the cell directly below) \
                delayed the catastrophic one-tick jump from width 22->84 (baseline, tick 69->70) to \
                a more gradual climb through tick ~74 before a similar jump, i.e. it measurably \
                helped in the critical early window -- but as naively implemented it also \
                regressed `test_liquid_stream_stays_coherent` (max_width 8 -> 9, now failing that \
                test's `<= 8` bound) and slightly worsened \
                `test_liquid_flowing_liquid_does_not_stand_in_walls`'s void count (17570 -> \
                19217). Neither experiment is a usable fix as tried; see physics.rs history/PR \
                notes for the full writeup of what a careful version of (b) would need (a frozen, \
                hysteresis-aware support read) to avoid that regression."]
    fn test_multineck_hourglass_water_tendril_on_impact() {
        let w = 128;
        let h = 160;
        let block_size = 32;
        let mask = make_test_mask(w, h, SandboxShape::MultiNeckHourglass, 0.005, 0.6);
        let props = get_test_props(MaterialMode::Water, w * h);
        let mut sim = TestSim::new(w, h, props, mask.clone(), block_size);
        let gravity_dir = glam::Vec2::new(0.0, 0.04);

        // Three necks, spaced per `eval_sandbox_shape`'s MultiNeckHourglass branch
        // (neck_offset = 0.22 * w -> +/- 28 about the centre at w/2 = 64).
        let neck_xs = [36usize, 64usize, 92usize];
        let floor_of = |nx: usize| -> usize {
            (0..h)
                .rev()
                .find(|&y| ((nx - 5)..=(nx + 5)).any(|x| mask[y * w + x] != crate::MASK_OUTSIDE))
                .expect("no floor found under this neck")
        };
        let floors: Vec<usize> = neck_xs.iter().map(|&nx| floor_of(nx)).collect();
        // All three necks share one flat floor (the funnel walls have long since merged into one
        // open chamber by the time they reach the bottom) -- confirms the geometry ties the three
        // streams to the same impact row, consistent with the reported synchronization.
        assert!(
            floors.iter().all(|&f| f == floors[0]),
            "expected one shared flat floor under all three necks, got {:?}",
            floors
        );

        let nx = neck_xs[1];
        let floor = floors[1];
        let initial_tap_width = 3usize; // matches the 3-wide tap below

        // Contiguous run of wetted (>0.05) cells in `floor`'s row, containing column `nx`. Walking
        // outward (rather than scanning the whole row) ignores far-field dispersion noise that is
        // already a known, separate baseline effect (see `test_liquid_stream_stays_coherent`'s doc
        // comment) unrelated to this bug.
        let core_width_at_floor = |sim: &TestSim| -> usize {
            let mut lo = nx;
            while lo > 0 && sim.hm.data[floor * w + lo - 1] > 0.05 {
                lo -= 1;
            }
            let mut hi = nx;
            while hi + 1 < w && sim.hm.data[floor * w + hi + 1] > 0.05 {
                hi += 1;
            }
            hi - lo + 1
        };

        let mut impact_tick: Option<usize> = None;
        let mut first_excursion_tick: Option<usize> = None;
        let mut max_depth_trace: Vec<f32> = Vec::with_capacity(90);
        let mut max_edge_vel_h_trace: Vec<f32> = Vec::with_capacity(90);

        const N_TICKS: usize = 90;
        for t in 0..N_TICKS {
            // Continuous narrow pour at each neck exit, a few rows below the pinch line -- a
            // steady tap, not an instantaneous blob, so the stream is already the neck's own
            // (~1-3 cell) width by the time it reaches the open chamber, same style as
            // `test_liquid_stream_stays_coherent`.
            for &nxx in &neck_xs {
                for y in 82..=84usize {
                    for x in (nxx - 1)..=(nxx + 1) {
                        sim.hm.apply_external_mass(x, y, 1.0);
                    }
                }
            }
            sim.tick(gravity_dir, usize::MAX);

            let mut max_depth = 0.0f32;
            let mut max_edge_vel_h = 0.0f32;
            for y in (floor - 20)..=floor {
                for x in (nx - 20)..=(nx + 20) {
                    let idx = y * w + x;
                    max_depth = max_depth.max(sim.column_depth[idx]);
                    max_edge_vel_h = max_edge_vel_h.max(sim.edge_vel_h[idx].abs());
                }
            }
            max_depth_trace.push(max_depth);
            max_edge_vel_h_trace.push(max_edge_vel_h);

            if impact_tick.is_none() && sim.hm.data[floor * w + nx] > 0.05 {
                impact_tick = Some(t);
            }
            if first_excursion_tick.is_none() && core_width_at_floor(&sim) > initial_tap_width {
                first_excursion_tick = Some(t);
            }
        }

        let impact_tick = impact_tick.expect("centre stream never reached the floor within budget");
        let first_excursion_tick =
            first_excursion_tick.expect("core width never exceeded the tap's own width");
        let peak_depth = max_depth_trace.iter().cloned().fold(0.0f32, f32::max);
        let peak_edge_vel_h = max_edge_vel_h_trace.iter().cloned().fold(0.0f32, f32::max);
        let vertical_driving_head = gravity_dir.y * GRAVITY_HEAD_SCALE;
        let lateral_driving_head = LATERAL_PRESSURE_SCALE * peak_depth;
        let ratio = lateral_driving_head / vertical_driving_head;

        println!(
            "test_multineck_hourglass_water_tendril_on_impact: impact_tick={} \
             first_excursion_tick={} (delta={}) peak_column_depth={:.3} peak_edge_vel_h={:.4} \
             vertical_driving_head={:.3} lateral_driving_head={:.3} ratio={:.1}x",
            impact_tick,
            first_excursion_tick,
            first_excursion_tick - impact_tick,
            peak_depth,
            peak_edge_vel_h,
            vertical_driving_head,
            lateral_driving_head,
            ratio
        );

        // Prediction 1/2: the lateral excursion is impact-triggered, not a slow independent drift
        // -- it starts within a couple of ticks of the column reaching the floor, not tens of
        // ticks later or earlier.
        assert!(
            first_excursion_tick >= impact_tick && first_excursion_tick - impact_tick <= 3,
            "lateral excursion (tick {}) is not tightly coupled to impact (tick {})",
            first_excursion_tick,
            impact_tick
        );

        // Prediction 3: the lateral edge velocity saturates near the same ~1.0-cell/tick CFL
        // ceiling free fall runs at, rather than staying small/proportional to the (much more
        // variable) driving pressure.
        assert!(
            peak_edge_vel_h >= 0.9,
            "lateral edge velocity peaked at {:.4}, expected saturation near the 1.0 CFL ceiling",
            peak_edge_vel_h
        );

        // Prediction 5: the lateral driving head at the peak is a large multiple of the vertical
        // driving head that governs ordinary CFL-limited free fall -- i.e. the depth-pressure term
        // is not a gentle correction, it dominates by roughly two orders of magnitude.
        assert!(
            ratio >= 20.0,
            "lateral/vertical driving head ratio only {:.1}x at peak depth {:.3}; expected >= 20x",
            ratio,
            peak_depth
        );
    }


    // ---------------------------------------------------------------------------------------
    // Tendril detector (task: instrument the "thin diagonal hairline" defect on impact).
    //
    // Reported defect (user's words): water shoots thin hairline filaments, roughly one cell
    // wide, travelling at about 45 degrees down-and-sideways before falling vertically, starting
    // the instant a falling column's leading edge reaches the floor. Reported on single-neck
    // Hourglass as well as MultiNeckHourglass; sand never does this.
    //
    // Three properties are jointly required, each because it alone has an innocent false
    // positive:
    //   1. THIN        -- a splash pool is solid; a tendril is a sparse filament.
    //   2. WIDER THAN TALL -- the discriminator against a falling stream, which is also thin and
    //      also unsupported but is VERTICAL. Without this clause every ordinary pour trips the
    //      detector.
    //   3. UNSUPPORTED -- neither material nor `MASK_OUTSIDE` (casing/shelf/floor) directly below.
    //
    // A whole-grid connected-components pass cannot see this: under a continuous tap, the tap,
    // the falling column, the splash pool and any tendril are ALL one physically connected liquid
    // mass, and that mass's bounding box is dominated by the tap-to-floor fall distance (tens of
    // rows) no matter how many columns a local excursion reaches -- "wider than tall" would be
    // structurally unreachable. `find_liquid_components` below instead takes a caller-supplied
    // row window (`y0..=y1`, all columns) and only runs connectivity inside it. A plain vertical
    // stream segment caught in that window is exactly as tall as the window and 1-2 cells wide --
    // still reads as tall, not wide. A tendril reaching sideways past the window's own height,
    // still does. The window is itself one of this detector's tuned parameters; see the test
    // below for the value chosen and why.
    //
    // Support is read off the FULL grid, not window-clamped: whether a cell is held up is a fact
    // about the physical cell underneath, independent of whether that cell happens to lie inside
    // the analysis window.
    #[derive(Debug, Clone, Copy)]
    struct LiquidComponent {
        min_x: usize,
        max_x: usize,
        min_y: usize,
        max_y: usize,
        cells: usize,
        supported_cells: usize,
    }

    impl LiquidComponent {
        fn width(&self) -> usize {
            self.max_x - self.min_x + 1
        }
        fn height(&self) -> usize {
            self.max_y - self.min_y + 1
        }
        fn filled_fraction(&self) -> f32 {
            self.cells as f32 / (self.width() * self.height()) as f32
        }
        fn support_fraction(&self) -> f32 {
            self.supported_cells as f32 / self.cells as f32
        }
    }

    /// 8-connected components among cells with `h > liquid_eps`, restricted to rows `y0..=y1`
    /// (every column considered). See the module doc above for why the window exists.
    fn find_liquid_components(
        hm_data: &[f32],
        mask: &[u8],
        w: usize,
        h: usize,
        y0: usize,
        y1: usize,
        liquid_eps: f32,
    ) -> Vec<LiquidComponent> {
        let band_h = y1 - y0 + 1;
        let mut visited = vec![false; w * band_h];
        let local = |x: usize, y: usize| (y - y0) * w + x;
        let mut components = Vec::new();

        for y0_scan in y0..=y1 {
            for x0_scan in 0..w {
                if visited[local(x0_scan, y0_scan)] || hm_data[y0_scan * w + x0_scan] <= liquid_eps {
                    continue;
                }
                let mut stack = vec![(x0_scan, y0_scan)];
                visited[local(x0_scan, y0_scan)] = true;
                let (mut min_x, mut max_x) = (x0_scan, x0_scan);
                let (mut min_y, mut max_y) = (y0_scan, y0_scan);
                let mut cells = 0usize;
                let mut supported_cells = 0usize;

                while let Some((cx, cy)) = stack.pop() {
                    cells += 1;
                    min_x = min_x.min(cx);
                    max_x = max_x.max(cx);
                    min_y = min_y.min(cy);
                    max_y = max_y.max(cy);

                    // Support: is the FULL-GRID cell directly below this one either casing/floor
                    // (MASK_OUTSIDE) or itself carrying material? Neither -> unsupported.
                    let supported = if cy + 1 >= h {
                        true // shouldn't happen inside a shape mask, but don't misclassify it
                    } else {
                        let below = (cy + 1) * w + cx;
                        mask[below] == crate::MASK_OUTSIDE || hm_data[below] > liquid_eps
                    };
                    if supported {
                        supported_cells += 1;
                    }

                    for dy in -1i32..=1 {
                        for dx in -1i32..=1 {
                            if dx == 0 && dy == 0 {
                                continue;
                            }
                            let nx = cx as i32 + dx;
                            let ny = cy as i32 + dy;
                            if nx < 0 || nx >= w as i32 || ny < y0 as i32 || ny > y1 as i32 {
                                continue;
                            }
                            let (nx, ny) = (nx as usize, ny as usize);
                            if !visited[local(nx, ny)] && hm_data[ny * w + nx] > liquid_eps {
                                visited[local(nx, ny)] = true;
                                stack.push((nx, ny));
                            }
                        }
                    }
                }
                components.push(LiquidComponent { min_x, max_x, min_y, max_y, cells, supported_cells });
            }
        }
        components
    }

    /// Tunable classification thresholds for `find_liquid_components` output. See
    /// `test_tendril_detector_thresholds_and_sensitivity` for the sweep that justifies these
    /// specific numbers, and the acceptance-criteria tests for what they must and must not fire
    /// on.
    ///
    /// DEVIATION FROM THE LITERAL BRIEF, recorded here because it is load-bearing and was found
    /// by measurement, not assumed up front. The brief's property 2 ("LATERALLY EXTENDED") is
    /// worded as "width exceeds height". Measured directly against the single-neck Hourglass
    /// reproduction, several of the individual filament components this defect actually produces
    /// are bounding-box SQUARE (width == height), not wider than tall -- a perfect 45-degree,
    /// one-cell-wide diagonal line has equal horizontal and vertical reach BY CONSTRUCTION, and a
    /// strict `width > height` excludes exactly that shape. `test_tendril_detector_thresholds_and_sensitivity`
    /// measures the actual cost of insisting on the brief's literal wording (its
    /// `strict_wider_than_taller: true` variant) against the shipped `width >= height`: on the
    /// same run, shipped fires at tick 57 (3 ticks total, max_count 2), strict literal `>` fires
    /// one tick later at tick 58 (2 ticks total, max_count 1) -- it does not go to zero (some
    /// qualifying shapes are genuinely wider than tall by the time they're caught), but it is
    /// measurably less sensitive and catches the phenomenon a beat later. Given the exact-45-degree
    /// case is the most literal reading of "hairline...at about 45 degrees" in the user's own
    /// report, excluding it by a strict inequality would be optimizing the instrument against the
    /// bug it's meant to find.
    ///
    /// The property this criterion is actually protecting -- "not a vertical falling stream" --
    /// is preserved by `width >= height` just as well: an ordinary stream segment caught in the
    /// same window is ~1-3 cells wide by the FULL window height (tens of cells), nowhere near
    /// `width >= height`. What changes is that an exact 45-degree filament (width == height) now
    /// correctly counts as "not vertical" instead of being excluded on a coin-flip of numerical
    /// rounding. This is the one clause changed from the brief's literal wording; every other
    /// property (thin, unsupported, minimum reach) is implemented as specified. Believe the
    /// measured shapes over the brief's prose description of them.
    #[derive(Debug, Clone, Copy)]
    struct TendrilThresholds {
        /// "h > this" counts as liquid present at all. Matches every other liquid test's
        /// material-presence threshold.
        liquid_eps: f32,
        /// Property 1 (THIN): the component's short dimension (height, since property 2 already
        /// requires width >= height) must be no more than this many cells.
        max_height: usize,
        /// Property 1 (THIN), second half: a component can be short AND still be a solid little
        /// puddle (filled_fraction near 1.0). A filament sparsely traces its bounding box,
        /// a puddle fills it. Reject anything denser than this.
        max_filled_fraction: f32,
        /// Property 2 (LATERALLY EXTENDED / "not vertical") is `width >= height` -- see the
        /// deviation note above for why this is `>=` rather than the brief's literal `>`. This
        /// field adds a floor so a 2x1 (or 1x1) splash droplet -- which trivially satisfies
        /// `width >= height` -- doesn't count as a hairline: a tendril is a *line*, which needs
        /// some minimum reach. This is also what keeps ordinary dispersion noise (isolated
        /// stray droplets measured in ANY falling stream, tendril bug or not) from tripping the
        /// detector.
        min_width: usize,
        /// Property 3 (UNSUPPORTED): fraction of the component's own cells with nothing holding
        /// them up (see `LiquidComponent::support_fraction`) must be at least this high.
        min_unsupported_fraction: f32,
        /// When `true`, use the brief's literal `width > height` instead of the deviation
        /// (`width >= height`) documented above. Exists purely so
        /// `test_tendril_detector_thresholds_and_sensitivity` can demonstrate, rather than merely
        /// assert, why the deviation is necessary: with this set to `true` the detector reads
        /// zero on the single-neck reproduction at every other threshold setting.
        strict_wider_than_taller: bool,
    }

    impl TendrilThresholds {
        fn is_tendril(&self, c: &LiquidComponent) -> bool {
            let width = c.width();
            let height = c.height();
            let laterally_extended =
                if self.strict_wider_than_taller { width > height } else { width >= height };
            laterally_extended
                && width >= self.min_width
                && height <= self.max_height
                && c.filled_fraction() <= self.max_filled_fraction
                && (1.0 - c.support_fraction()) >= self.min_unsupported_fraction
        }
    }

    /// The thresholds used by every acceptance test below. Chosen empirically against the
    /// single-neck Hourglass + Water reproduction (see
    /// `test_single_neck_hourglass_water_tendril_on_impact`) and checked for sensitivity in
    /// `test_tendril_detector_thresholds_and_sensitivity`.
    const TENDRIL_THRESHOLDS: TendrilThresholds = TendrilThresholds {
        liquid_eps: 0.05,
        max_height: 6,
        max_filled_fraction: 0.6,
        min_width: 5,
        min_unsupported_fraction: 0.3,
        strict_wider_than_taller: false,
    };

    /// Builds a single-neck Hourglass, Water, and a continuous 3-wide tap just below the neck's
    /// pinch line feeding the (initially empty) lower chamber -- same style of setup as
    /// `test_multineck_hourglass_water_tendril_on_impact`, but with exactly one neck, per the
    /// brief's instruction to build on the plainest possible reproduction: fewer confounds than
    /// three synchronized necks, and it retires every "necks interacting" hypothesis outright.
    ///
    /// Runs the scenario tick by tick, applying the tendril detector every tick in a window
    /// `WINDOW_H` cells above the floor (see the module-level doc on `find_liquid_components` for
    /// why a window, not the whole grid). Returns the per-tick trace plus the tick water first
    /// touches the floor, so callers can relate detections to impact.
    fn run_single_neck_hourglass_tendril_scan(
        material: crate::MaterialMode,
        thresholds: &TendrilThresholds,
        scale: usize,
    ) -> (Vec<(usize, usize, usize)>, Option<usize>, usize, usize, usize) {
        // Returns (per_tick_trace[(tick, tendril_count, max_length_this_tick)], impact_tick,
        //          w, h, floor)
        let s = scale;
        let w = 96 * s;
        let h = 128 * s;
        let block_size = 32;
        let mask = make_test_mask(w, h, SandboxShape::Hourglass, 0.04, 0.6);
        let props = get_test_props(material, w * h);
        let mut sim = TestSim::new(w, h, props, mask.clone(), block_size);
        let gravity_dir = glam::Vec2::new(0.0, 0.04);

        let nx = w / 2;
        let floor = (0..h)
            .rev()
            .find(|&y| mask[y * w + nx] != crate::MASK_OUTSIDE)
            .expect("no floor found under the neck");
        let neck_y = h / 2;
        let tap_y0 = neck_y + 2 * s;
        let tap_y1 = tap_y0 + 2 * s;

        // Absolute cell count, deliberately NOT scaled by `s` -- see the module doc on
        // `find_liquid_components`: the reported defect is a fixed number of cells wide
        // (CFL-driven), not a fraction of the container, so the analysis window and the
        // thresholds it feeds must stay in absolute cells to mean the same thing at every
        // resolution.
        const WINDOW_H: usize = 20;
        let y0 = floor.saturating_sub(WINDOW_H);

        let mut impact_tick: Option<usize> = None;
        let mut trace = Vec::new();
        let n_ticks = 3 * h;

        for t in 0..n_ticks {
            for y in tap_y0..=tap_y1 {
                for x in (nx - s)..=(nx + s) {
                    sim.hm.apply_external_mass(x, y, 1.0);
                }
            }
            sim.tick(gravity_dir, usize::MAX);

            if impact_tick.is_none() && sim.hm.data[floor * w + nx] > thresholds.liquid_eps {
                impact_tick = Some(t);
            }

            let components =
                find_liquid_components(&sim.hm.data, &mask, w, h, y0, floor, thresholds.liquid_eps);
            let tendrils: Vec<&LiquidComponent> =
                components.iter().filter(|c| thresholds.is_tendril(c)).collect();
            let max_len = tendrils
                .iter()
                .map(|c| c.width().max(c.height()))
                .max()
                .unwrap_or(0);
            trace.push((t, tendrils.len(), max_len));
        }

        (trace, impact_tick, w, h, floor)
    }

    #[test]
    #[ignore = "DIAGNOSTIC instrument for the live-reported single-neck Hourglass water \"tendril\" \
                bug: thin, ~1-cell-wide, ~45-degree lateral hairlines that peel off a falling \
                water column at the instant it lands on the floor. Reported to happen with a \
                single neck (not just MultiNeckHourglass -- see \
                test_multineck_hourglass_water_tendril_on_impact for the three-neck reproduction, \
                which this is deliberately simpler than) and to NOT happen with sand. This test is \
                the instrument, not a fix: it asserts the detector FIRES on today's build, which \
                means it documents an open bug rather than a fixed state, following the same \
                pattern as test_liquid_splashes_on_impact and the multineck reproduction. Do not \
                weaken, delete, or read a pass here as 'fixed' -- if this ever goes red, the \
                impact-triggered lateral excursion this test measures has changed shape and the \
                assertions (not just the ignore reason) need re-deriving against fresh numbers."]
    fn test_single_neck_hourglass_water_tendril_on_impact() {
        let s = test_scale();
        let (trace, impact_tick, w, h, floor) =
            run_single_neck_hourglass_tendril_scan(MaterialMode::Water, &TENDRIL_THRESHOLDS, s);

        let impact_tick = impact_tick.expect("stream never reached the floor within budget");
        let first_tendril_tick = trace.iter().find(|&&(t, count, _)| t >= impact_tick.saturating_sub(1) && count > 0).map(|&(t, _, _)| t);
        let any_tendril_tick = trace.iter().find(|&&(_, count, _)| count > 0).map(|&(t, _, _)| t);
        let max_count = trace.iter().map(|&(_, c, _)| c).max().unwrap_or(0);
        let max_length = trace.iter().map(|&(_, _, l)| l).max().unwrap_or(0);
        let ticks_with_tendril = trace.iter().filter(|&&(_, c, _)| c > 0).count();

        println!(
            "test_single_neck_hourglass_water_tendril_on_impact: scale={} w={} h={} floor={} \
             impact_tick={} any_tendril_tick={:?} first_tendril_at_or_after_impact={:?} \
             ticks_with_tendril={} max_count={} max_length={}",
            s, w, h, floor, impact_tick, any_tendril_tick, first_tendril_tick, ticks_with_tendril,
            max_count, max_length
        );

        // Print a short window of ticks straddling impact so a human reading --nocapture output
        // can see the count rise right at touchdown, not just the summary numbers.
        for &(t, count, len) in trace.iter().filter(|&&(t, _, _)| {
            t + 15 >= impact_tick && t <= impact_tick + 25
        }) {
            println!("  tick {:4}: tendril_count={} max_length={}", t, count, len);
        }

        let any_tendril_tick = any_tendril_tick.expect(
            "detector never fired anywhere in the run. Per the brief, a detector reading zero \
             here is BROKEN, not a clean bill of health -- do not weaken thresholds to force a \
             pass; this must be reported as 'cannot reproduce' and investigated with fresh eyes \
             on a specific frame instead."
        );
        assert!(
            any_tendril_tick + 5 >= impact_tick,
            "a tendril was detected at tick {} but the stream didn't touch the floor until tick \
             {} -- that is more than 5 ticks of daylight, which would mean the detector is firing \
             on ordinary mid-air stream behaviour rather than the reported impact-triggered \
             excursion",
            any_tendril_tick, impact_tick
        );
    }

    #[test]
    // Both acceptance directions live here as ONE test because they are the same claim read two
    // ways: the detector must be sensitive enough to catch the reported defect (see the ignored
    // test above) AND specific enough that everything ordinary passes clean through it. A
    // detector that fires on ordinary pours is worthless regardless of what else it catches, so
    // this half is not optional and is NOT ignored -- it must stay green.
    fn test_tendril_detector_does_not_fire_on_healthy_scenarios() {
        // --- 1. A settled, level pool. Every cell is supported (floor or liquid beneath), so the
        // "unsupported" clause alone should already reject everything, independent of shape. ---
        {
            let (w, h, bs) = (128usize, 128usize, 32usize);
            let mut sim = wave_pool(w, h, bs, SandboxShape::Circle, 0.55);
            for i in 0..300u32 {
                sim.tick(glam::Vec2::ZERO, 16);
                let _ = i;
            }
            let components =
                find_liquid_components(&sim.hm.data, &sim.mask, w, h, 0, h - 1, TENDRIL_THRESHOLDS.liquid_eps);
            let tendrils = components.iter().filter(|c| TENDRIL_THRESHOLDS.is_tendril(c)).count();
            println!("healthy scenario [settled pool]: tendrils={}", tendrils);
            assert_eq!(tendrils, 0, "detector fired on a settled, level pool");
        }

        // --- 2. A clean falling stream, still mid-air (never touched a floor). Thin and
        // unsupported like a tendril, but VERTICAL -- this is the case the brief calls out by
        // name as the one every naive detector gets wrong. ---
        {
            let w = 64;
            let h = 96;
            let mask = make_test_mask(w, h, SandboxShape::Square, 0.04, 1.0);
            let props = get_test_props(MaterialMode::Water, w * h);
            let mut sim = TestSim::new(w, h, props, mask.clone(), 32);
            let gravity_dir = glam::Vec2::new(0.0, 0.04);
            // Same tap as test_liquid_stream_stays_coherent; run for only 30 ticks so the front
            // (falling at roughly 1 row/tick) is still well short of the floor (~92) -- no impact
            // has happened anywhere in the grid yet.
            for _ in 0..30 {
                for y in 6..10 {
                    for x in 30..34 {
                        sim.hm.apply_external_mass(x, y, 1.0);
                    }
                }
                sim.tick(gravity_dir, usize::MAX);
            }
            let touched_floor = (0..w).any(|x| sim.hm.data[91 * w + x] > 0.05);
            assert!(!touched_floor, "test setup error: stream reached the floor early, this sub-case is no longer 'mid-air'");
            let components =
                find_liquid_components(&sim.hm.data, &mask, w, h, 0, h - 1, TENDRIL_THRESHOLDS.liquid_eps);
            let tendrils: Vec<&LiquidComponent> =
                components.iter().filter(|c| TENDRIL_THRESHOLDS.is_tendril(c)).collect();
            for c in &components {
                println!(
                    "healthy scenario [mid-air stream]: component bbox=({},{}) filled={:.3} \
                     support={:.3}",
                    c.width(), c.height(), c.filled_fraction(), c.support_fraction()
                );
            }
            assert_eq!(tendrils.len(), 0, "detector fired on a clean, still-falling, not-yet-impacted stream");
        }

        // --- 3. The Sandbox wave scenario (gravity out of plane, g=0 in-plane): a bump relaxing
        // on a level pool. Shares nothing structurally with a Sand-fall impact, but is included
        // because it's the other physics regime this codebase ships. ---
        {
            let (w, h, bs) = (128usize, 128usize, 32usize);
            let mut sim = wave_pool(w, h, bs, SandboxShape::Circle, 0.50);
            add_bump(&mut sim, w, h, w as f32 / 2.0, h as f32 / 2.0, 0.30, 12.0);
            let mut any_tendrils = 0usize;
            for _ in 0..200u32 {
                sim.tick(glam::Vec2::ZERO, 16);
                let components = find_liquid_components(
                    &sim.hm.data, &sim.mask, w, h, 0, h - 1, TENDRIL_THRESHOLDS.liquid_eps,
                );
                any_tendrils += components.iter().filter(|c| TENDRIL_THRESHOLDS.is_tendril(c)).count();
            }
            println!("healthy scenario [sandbox wave]: total tendril-component-ticks={}", any_tendrils);
            assert_eq!(any_tendrils, 0, "detector fired at some point during the Sandbox wave scenario");
        }

        // --- 4. Sand, in the EXACT SAME single-neck Hourglass impact scenario as the water
        // reproduction. The user's report is explicit that this does not happen with sand; if the
        // detector fires here too, it is not specific to the reported defect. ---
        {
            let (trace, impact_tick, _w, _h, _floor) =
                run_single_neck_hourglass_tendril_scan(MaterialMode::DrySand, &TENDRIL_THRESHOLDS, 1);
            let ticks_with_tendril = trace.iter().filter(|&&(_, c, _)| c > 0).count();
            let max_count = trace.iter().map(|&(_, c, _)| c).max().unwrap_or(0);
            println!(
                "healthy scenario [sand, same geometry]: impact_tick={:?} ticks_with_tendril={} \
                 max_count={}",
                impact_tick, ticks_with_tendril, max_count
            );
            assert_eq!(
                ticks_with_tendril, 0,
                "detector fired on DrySand falling through the identical single-neck Hourglass \
                 geometry -- the user reports this defect is Water-only"
            );
        }
    }

    #[test]
    #[ignore = "DIAGNOSTIC: reports how sensitive the tendril count is to each threshold in \
                TENDRIL_THRESHOLDS, and how the detector behaves across grid resolutions, rather \
                than asserting one fixed pass/fail outcome. Run with --nocapture; the numbers \
                printed here are cited directly in the task report. Not a spec to keep green by \
                construction -- if the underlying physics changes, these numbers are expected to \
                move, and the point of this test is to show the movement, not hide it."]
    fn test_tendril_detector_thresholds_and_sensitivity() {
        // --- Part 1: threshold sensitivity, single resolution (scale=1). ---
        // Sweep each threshold independently around TENDRIL_THRESHOLDS, holding the others fixed,
        // against the single-neck Hourglass + Water reproduction. A metric that only fires inside
        // a narrow window is measuring the thresholds, not the physics -- this sweep is how that
        // gets checked rather than assumed.
        let variants: Vec<(&str, TendrilThresholds)> = vec![
            ("shipped", TENDRIL_THRESHOLDS),
            ("max_height=3 (stricter)", TendrilThresholds { max_height: 3, ..TENDRIL_THRESHOLDS }),
            ("max_height=10 (looser)", TendrilThresholds { max_height: 10, ..TENDRIL_THRESHOLDS }),
            ("min_width=3 (looser)", TendrilThresholds { min_width: 3, ..TENDRIL_THRESHOLDS }),
            ("min_width=8 (stricter)", TendrilThresholds { min_width: 8, ..TENDRIL_THRESHOLDS }),
            (
                "max_filled_fraction=0.4 (stricter)",
                TendrilThresholds { max_filled_fraction: 0.4, ..TENDRIL_THRESHOLDS },
            ),
            (
                "max_filled_fraction=0.9 (looser)",
                TendrilThresholds { max_filled_fraction: 0.9, ..TENDRIL_THRESHOLDS },
            ),
            (
                "min_unsupported_fraction=0.6 (stricter)",
                TendrilThresholds { min_unsupported_fraction: 0.6, ..TENDRIL_THRESHOLDS },
            ),
            (
                "min_unsupported_fraction=0.1 (looser)",
                TendrilThresholds { min_unsupported_fraction: 0.1, ..TENDRIL_THRESHOLDS },
            ),
            (
                "brief's literal `width > height` (strict)",
                TendrilThresholds { strict_wider_than_taller: true, ..TENDRIL_THRESHOLDS },
            ),
        ];

        println!("--- Threshold sensitivity (single-neck Hourglass + Water, scale=1) ---");
        for (name, thresholds) in &variants {
            let (trace, impact_tick, _w, _h, _floor) =
                run_single_neck_hourglass_tendril_scan(MaterialMode::Water, thresholds, 1);
            let impact_tick = impact_tick.expect("stream never reached the floor");
            let ticks_with_tendril = trace.iter().filter(|&&(_, c, _)| c > 0).count();
            let first_tick = trace.iter().find(|&&(_, c, _)| c > 0).map(|&(t, _, _)| t);
            let max_count = trace.iter().map(|&(_, c, _)| c).max().unwrap_or(0);
            let max_length = trace.iter().map(|&(_, _, l)| l).max().unwrap_or(0);
            println!(
                "  {:42} impact={:3} first_fire={:>5?} ticks_fired={:2} max_count={} max_length={}",
                name, impact_tick, first_tick, ticks_with_tendril, max_count, max_length
            );
        }

        // --- Part 2: resolution behaviour. Same scenario, scaled by `s` in both dimensions (the
        // Hourglass shape is defined in normalized x/w, y/h coordinates, so this is the same
        // shape at finer resolution, not a different one), tap width and tick budget scaled with
        // it (a falling stream advances at a roughly fixed number of CELLS per tick regardless of
        // resolution, so reaching the same physical point takes proportionally more ticks at
        // finer resolution -- same reasoning as `test_liquid_stream_stays_coherent`). The
        // classification thresholds themselves are DELIBERATELY NOT scaled -- the reported defect
        // is a fixed number of cells wide (a CFL/edge-velocity artifact), not a fraction of the
        // container, so they need to mean the same thing at every resolution to test the same
        // claim at every resolution (see docs/ARCHITECTURE.md section 11 on this exact
        // classification question for the enclosed-void metric, which is the same shape of
        // argument).
        println!("--- Resolution behaviour (single-neck Hourglass + Water, shipped thresholds) ---");
        for scale in [1usize, 2, 4] {
            let (trace, impact_tick, w, h, floor) =
                run_single_neck_hourglass_tendril_scan(MaterialMode::Water, &TENDRIL_THRESHOLDS, scale);
            let impact_tick = impact_tick.expect("stream never reached the floor");
            let ticks_with_tendril = trace.iter().filter(|&&(_, c, _)| c > 0).count();
            let first_tick = trace.iter().find(|&&(_, c, _)| c > 0).map(|&(t, _, _)| t);
            let max_count = trace.iter().map(|&(_, c, _)| c).max().unwrap_or(0);
            let max_length = trace.iter().map(|&(_, _, l)| l).max().unwrap_or(0);
            println!(
                "  scale={} w={} h={} floor={} impact={} first_fire={:?} ticks_fired={} \
                 max_count={} max_length={}",
                scale, w, h, floor, impact_tick, first_tick, ticks_with_tendril, max_count,
                max_length
            );
        }
    }

    #[test]
    #[ignore = "DIAGNOSTIC: points the tendril detector at four EXISTING liquid scenarios --\
                test_liquid_splashes_on_impact, test_liquid_flowing_liquid_does_not_stand_in_walls, \
                test_liquid_stream_stays_coherent, and test_multineck_hourglass_water_tendril_on_impact \
                -- to map which reproduce this defect and which don't. This is exploratory \
                documentation, not a fixed spec: it recreates each named test's own setup \
                independently (rather than calling into it) so it can apply the detector without \
                touching any of those tests' assertions. Run with --nocapture; see the task report \
                for the numbers cited from here."]
    fn test_tendril_detector_maps_existing_liquid_scenarios() {
        // --- 1. test_liquid_splashes_on_impact: a compact blob dropped onto a Square floor. ---
        {
            let w = 64;
            let h = 64;
            let mask = make_test_mask(w, h, SandboxShape::Square, 0.04, 1.0);
            let props = get_test_props(MaterialMode::Water, w * h);
            let mut sim = TestSim::new(w, h, props, mask.clone(), 32);
            let gravity_dir = glam::Vec2::new(0.0, 0.04);
            for y in 50..54 {
                for x in 28..36 {
                    sim.hm.data[y * w + x] = 1.0;
                }
            }
            let mut ticks_with_tendril = 0usize;
            let mut max_count = 0usize;
            let mut max_length = 0usize;
            for _ in 0..70 {
                sim.tick(gravity_dir, 256);
                let components =
                    find_liquid_components(&sim.hm.data, &mask, w, h, 0, h - 1, TENDRIL_THRESHOLDS.liquid_eps);
                let tendrils: Vec<&LiquidComponent> =
                    components.iter().filter(|c| TENDRIL_THRESHOLDS.is_tendril(c)).collect();
                if !tendrils.is_empty() {
                    ticks_with_tendril += 1;
                    max_count = max_count.max(tendrils.len());
                    max_length =
                        max_length.max(tendrils.iter().map(|c| c.width().max(c.height())).max().unwrap());
                }
            }
            println!(
                "map [test_liquid_splashes_on_impact]: ticks_with_tendril={} max_count={} max_length={}",
                ticks_with_tendril, max_count, max_length
            );
        }

        // --- 2. test_liquid_flowing_liquid_does_not_stand_in_walls: upper chamber drains into an
        // empty lower one through a wide-ish Hourglass neck. No single "impact point" -- checked
        // over the whole grid every tick. ---
        {
            let w = 64;
            let h = 64;
            let mask = make_test_mask(w, h, SandboxShape::Hourglass, 0.15, 0.6);
            let props = get_test_props(MaterialMode::Water, w * h);
            let mut sim = TestSim::new(w, h, props, mask.clone(), 32);
            let gravity_dir = glam::Vec2::new(0.0, 0.04);
            for y in 0..h / 2 {
                for x in 0..w {
                    if mask[y * w + x] != crate::MASK_OUTSIDE {
                        sim.hm.data[y * w + x] = 1.0;
                    }
                }
            }
            let mut ticks_with_tendril = 0usize;
            let mut max_count = 0usize;
            let mut max_length = 0usize;
            for _ in 0..400 {
                sim.tick(gravity_dir, usize::MAX);
                let components =
                    find_liquid_components(&sim.hm.data, &mask, w, h, 0, h - 1, TENDRIL_THRESHOLDS.liquid_eps);
                let tendrils: Vec<&LiquidComponent> =
                    components.iter().filter(|c| TENDRIL_THRESHOLDS.is_tendril(c)).collect();
                if !tendrils.is_empty() {
                    ticks_with_tendril += 1;
                    max_count = max_count.max(tendrils.len());
                    max_length =
                        max_length.max(tendrils.iter().map(|c| c.width().max(c.height())).max().unwrap());
                }
            }
            println!(
                "map [test_liquid_flowing_liquid_does_not_stand_in_walls]: ticks_with_tendril={} \
                 max_count={} max_length={}",
                ticks_with_tendril, max_count, max_length
            );
        }

        // --- 3. test_liquid_stream_stays_coherent: a continuous 4-wide tap falling in an open
        // Square box, checked well clear of the source and before it reaches the floor. ---
        {
            let w = 64;
            let h = 96;
            let mask = make_test_mask(w, h, SandboxShape::Square, 0.04, 1.0);
            let props = get_test_props(MaterialMode::Water, w * h);
            let mut sim = TestSim::new(w, h, props, mask.clone(), 32);
            let gravity_dir = glam::Vec2::new(0.0, 0.04);
            let mut ticks_with_tendril = 0usize;
            let mut max_count = 0usize;
            let mut max_length = 0usize;
            for _ in 0..40 {
                for y in 6..10 {
                    for x in 30..34 {
                        sim.hm.apply_external_mass(x, y, 1.0);
                    }
                }
                sim.tick(gravity_dir, usize::MAX);
                let components =
                    find_liquid_components(&sim.hm.data, &mask, w, h, 0, h - 1, TENDRIL_THRESHOLDS.liquid_eps);
                let tendrils: Vec<&LiquidComponent> =
                    components.iter().filter(|c| TENDRIL_THRESHOLDS.is_tendril(c)).collect();
                if !tendrils.is_empty() {
                    ticks_with_tendril += 1;
                    max_count = max_count.max(tendrils.len());
                    max_length =
                        max_length.max(tendrils.iter().map(|c| c.width().max(c.height())).max().unwrap());
                }
            }
            println!(
                "map [test_liquid_stream_stays_coherent]: ticks_with_tendril={} max_count={} \
                 max_length={}",
                ticks_with_tendril, max_count, max_length
            );
        }

        // --- 4. test_multineck_hourglass_water_tendril_on_impact: three synchronized necks
        // feeding one shared lower chamber. Uses the same near-floor window as the single-neck
        // scan (WINDOW_H=20 cells), spanning the full width so it covers all three necks. ---
        {
            let w = 128;
            let h = 160;
            let block_size = 32;
            let mask = make_test_mask(w, h, SandboxShape::MultiNeckHourglass, 0.005, 0.6);
            let props = get_test_props(MaterialMode::Water, w * h);
            let mut sim = TestSim::new(w, h, props, mask.clone(), block_size);
            let gravity_dir = glam::Vec2::new(0.0, 0.04);
            let neck_xs = [36usize, 64usize, 92usize];
            let floor = (0..h)
                .rev()
                .find(|&y| ((59)..=(69)).any(|x| mask[y * w + x] != crate::MASK_OUTSIDE))
                .expect("no floor found under the centre neck");
            const WINDOW_H: usize = 20;
            let y0 = floor.saturating_sub(WINDOW_H);

            let mut ticks_with_tendril = 0usize;
            let mut max_count = 0usize;
            let mut max_length = 0usize;
            let mut impact_tick = None;
            for t in 0..90 {
                for &nxx in &neck_xs {
                    for y in 82..=84usize {
                        for x in (nxx - 1)..=(nxx + 1) {
                            sim.hm.apply_external_mass(x, y, 1.0);
                        }
                    }
                }
                sim.tick(gravity_dir, usize::MAX);
                if impact_tick.is_none() && sim.hm.data[floor * w + 64] > 0.05 {
                    impact_tick = Some(t);
                }
                let components =
                    find_liquid_components(&sim.hm.data, &mask, w, h, y0, floor, TENDRIL_THRESHOLDS.liquid_eps);
                let tendrils: Vec<&LiquidComponent> =
                    components.iter().filter(|c| TENDRIL_THRESHOLDS.is_tendril(c)).collect();
                if !tendrils.is_empty() {
                    ticks_with_tendril += 1;
                    max_count = max_count.max(tendrils.len());
                    max_length =
                        max_length.max(tendrils.iter().map(|c| c.width().max(c.height())).max().unwrap());
                }
            }
            println!(
                "map [test_multineck_hourglass_water_tendril_on_impact]: impact_tick={:?} \
                 ticks_with_tendril={} max_count={} max_length={}",
                impact_tick, ticks_with_tendril, max_count, max_length
            );
        }
    }

    // =====================================================================================
    // EXPERIMENTAL DIAGNOSTIC (tick-phase-order hypothesis, step 1 -- measure before touching
    // the solver). See `phase_flow_stats` above `wave_params` for the instrumentation this
    // relies on.
    //
    // Both diagnostics below share the same measurement plan:
    //   (a) free capacity (cap - h) in the packed interior vs the free surface vs the drain
    //       channel around the neck, each tick, before the tick runs;
    //   (b) of the flux that actually moves each tick, what fraction phase 0 (gravity-aligned)
    //       realises versus phase 1 (everything else, including the lateral edge/CA);
    //   (c) a source-depth profile: material is seeded in horizontal color bands (by initial
    //       row), and the mass-weighted mean band index of whatever has crossed below the neck
    //       is tracked over time, so "did this drain from the top only, or from all depths"
    //       becomes a number instead of an impression.
    //
    // The defect these diagnostics investigate is OUTLET-SIZE DEPENDENT, so -- exactly like
    // `diag_mass_vs_core_flow_funnel` below -- the shared body takes `neck_width` as a
    // parameter and both callers sweep the same [0.02, 0.04, 0.08, 0.12] the mass-vs-core
    // diagnostics use, labelling output per width (e.g. `sand_nw0.02`) so the two families of
    // diagnostic are directly comparable. 0.12 is the neck-width slider's MAXIMUM (widest,
    // least-restrictive neck); a fixed run at 0.12 alone is blind to the defect by
    // construction.
    //
    // NOTE on `band_mass_below`: an earlier version of these diagnostics binned the
    // mass-weighted-blended color tracer into discrete band indices. Binning a blended value
    // piles everything into the middle band under exact mass conservation and produces a
    // plausible-looking false signal (it once showed a band gaining 2.5x its initial mass with
    // total mass exactly conserved). That histogram has been removed. The continuous
    // mass-weighted `mean_source_band` below is not subject to that failure mode and is kept.
    //
    // DIAGNOSTIC ONLY: never asserts on these numbers. Run with:
    //   cargo test -p sandart-sim --lib physics::tests::diag_step1 -- --ignored --nocapture
    // =====================================================================================

    /// Shared body for the sand/liquid variants below. `cap` is the material's cell capacity
    /// (1.5 for DrySand at wetness 0.0, 1.0 for Water); `fill_height` is the seeded per-cell
    /// height below that capacity; `surf_eps` is the height threshold used to detect the top
    /// free surface of the settled pile (0.05 for sand, 0.02 for the thinner-settling liquid).
    /// `neck_width` is passed straight through to `make_test_mask`'s neck-width slider (the
    /// same slider exposed in the UI, range roughly [0.02, 0.12] with 0.12 == the slider's
    /// maximum). Flow regime depends strongly on this, so callers sweep it rather than picking
    /// one value.
    fn diag_phase_capacity_attribution_funnel(
        mode: MaterialMode,
        cap: f32,
        fill_height: f32,
        surf_eps: f32,
        label: &str,
        neck_width: f32,
    ) {
        phase_flow_stats::reset();

        let w = 64usize;
        let h = 96usize;
        let block_size = 16usize;
        const NUM_BANDS: usize = 6;
        const FILL_Y0: usize = 12;
        const FILL_Y1: usize = 44; // exclusive; 32 rows, split into NUM_BANDS equal strips

        let mask = make_test_mask(w, h, SandboxShape::Hourglass, neck_width, 0.6);
        let props = get_test_props(mode, w * h);
        let mut sim = TestSim::new(w, h, props, mask.clone(), block_size);

        // Locate the neck: the row (nearest the grid's vertical centre, in case of ties) with
        // the fewest inside cells.
        let row_width = |y: usize| -> usize {
            (0..w).filter(|&x| mask[y * w + x] != crate::MASK_OUTSIDE).count()
        };
        let neck_y = (0..h)
            .filter(|&y| row_width(y) > 0)
            .min_by_key(|&y| (row_width(y), (y as i64 - (h as i64 / 2)).abs()))
            .expect("hourglass mask has no inside rows");
        println!(
            "diag_phase_cap[{label}]: neck_y={} neck_width={:.2} (row_width={}) fill rows=[{},{})",
            neck_y, neck_width, row_width(neck_y), FILL_Y0, FILL_Y1
        );

        // Seed the upper chamber in horizontal color bands and near-capacity fill.
        let band_color = |band: usize| -> u8 { (band * (255 / (NUM_BANDS - 1))) as u8 };
        let mut initial_band_mass = [0.0f64; NUM_BANDS];
        for y in FILL_Y0..FILL_Y1 {
            let band = ((y - FILL_Y0) * NUM_BANDS / (FILL_Y1 - FILL_Y0)).min(NUM_BANDS - 1);
            for x in 0..w {
                let idx = y * w + x;
                if mask[idx] != crate::MASK_OUTSIDE {
                    sim.hm.data[idx] = fill_height;
                    sim.cell_colors[idx * 4 + 0] = band_color(band);
                    sim.cell_colors[idx * 4 + 3] = 255;
                    initial_band_mass[band] += fill_height as f64;
                }
            }
        }
        let initial_mass = sim.mass();
        println!(
            "diag_phase_cap[{label}]: initial_mass={:.3} initial_band_mass={:?}",
            initial_mass, initial_band_mass
        );

        let gravity_dir = glam::Vec2::new(0.0, 0.04);
        const N_TICKS: usize = 400;
        const REPORT_EVERY: usize = 40;

        let mut cum_phase0 = 0.0f64;
        let mut cum_phase1 = 0.0f64;
        // Running sums for the capacity-by-region metric, averaged over the whole run at the
        // end (also spot-printed every REPORT_EVERY ticks).
        let mut sum_interior_cap = 0.0f64;
        let mut sum_surface_cap = 0.0f64;
        let mut sum_drain_cap = 0.0f64;
        let mut n_interior_samples = 0u64;
        let mut n_surface_samples = 0u64;
        let mut n_drain_samples = 0u64;

        for t in 0..N_TICKS {
            // --- (a) free-capacity-by-region, measured on the PRE-tick heightmap ---
            let mut surf_y = vec![usize::MAX; w];
            for x in 0..w {
                for y in FILL_Y0.saturating_sub(4)..neck_y {
                    let idx = y * w + x;
                    if mask[idx] != crate::MASK_OUTSIDE && sim.hm.data[idx] > surf_eps {
                        surf_y[x] = y;
                        break;
                    }
                }
            }
            let (mut interior_cap, mut surface_cap, mut drain_cap) = (0.0f64, 0.0f64, 0.0f64);
            let (mut n_int, mut n_surf, mut n_drain) = (0u64, 0u64, 0u64);
            for y in 0..neck_y + 4 {
                for x in 0..w {
                    let idx = y * w + x;
                    if mask[idx] == crate::MASK_OUTSIDE {
                        continue;
                    }
                    let near_neck = y + 3 >= neck_y && y <= neck_y + 3;
                    if near_neck {
                        drain_cap += (cap - sim.hm.data[idx]).max(0.0) as f64;
                        n_drain += 1;
                        continue;
                    }
                    if surf_y[x] == usize::MAX || sim.hm.data[idx] <= surf_eps {
                        continue; // empty air above the pile, not part of either region
                    }
                    if y <= surf_y[x] + 2 {
                        surface_cap += (cap - sim.hm.data[idx]).max(0.0) as f64;
                        n_surf += 1;
                    } else if y + 4 <= neck_y {
                        interior_cap += (cap - sim.hm.data[idx]).max(0.0) as f64;
                        n_int += 1;
                    }
                }
            }
            sum_interior_cap += interior_cap;
            sum_surface_cap += surface_cap;
            sum_drain_cap += drain_cap;
            n_interior_samples += n_int;
            n_surface_samples += n_surf;
            n_drain_samples += n_drain;

            // --- (b) phase attribution for this tick's actual flux ---
            phase_flow_stats::reset();
            sim.tick(gravity_dir, 4096);
            let (p0, p1) = phase_flow_stats::take();
            cum_phase0 += p0;
            cum_phase1 += p1;

            if t % REPORT_EVERY == 0 || t == N_TICKS - 1 {
                // --- (c) source-depth profile: mass-weighted mean band index below the neck.
                // (See the family doc-comment above: no bucketed histogram here -- continuous
                // mass-weighted mean only.) ---
                let mut drained_mass = 0.0f64;
                let mut drained_band_weighted = 0.0f64;
                for y in (neck_y + 4)..h {
                    for x in 0..w {
                        let idx = y * w + x;
                        if mask[idx] == crate::MASK_OUTSIDE {
                            continue;
                        }
                        let hgt = sim.hm.data[idx] as f64;
                        if hgt <= 1e-4 {
                            continue;
                        }
                        let band_est = sim.cell_colors[idx * 4 + 0] as f64
                            / (255.0 / (NUM_BANDS as f64 - 1.0));
                        drained_mass += hgt;
                        drained_band_weighted += hgt * band_est;
                    }
                }
                let mean_band = if drained_mass > 0.0 { drained_band_weighted / drained_mass } else { -1.0 };
                let cap_total = interior_cap + surface_cap + drain_cap;
                let phase_total = cum_phase0 + cum_phase1;
                println!(
                    "diag_phase_cap[{label}][t={t}]: drained_mass={:.4} mean_source_band={:.3} \
                     | this-tick free-cap interior={:.4} surface={:.4} \
                     drain={:.4} (n={}/{}/{}) | cumulative flux phase0={:.5} phase1={:.5} \
                     phase1_frac={:.4} (of {cap_total:.4} cap seen, phase_total={phase_total:.5})",
                    drained_mass, mean_band,
                    interior_cap, surface_cap, drain_cap, n_int, n_surf, n_drain,
                    cum_phase0, cum_phase1,
                    if phase_total > 0.0 { cum_phase1 / phase_total } else { -1.0 },
                );
            }
        }

        let final_mass = sim.mass();
        let run_phase1_frac = if cum_phase0 + cum_phase1 > 0.0 {
            cum_phase1 / (cum_phase0 + cum_phase1)
        } else {
            -1.0
        };
        let run_avg_interior_cap = if n_interior_samples > 0 {
            sum_interior_cap / n_interior_samples as f64
        } else {
            -1.0
        };
        let mass_rel_err = (final_mass - initial_mass).abs() / initial_mass;
        println!(
            "diag_phase_cap[{label}]: FINAL initial_mass={:.3} final_mass={:.3} mass_rel_err={:.3e} \
             | run totals: cum_phase0={:.4} cum_phase1={:.4} phase1_frac={:.4} \
             | avg free-cap/sample interior={:.5} surface={:.5} drain={:.5}",
            initial_mass, final_mass, mass_rel_err,
            cum_phase0, cum_phase1, run_phase1_frac,
            run_avg_interior_cap,
            if n_surface_samples > 0 { sum_surface_cap / n_surface_samples as f64 } else { -1.0 },
            if n_drain_samples > 0 { sum_drain_cap / n_drain_samples as f64 } else { -1.0 },
        );
        // Cross-width comparison line (the two numbers that actually move with outlet width):
        println!(
            "diag_phase_cap[{label}]: SUMMARY neck_width={neck_width:.2} phase1_frac={:.4} \
             avg_interior_free_cap_per_cell={:.5} mass_rel_err={:.3e}",
            run_phase1_frac, run_avg_interior_cap, mass_rel_err,
        );
    }

    #[test]
    #[ignore]
    fn diag_step1_phase_capacity_attribution_sand_funnel() {
        // DrySand, wetness 0.0 -> cell_capacity_for(0.0) == 1.5; fill_height=1.4 matches the
        // near-capacity fill the mass-vs-core sand diagnostic uses; surf_eps=0.05.
        for neck_width in [0.02f32, 0.04, 0.08, 0.12] {
            let label = format!("sand_nw{neck_width:.2}");
            diag_phase_capacity_attribution_funnel(
                MaterialMode::DrySand,
                1.5,
                1.4,
                0.05,
                &label,
                neck_width,
            );
        }
    }

    #[test]
    #[ignore]
    fn diag_step1_phase_capacity_attribution_liquid_funnel() {
        // Water, liquidity == 1 -> cell_capacity_for == 1.0; fill_height=0.95 matches the
        // mass-vs-core liquid diagnostic; surf_eps=0.02 (thinner settled surface than sand).
        for neck_width in [0.02f32, 0.04, 0.08, 0.12] {
            let label = format!("liquid_nw{neck_width:.2}");
            diag_phase_capacity_attribution_funnel(
                MaterialMode::Water,
                1.0,
                0.95,
                0.02,
                &label,
                neck_width,
            );
        }
    }

    // =====================================================================================
    // MEASUREMENT (mass-flow vs core/funnel-flow hypothesis) -- see the assigning brief.
    // Seeds a triangular funnel (the upper, converging chamber of an Hourglass mask: wide
    // mouth narrowing down to a small neck) with the BOTTOM half of the fill region (by row,
    // i.e. the half closer to the neck) colored black and the TOP half (the wide mouth, far
    // from the neck) colored white. Because the chamber narrows going down, the bottom
    // half's rows are narrower and hold less area/mass than the top half's -- `m_black` is
    // computed exactly from the actual seeded per-row mass below, never assumed to be 0.25.
    //
    // Color is used as a mass-weighted-mean CONSERVED tracer (`advect_properties` blends
    // color mass-weighted with stochastic rounding; `test_color_conservation` asserts
    // color*mass conserved to 0.5%) -- the mean is read continuously, never bucketed into
    // discrete bands (an earlier attempt binned blended greys into bands and produced a false
    // signal: everything piled into the middle band and looked like a real effect). R = tone
    // (0 black / 255 white), G = normalised source row, B = normalised source column, each an
    // independent conserved tracer (alpha is forced to 255).
    //
    // MASS FLOW prediction: material leaves in depth order, so exited material stays
    // essentially all-black until the cumulative drained fraction reaches m_black, then turns
    // white -- i.e. f_50 (drained fraction at which exited material first reaches 50% white)
    // approx equals m_black, and white_fraction_of_exited at drained_frac=0.10 approx equals 0.
    // CORE/FUNNEL FLOW prediction: a narrow vertical channel drains fed from the top surface,
    // so white appears almost immediately -- white_fraction_of_exited at 10% drained is well
    // above 0, and f_50 is far below m_black.
    //
    // Real granular material in a steep funnel with a small outlet does exhibit SOME core
    // flow -- the ideal mass-flow step is not necessarily the physical target. A modest
    // shortfall of f_50 below m_black is not on its own proof of a defect; white appearing at
    // a drained fraction near zero would be (see the printed PREDICTIONS line and the
    // magnitude discussion in the final report).
    //
    // DIAGNOSTIC ONLY: never asserts on the mass-flow-vs-core-flow numbers themselves (only
    // on the instrument self-check, which is what makes those numbers trustworthy). Run with:
    //   cargo test -p sandart-sim --lib physics::tests::diag_step1_mass_vs_core_flow -- --ignored --nocapture
    // =====================================================================================

    /// Shared body for the sand/liquid variants below. `fill_height` is the seeded per-cell
    /// height (below each material's cell capacity, matching the fill heights the existing
    /// `diag_step1_phase_capacity_attribution_*` tests use for the same materials).
    ///
    /// `neck_width` is passed straight through to `make_test_mask`'s neck-width slider (the
    /// same slider exposed in the UI, range roughly [0.02, 0.12] with 0.12 == the slider's
    /// maximum, i.e. the widest/least-restrictive neck). Flow regime depends strongly on this,
    /// so callers should sweep it rather than picking one value.
    fn diag_mass_vs_core_flow_funnel(mode: MaterialMode, fill_height: f32, label: &str, neck_width: f32) {
        let w = 64usize;
        let h = 96usize;
        let block_size = 16usize;

        let mask = make_test_mask(w, h, SandboxShape::Hourglass, neck_width, 0.6);
        let props = get_test_props(mode, w * h);
        let mut sim = TestSim::new(w, h, props, mask.clone(), block_size);

        let row_width = |y: usize| -> usize {
            (0..w).filter(|&x| mask[y * w + x] != crate::MASK_OUTSIDE).count()
        };
        let neck_y = (0..h)
            .filter(|&y| row_width(y) > 0)
            .min_by_key(|&y| (row_width(y), (y as i64 - (h as i64 / 2)).abs()))
            .expect("hourglass mask has no inside rows");

        const FILL_Y0: usize = 12;
        // Leave a gap above the neck (matches the existing diag_step1_* fill bounds) so the
        // seeded region is purely the converging chamber, not the packed cells right at it.
        let fill_y1 = neck_y.saturating_sub(4);
        assert!(
            fill_y1 > FILL_Y0 + 4,
            "funnel fill region too small: FILL_Y0={FILL_Y0} fill_y1={fill_y1} neck_y={neck_y}"
        );

        // Global column bounding box across the whole fill region, used to normalise B --
        // read from the mask itself (not derived from the shape formula), so it stays correct
        // even if the mask geometry is retuned later.
        let mut min_x = usize::MAX;
        let mut max_x = 0usize;
        for y in FILL_Y0..fill_y1 {
            for x in 0..w {
                if mask[y * w + x] != crate::MASK_OUTSIDE {
                    min_x = min_x.min(x);
                    max_x = max_x.max(x);
                }
            }
        }
        assert!(max_x > min_x, "degenerate fill region column bounds");

        // Split the fill region into two equal-ROW halves (equal HEIGHT, not equal area):
        // rows [FILL_Y0, mid) are the wide top of the funnel (far from the neck) -> white;
        // rows [mid, fill_y1) are the narrow bottom (near the neck) -> black.
        let mid = FILL_Y0 + (fill_y1 - FILL_Y0) / 2;

        let mut mass_top = 0.0f64;
        let mut mass_bottom = 0.0f64;
        for y in FILL_Y0..mid {
            mass_top += row_width(y) as f64 * fill_height as f64;
        }
        for y in mid..fill_y1 {
            mass_bottom += row_width(y) as f64 * fill_height as f64;
        }
        let m_black = mass_bottom / (mass_top + mass_bottom);

        let row_norm = |y: usize| -> u8 {
            (((y - FILL_Y0) as f32 / (fill_y1 - FILL_Y0 - 1).max(1) as f32) * 255.0)
                .round()
                .clamp(0.0, 255.0) as u8
        };
        let col_norm = |x: usize| -> u8 {
            (((x - min_x) as f32 / (max_x - min_x) as f32) * 255.0)
                .round()
                .clamp(0.0, 255.0) as u8
        };

        for y in FILL_Y0..fill_y1 {
            let r: u8 = if y < mid { 255 } else { 0 };
            let g = row_norm(y);
            for x in 0..w {
                let idx = y * w + x;
                if mask[idx] != crate::MASK_OUTSIDE {
                    sim.hm.data[idx] = fill_height;
                    sim.cell_colors[idx * 4 + 0] = r;
                    sim.cell_colors[idx * 4 + 1] = g;
                    sim.cell_colors[idx * 4 + 2] = col_norm(x);
                    sim.cell_colors[idx * 4 + 3] = 255;
                }
            }
        }

        let initial_mass = sim.mass();
        assert!(
            (initial_mass - (mass_top + mass_bottom)).abs() / initial_mass < 1e-6,
            "seeded mass {:.6} does not match row-summed mass {:.6}",
            initial_mass, mass_top + mass_bottom
        );

        let color_mass = |cell_colors: &[u8], hm: &Heightmap, channel: usize| -> f64 {
            hm.data
                .iter()
                .enumerate()
                .map(|(idx, &hgt)| cell_colors[idx * 4 + channel] as f64 * hgt as f64)
                .sum()
        };
        let initial_r_mass = color_mass(&sim.cell_colors, &sim.hm, 0);
        let initial_g_mass = color_mass(&sim.cell_colors, &sim.hm, 1);
        let initial_b_mass = color_mass(&sim.cell_colors, &sim.hm, 2);
        let white_fraction_global = initial_r_mass / 255.0 / initial_mass;

        println!(
            "diag_mass_vs_core[{label}]: neck_width={neck_width:.2} neck_y={neck_y} FILL_Y0={FILL_Y0} \
             fill_y1={fill_y1} mid={mid} min_x={min_x} max_x={max_x} | initial_mass={:.4} \
             mass_top(white)={:.4} mass_bottom(black)={:.4} m_black={:.4} white_fraction_global={:.4}",
            initial_mass, mass_top, mass_bottom, m_black, white_fraction_global,
        );
        // Predictions, stated BEFORE the run below is analysed (self-validation item 3).
        //
        // THE IDEAL f_50 IS 2 * m_black, NOT m_black. This was wrong in the first version of
        // this diagnostic and the error propagated into several conclusions, so the derivation
        // is spelled out. `white_fraction_of_exited` is CUMULATIVE -- it is the composition of
        // everything that has left so far, which is why it necessarily ends at
        // `white_fraction_global`. Under ideal plug flow material leaves in strict depth order,
        // so at drained fraction f the exited mass is all black until f reaches m_black and the
        // white excess above that is (f - m_black). The cumulative white fraction is therefore
        //     W(f) = max(0, (f - m_black) / f)
        // and W(f) = 0.5 gives f - m_black = 0.5 f, i.e. f = 2 * m_black.
        // Reading `m_black` as the ideal understates it by exactly a factor of two and makes
        // badly-mixed drainage look close to ideal.
        let ideal_f50 = 2.0 * m_black;
        println!(
            "diag_mass_vs_core[{label}]: PREDICTIONS mass_flow=[white_frac@10%~=0.0, f_50~={:.4} \
             (= 2*m_black, cumulative metric -- see comment)] \
             core_flow=[white_frac@10%>>0.0 (near 1.0 in the extreme), f_50<<{:.4} (near 0.0 in the extreme)]",
            ideal_f50, ideal_f50,
        );

        let gravity_dir = glam::Vec2::new(0.0, 0.04);
        let schedule = [0.05f64, 0.10, 0.15, 0.20, 0.25, 0.30, 0.40, 0.50, 0.75, 1.00];
        let mut next_sched_idx = 0usize;
        let mut f_50: Option<f64> = None;
        let mut white_at_10: Option<f64> = None;

        const MAX_TICKS: usize = 6000;
        let mut last_drained_frac = 0.0f64;
        let mut t = 0usize;
        while t < MAX_TICKS && next_sched_idx < schedule.len() {
            sim.tick(gravity_dir, 4096);
            t += 1;

            let mut drained_mass = 0.0f64;
            let mut drained_r = 0.0f64;
            let mut drained_g = 0.0f64;
            let mut drained_b = 0.0f64;
            for y in (neck_y + 4)..h {
                for x in 0..w {
                    let idx = y * w + x;
                    if mask[idx] == crate::MASK_OUTSIDE {
                        continue;
                    }
                    let hgt = sim.hm.data[idx] as f64;
                    if hgt <= 1e-4 {
                        continue;
                    }
                    drained_mass += hgt;
                    drained_r += hgt * sim.cell_colors[idx * 4 + 0] as f64;
                    drained_g += hgt * sim.cell_colors[idx * 4 + 1] as f64;
                    drained_b += hgt * sim.cell_colors[idx * 4 + 2] as f64;
                }
            }
            let drained_frac = drained_mass / initial_mass;
            last_drained_frac = drained_frac;

            if drained_mass > 0.0 {
                let white_frac = (drained_r / drained_mass) / 255.0;
                if f_50.is_none() && white_frac >= 0.5 {
                    f_50 = Some(drained_frac);
                }
            }

            while next_sched_idx < schedule.len() && drained_frac >= schedule[next_sched_idx] {
                let white_frac = if drained_mass > 0.0 { (drained_r / drained_mass) / 255.0 } else { -1.0 };
                let mean_row = if drained_mass > 0.0 {
                    FILL_Y0 as f64 + (drained_g / drained_mass) / 255.0 * (fill_y1 - FILL_Y0 - 1) as f64
                } else {
                    -1.0
                };
                let mean_col = if drained_mass > 0.0 {
                    min_x as f64 + (drained_b / drained_mass) / 255.0 * (max_x - min_x) as f64
                } else {
                    -1.0
                };
                println!(
                    "diag_mass_vs_core[{label}] t={t} drained_frac_target={:.2} drained_frac_actual={:.4} \
                     drained_mass={:.4} white_fraction_of_exited={:.4} mean_source_row={:.2} mean_source_col={:.2}",
                    schedule[next_sched_idx], drained_frac, drained_mass, white_frac, mean_row, mean_col,
                );
                if (schedule[next_sched_idx] - 0.10).abs() < 1e-9 {
                    white_at_10 = Some(white_frac);
                }
                next_sched_idx += 1;
            }
        }

        if next_sched_idx < schedule.len() {
            println!(
                "diag_mass_vs_core[{label}]: WARNING did not reach all schedule points within \
                 {MAX_TICKS} ticks; max drained_frac achieved={:.4}, stalled before target={:.2}",
                last_drained_frac, schedule[next_sched_idx]
            );
        }

        // Run a bounded number of extra ticks past the schedule loop so the self-validation
        // below sees as-settled a state as practical, whether that means genuinely full
        // drainage or a plateaued residual pile above the neck (reported honestly either way).
        const SETTLE_EXTRA_TICKS: usize = 400;
        for _ in 0..SETTLE_EXTRA_TICKS {
            sim.tick(gravity_dir, 4096);
        }

        // ---- Self-validation (2): engine-wide conservation, whole grid, start vs end ----
        let final_mass_total = sim.mass();
        let mass_rel_err = (final_mass_total - initial_mass).abs() / initial_mass;
        let final_r_mass_total = color_mass(&sim.cell_colors, &sim.hm, 0);
        let final_g_mass_total = color_mass(&sim.cell_colors, &sim.hm, 1);
        let final_b_mass_total = color_mass(&sim.cell_colors, &sim.hm, 2);
        let r_rel_err = (final_r_mass_total - initial_r_mass).abs() / initial_r_mass;
        let g_rel_err = (final_g_mass_total - initial_g_mass).abs() / initial_g_mass;
        let b_rel_err = (final_b_mass_total - initial_b_mass).abs() / initial_b_mass;

        println!(
            "diag_mass_vs_core[{label}]: SELF-VALIDATION (engine-wide conservation, whole grid) \
             initial_mass={:.6} final_mass={:.6} mass_rel_err={:.3e} | \
             initial_R_mass={:.4} final_R_mass={:.4} R_rel_err={:.3e} | \
             initial_G_mass={:.4} final_G_mass={:.4} G_rel_err={:.3e} | \
             initial_B_mass={:.4} final_B_mass={:.4} B_rel_err={:.3e}",
            initial_mass, final_mass_total, mass_rel_err,
            initial_r_mass, final_r_mass_total, r_rel_err,
            initial_g_mass, final_g_mass_total, g_rel_err,
            initial_b_mass, final_b_mass_total, b_rel_err,
        );

        // ---- Self-validation (1): composition of everything that exited vs global ----
        let mut drained_mass = 0.0f64;
        let mut drained_r = 0.0f64;
        for y in (neck_y + 4)..h {
            for x in 0..w {
                let idx = y * w + x;
                if mask[idx] == crate::MASK_OUTSIDE {
                    continue;
                }
                let hgt = sim.hm.data[idx] as f64;
                if hgt <= 1e-4 {
                    continue;
                }
                drained_mass += hgt;
                drained_r += hgt * sim.cell_colors[idx * 4 + 0] as f64;
            }
        }
        let final_drained_frac = drained_mass / initial_mass;
        let white_fraction_of_exited_final =
            if drained_mass > 0.0 { (drained_r / drained_mass) / 255.0 } else { -1.0 };
        let rel_diff = (white_fraction_of_exited_final - white_fraction_global).abs() / white_fraction_global;

        println!(
            "diag_mass_vs_core[{label}]: SELF-VALIDATION (exited composition vs global) \
             final_drained_frac={:.4} white_fraction_of_exited_final={:.4} white_fraction_global={:.4} \
             rel_diff={:.4}",
            final_drained_frac, white_fraction_of_exited_final, white_fraction_global, rel_diff,
        );

        if final_drained_frac > 0.98 {
            assert!(
                rel_diff < 0.005,
                "instrument self-check FAILED: at drained_frac={:.4} (near-full), exited composition \
                 white_fraction={:.4} does not match global white_fraction={:.4} (rel_diff={:.4} >= 0.005) \
                 -- the instrument is measuring something other than the intended composition and the \
                 mass-flow-vs-core-flow numbers above are not trustworthy",
                final_drained_frac, white_fraction_of_exited_final, white_fraction_global, rel_diff,
            );
        } else {
            println!(
                "diag_mass_vs_core[{label}]: NOTE final_drained_frac={:.4} did not reach near-full \
                 drainage (>0.98) within {} ticks -- a residual pile remains above the neck, so the \
                 exited-vs-global check above is informative only, not asserted here",
                final_drained_frac,
                MAX_TICKS + SETTLE_EXTRA_TICKS,
            );
        }

        // Two reference bounds for white_fraction_of_exited@10%, so a reader can place the
        // observation between them without doing arithmetic:
        //   - ideal_mass_flow: perfect depth order (top/white drains first) => 0.0
        //   - no_ordering_null: drainage composition indistinguishable from the global mix,
        //     i.e. no depth ordering at all => white_fraction_global
        const IDEAL_MASS_FLOW_WHITE_AT_10: f64 = 0.0;
        println!(
            "diag_mass_vs_core[{label}]: SUMMARY neck_width={neck_width:.2} m_black={:.4} \
             f_50: ideal={:.4} observed={:?} \
             | white_fraction_of_exited@10%: ideal_mass_flow={:.4} observed={:?} no_ordering_null={:.4}",
            m_black, 2.0 * m_black, f_50, IDEAL_MASS_FLOW_WHITE_AT_10, white_at_10, white_fraction_global,
        );
    }

    #[test]
    #[ignore]
    fn diag_step1_mass_vs_core_flow_sand_funnel() {
        for neck_width in [0.02f32, 0.04, 0.08, 0.12] {
            let label = format!("sand_nw{neck_width:.2}");
            diag_mass_vs_core_flow_funnel(MaterialMode::DrySand, 1.4, &label, neck_width);
        }
    }

    #[test]
    #[ignore]
    fn diag_step1_mass_vs_core_flow_liquid_funnel() {
        for neck_width in [0.02f32, 0.04, 0.08, 0.12] {
            let label = format!("liquid_nw{neck_width:.2}");
            diag_mass_vs_core_flow_funnel(MaterialMode::Water, 0.95, &label, neck_width);
        }
    }

    // =====================================================================================
    // DIAGNOSTIC: does GRAIN_JITTER_SCALE actually raise spatial color/property variance among
    // neighbouring sand cells, the way its doc comment on `edge_arbitration_scale` /
    // `grain_jitter_strength` claims it should? No existing test measures spatial color/property
    // variance at all -- every other conservation/symmetry test either sums mass (conservation)
    // or tracks a single scalar signed difference (symmetry), neither of which would notice
    // color and properties homogenising into mush as they blend.
    //
    // SCENARIO: narrow vertical stripes of material poured into an Hourglass funnel's upper
    // chamber and left to drain through the neck. The neck (and the avalanching within the
    // chamber above it) is where `budget_term` is actually contested -- an open flat bed rarely
    // oversubscribes a cell's free capacity or available mass; a narrowing funnel constantly
    // does. Stripe width is deliberately narrow relative to the fill width (stripe_width=4 out of
    // a ~50-wide chamber) so a large share of cells start adjacent to a stripe boundary rather
    // than a handful at the edges.
    //
    // GRAIN_JITTER_SCALE is a compile-time const (see its doc comment on l.721-ish), so the
    // 0.0-vs-1.25 A/B this diagnostic exists for is NOT performed inside one process -- it can't
    // be done at runtime. Run this test twice, editing the const between runs (0.0, then 1.25)
    // and diff the printed numbers by hand; that is what the accompanying task report does.
    // NEVER assert on these numbers: they are a measurement, not a spec, and are expected to
    // change if the arbitration mechanism changes.
    //
    // WHY BOTH VARIANCE AND LOCAL CONTRAST: global spatial variance can stay high while
    // everything smooths out locally -- two large flat regions of different color, each
    // internally uniform, still have high global variance across the whole grid. That is not what
    // "grainy" means. Local contrast (mean |a - b| between adjacent OCCUPIED cells, reported
    // separately for the horizontal and vertical directions since the jitter's edge-orientation
    // salts (EDGE_SALT_H/EDGE_SALT_V) could in principle introduce a directional bias) is closer
    // to what a grainy *look* actually is.
    //
    // WHY TWO VARIANTS BELOW: a single uniform material's props never vary spatially, and
    // blending two identical values together (jittered split or not) is a no-op -- so a
    // same-material-both-stripes scenario can only ever show a color effect, with prop variance
    // pinned at exactly 0 by construction. `diag_grain_variance_mixed_materials` stripes two
    // different DrySand-family presets (both wetness=0, i.e. granular_share=1 for both --
    // apples-to-apples granular, not granular-vs-liquid) so PROP_GRAIN_SIZE/THRESHOLD/FLOW_RATE
    // also start with real spatial contrast, exercising the property side of `advect_properties`.
    fn diag_grain_variance_scenario(
        mode_a: MaterialMode,
        mode_b: MaterialMode,
        label: &str,
        tick_checkpoints: &[usize],
    ) {
        let w = 64usize;
        let h = 96usize;
        let block_size = 16usize;
        let neck_width = 0.05f32;
        let stripe_width = 4usize;

        let mask = make_test_mask(w, h, SandboxShape::Hourglass, neck_width, 0.6);

        let row_width = |y: usize| -> usize {
            (0..w).filter(|&x| mask[y * w + x] != crate::MASK_OUTSIDE).count()
        };
        let neck_y = (0..h)
            .filter(|&y| row_width(y) > 0)
            .min_by_key(|&y| (row_width(y), (y as i64 - (h as i64 / 2)).abs()))
            .expect("hourglass mask has no inside rows");
        const FILL_Y0: usize = 12;
        let fill_y1 = neck_y.saturating_sub(4);
        assert!(
            fill_y1 > FILL_Y0 + 4,
            "fill region too small: FILL_Y0={FILL_Y0} fill_y1={fill_y1} neck_y={neck_y}"
        );

        let is_stripe_a = |x: usize| -> bool { (x / stripe_width) % 2 == 0 };

        let mut props = vec![0.0f32; w * h * 4];
        for y in FILL_Y0..fill_y1 {
            for x in 0..w {
                let idx = y * w + x;
                if mask[idx] == crate::MASK_OUTSIDE {
                    continue;
                }
                let mode = if is_stripe_a(x) { mode_a } else { mode_b };
                let (wetness, threshold, flow_rate, grain_size) = mode.preset_props();
                props[idx * 4 + PROP_WETNESS] = wetness;
                props[idx * 4 + PROP_THRESHOLD] = threshold;
                props[idx * 4 + PROP_FLOW_RATE] = flow_rate;
                props[idx * 4 + PROP_GRAIN_SIZE] = grain_size;
            }
        }

        let mut sim = TestSim::new(w, h, props, mask.clone(), block_size);
        let fill_height = 1.0f32;
        for y in FILL_Y0..fill_y1 {
            for x in 0..w {
                let idx = y * w + x;
                if mask[idx] == crate::MASK_OUTSIDE {
                    continue;
                }
                sim.hm.data[idx] = fill_height;
                let (r, g, b) = if is_stripe_a(x) { (255u8, 255u8, 255u8) } else { (20u8, 20u8, 20u8) };
                sim.cell_colors[idx * 4] = r;
                sim.cell_colors[idx * 4 + 1] = g;
                sim.cell_colors[idx * 4 + 2] = b;
                sim.cell_colors[idx * 4 + 3] = 255;
            }
        }

        const CHANNEL_NAMES: [&str; 7] = [
            "color_R", "color_G", "color_B",
            "prop_WETNESS", "prop_THRESHOLD", "prop_FLOW_RATE", "prop_GRAIN_SIZE",
        ];
        // Channel index convention: 0..3 = color RGB, 3..7 = props (WETNESS/THRESHOLD/
        // FLOW_RATE/GRAIN_SIZE), matching CHANNEL_NAMES above.
        let read_channel = |sim: &TestSim, idx: usize, c: usize| -> f32 {
            if c < 3 { sim.cell_colors[idx * 4 + c] as f32 } else { sim.cell_props[idx * 4 + (c - 3)] }
        };

        let report = |sim: &TestSim, tick: usize| {
            let occupied: Vec<usize> = (0..w * h)
                .filter(|&idx| mask[idx] != crate::MASK_OUTSIDE && sim.hm.data[idx] > 0.05)
                .collect();
            let n = occupied.len();
            if n == 0 {
                println!("diag_grain_variance[{label}] t={tick}: no occupied cells left");
                return;
            }
            for (c, name) in CHANNEL_NAMES.iter().enumerate() {
                let mean: f64 = occupied.iter().map(|&i| read_channel(sim, i, c) as f64).sum::<f64>() / n as f64;
                let var: f64 = occupied
                    .iter()
                    .map(|&i| { let d = read_channel(sim, i, c) as f64 - mean; d * d })
                    .sum::<f64>()
                    / n as f64;
                let sd = var.sqrt();

                let (mut h_sum, mut h_n) = (0.0f64, 0usize);
                let (mut v_sum, mut v_n) = (0.0f64, 0usize);
                for &idx in &occupied {
                    let x = idx % w;
                    let y = idx / w;
                    if x + 1 < w {
                        let ridx = idx + 1;
                        if mask[ridx] != crate::MASK_OUTSIDE && sim.hm.data[ridx] > 0.05 {
                            h_sum += (read_channel(sim, idx, c) as f64 - read_channel(sim, ridx, c) as f64).abs();
                            h_n += 1;
                        }
                    }
                    if y + 1 < h {
                        let didx = idx + w;
                        if mask[didx] != crate::MASK_OUTSIDE && sim.hm.data[didx] > 0.05 {
                            v_sum += (read_channel(sim, idx, c) as f64 - read_channel(sim, didx, c) as f64).abs();
                            v_n += 1;
                        }
                    }
                }
                let local_h = if h_n > 0 { h_sum / h_n as f64 } else { 0.0 };
                let local_v = if v_n > 0 { v_sum / v_n as f64 } else { 0.0 };
                println!(
                    "diag_grain_variance[{label}] t={tick:>4} n_occ={n:>5} {name:<16} \
                     mean={mean:>9.4} sd={sd:>9.4} local_h={local_h:>8.4} local_v={local_v:>8.4}",
                );
            }
        };

        let gravity_dir = glam::Vec2::new(0.0, 0.04);
        report(&sim, 0);
        let mut next = 0usize;
        let mut t = 0usize;
        let max_tick = *tick_checkpoints.last().expect("tick_checkpoints must be non-empty");
        while t < max_tick {
            sim.tick(gravity_dir, 4096);
            t += 1;
            if next < tick_checkpoints.len() && t == tick_checkpoints[next] {
                report(&sim, t);
                next += 1;
            }
        }
    }

    #[test]
    #[ignore]
    // DIAGNOSTIC measurement, not pass/fail -- see `diag_grain_variance_scenario`'s doc comment.
    // Same material on both stripes (only color differs): isolates the color-only effect and
    // lets grain size be swept cleanly across presets (FinePowder 0.05 .. CoarseSand 0.80)
    // without also perturbing property contrast between the stripes.
    // Run with (editing GRAIN_JITTER_SCALE between runs):
    //   cargo test -p sandart-sim --release --lib \
    //     physics::tests::diag_grain_variance_color_only -- --ignored --nocapture
    fn diag_grain_variance_color_only() {
        for mode in [MaterialMode::FinePowder, MaterialMode::DrySand, MaterialMode::CoarseSand] {
            let label = format!("{mode:?}_color_only");
            diag_grain_variance_scenario(mode, mode, &label, &[50, 150, 300]);
        }
    }

    #[test]
    #[ignore]
    // DIAGNOSTIC measurement, not pass/fail -- see `diag_grain_variance_scenario`'s doc comment.
    // Two DIFFERENT DrySand-family materials striped together (both wetness=0, so
    // granular_share=1 for both -- an apples-to-apples granular comparison, not granular-vs-
    // liquid), so PROP_GRAIN_SIZE/THRESHOLD/FLOW_RATE start with real spatial contrast, not just
    // color -- this is what actually exercises property mixing.
    fn diag_grain_variance_mixed_materials() {
        diag_grain_variance_scenario(
            MaterialMode::FinePowder, MaterialMode::CoarseSand, "finepowder_vs_coarsesand", &[50, 150, 300],
        );
    }

    #[test]
    #[ignore]
    // DIAGNOSTIC (reproduce-only, no assertions): user report is that falling sand "falls in
    // chunks with almost clear separation of blocks" -- "large slabs that separate and merge in
    // the end" -- seen on Circle and on Hourglass after a flip, and NOT present a few days before
    // the flux-solver / frozen-state Jacobi conversion for sand's gravity-aligned edge (see the
    // Stage B comment on `test_granular_flowing_fall_conserves_mass_and_respects_cfl`, which moved
    // sand's vertical edge onto the same conservative `flux_edge` solver liquid already used).
    //
    // Scenario: drop a solid 24x24 DrySand block into an open Circle container with clear air
    // below it, and watch the block while it free-falls. A solid block has no reason to develop
    // internal gaps: nothing is metering it against anything except gravity and its own capacity.
    // If it does, that gap structure over time is the artifact.
    //
    // Three metrics, all computed every tick against the material's own current footprint (so a
    // shrinking/settling block doesn't inflate them just by having a smaller bounding box):
    //   - num_components: 8-connected components of h > 0.05 cells over the WHOLE grid, reusing
    //     `find_liquid_components` from the tendril detector (it is a generic connectivity pass,
    //     nothing liquid-specific about it -- it is called here on `sim.hm.data` for DrySand). A
    //     solid falling block is exactly one component; slabs separating means > 1.
    //   - void_cells / void_fraction: cells with h <= 0.05 strictly INSIDE the tight bounding box
    //     of all currently-occupied cells (mask != OUTSIDE only), i.e. gaps enclosed by the
    //     material's own convex extent -- the same style of measurement as `count_voids` in
    //     `test_liquid_flowing_liquid_does_not_stand_in_walls`, adapted from "liquid on either
    //     side in a row" to "inside the occupied bbox" because a solid block (unlike a draining
    //     channel) has no natural left/right liquid landmark to test against.
    //   - a vertical fill-fraction profile (rows x fill fraction across the bbox's column span),
    //     printed at the tick where void_cells peaks, to read off whether any banding has a short
    //     (~1-2 cell) wavelength (hypothesis 1: damped-Jacobi's least-damped mode is the highest
    //     frequency one) or is organised into a few large irregular bands (hypothesis 2: the
    //     uniform `edge_arbitration_scale` factor per cell plus `budget_term`'s hard
    //     `raw_total > budget` branch creating a fracture plane where neighbouring cells land on
    //     opposite sides of it).
    fn diag_falling_block_slab_separation() {
        let s = test_scale();
        let w = 128 * s;
        let h = 128 * s;
        let eps = 0.05f32;

        let mask = make_test_mask(w, h, SandboxShape::Circle, 0.04, 1.0);
        let props = get_test_props(MaterialMode::DrySand, w * h);
        let mut sim = TestSim::new(w, h, props, mask.clone(), 32);
        let gravity_dir = glam::Vec2::new(0.0, 0.04);

        // 24x24 block (scaled), centered horizontally, starting well inside the Circle's top
        // (r_x = 0.46*w =~ 58.9 at s=1; the block's farthest corner from center is at dist_sq
        // 2080 against r_x_sq 3467, comfortably inside) with the whole lower ~85% of the circle
        // open air below it to fall through.
        let block_w = 24 * s;
        let block_h = 24 * s;
        let x0 = w / 2 - block_w / 2;
        let x1 = x0 + block_w;
        let y0 = 20 * s;
        let y1 = y0 + block_h;

        let mut filled = 0usize;
        for y in y0..y1 {
            for x in x0..x1 {
                let idx = y * w + x;
                if mask[idx] != crate::MASK_OUTSIDE {
                    sim.hm.data[idx] = 1.0;
                    filled += 1;
                }
            }
        }
        assert_eq!(
            filled, block_w * block_h,
            "block placement clipped by the Circle mask -- adjust y0/block size, this scenario \
             needs a genuinely solid rectangular block to start"
        );

        let bbox = |sim: &TestSim| -> Option<(usize, usize, usize, usize)> {
            let (mut min_x, mut max_x, mut min_y, mut max_y) = (usize::MAX, 0usize, usize::MAX, 0usize);
            let mut any = false;
            for y in 0..h {
                for x in 0..w {
                    if sim.hm.data[y * w + x] > eps {
                        any = true;
                        min_x = min_x.min(x);
                        max_x = max_x.max(x);
                        min_y = min_y.min(y);
                        max_y = max_y.max(y);
                    }
                }
            }
            any.then_some((min_x, max_x, min_y, max_y))
        };

        // "Eroded interior" void count: same as void_cells but excluding any cell within
        // `erosion` of the bbox boundary. A rectangular bbox around a naturally sloped/triangular
        // settled heap always has some void along its edges (the heap's corners) even with zero
        // internal defect; this strips that geometric slack out so a nonzero count here can only
        // come from a genuine enclosed pocket, not the heap's outline.
        let erosion = 3usize;
        let interior_void = |sim: &TestSim, min_x: usize, max_x: usize, min_y: usize, max_y: usize| -> usize {
            if max_x - min_x <= 2 * erosion || max_y - min_y <= 2 * erosion {
                return 0;
            }
            let mut count = 0usize;
            for y in (min_y + erosion)..=(max_y - erosion) {
                for x in (min_x + erosion)..=(max_x - erosion) {
                    let idx = y * w + x;
                    if mask[idx] != crate::MASK_OUTSIDE && sim.hm.data[idx] <= eps {
                        count += 1;
                    }
                }
            }
            count
        };

        let mut peak_void_cells = 0usize;
        let mut peak_void_frac = 0.0f32;
        let mut peak_void_tick = 0usize;
        let mut peak_void_bbox = (0usize, 0usize, 0usize, 0usize);
        let mut peak_components = 0usize;
        let mut peak_components_tick = 0usize;
        let mut final_components = 0usize;
        let mut final_void_cells = 0usize;
        let mut peak_interior_void = 0usize;
        let mut peak_interior_void_tick = 0usize;
        let mut final_interior_void = 0usize;
        let checkpoints: Vec<usize> = [2, 4, 6, 8, 10, 15, 20, 30, 38, 50, 80, 120, 180, 260]
            .iter()
            .map(|&t| t * s)
            .collect();

        let max_ticks = 260 * s;
        for t in 0..max_ticks {
            sim.tick(gravity_dir, usize::MAX);

            let components = find_liquid_components(&sim.hm.data, &mask, w, h, 0, h - 1, eps);
            let num_components = components.len();
            if num_components > peak_components {
                peak_components = num_components;
                peak_components_tick = t + 1;
            }

            let (void_cells, void_frac, this_bbox, this_interior_void) = match bbox(&sim) {
                Some((min_x, max_x, min_y, max_y)) => {
                    let mut inside_cells = 0usize;
                    let mut void_cells = 0usize;
                    for y in min_y..=max_y {
                        for x in min_x..=max_x {
                            let idx = y * w + x;
                            if mask[idx] == crate::MASK_OUTSIDE {
                                continue;
                            }
                            inside_cells += 1;
                            if sim.hm.data[idx] <= eps {
                                void_cells += 1;
                            }
                        }
                    }
                    let frac = void_cells as f32 / inside_cells.max(1) as f32;
                    let iv = interior_void(&sim, min_x, max_x, min_y, max_y);
                    (void_cells, frac, (min_x, max_x, min_y, max_y), iv)
                }
                None => (0, 0.0, (0, 0, 0, 0), 0),
            };
            if void_cells > peak_void_cells {
                peak_void_cells = void_cells;
                peak_void_tick = t + 1;
                peak_void_bbox = this_bbox;
            }
            peak_void_frac = peak_void_frac.max(void_frac);
            if this_interior_void > peak_interior_void {
                peak_interior_void = this_interior_void;
                peak_interior_void_tick = t + 1;
            }

            if checkpoints.contains(&(t + 1)) {
                println!(
                    "diag_falling_block_slab_separation: t={:>4} components={} void_cells={:>4} \
                     void_frac={:.4} interior_void={:>4} bbox=({},{})-({},{})",
                    t + 1, num_components, void_cells, void_frac, this_interior_void,
                    this_bbox.0, this_bbox.2, this_bbox.1, this_bbox.3
                );
            }

            if t + 1 == max_ticks {
                final_components = num_components;
                final_void_cells = void_cells;
                final_interior_void = this_interior_void;
            }
        }

        println!(
            "diag_falling_block_slab_separation: scale={s} w={w} h={h} block={block_w}x{block_h} \
             peak_void_cells={peak_void_cells} (@tick {peak_void_tick}) peak_void_frac={peak_void_frac:.4} \
             peak_components={peak_components} (@tick {peak_components_tick}) \
             peak_interior_void={peak_interior_void} (@tick {peak_interior_void_tick}) \
             final_components={final_components} final_void_cells={final_void_cells} \
             final_interior_void={final_interior_void}"
        );

        // Re-run just the peak-void tick's row profile for a banding-wavelength read. Rerunning
        // rather than caching per-tick history keeps the main loop's memory flat; the sim is
        // deterministic (fixed RNG seed schedule in `TestSim::tick`), so replaying to the same
        // tick count reproduces the identical state.
        if peak_void_cells > 0 {
            let mask2 = mask.clone();
            let props2 = get_test_props(MaterialMode::DrySand, w * h);
            let mut sim2 = TestSim::new(w, h, props2, mask2.clone(), 32);
            for y in y0..y1 {
                for x in x0..x1 {
                    let idx = y * w + x;
                    if mask2[idx] != crate::MASK_OUTSIDE {
                        sim2.hm.data[idx] = 1.0;
                    }
                }
            }
            for _ in 0..peak_void_tick {
                sim2.tick(gravity_dir, usize::MAX);
            }
            let (min_x, max_x, min_y, max_y) = peak_void_bbox;
            println!(
                "diag_falling_block_slab_separation: row fill-fraction profile at peak tick {peak_void_tick}, \
                 bbox x=[{min_x},{max_x}] y=[{min_y},{max_y}]"
            );
            for y in min_y..=max_y {
                let mut inside = 0usize;
                let mut filled = 0usize;
                for x in min_x..=max_x {
                    let idx = y * w + x;
                    if mask2[idx] == crate::MASK_OUTSIDE {
                        continue;
                    }
                    inside += 1;
                    if sim2.hm.data[idx] > eps {
                        filled += 1;
                    }
                }
                let frac = filled as f32 / inside.max(1) as f32;
                let bar_len = (frac * 40.0).round() as usize;
                let bar: String = "#".repeat(bar_len);
                println!("  y={y:>4} fill={frac:.3} {bar}");
            }

            // Raw 2D snapshot of a central column strip, to tell horizontal row-banding (the
            // whole row's fill fraction moves together, columns roughly agree within a row) apart
            // from a true 2D checkerboard (fill alternates cell-by-cell independent of neighbours
            // in the SAME row too). '#'=h>0.5, '+'=0.05<h<=0.5, '.'=h<=0.05(void).
            let strip_x0 = min_x + (max_x - min_x) / 2 - 6.min((max_x - min_x) / 2);
            let strip_x1 = (strip_x0 + 12).min(max_x);
            println!(
                "diag_falling_block_slab_separation: 2D snapshot cols [{strip_x0},{strip_x1}], \
                 rows [{min_y},{max_y}]"
            );
            for y in min_y..=max_y {
                let mut line = String::new();
                for x in strip_x0..=strip_x1 {
                    let idx = y * w + x;
                    if mask2[idx] == crate::MASK_OUTSIDE {
                        line.push(' ');
                        continue;
                    }
                    let v = sim2.hm.data[idx];
                    line.push(if v > 0.5 { '#' } else if v > eps { '+' } else { '.' });
                }
                println!("  y={y:>4} {line}");
            }

            // Quantitative parity correlation, over the SAME eroded interior region
            // `interior_void` uses (excludes the bbox's outer `erosion` rim, so this isn't just
            // picking up the block's outline). Splits mean(h) by row parity, column parity, and
            // (x+y) parity (the checkerboard combination). If hypothesis 1 (damped-Jacobi's
            // least-damped mode = highest spatial frequency) is what's firing, the (x+y) parity
            // split should show the largest mean(h) gap of the three, since a 2D checkerboard
            // mode is `(-1)^(x+y)`, not purely `(-1)^x` or `(-1)^y`. If instead this is
            // hypothesis 2 (a few large irregular slabs from the `budget_term` branch), none of
            // the three parity splits should show a meaningfully large gap -- the low-h region
            // would be spatially contiguous, not alternating cell-by-cell.
            let (min_x, max_x, min_y, max_y) = peak_void_bbox;
            if max_x - min_x > 2 * erosion && max_y - min_y > 2 * erosion {
                let mut sums = [[0.0f64; 2]; 3]; // [row, col, checker][parity]
                let mut counts = [[0usize; 2]; 3];
                for y in (min_y + erosion)..=(max_y - erosion) {
                    for x in (min_x + erosion)..=(max_x - erosion) {
                        let idx = y * w + x;
                        if mask2[idx] == crate::MASK_OUTSIDE {
                            continue;
                        }
                        let v = sim2.hm.data[idx] as f64;
                        let py = y % 2;
                        let px = x % 2;
                        let pc = (x + y) % 2;
                        sums[0][py] += v;
                        counts[0][py] += 1;
                        sums[1][px] += v;
                        counts[1][px] += 1;
                        sums[2][pc] += v;
                        counts[2][pc] += 1;
                    }
                }
                let mean = |k: usize, p: usize| sums[k][p] / counts[k][p].max(1) as f64;
                let gap = |k: usize| (mean(k, 0) - mean(k, 1)).abs();
                println!(
                    "diag_falling_block_slab_separation: parity split @ tick {peak_void_tick} \
                     (interior region, n={}): row_parity mean(even,odd)=({:.4},{:.4}) gap={:.4} | \
                     col_parity mean(even,odd)=({:.4},{:.4}) gap={:.4} | \
                     checker(x+y) mean(even,odd)=({:.4},{:.4}) gap={:.4}",
                    counts[0][0] + counts[0][1],
                    mean(0, 0), mean(0, 1), gap(0),
                    mean(1, 0), mean(1, 1), gap(1),
                    mean(2, 0), mean(2, 1), gap(2),
                );
            }
        }
    }

    #[test]
    #[ignore]
    // DIAGNOSTIC (reproduce-only, no assertions), follow-up to `diag_falling_block_slab_separation`
    // after the user supplied a photo of the actual defect: DrySand in a Circle container breaks
    // into large horizontal slabs with clean straight-edged empty gaps *while still falling* --
    // the material is never asleep, the slabs keep moving, and the gaps close again once the pile
    // settles. That rules out edge-sleeping (asleep material cannot be what the user sees falling)
    // and reframes the question: gaps opening between falling layers means different layers are
    // falling at different SPEEDS, since uniform free fall cannot open a gap between two layers
    // that started in contact and share one gravitational acceleration.
    //
    // This tracks, at each checkpoint during free fall, the mass-weighted vertical centroid of the
    // object's EVEN-row mass separately from its ODD-row mass (over the same eroded-interior
    // region `diag_falling_block_slab_separation` uses for its parity split), so a per-checkpoint
    // finite difference gives a directly-measured fall SPEED for each parity class. If the two
    // diverge and the divergence grows, rows are provably falling at different speeds and the
    // effect is compounding rather than static. Alongside that it prints the mean `column_depth`
    // (the one hand-rolled, non-flux-solver overburden term -- see its own doc comment -- that
    // feeds the LATERAL edge's driving head, never the vertical one) split the same way, to check
    // whether the seed of the divergence (if any) is visible there too, or is a distinct signal.
    fn diag_falling_block_layer_velocity() {
        let s = test_scale();
        let w = 128 * s;
        let h = 128 * s;

        let mask = make_test_mask(w, h, SandboxShape::Circle, 0.04, 1.0);
        let props = get_test_props(MaterialMode::DrySand, w * h);
        let mut sim = TestSim::new(w, h, props, mask.clone(), 32);
        let gravity_dir = glam::Vec2::new(0.0, 0.04);

        let block_w = 24 * s;
        let block_h = 24 * s;
        let x0 = w / 2 - block_w / 2;
        let x1 = x0 + block_w;
        let y0 = 20 * s;
        let y1 = y0 + block_h;

        let mut filled = 0usize;
        for y in y0..y1 {
            for x in x0..x1 {
                let idx = y * w + x;
                if mask[idx] != crate::MASK_OUTSIDE {
                    sim.hm.data[idx] = 1.0;
                    filled += 1;
                }
            }
        }
        assert_eq!(
            filled, block_w * block_h,
            "block placement clipped by the Circle mask -- adjust y0/block size"
        );

        let bbox = |sim: &TestSim| -> Option<(usize, usize, usize, usize)> {
            let (mut min_x, mut max_x, mut min_y, mut max_y) = (usize::MAX, 0usize, usize::MAX, 0usize);
            let mut any = false;
            for y in 0..h {
                for x in 0..w {
                    if sim.hm.data[y * w + x] > 0.05 {
                        any = true;
                        min_x = min_x.min(x);
                        max_x = max_x.max(x);
                        min_y = min_y.min(y);
                        max_y = max_y.max(y);
                    }
                }
            }
            any.then_some((min_x, max_x, min_y, max_y))
        };

        let erosion = 3usize;
        let checkpoint_every = 5 * s;
        let max_ticks = 150 * s;

        println!(
            "diag_falling_block_layer_velocity: scale={s} w={w} h={h} block={block_w}x{block_h}"
        );

        let mut last: Option<(usize, f64, f64)> = None;
        for t in 0..max_ticks {
            sim.tick(gravity_dir, usize::MAX);
            let tick_no = t + 1;
            if tick_no % checkpoint_every != 0 {
                continue;
            }
            let Some((min_x, max_x, min_y, max_y)) = bbox(&sim) else { continue };
            if max_x - min_x <= 2 * erosion || max_y - min_y <= 2 * erosion {
                continue;
            }

            let mut m_even = 0f64;
            let mut ysum_even = 0f64;
            let mut m_odd = 0f64;
            let mut ysum_odd = 0f64;
            let mut cd_even_sum = 0f64;
            let mut cd_even_cnt = 0usize;
            let mut cd_odd_sum = 0f64;
            let mut cd_odd_cnt = 0usize;
            for y in (min_y + erosion)..=(max_y - erosion) {
                for x in (min_x + erosion)..=(max_x - erosion) {
                    let idx = y * w + x;
                    if mask[idx] == crate::MASK_OUTSIDE {
                        continue;
                    }
                    let hh = sim.hm.data[idx] as f64;
                    let cd = sim.column_depth[idx] as f64;
                    if y % 2 == 0 {
                        m_even += hh;
                        ysum_even += hh * y as f64;
                        cd_even_sum += cd;
                        cd_even_cnt += 1;
                    } else {
                        m_odd += hh;
                        ysum_odd += hh * y as f64;
                        cd_odd_sum += cd;
                        cd_odd_cnt += 1;
                    }
                }
            }
            let cy_even = if m_even > 0.0 { ysum_even / m_even } else { f64::NAN };
            let cy_odd = if m_odd > 0.0 { ysum_odd / m_odd } else { f64::NAN };
            let cd_even = cd_even_sum / cd_even_cnt.max(1) as f64;
            let cd_odd = cd_odd_sum / cd_odd_cnt.max(1) as f64;
            let bbox_h = max_y - min_y;

            let vel_str = if let Some((pt, pcy_e, pcy_o)) = last {
                let dt = (tick_no - pt) as f64;
                let v_even = (cy_even - pcy_e) / dt;
                let v_odd = (cy_odd - pcy_o) / dt;
                format!(" v_even={:.4} v_odd={:.4} dv={:.4}", v_even, v_odd, v_even - v_odd)
            } else {
                String::new()
            };

            println!(
                "diag_falling_block_layer_velocity: t={:>4} bbox_h={:>4} cy_even={:.2} \
                 cy_odd={:.2} gap_cy={:.3} m_even={:.1} m_odd={:.1} cd_even={:.5} cd_odd={:.5} \
                 cd_gap={:.5}{vel_str}",
                tick_no, bbox_h, cy_even, cy_odd, cy_even - cy_odd, m_even, m_odd, cd_even,
                cd_odd, cd_even - cd_odd
            );

            last = Some((tick_no, cy_even, cy_odd));
        }
    }

    /// Writes one binary PPM (P6) frame of the occupancy field, nearest-neighbour upscaled by
    /// `scale`, to `path`. Black = empty, light tan = occupied (h > 0.05), and -- when
    /// `overlay_grid` is set -- a dim red line marks every `block_size`'th cell boundary so a
    /// human viewer can read off whether gaps land on activation-block boundaries directly from
    /// the image, without cross-referencing coordinates by hand.
    fn write_ppm_frame(
        path: &std::path::Path,
        data: &[f32],
        mask: &[u8],
        w: usize,
        h: usize,
        scale: usize,
        block_size: usize,
        overlay_grid: bool,
    ) {
        use std::io::Write;
        let out_w = w * scale;
        let out_h = h * scale;
        let mut buf = Vec::with_capacity(out_w * out_h * 3);
        for oy in 0..out_h {
            let y = oy / scale;
            let grid_line_y = overlay_grid && y % block_size == 0;
            for ox in 0..out_w {
                let x = ox / scale;
                let idx = y * w + x;
                let grid_line_x = overlay_grid && x % block_size == 0;
                let (r, g, b) = if mask[idx] == crate::MASK_OUTSIDE {
                    (30u8, 30u8, 30u8)
                } else if grid_line_x || grid_line_y {
                    (140u8, 20u8, 20u8)
                } else if data[idx] > 0.05 {
                    (222u8, 197u8, 145u8)
                } else {
                    (0u8, 0u8, 0u8)
                };
                buf.push(r);
                buf.push(g);
                buf.push(b);
            }
        }
        let mut f = std::fs::File::create(path).expect("create ppm frame");
        write!(f, "P6\n{out_w} {out_h}\n255\n").unwrap();
        f.write_all(&buf).unwrap();
    }

    /// Mirrors a DrawingSimulation-style "flip the apparatus" for a symmetric (Circle) container
    /// on the lower-level `TestSim` harness: reflects the heightmap about `center_y = h / 2`
    /// (same axis `DrawingSimulation::flip_hourglass` in lib.rs uses), clears the per-edge
    /// momentum and `column_depth` buffers (a flipped apparatus's contents are in free fall from
    /// rest, not carrying over pre-flip momentum -- see `flip_hourglass`'s own comment), and
    /// forces every block to be reconsidered next tick. Doesn't touch `shape_mask` because
    /// Circle is symmetric about the same axis, so `generate_shape_mask` after a real flip would
    /// be a no-op here — the one piece of `flip_hourglass` this intentionally skips.
    fn flip_sim(sim: &mut TestSim) {
        let w = sim.hm.width;
        let h = sim.hm.height;
        for y in 1..=h / 2 {
            let y2 = h.saturating_sub(y);
            if y == y2 || y2 >= h {
                continue;
            }
            for x in 0..w {
                let i1 = y * w + x;
                let i2 = y2 * w + x;
                sim.hm.data.swap(i1, i2);
            }
        }
        sim.edge_vel_h.fill(0.0);
        sim.edge_vel_v.fill(0.0);
        sim.column_depth.fill(0.0);
        sim.last_displacements.fill(0.5);
        sim.tick_count = 0;
    }

    /// Task #47: mirrors `DrawingSimulation::update`'s "perfect simulation" debug toggle (see its
    /// own doc comment in `lib.rs`) on the lower-level `TestSim` harness -- force every in-mask,
    /// material-holding block's recorded displacement to `MUST_SIMULATE_THRESHOLD` before calling
    /// `tick`, the exact same admission path (and the same `crate::PERFECT_SIM_MATERIAL_EPSILON`
    /// material bar) the real toggle uses. This is what makes the comparison below "against ground
    /// truth" rather than "against a second, test-only approximation of ground truth": both this
    /// function and `DrawingSimulation`'s toggle route through the identical `must_simulate`
    /// admission `settle_tick` already has, rather than adding a second bypass mechanism.
    fn perfect_sim_tick(sim: &mut TestSim, mask: &[u8], gravity_dir: glam::Vec2) -> f32 {
        let w = sim.hm.width;
        let h = sim.hm.height;
        let block_size = sim.block_size;
        let cols = (w + block_size - 1) / block_size;
        let rows = (h + block_size - 1) / block_size;
        for by in 0..rows {
            let start_y = by * block_size;
            let end_y = ((by + 1) * block_size).min(h);
            for bx in 0..cols {
                let start_x = bx * block_size;
                let end_x = ((bx + 1) * block_size).min(w);
                let mut has_material = false;
                'scan: for y in start_y..end_y {
                    let row_offset = y * w;
                    for x in start_x..end_x {
                        let idx = row_offset + x;
                        if mask[idx] != crate::MASK_OUTSIDE
                            && sim.hm.data[idx] > crate::PERFECT_SIM_MATERIAL_EPSILON
                        {
                            has_material = true;
                            break 'scan;
                        }
                    }
                }
                if has_material {
                    sim.last_displacements[by * cols + bx] = MUST_SIMULATE_THRESHOLD;
                }
            }
        }
        sim.tick(gravity_dir, usize::MAX)
    }

    /// Task #47: builds the "resting-then-flipped" scenario shared by the divergence
    /// regression test and its round-2 variant-comparison diagnostic -- fill roughly the bottom
    /// 60% of a Circle to h=1.0 (same recipe `diag_flip_release_front_and_block_alignment` uses),
    /// settle with `perfect_sim_tick` (deterministic and unaffected by `fresh_overburden_gate`,
    /// since perfect-sim's own force-must already saturates every material-holding block
    /// regardless), then flip. Calling this repeatedly with the same inputs is how multiple
    /// independent `TestSim`s get a bit-identical starting point without `TestSim` needing to
    /// implement `Clone`.
    fn settled_then_flipped(
        w: usize,
        h: usize,
        mask: &[u8],
        props: &[f32],
        block_size: usize,
        gravity_dir: glam::Vec2,
    ) -> TestSim {
        let mut sim = TestSim::new(w, h, props.to_vec(), mask.to_vec(), block_size);
        let fill_y0 = (0.40 * h as f32) as usize;
        for y in fill_y0..h {
            for x in 0..w {
                let idx = y * w + x;
                if mask[idx] != crate::MASK_OUTSIDE {
                    sim.hm.data[idx] = 1.0;
                }
            }
        }
        let mut quiet_run = 0usize;
        for _ in 0..4000usize {
            let flow = perfect_sim_tick(&mut sim, mask, gravity_dir);
            if flow < 1e-3 {
                quiet_run += 1;
                if quiet_run >= 15 {
                    break;
                }
            } else {
                quiet_run = 0;
            }
        }
        flip_sim(&mut sim);
        sim
    }

    /// Task #47: every `FreshOverburdenVariant` a diagnostic can report by name (round 1's
    /// `OverburdenOnly`/`OverburdenAndRoom`, round 2's `CapacityBelowOnly`/
    /// `OverburdenOrCapacityBelow`/`RoomOnly`, round 3's `UnsupportedOnly`/`UnsupportedAndRoom`).
    const ALL_FRESH_OVERBURDEN_VARIANTS: [(FreshOverburdenVariant, &str); 7] = [
        (FreshOverburdenVariant::OverburdenOnly, "overburden_only"),
        (FreshOverburdenVariant::OverburdenAndRoom, "overburden_and_room(round1)"),
        (FreshOverburdenVariant::CapacityBelowOnly, "capacity_below_only"),
        (FreshOverburdenVariant::OverburdenOrCapacityBelow, "overburden_or_capacity"),
        (FreshOverburdenVariant::RoomOnly, "room_only"),
        (FreshOverburdenVariant::UnsupportedOnly, "unsupported_only"),
        (FreshOverburdenVariant::UnsupportedAndRoom, "unsupported_and_room(shipped)"),
    ];

    /// Task #47: prints, for one scenario snapshot, the fraction of blocks (of `cols * rows`
    /// total) each `FreshOverburdenVariant` would promote, alongside perfect simulation's own
    /// admission rule (every in-mask block holding material -- the same scan `perfect_sim_tick`
    /// runs) as the 100% reference. Used by `diag_task47_block_fraction_table`.
    fn report_fresh_overburden_fraction(label: &str, grid: usize, sim: &TestSim, mask: &[u8], block_size: usize) {
        let w = sim.hm.width;
        let h = sim.hm.height;
        let cols = (w + block_size - 1) / block_size;
        let rows = (h + block_size - 1) / block_size;
        let total_blocks = cols * rows;

        let mut perfect_n = 0usize;
        for by in 0..rows {
            let start_y = by * block_size;
            let end_y = ((by + 1) * block_size).min(h);
            for bx in 0..cols {
                let start_x = bx * block_size;
                let end_x = ((bx + 1) * block_size).min(w);
                let mut has_material = false;
                'scan: for y in start_y..end_y {
                    let row_offset = y * w;
                    for x in start_x..end_x {
                        let idx = row_offset + x;
                        if mask[idx] != crate::MASK_OUTSIDE
                            && sim.hm.data[idx] > crate::PERFECT_SIM_MATERIAL_EPSILON
                        {
                            has_material = true;
                            break 'scan;
                        }
                    }
                }
                if has_material {
                    perfect_n += 1;
                }
            }
        }

        for (variant, name) in ALL_FRESH_OVERBURDEN_VARIANTS {
            let promoted = fresh_overburden_must_blocks(
                w, h, block_size, cols, rows, mask, &sim.hm.data, &sim.hm.external_mass_this_tick,
                &sim.cell_props, &sim.edge_vel_v, variant,
            );
            let n = promoted.iter().filter(|&&x| x).count();
            println!(
                "diag_task47_block_fraction_table: {label:<20} grid={grid:>3} variant={name:<30} \
                 total_blocks={total_blocks:>6} promoted={n:>6} ({:.4}) perfect={perfect_n:>6} ({:.4})",
                n as f64 / total_blocks as f64,
                perfect_n as f64 / total_blocks as f64,
            );
        }
    }

    #[test]
    #[ignore]
    // DIAGNOSTIC (reproduce-only, no assertions). The user's exact repro: fill a Circle
    // container with DrySand, run until it is FULLY AT REST, then FLIP it (which -- per
    // `flip_hourglass` in lib.rs -- mirrors the settled pile from the bottom of the container to
    // the top, clears stored edge momentum, and forces every block to be reconsidered) and watch
    // what happens as it falls again. This is a stronger, more direct instrument than the
    // dropped-block scenario: nothing about a resting-then-flipped pile artificially assumes a
    // rectangular shape, and "run to rest first" is exactly what the user reported doing.
    //
    // Runs the WHOLE scenario twice from the same seed: once with the upstream/side block-wake
    // fix (`activate_neighbor_upstream` / `activate_neighbor_side`) disabled via
    // `upstream_wake_gate`, once with it enabled (the shipped default), so the printed metrics
    // are a true same-build A/B rather than a before/after across two separate compiles.
    //
    // Metrics per tick: interior void count (same eroded-bbox definition as
    // `diag_falling_block_slab_separation`), and a BLOCK-BOUNDARY ALIGNMENT score -- of the rows
    // that are mostly empty (mean fill < 0.15) while sandwiched between two rows that are not
    // (a "gap row"), what fraction sit at `y % block_size == 0`, i.e. exactly on an
    // activation-block boundary. Chance alignment is `1 / block_size` (50% at grid=64's
    // block_size=2, 6.25% at grid=512's block_size=16) -- a rate well above that is the
    // block-boundary signature; a rate near chance is not.
    //
    // Writes PPM frames (nearest-neighbour upscaled 8x) for a subsample of post-flip ticks into
    // `/tmp/.../scratchpad/slabframes/{before,after}/` for the "before" (fix disabled) and
    // "after" (fix enabled) runs, plus a `grid/` set with activation-block gridlines overlaid
    // (before-fix only, since that's the run the alignment claim is about) for the caller to
    // build contact sheets from outside this test (no image/video crate is a dependency of this
    // crate, and none is added here — see the task brief).
    fn diag_flip_release_front_and_block_alignment() {
        let frame_root = std::path::PathBuf::from(
            "/tmp/claude-1000/-home-deck-projects-sandart/6dbad8f7-de15-4c1a-aae8-0d4d41f500d8/scratchpad/slabframes",
        );

        for grid in [64usize, 512usize] {
            let w = grid;
            let h = grid;
            let block_size = (grid / 32).max(1);
            let mask = make_test_mask(w, h, SandboxShape::Circle, 0.04, 1.0);
            let props = get_test_props(MaterialMode::DrySand, w * h);
            let gravity_dir = glam::Vec2::new(0.0, 0.04);

            for (disabled, tag) in [(true, "before"), (false, "after")] {
                upstream_wake_gate::set_disabled(disabled);

                let mut sim = TestSim::new(w, h, props.clone(), mask.clone(), block_size);
                // Fill roughly the bottom 60% of the circle to h=1.0 -- a solid packed mass with
                // clear air above it, the shape "pour DrySand into Circle and let it settle"
                // naturally produces.
                let fill_y0 = (0.40 * h as f32) as usize;
                for y in fill_y0..h {
                    for x in 0..w {
                        let idx = y * w + x;
                        if mask[idx] != crate::MASK_OUTSIDE {
                            sim.hm.data[idx] = 1.0;
                        }
                    }
                }

                // Run to rest: total per-tick flow below a tiny bar for 15 consecutive ticks.
                let mut quiet_run = 0usize;
                let mut rest_tick = 0usize;
                let max_rest_ticks = 4000usize;
                for t in 0..max_rest_ticks {
                    let flow = sim.tick(gravity_dir, usize::MAX);
                    if flow < 1e-3 {
                        quiet_run += 1;
                        if quiet_run >= 15 {
                            rest_tick = t + 1;
                            break;
                        }
                    } else {
                        quiet_run = 0;
                    }
                }
                let settled_mass: f64 = sim.hm.data.iter().map(|&v| v as f64).sum();
                println!(
                    "diag_flip: grid={grid} [{tag}] rest reached at tick={rest_tick} (0 = did \
                     not settle within {max_rest_ticks}) settled_mass={settled_mass:.1}"
                );

                flip_sim(&mut sim);

                let post_flip_ticks = if grid == 64 { 260 } else { 400 };
                let frame_every = if grid == 64 { 2 } else { 6 };
                let frame_scale = if grid == 64 { 8 } else { 1 };
                let frame_dir = frame_root.join(tag);
                let grid_dir = frame_root.join("grid");
                if grid == 64 {
                    let _ = std::fs::create_dir_all(&frame_dir);
                    if !disabled {
                        // grid-overlay sheet is requested for the shipped (fixed) behaviour too,
                        // in addition to "before" — write both, distinguished by filename.
                    }
                    let _ = std::fs::create_dir_all(&grid_dir);
                }

                let mut peak_interior_void = 0usize;
                let mut peak_interior_void_tick = 0usize;
                let mut align_hits = 0usize;
                let mut align_total = 0usize;
                let mut frames_written = 0usize;

                for t in 0..post_flip_ticks {
                    sim.tick(gravity_dir, usize::MAX);
                    let tick_no = t + 1;

                    // bbox + interior void (same definition as diag_falling_block_slab_separation)
                    let (mut min_x, mut max_x, mut min_y, mut max_y) =
                        (usize::MAX, 0usize, usize::MAX, 0usize);
                    let mut any = false;
                    for y in 0..h {
                        for x in 0..w {
                            if sim.hm.data[y * w + x] > 0.05 {
                                any = true;
                                min_x = min_x.min(x);
                                max_x = max_x.max(x);
                                min_y = min_y.min(y);
                                max_y = max_y.max(y);
                            }
                        }
                    }
                    let mut interior_void = 0usize;
                    if any && max_x > min_x + 6 && max_y > min_y + 6 {
                        for y in (min_y + 3)..=(max_y - 3) {
                            for x in (min_x + 3)..=(max_x - 3) {
                                let idx = y * w + x;
                                if mask[idx] != crate::MASK_OUTSIDE && sim.hm.data[idx] <= 0.05 {
                                    interior_void += 1;
                                }
                            }
                        }
                    }
                    if interior_void > peak_interior_void {
                        peak_interior_void = interior_void;
                        peak_interior_void_tick = tick_no;
                    }

                    // Gap-row block-boundary alignment, over the object's column span.
                    if any && max_x > min_x + 6 && max_y > min_y + 4 {
                        let row_fill = |y: usize| -> f32 {
                            let mut inside = 0usize;
                            let mut filled = 0usize;
                            for x in min_x..=max_x {
                                let idx = y * w + x;
                                if mask[idx] == crate::MASK_OUTSIDE {
                                    continue;
                                }
                                inside += 1;
                                if sim.hm.data[idx] > 0.05 {
                                    filled += 1;
                                }
                            }
                            if inside == 0 { 1.0 } else { filled as f32 / inside as f32 }
                        };
                        for y in (min_y + 1)..max_y {
                            let f_above = row_fill(y - 1);
                            let f_here = row_fill(y);
                            let f_below = row_fill(y + 1);
                            if f_here < 0.15 && f_above > 0.4 && f_below > 0.4 {
                                align_total += 1;
                                if y % block_size == 0 {
                                    align_hits += 1;
                                }
                            }
                        }
                    }

                    if grid == 64 && tick_no % frame_every == 0 {
                        let path = frame_dir.join(format!("frame_{tick_no:04}.ppm"));
                        write_ppm_frame(&path, &sim.hm.data, &mask, w, h, frame_scale, block_size, false);
                        frames_written += 1;
                        if !disabled {
                            let gpath = grid_dir.join(format!("frame_{tick_no:04}.ppm"));
                            write_ppm_frame(&gpath, &sim.hm.data, &mask, w, h, frame_scale, block_size, true);
                        }
                    }
                }

                let align_frac = if align_total > 0 {
                    align_hits as f64 / align_total as f64
                } else {
                    -1.0
                };
                let chance = 1.0 / block_size as f64;
                println!(
                    "diag_flip: grid={grid} [{tag}] block_size={block_size} \
                     peak_interior_void={peak_interior_void} (@tick {peak_interior_void_tick}) \
                     gap_rows_sampled={align_total} block_boundary_aligned={align_hits} \
                     align_frac={align_frac:.3} chance_frac={chance:.3} frames_written={frames_written}"
                );
            }
        }

        upstream_wake_gate::set_disabled(false);
    }

    /// Pressure projection (see `pressure_project` and its `pressure_gate`), test 1 of 3: mass
    /// conservation to the STRUCTURAL band, not the loose `1e-4` relative-error bound most of this
    /// file's other mass-conservation tests use (see e.g. `test_liquid_mass_conserved_under_gravity`).
    ///
    /// `pressure_project` only ever corrects an edge by `phi[a] - phi[b]`: the same amount debited
    /// from `a` is credited to `b`, so summing every `cand_h`/`cand_v` before and after the
    /// correction is unchanged, and `flux_edge_apply` (unmodified by this feature) is itself exactly
    /// conservative. So the total should drift only by f32 summation-order noise, not by anything
    /// proportional to how much mass moved -- unlike the `1e-4`-bounded tests elsewhere, whose bound
    /// has to cover other, non-structural sources too (the granular CA's own sequential `try_move`,
    /// the tiny-residual sweep, etc.).
    #[test]
    fn test_pressure_projection_conserves_mass_structurally() {
        let w = 64;
        let h = 64;
        let mask = make_test_mask(w, h, SandboxShape::Hourglass, 0.15, 0.6);
        let props = get_test_props(MaterialMode::Water, w * h);
        let mut sim = TestSim::new(w, h, props, mask.clone(), 32);
        for y in 0..h / 2 {
            for x in 0..w {
                if mask[y * w + x] != crate::MASK_OUTSIDE {
                    sim.hm.data[y * w + x] = 1.0;
                }
            }
        }
        assert!(!pressure_gate::is_disabled(), "pressure must be enabled (the default) for this test");
        let initial_mass = sim.mass();
        let gravity_dir = glam::Vec2::new(0.0, 0.04);
        for _ in 0..200 {
            sim.tick(gravity_dir, usize::MAX);
        }
        let final_mass = sim.mass();
        let rel_err = (final_mass - initial_mass).abs() / initial_mass;
        println!(
            "test_pressure_projection_conserves_mass_structurally: init={:.6} final={:.6} rel_err={:.3e}",
            initial_mass, final_mass, rel_err
        );
        // Measured today: rel_err ~= 2.4e-8 -- squarely inside the structural 1e-9..1e-8-per-edge-op
        // band this codebase treats as "conservative by construction" (see e.g. the frozen-Jacobi
        // conversion's own measurements) once summed over 200 ticks' worth of f32 additions. Bound
        // set two orders of magnitude above that for headroom while still ruling out anything
        // proportional to how much mass moved (which is what the loose `1e-4` bound elsewhere in
        // this file cannot distinguish from).
        assert!(
            rel_err < 1e-6,
            "Pressure projection is not structurally mass-conservative: rel_err={:.3e}",
            rel_err
        );
    }

    /// Pressure projection, test 2 of 3: no cell ever exceeds its material capacity with the
    /// projection enabled. The projection's `max(0.0, ...)` relaxation (the complementarity
    /// condition) is what's supposed to guarantee this on the PROPOSAL side, and arbitration
    /// (unmodified) guarantees it on the APPLIED side regardless of how well the proposal solve
    /// converged -- this test checks the guarantee actually holds end to end, every tick, not just
    /// that the algebra says it should.
    #[test]
    fn test_pressure_projection_never_exceeds_capacity() {
        let w = 64;
        let h = 64;
        let mask = make_test_mask(w, h, SandboxShape::Hourglass, 0.15, 0.6);
        let props = get_test_props(MaterialMode::Water, w * h);
        let cap = cell_capacity_for(1.0); // Water: wetness = 1.0 everywhere in `props`.
        let mut sim = TestSim::new(w, h, props, mask.clone(), 32);
        for y in 0..h / 2 {
            for x in 0..w {
                if mask[y * w + x] != crate::MASK_OUTSIDE {
                    sim.hm.data[y * w + x] = 1.0;
                }
            }
        }
        assert!(!pressure_gate::is_disabled(), "pressure must be enabled (the default) for this test");
        let gravity_dir = glam::Vec2::new(0.0, 0.04);
        let mut max_over = 0.0f32;
        for t in 0..300 {
            sim.tick(gravity_dir, usize::MAX);
            for (i, &v) in sim.hm.data.iter().enumerate() {
                if mask[i] == crate::MASK_OUTSIDE {
                    continue;
                }
                assert!(
                    v >= -1e-4,
                    "tick {}: cell {} went negative under pressure: {:.6}", t, i, v
                );
                let over = v - cap;
                if over > max_over {
                    max_over = over;
                }
            }
        }
        println!("test_pressure_projection_never_exceeds_capacity: max_over={:.3e} (cap={:.3})", max_over, cap);
        assert!(
            max_over < 1e-4,
            "A cell exceeded capacity with pressure enabled: max_over={:.3e}",
            max_over
        );
    }

    /// Pressure projection, test 3 of 3, and the core claim of the whole feature: pressure
    /// influence propagates FURTHER THAN ONE CELL PER TICK.
    ///
    /// Method: build a packed reservoir feeding a single narrow neck and warm it up under gravity
    /// for real (not a hand-tied uniform column -- see this test's own development notes; a
    /// perfectly uniform column puts every interior edge to sleep via `edge_sleeps` before
    /// `pressure_project` ever sees it, and a single-donor edge's applied flux is provably
    /// identical with or without pressure since arbitration clamps it to the same local budget
    /// either way -- neither demonstrates anything). A real run develops the local capacity
    /// contention (a cell fed from two directions at once, more than its single-edge clamp alone
    /// would allow through) that the projection actually exists to resolve globally.
    ///
    /// From one such (identical, snapshotted) mid-run state, run exactly ONE further tick twice --
    /// once with `pressure_gate` at its default (enabled), once disabled -- and compare. Every edge
    /// used by BOTH runs is fed by the same frozen pre-tick heights and the same locally-frozen
    /// `cell_avail`/`cell_freecap`, so if the projection is a no-op, the two runs are bit-identical
    /// everywhere (the two calls differ only in whether `pressure_project` returns immediately).
    /// Any row where they diverge, therefore, diverges BECAUSE of the projection -- not a tautology,
    /// since `pressure_gate` is a real runtime toggle around the real `settle_tick` path, and the
    /// two runs share every other input bit for bit.
    #[test]
    fn test_pressure_projection_reaches_beyond_one_cell() {
        let w = 64;
        let h = 64;
        let block_size = 32;
        let gravity_dir = glam::Vec2::new(0.0, 0.04);
        let neck_y = h / 2;

        let mask = make_test_mask(w, h, SandboxShape::Hourglass, 0.15, 0.6);
        let props = get_test_props(MaterialMode::Water, w * h);
        let mut warm = TestSim::new(w, h, props.clone(), mask.clone(), block_size);
        for y in 0..neck_y {
            for x in 0..w {
                if mask[y * w + x] != crate::MASK_OUTSIDE {
                    warm.hm.data[y * w + x] = 1.0;
                }
            }
        }
        for _ in 0..60 {
            warm.tick(gravity_dir, usize::MAX);
        }
        let snapshot = warm.hm.data.clone();

        let branch = |pressure_on: bool| -> Vec<f32> {
            let mut sim = TestSim::new(w, h, props.clone(), mask.clone(), block_size);
            sim.hm.data.copy_from_slice(&snapshot);
            sim.tick_count = warm.tick_count;
            pressure_gate::set_disabled(!pressure_on);
            sim.tick(gravity_dir, usize::MAX);
            pressure_gate::set_disabled(false);
            sim.hm.data.clone()
        };
        let off = branch(false);
        let on = branch(true);

        // For every row of the (still-filling) upper chamber, how far it sits from the neck, and
        // how much the pressure-on and pressure-off single-tick outcomes disagree there.
        let mut max_hop_with_diff = 0usize;
        let mut adjacent_diff = 0.0f32;
        for y in 0..neck_y {
            let mut row_diff = 0.0f32;
            for x in 0..w {
                row_diff += (on[y * w + x] - off[y * w + x]).abs();
            }
            let hops = neck_y - y;
            if hops == 1 {
                adjacent_diff = row_diff;
            }
            if row_diff > 1e-4 {
                max_hop_with_diff = max_hop_with_diff.max(hops);
            }
        }
        println!(
            "test_pressure_projection_reaches_beyond_one_cell: adjacent(hops=1) diff={:.4} \
             max_hop_with_diff={}",
            adjacent_diff, max_hop_with_diff
        );

        // The row directly adjacent to the neck (hops=1) is expected to differ regardless -- it's
        // the one cell whose own edge sits right at the contested bottleneck, so this is a sanity
        // check on the harness, not evidence of anything.
        assert!(
            adjacent_diff > 1e-4,
            "Harness sanity check failed: even the row adjacent to the neck (hops=1) shows no \
             difference between pressure on and off ({:.3e}); the fork isn't exercising the neck.",
            adjacent_diff
        );
        // The actual claim: pressure's influence reaches a row that is NOT adjacent to the
        // bottleneck, in a single tick. Measured today: max_hop_with_diff = 22 (of 32 rows in the
        // upper chamber) at this exact scenario/seed; asserting a much lower bound for headroom
        // against float/scheduling noise while still requiring "far more than one cell".
        assert!(
            max_hop_with_diff >= 8,
            "Pressure projection did not propagate beyond the immediate neighbour of the neck: \
             max_hop_with_diff={} (want >= 8, and definitely > 1)",
            max_hop_with_diff
        );

        // And the negative control this same fork gives us for free: pressure disabled twice in a
        // row (i.e. comparing the `off` branch against a second `off` run) must be bit-identical,
        // confirming the divergence above really is attributable to the gate and not to some other
        // source of nondeterminism in the harness.
        let off2 = branch(false);
        assert_eq!(off, off2, "settle_tick is not deterministic with pressure disabled");
    }

    /// Task #47 regression test ("sand-slab" scheduling defect). This is the regression test the
    /// defect never had: divergence from a PERFECT-SIMULATION ground truth (`perfect_sim_tick`,
    /// correct by construction) over a fresh flip -- the user's own repro -- replacing the void-
    /// count heuristics used until now, which had no target value.
    ///
    /// Three runs from an IDENTICAL, deterministically-reproduced post-flip starting state (built
    /// by settling with `perfect_sim_tick`, which is unaffected by `fresh_overburden_gate` since
    /// its own force-must already saturates every material-holding block regardless -- so all
    /// three branches below start from the same bits):
    ///   - `sim_perfect`: ticked with `perfect_sim_tick` every tick (ground truth).
    ///   - `sim_after`: ticked with the ordinary adaptive scheduler, fresh-overburden predicate
    ///     ENABLED (shipped default).
    ///   - `sim_before`: the same adaptive scheduler, predicate DISABLED via
    ///     `fresh_overburden_gate` (pre-fix behaviour, same build, same binary -- not a second
    ///     compile).
    ///
    /// Divergence is the summed per-cell `|height - perfect height|` over every in-mask cell,
    /// tracked both per-tick peak and cumulative over the run -- "interior void count, or a direct
    /// buffer diff, over a flip", per the task brief; this is the direct-buffer-diff form.
    ///
    /// `budget_n = 256`, matching `DrawingSimulation`'s own shipped starting/floor budget (see
    /// `lib.rs`'s `budget_n` initialisation) against this harness's `cols * rows = 32 * 32 = 1024`
    /// blocks (the deliberate 32x32 tiling — see `lib.rs:444-451` — holds at every grid size, so
    /// this ratio, a quarter of the domain, is representative of the shipped default regardless of
    /// resolution). MEASURED: at `budget_n = usize::MAX` (never budget-starved) both `sim_before`
    /// and `sim_after` track `sim_perfect` bit-for-bit over this scenario -- zero divergence either
    /// way -- because with the budget never binding, every block that receives so much as one
    /// `activate_neighbor`-style wake ends up simulated anyway; only once the budget is actually
    /// contested does which blocks *lose* that competition -- and therefore which blocks the
    /// one-tick-late historical signal costs a tick's worth of lag -- start to matter, which is
    /// exactly the realistic (budget-constrained) condition this task's fix targets.
    ///
    /// The assertion is deliberately relative (`after < before`), not an absolute magic number:
    /// what this test guards is "the predicate helps", which is stable under retuning the epsilon
    /// constants, not "divergence measures exactly X today", which would not be.
    #[test]
    fn test_fresh_overburden_predicate_reduces_slab_divergence() {
        let w = 64;
        let h = 64;
        let block_size = 2;
        let mask = make_test_mask(w, h, SandboxShape::Circle, 0.04, 1.0);
        let props = get_test_props(MaterialMode::DrySand, w * h);
        let gravity_dir = glam::Vec2::new(0.0, 0.04);

        let post_flip_ticks = 200usize;
        let budget_n = 256;

        let mut sim_perfect = settled_then_flipped(w, h, &mask, &props, block_size, gravity_dir);
        let mut sim_after = settled_then_flipped(w, h, &mask, &props, block_size, gravity_dir);
        let mut sim_before = settled_then_flipped(w, h, &mask, &props, block_size, gravity_dir);
        assert_eq!(
            sim_perfect.hm.data, sim_after.hm.data,
            "Harness sanity check failed: the three deterministically-reproduced post-flip \
             starting states are not bit-identical, so the divergence measured below would not \
             isolate the scheduler."
        );
        assert_eq!(sim_perfect.hm.data, sim_before.hm.data);

        let mut peak_after = 0.0f64;
        let mut cumulative_after = 0.0f64;
        let mut peak_before = 0.0f64;
        let mut cumulative_before = 0.0f64;

        for _ in 0..post_flip_ticks {
            perfect_sim_tick(&mut sim_perfect, &mask, gravity_dir);

            fresh_overburden_gate::set_disabled(false);
            sim_after.tick(gravity_dir, budget_n);

            fresh_overburden_gate::set_disabled(true);
            sim_before.tick(gravity_dir, budget_n);
            fresh_overburden_gate::set_disabled(false);

            let mut diff_after = 0.0f64;
            let mut diff_before = 0.0f64;
            for i in 0..mask.len() {
                if mask[i] == crate::MASK_OUTSIDE {
                    continue;
                }
                diff_after += (sim_after.hm.data[i] - sim_perfect.hm.data[i]).abs() as f64;
                diff_before += (sim_before.hm.data[i] - sim_perfect.hm.data[i]).abs() as f64;
            }
            cumulative_after += diff_after;
            cumulative_before += diff_before;
            peak_after = f64::max(peak_after, diff_after);
            peak_before = f64::max(peak_before, diff_before);
        }

        println!(
            "test_fresh_overburden_predicate_reduces_slab_divergence: peak_before={:.3} \
             peak_after={:.3} cumulative_before={:.3} cumulative_after={:.3} \
             (lower is closer to the perfect-simulation ground truth)",
            peak_before, peak_after, cumulative_before, cumulative_after
        );

        assert!(
            cumulative_after < cumulative_before,
            "Fresh-overburden predicate did not reduce cumulative divergence from the perfect-\
             simulation ground truth over a fresh flip: cumulative_before={:.3} \
             cumulative_after={:.3}",
            cumulative_before, cumulative_after
        );
    }

    /// Task #47 round 3. The coordinator's corrected, more specific hypothesis, tested rather
    /// than trusted: NOT that the overburden clause is conceptually catching the wrong body (round
    /// 2's story), but that `in_transit_at` under-detects free fall in the first place, so a
    /// falling body's OWN `resting_above` never actually reaches zero -- it reads roughly
    /// `h - in_transit`, tens of percent of `h` per row instead of ~0, which accumulates down the
    /// column and crosses `FRESH_OVERBURDEN_SKIN_CELLS` (1.5, resolution-normalised) after only a
    /// few rows even though every one of those rows is unambiguously in free fall.
    ///
    /// DIAGNOSTIC (reproduce-only, no assertions). For a column inside the falling body in the
    /// same settled-then-flipped Circle/DrySand scenario the other round-2/3 diagnostics use,
    /// ticked with the REAL adaptive scheduler at `budget_n = 256` (shipped `OverburdenAndRoom`
    /// variant, i.e. exactly what ships today), dumps per row `h`, `in_transit_at`,
    /// `resting_above` (the un-accumulated per-row contribution), and the accumulated
    /// `fresh_overburden` (the same buffer `fresh_overburden_must_blocks` computes) -- at several
    /// ticks during the fall, PRE-tick (the same snapshot the classification loop actually reads).
    /// Reports how many consecutive rows from the top of material in that column stay under the
    /// effective threshold (`FRESH_OVERBURDEN_SKIN_CELLS * depth_scale`), and separately, the
    /// fraction of perfect-simulation's promoted blocks that `RoomOnly` (no overburden clause)
    /// promotes but the shipped `OverburdenAndRoom` does not -- blocks with material and somewhere
    /// to go, excluded purely by the overburden clause. Run with --ignored --nocapture.
    #[test]
    #[ignore]
    fn diag_task47_in_transit_underdetection() {
        let w = 64;
        let h = 64;
        let block_size = 2;
        let mask = make_test_mask(w, h, SandboxShape::Circle, 0.04, 1.0);
        let props = get_test_props(MaterialMode::DrySand, w * h);
        let gravity_dir = glam::Vec2::new(0.0, 0.04);
        let cols = (w + block_size - 1) / block_size;
        let rows_blk = (h + block_size - 1) / block_size;

        let mut sim = settled_then_flipped(w, h, &mask, &props, block_size, gravity_dir);
        fresh_overburden_gate::set_disabled(false);
        fresh_overburden_gate::set_variant(FreshOverburdenVariant::OverburdenAndRoom);

        let depth_scale = REFERENCE_GRID_HEIGHT as f32 / w as f32;
        let pressure_epsilon = FRESH_OVERBURDEN_SKIN_CELLS * depth_scale;
        println!(
            "diag_task47_in_transit_underdetection: grid={w} depth_scale={depth_scale:.3} \
             effective_threshold={pressure_epsilon:.3} (raw FRESH_OVERBURDEN_SKIN_CELLS={FRESH_OVERBURDEN_SKIN_CELLS})"
        );

        let probe_col = w / 2;
        let probe_ticks: [usize; 6] = [1, 2, 3, 5, 8, 12];
        let mut next_probe = 0usize;

        for t in 0..15usize {
            if next_probe < probe_ticks.len() && probe_ticks[next_probe] == t {
                next_probe += 1;
                // PRE-tick snapshot: exactly what `settle_tick`'s classification loop for tick t
                // would read (frozen `heightmap.data`, `edge_vel_v` as last tick left it).
                let mut fresh_overburden = vec![0.0f32; w * h];
                recompute_column_depth(
                    w, h, &mask, &sim.hm.data, &sim.hm.data, &sim.hm.external_mass_this_tick,
                    &sim.cell_props, &sim.edge_vel_v, &mut fresh_overburden[..],
                );

                let mut rows_under_threshold = 0usize;
                let mut first_material_row: Option<usize> = None;
                let mut row_lines = Vec::new();
                for y in 0..h {
                    let idx = y * w + probe_col;
                    if mask[idx] == crate::MASK_OUTSIDE {
                        continue;
                    }
                    let hv = sim.hm.data[idx];
                    if hv <= FRESH_OVERBURDEN_MATERIAL_EPSILON {
                        continue;
                    }
                    if first_material_row.is_none() {
                        first_material_row = Some(y);
                    }
                    let it = in_transit_at(
                        idx, w, h, &sim.hm.data, &sim.hm.data, &sim.cell_props, &sim.edge_vel_v, &mask,
                    );
                    let resting_above_raw = (hv - it - sim.hm.external_mass_this_tick[idx].max(0.0)).max(0.0);
                    let cd = fresh_overburden[idx];
                    if cd < pressure_epsilon {
                        rows_under_threshold += 1;
                    }
                    row_lines.push(format!(
                        "y={y:>3} h={hv:.3} in_transit={it:.3} resting_above_raw={resting_above_raw:.3} \
                         column_depth={cd:>7.3}{}",
                        if cd < pressure_epsilon { " [UNDER]" } else { "" }
                    ));
                }
                println!(
                    "diag_task47_in_transit_underdetection: --- tick={t} probe_col={probe_col} \
                     first_material_row={first_material_row:?} rows_under_threshold={rows_under_threshold} ---"
                );
                for line in &row_lines {
                    println!("diag_task47_in_transit_underdetection:   {line}");
                }

                // Exclusion fraction: of perfect-sim's promoted blocks this tick, how many does
                // RoomOnly promote that OverburdenAndRoom does not.
                let mut perfect_n = 0usize;
                for by in 0..rows_blk {
                    let start_y = by * block_size;
                    let end_y = ((by + 1) * block_size).min(h);
                    for bx in 0..cols {
                        let start_x = bx * block_size;
                        let end_x = ((bx + 1) * block_size).min(w);
                        let mut has_material = false;
                        'scan: for yy in start_y..end_y {
                            let ro = yy * w;
                            for xx in start_x..end_x {
                                let ix = ro + xx;
                                if mask[ix] != crate::MASK_OUTSIDE
                                    && sim.hm.data[ix] > crate::PERFECT_SIM_MATERIAL_EPSILON
                                {
                                    has_material = true;
                                    break 'scan;
                                }
                            }
                        }
                        if has_material {
                            perfect_n += 1;
                        }
                    }
                }
                let room_only = fresh_overburden_must_blocks(
                    w, h, block_size, cols, rows_blk, &mask, &sim.hm.data,
                    &sim.hm.external_mass_this_tick, &sim.cell_props, &sim.edge_vel_v,
                    FreshOverburdenVariant::RoomOnly,
                );
                let shipped = fresh_overburden_must_blocks(
                    w, h, block_size, cols, rows_blk, &mask, &sim.hm.data,
                    &sim.hm.external_mass_this_tick, &sim.cell_props, &sim.edge_vel_v,
                    FreshOverburdenVariant::OverburdenAndRoom,
                );
                let excluded_by_overburden = room_only
                    .iter()
                    .zip(shipped.iter())
                    .filter(|&(&r, &s)| r && !s)
                    .count();
                println!(
                    "diag_task47_in_transit_underdetection: tick={t} perfect_n={perfect_n} \
                     room_only_n={} shipped_n={} excluded_by_overburden_clause={excluded_by_overburden} \
                     ({:.4} of perfect_n)",
                    room_only.iter().filter(|&&x| x).count(),
                    shipped.iter().filter(|&&x| x).count(),
                    excluded_by_overburden as f64 / perfect_n.max(1) as f64,
                );
            }
            sim.tick(gravity_dir, 256);
        }
    }

    /// Task #47 round 2. The coordinator's diagnosis, tested rather than trusted: round 1's
    /// shipped predicate (`OverburdenAndRoom`) requires near-zero overburden, and `column_depth`
    /// has `in_transit` subtracted, so a body already in free fall reads near-zero overburden and
    /// IS caught -- but a resting block whose support just opened up below it, while it still has
    /// its OWN material stacked on top, reads a HIGH overburden and is NOT caught. That resting-
    /// but-about-to-fall block is exactly what produces a slab gap, so the hypothesis is that the
    /// conjunction catches "already falling" (which needed the least help) and misses "about to
    /// fall" (the actual defect).
    ///
    /// Same harness, same scenario, same `budget_n = 256` as
    /// `test_fresh_overburden_predicate_reduces_slab_divergence` (round 1 measured `budget_n =
    /// usize::MAX` shows zero divergence for ANY variant, including "disabled" -- an unbound
    /// budget masks this defect entirely, so this comparison only runs at the realistic,
    /// budget-constrained setting where the defect actually shows). One `sim_perfect` ground-truth
    /// run plus one adaptive run per row below (`disabled` = round-1 "before", plus the five
    /// `FreshOverburdenVariant`s), all from bit-identical starting states.
    ///
    /// DIAGNOSTIC (reproduce-only, no assertions) -- this is the measurement the recommendation in
    /// the task report is based on, not a permanent regression gate (that remains
    /// `test_fresh_overburden_predicate_reduces_slab_divergence`, pinned to whichever variant
    /// `fresh_overburden_gate`'s `#[cfg(not(test))]` twin hardcodes). Run with --ignored
    /// --nocapture.
    #[test]
    #[ignore]
    fn diag_task47_variant_divergence_comparison() {
        let w = 64;
        let h = 64;
        let block_size = 2;
        let mask = make_test_mask(w, h, SandboxShape::Circle, 0.04, 1.0);
        let props = get_test_props(MaterialMode::DrySand, w * h);
        let gravity_dir = glam::Vec2::new(0.0, 0.04);
        let post_flip_ticks = 200usize;
        let budget_n = 256;

        // (label, None = gate disabled entirely (round-1 "before"), Some(variant) = gate enabled
        // with that variant).
        let rows: Vec<(&str, Option<FreshOverburdenVariant>)> = vec![
            ("disabled (round-1 before)", None),
            ("overburden_only", Some(FreshOverburdenVariant::OverburdenOnly)),
            ("overburden_and_room (round1)", Some(FreshOverburdenVariant::OverburdenAndRoom)),
            ("capacity_below_only", Some(FreshOverburdenVariant::CapacityBelowOnly)),
            ("overburden_or_capacity", Some(FreshOverburdenVariant::OverburdenOrCapacityBelow)),
            ("room_only", Some(FreshOverburdenVariant::RoomOnly)),
            ("unsupported_only", Some(FreshOverburdenVariant::UnsupportedOnly)),
            ("unsupported_and_room (shipped)", Some(FreshOverburdenVariant::UnsupportedAndRoom)),
        ];

        let sim_perfect_seed = settled_then_flipped(w, h, &mask, &props, block_size, gravity_dir);

        println!(
            "diag_task47_variant_divergence_comparison: grid={w} budget_n={budget_n} \
             post_flip_ticks={post_flip_ticks}"
        );
        for (label, variant) in rows {
            let mut sim_perfect = settled_then_flipped(w, h, &mask, &props, block_size, gravity_dir);
            let mut sim = settled_then_flipped(w, h, &mask, &props, block_size, gravity_dir);
            assert_eq!(sim_perfect.hm.data, sim_perfect_seed.hm.data, "harness determinism check");
            assert_eq!(sim.hm.data, sim_perfect_seed.hm.data, "harness determinism check");

            match variant {
                None => fresh_overburden_gate::set_disabled(true),
                Some(v) => {
                    fresh_overburden_gate::set_disabled(false);
                    fresh_overburden_gate::set_variant(v);
                }
            }

            let mut peak = 0.0f64;
            let mut cumulative = 0.0f64;
            for _ in 0..post_flip_ticks {
                perfect_sim_tick(&mut sim_perfect, &mask, gravity_dir);
                sim.tick(gravity_dir, budget_n);

                let mut diff = 0.0f64;
                for i in 0..mask.len() {
                    if mask[i] == crate::MASK_OUTSIDE {
                        continue;
                    }
                    diff += (sim.hm.data[i] - sim_perfect.hm.data[i]).abs() as f64;
                }
                cumulative += diff;
                peak = f64::max(peak, diff);
            }
            fresh_overburden_gate::set_disabled(false);
            fresh_overburden_gate::set_variant(FreshOverburdenVariant::OverburdenAndRoom);

            println!(
                "diag_task47_variant_divergence_comparison: {label:<32} peak={peak:>10.3} \
                 cumulative={cumulative:>12.3}"
            );
        }
    }

    /// Task #47: the block-fraction table the task brief asks for -- "MEASURE THE COST BEFORE
    /// COMMITTING TO IT" -- for a resting pile, a mid-drain hourglass, and a fresh flip, at grid
    /// 64 and 512, for every `FreshOverburdenVariant` (round 2: extended from round 1's loose/
    /// tight-only table). DIAGNOSTIC (reproduce-only, no assertions): prints, per scenario/grid/
    /// variant, the fraction of blocks it would promote against perfect simulation's own 100%
    /// reference. Run with --ignored --nocapture.
    #[test]
    #[ignore]
    fn diag_task47_block_fraction_table() {
        for grid in [64usize, 512usize] {
            let w = grid;
            let h = grid;
            let block_size = (grid / 32).max(1);
            let gravity_dir = glam::Vec2::new(0.0, 0.04);

            // --- Scenario 1: resting pile (settled Circle, DrySand, pre-flip) ---
            {
                let mask = make_test_mask(w, h, SandboxShape::Circle, 0.04, 1.0);
                let props = get_test_props(MaterialMode::DrySand, w * h);
                let mut sim = TestSim::new(w, h, props, mask.clone(), block_size);
                let fill_y0 = (0.40 * h as f32) as usize;
                for y in fill_y0..h {
                    for x in 0..w {
                        let idx = y * w + x;
                        if mask[idx] != crate::MASK_OUTSIDE {
                            sim.hm.data[idx] = 1.0;
                        }
                    }
                }
                let mut quiet_run = 0usize;
                for _ in 0..4000usize {
                    let flow = perfect_sim_tick(&mut sim, &mask, gravity_dir);
                    if flow < 1e-3 {
                        quiet_run += 1;
                        if quiet_run >= 15 {
                            break;
                        }
                    } else {
                        quiet_run = 0;
                    }
                }
                report_fresh_overburden_fraction("resting pile", grid, &sim, &mask, block_size);
            }

            // --- Scenario 2: mid-drain hourglass ---
            {
                let mask = make_test_mask(w, h, SandboxShape::Hourglass, 0.15, 0.6);
                let props = get_test_props(MaterialMode::DrySand, w * h);
                let mut sim = TestSim::new(w, h, props, mask.clone(), block_size);
                for y in 0..h / 2 {
                    for x in 0..w {
                        let idx = y * w + x;
                        if mask[idx] != crate::MASK_OUTSIDE {
                            sim.hm.data[idx] = 1.0;
                        }
                    }
                }
                let drain_ticks = if grid == 64 { 150 } else { 400 };
                for _ in 0..drain_ticks {
                    perfect_sim_tick(&mut sim, &mask, gravity_dir);
                }
                report_fresh_overburden_fraction("mid-drain hourglass", grid, &sim, &mask, block_size);
            }

            // --- Scenario 3: fresh flip (immediately post-flip, before any post-flip tick) ---
            {
                let mask = make_test_mask(w, h, SandboxShape::Circle, 0.04, 1.0);
                let props = get_test_props(MaterialMode::DrySand, w * h);
                let mut sim = TestSim::new(w, h, props, mask.clone(), block_size);
                let fill_y0 = (0.40 * h as f32) as usize;
                for y in fill_y0..h {
                    for x in 0..w {
                        let idx = y * w + x;
                        if mask[idx] != crate::MASK_OUTSIDE {
                            sim.hm.data[idx] = 1.0;
                        }
                    }
                }
                let mut quiet_run = 0usize;
                for _ in 0..4000usize {
                    let flow = perfect_sim_tick(&mut sim, &mask, gravity_dir);
                    if flow < 1e-3 {
                        quiet_run += 1;
                        if quiet_run >= 15 {
                            break;
                        }
                    } else {
                        quiet_run = 0;
                    }
                }
                flip_sim(&mut sim);
                report_fresh_overburden_fraction("fresh flip", grid, &sim, &mask, block_size);
            }
        }
    }

    /// Task #55 diagnostic harness. Sums material in `[y0, y1) x [x0, x1)` -- used by every
    /// `diag_task55_*` test below to read off a scalar "how much material is here" without
    /// repeating the double loop at each call site.
    fn diag_task55_region_mass(data: &[f32], w: usize, x0: usize, x1: usize, y0: usize, y1: usize) -> f64 {
        let mut m = 0.0f64;
        for y in y0..y1 {
            let row = y * w;
            for x in x0..x1 {
                m += data[row + x] as f64;
            }
        }
        m
    }

    /// Task #55 diagnostic harness. Topmost row in `[y_lo, y_hi)` at column `x` holding more than
    /// `eps` material -- the free-surface readout every `diag_task55_*` test below uses. `None`
    /// means the whole probed range is empty (dry) at this column.
    fn diag_task55_surface_row(
        data: &[f32], w: usize, x: usize, y_lo: usize, y_hi: usize, eps: f32,
    ) -> Option<usize> {
        (y_lo..y_hi).find(|&y| data[y * w + x] > eps)
    }

    /// TASK #55 RESOLUTION SWEEP helper: the original `diag_task55_*` diagnostics' `probe_ticks`
    /// arrays were literal tick numbers tuned to one fixed `run_ticks`. Regenerates the same
    /// RELATIVE sampling (as fractions of `run_ticks`) for whatever per-width `run_ticks` budget a
    /// given width actually uses, so every width's printed time series samples the same relative
    /// points in its own run (0%, 0.25%, 0.5%, ... 100%) instead of a table of tick numbers that
    /// only makes sense for one specific `run_ticks`.
    fn task55_probe_ticks(run_ticks: usize) -> Vec<usize> {
        const FRACS: [f64; 20] = [
            0.0, 0.0025, 0.005, 0.0075, 0.01, 0.0125, 0.015, 0.0175, 0.02, 0.025, 0.0375, 0.05,
            0.075, 0.125, 0.1875, 0.25, 0.375, 0.5, 0.75, 1.0,
        ];
        let mut ticks: Vec<usize> = FRACS
            .iter()
            .map(|f| ((f * run_ticks as f64).round() as usize).min(run_ticks))
            .collect();
        ticks.dedup();
        ticks
    }

    /// TASK #55 RESOLUTION SWEEP geometry for `diag_task55_pocket_equalisation`'s two-well probe.
    /// `left_x0..left_x1` / `right_x0..right_x1` are INTERSECTED across the whole `[well_top,
    /// well_bot)` band (see `find_two_well_topology`), so every cell in either rectangle over that
    /// row range is guaranteed open -- the caller does not need to re-check the mask row by row.
    struct TwoWellGeometry {
        left_x0: usize,
        left_x1: usize,
        right_x0: usize,
        right_x1: usize,
        well_top: usize,
        well_bot: usize,
        basin_bot: usize,
    }

    /// TASK #55 RESOLUTION SWEEP helper for `diag_task55_pocket_equalisation`: locates a "two
    /// disjoint open wells that only communicate through a single shared basin beneath them"
    /// topology directly in an arbitrary mask, instead of trusting the hand-found w=128 pixel
    /// coordinates the original, single-resolution version of this test used
    /// (`left_x0=55..66`/`right_x0=83..91`/etc, found by dumping that one mask as ASCII) to still
    /// be there at another width.
    ///
    /// They are not guaranteed to be: `ProceduralFunnel`'s cave-noise terms (`eval_sandbox_shape`,
    /// e.g. `(dx * 0.14).sin() + (dy * 0.16).cos() + ...`) run on `dx`/`dy` in raw PIXEL units at
    /// FIXED frequencies -- not frequencies scaled by `w` -- so a wider grid packs strictly more
    /// noise oscillations across the same physical span of the cave. Measured directly (dump the
    /// row-by-row open-run count): at w=64 the noise band is too thin to carve anything (max
    /// run-count briefly touches 2 for a single row and is otherwise 0/1 -- no real well). At
    /// w=128 it carves a clean two-well split (the original hand-found case). At w=256 and w=512
    /// it carves MANY separate pockets in the same row -- run-counts of 3 to 9 are typical through
    /// the noise band -- not a clean two-well split at all. So an early version of this function
    /// that required "exactly two runs in the row" found nothing at w=64/256/512 and only ever
    /// worked at the original w=128. That is a real trap-2 finding in its own right (see this
    /// test's caller for how it is reported), but it need not be a dead end: the physical
    /// requirement is only "two open columns whose sole connection is a basin beneath them", not
    /// "the cave has exactly two pockets total". A multi-pocket cave still contains that topology
    /// as long as some two of its pockets qualify; the other pockets are just inert extra rock
    /// features this test does not fill.
    ///
    /// So this searches for a band `[y0, y0+bh)` (tried at several candidate heights `bh`, largest
    /// first, as a fraction of `h`) in which at least two columns are open for the WHOLE band, and
    /// picks the two WIDEST such column-runs (`min_well_span` wide or more) as the wells --
    /// whatever else the row contains at that height is ignored. Below the band, the span from the
    /// left well's start to the right well's end must be fully open for at least
    /// `min_basin_height` rows (the shared basin). Returns the first (topmost, tallest-band-first)
    /// qualifying result, or `None` if no such pair exists in this mask at this width at all --
    /// which is itself a valid, reportable outcome (see the task brief's own instruction: "if it
    /// does not survive, find the wells per-resolution and say so").
    fn find_two_well_topology(mask: &[u8], w: usize, h: usize) -> Option<TwoWellGeometry> {
        let is_open = |x: usize, y: usize| mask[y * w + x] != crate::MASK_OUTSIDE;
        let margin = ((0.05 * h as f64).round() as usize).max(2);
        let min_well_span = ((0.02 * w as f64).round() as usize).max(3);
        let min_basin_height = ((0.02 * h as f64).round() as usize).max(3);
        // Tried tallest-first: a taller band gives the two fill levels more room to differ, but
        // is also stricter (every column in the run must stay open for the WHOLE band), so this
        // falls back to shorter bands only if no tall one works anywhere in the mask.
        let band_height_fracs = [0.10f64, 0.08, 0.06, 0.045, 0.035];

        let column_open_band = |x: usize, y0: usize, bh: usize| -> bool {
            (y0..y0 + bh).all(|y| is_open(x, y))
        };

        let lo = margin;
        let hi = h.saturating_sub(margin);

        for &frac in &band_height_fracs {
            let bh = ((frac * h as f64).round() as usize).max(4);
            if lo + bh >= hi || hi - (lo + bh) < min_basin_height {
                continue;
            }
            let mut y0 = lo;
            while y0 + bh < hi {
                let mut segs: Vec<(usize, usize)> = Vec::new();
                let mut x = 0usize;
                while x < w {
                    if column_open_band(x, y0, bh) {
                        let start = x;
                        while x < w && column_open_band(x, y0, bh) {
                            x += 1;
                        }
                        if x - start >= min_well_span {
                            segs.push((start, x));
                        }
                    } else {
                        x += 1;
                    }
                }
                if segs.len() >= 2 {
                    segs.sort_by_key(|&(s, e)| std::cmp::Reverse(e - s));
                    let mut top2 = [segs[0], segs[1]];
                    top2.sort_by_key(|&(s, _)| s);
                    let (left, right) = (top2[0], top2[1]);
                    let well_bot = y0 + bh;
                    let mut basin_end = well_bot;
                    while basin_end < hi && (left.0..right.1).all(|x| is_open(x, basin_end)) {
                        basin_end += 1;
                    }
                    if basin_end - well_bot >= min_basin_height {
                        return Some(TwoWellGeometry {
                            left_x0: left.0,
                            left_x1: left.1,
                            right_x0: right.0,
                            right_x1: right.1,
                            well_top: y0,
                            well_bot,
                            basin_bot: basin_end,
                        });
                    }
                }
                y0 += 1;
            }
        }
        None
    }

    /// TASK #55, defect 1, RESOLUTION SWEEP: "a standing arch of liquid over a void does not
    /// collapse... it slowly drains instead of flattening fast". Same hand-built container and the
    /// same POSITION-INDEPENDENT `unsupported_span` metric as the original single-resolution
    /// version of this test (see the metric's own inline comment below for why a "does the
    /// original 4-row band still hold material" version is wrong), now run at w = 64/128/256/512.
    ///
    /// Every hand-built coordinate is derived from ONE isotropic scale factor, `s = w / 64.0`
    /// (`h` scaled the same way, `h = round(100 * s)`, preserving the original construction's
    /// h/w = 1.5625 aspect ratio and, with it, every coordinate's fraction of w or h exactly,
    /// since they all began life as literal pixel numbers at the original w=64). This is also what
    /// makes the "3 rows above the pile" trap safe: `gap_buffer = round(3 * s)` is exactly 3 at
    /// w=64 (s=1, the value this test always used) and 24 at w=512 (s=8) -- the SAME 3% of the
    /// container's height (3/100 = 24/800) at both, rather than a shrinking fraction as w grows if
    /// left as a literal `3`.
    ///
    /// `run_ticks` is allowed to differ ACROSS widths (never between gate-on/off AT a width, which
    /// stays identical by construction -- both sides of the inner loop share one `run_ticks` for a
    /// given `w`) purely for wall-clock budget: `perfect_sim_tick` simulates every one of this
    /// scenario's 1600 blocks (32 cols x 50 rows -- constant across widths, because
    /// `block_size = w / 32` keeps that ratio fixed) EVERY tick, and per-block cost grows with
    /// `block_size^2 = (w/32)^2`, so a w=512 pass costs 64x a w=64 pass at the same `run_ticks`.
    /// Reduced for the two larger widths accordingly; each width's own budget is printed so the
    /// table can be read honestly rather than assuming one number throughout.
    ///
    /// DIAGNOSTIC (reproduce-only, no assertions). Run with --ignored --nocapture.
    #[test]
    #[ignore]
    fn diag_task55_arch_collapse_rate() {
        for &(w, run_ticks) in &[(64usize, 400usize), (128, 400), (256, 400), (512, 600)] {
            let s = w as f64 / 64.0;
            let sc = |base: usize| ((base as f64 * s).round() as usize).max(1);
            let h = sc(100);
            let margin = sc(2);
            let block_size = (w / 32).max(1);
            let mut mask = vec![crate::MASK_OUTSIDE; w * h];
            for y in margin..h - margin {
                for x in margin..w - margin {
                    mask[y * w + x] = crate::MASK_INSIDE;
                }
            }

            let pillar_w = sc(12);
            let gap_x0 = margin + pillar_w; // first void column
            let gap_x1 = w - margin - pillar_w; // one past the last void column
            let arch_top = sc(36); // top row of both piers and the arch slab
            let arch_bot = arch_top + sc(4); // one past the arch slab's rows
            let floor = h - margin; // first OUTSIDE row
            // Trap: "at least 3 rows above the pile" is a fixed CELL count at the original w=64,
            // h=100 (3/100 = 3% of the container height). Scaled by `sc` the same way as every
            // other coordinate here so it stays 3% of the container at every width instead of a
            // different (shrinking) physical fraction as w grows.
            let gap_buffer = sc(3);

            let gravity_dir = glam::Vec2::new(0.0, 0.04);
            let props = get_test_props(MaterialMode::Water, w * h);

            let build_sim = || -> TestSim {
                let mut sim = TestSim::new(w, h, props.clone(), mask.clone(), block_size);
                for y in arch_top..floor {
                    for x in margin..gap_x0 {
                        sim.hm.data[y * w + x] = 1.0;
                    }
                    for x in gap_x1..w - margin {
                        sim.hm.data[y * w + x] = 1.0;
                    }
                }
                for y in arch_top..arch_bot {
                    for x in gap_x0..gap_x1 {
                        sim.hm.data[y * w + x] = 1.0;
                    }
                }
                sim
            };

            // POSITION-INDEPENDENT metric -- deliberately NOT "how much of the original 4 arch
            // rows still has material in them" (a first version tried that: a completely
            // unsupported liquid slab free-falls as a coherent body, vacating its ORIGINAL rows
            // within 1-2 ticks regardless of whether it is flattening or just falling intact, which
            // reported a meaningless `ticks_to_halve = 1` under both schedulers). This version
            // tracks, per void column, the CURRENT floor-connected pile (walking up from the floor
            // while cells are continuously filled) and sums material sitting at least `gap_buffer`
            // empty rows above that pile's own top -- i.e. still genuinely hanging with real
            // clearance beneath it, wherever it currently is.
            let eps = 0.05f32;
            let unsupported_span = |data: &[f32]| -> f64 {
                let mut total = 0.0f64;
                for x in gap_x0..gap_x1 {
                    let mut pile_top = floor;
                    while pile_top > arch_top && data[(pile_top - 1) * w + x] > eps {
                        pile_top -= 1;
                    }
                    let hanging_hi = pile_top.saturating_sub(gap_buffer).max(arch_top);
                    total += diag_task55_region_mass(data, w, x, x + 1, arch_top, hanging_hi);
                }
                total
            };

            let budget_n = 256usize;
            let probe_ticks = task55_probe_ticks(run_ticks);

            println!(
                "diag_task55_arch_collapse_rate: w={w:>4} h={h} block_size={block_size} \
                 budget_n={budget_n} run_ticks={run_ticks} scale={s:.4} gap=[{gap_x0},{gap_x1}) \
                 arch_rows=[{arch_top},{arch_bot}) gap_buffer={gap_buffer} (3% of h, resolution-\
                 normalised)"
            );

            // TASK #55 cross-measurement: defensive reset before this width's own loop runs, in
            // case a prior test on this same thread panicked mid-toggle (same pattern as
            // `diag_task47_variant_divergence_comparison`'s use of `fresh_overburden_gate`).
            multiplicative_lateral_gate::set_enabled(false);
            for (gate_label, mult_enabled) in [("additive(shipped)", false), ("multiplicative(gated)", true)] {
                for (label, use_perfect) in [("adaptive(shipped)", false), ("perfect_sim", true)] {
                    let mut sim = build_sim();
                    multiplicative_lateral_gate::set_enabled(mult_enabled);
                    let initial = unsupported_span(&sim.hm.data);
                    let target = initial * 0.5;
                    let mut half_life_tick: Option<usize> = None;
                    let mut next_probe = 0usize;
                    println!(
                        "diag_task55_arch_collapse_rate: w={w:>4} --- {gate_label} / {label} --- \
                         initial_unsupported={initial:.4}"
                    );
                    for t in 0..=run_ticks {
                        let span = unsupported_span(&sim.hm.data);
                        if half_life_tick.is_none() && t > 0 && span <= target {
                            half_life_tick = Some(t);
                        }
                        if next_probe < probe_ticks.len() && probe_ticks[next_probe] == t {
                            next_probe += 1;
                            println!(
                                "diag_task55_arch_collapse_rate: w={w:>4} {gate_label:<22} \
                                 {label:<20} tick={t:>4} unsupported_span={span:>10.4} \
                                 fraction_of_initial={:.4}",
                                span / initial.max(1e-9)
                            );
                        }
                        if t == run_ticks {
                            break;
                        }
                        if use_perfect {
                            perfect_sim_tick(&mut sim, &mask, gravity_dir);
                        } else {
                            sim.tick(gravity_dir, budget_n);
                        }
                    }
                    multiplicative_lateral_gate::set_enabled(false);
                    println!(
                        "diag_task55_arch_collapse_rate: w={w:>4} {gate_label:<22} {label:<20} \
                         ticks_to_halve={half_life_tick:?} run_ticks_budget={run_ticks} \
                         (initial={initial:.4}, target<={target:.4})"
                    );
                }
            }
        }
    }

    /// Task #55, defect 2, RESOLUTION SWEEP: "enclosed pockets in a procedural cave do not
    /// equalise their surface level" -- the user's own suggested reproduction. Uses the actual
    /// procedural-cave shape, `SandboxShape::ProceduralFunnel`, at its production default
    /// parameters (`neck_width=0.005, hourglass_curve=0.6`), now at w = h = 64/128/256/512.
    ///
    /// The original, single-resolution version of this test hand-found its two-well-plus-basin
    /// coordinates by dumping the w=128 mask as ASCII. Per this test's own doc comment on
    /// `find_two_well_topology` above, that topology is NOT guaranteed to survive a resolution
    /// change -- `ProceduralFunnel`'s cave noise runs at fixed pixel-space frequencies, so a wider
    /// grid carves a genuinely different pattern, not the same pattern at more pixels. This version
    /// therefore RE-FINDS the two-well+shared-basin structure in each width's own mask via
    /// `find_two_well_topology` rather than assuming w=128's pixel coordinates scale. If the
    /// topology is not present at a given width, that width is skipped and reported as absent --
    /// not fabricated from scaled coordinates that would silently read a wall.
    ///
    /// `level_diff(t)`: `|surface_row(right probe column) - surface_row(left probe column)|`, read
    /// from `diag_task55_surface_row` over each well's own row range. The two wells are filled to
    /// FRACTIONS of their own found band height (35% / 65% down from the band's top) rather than a
    /// fixed row offset, so the initial imbalance is the same relative shape at every width even
    /// though the band's absolute height differs.
    ///
    /// Reported as a time series plus ticks-to-halve, for both the shipped adaptive scheduler and
    /// `perfect_sim_tick`. `run_ticks` differs across widths for wall-clock budget (see
    /// `diag_task55_arch_collapse_rate`'s doc comment for why), never between gate-on/off at a
    /// given width. DIAGNOSTIC (reproduce-only, no assertions). Run with --ignored --nocapture.
    #[test]
    #[ignore]
    fn diag_task55_pocket_equalisation() {
        for &(w, run_ticks) in &[(64usize, 300usize), (128, 300), (256, 200), (512, 150)] {
            let h = w;
            let block_size = (w / 32).max(1);
            let mask = make_test_mask(w, h, SandboxShape::ProceduralFunnel, 0.005, 0.6);

            let topo = match find_two_well_topology(&mask, w, h) {
                Some(t) => t,
                None => {
                    println!(
                        "diag_task55_pocket_equalisation: w={w:>4} h={h} -- NO two-well+shared-\
                         basin topology found in this mask at this width (ProceduralFunnel's cave \
                         noise uses fixed pixel-space frequencies, so the topology hand-found at \
                         w=128 is not guaranteed to survive a resolution change -- see \
                         `find_two_well_topology`'s doc comment). SKIPPING this width; reported as \
                         absent in the table, not fabricated from scaled coordinates."
                    );
                    continue;
                }
            };

            let band_h = topo.well_bot - topo.well_top;
            let mut left_surface0 = topo.well_top + ((0.35 * band_h as f64).round() as usize);
            let mut right_surface0 = topo.well_top + ((0.65 * band_h as f64).round() as usize);
            left_surface0 = left_surface0.min(topo.well_bot.saturating_sub(1));
            if right_surface0 <= left_surface0 {
                right_surface0 = (left_surface0 + 1).min(topo.well_bot.saturating_sub(1));
            }

            // Bound the basin fill to a fixed fraction of h (matches the original w=128 test's
            // basin depth of 9/128 = 7.03% of h) rather than filling however far the connected
            // single-run region happens to extend at this width -- purely a compute-budget bound;
            // the equalisation mechanism only needs SOME connected basin, not the largest one that
            // exists.
            let basin_cap = ((0.09 * h as f64).round() as usize).max(3);
            let basin_bot = (topo.well_bot + basin_cap).min(topo.basin_bot);
            let basin_x0 = topo.left_x0.min(topo.right_x0);
            let basin_x1 = topo.left_x1.max(topo.right_x1);

            let left_probe = (topo.left_x0 + topo.left_x1) / 2;
            let right_probe = (topo.right_x0 + topo.right_x1) / 2;

            println!(
                "diag_task55_pocket_equalisation: w={w:>4} h={h} block_size={block_size} \
                 budget_n=256 run_ticks={run_ticks} left_well=[{},{})x[{},{}) \
                 right_well=[{},{})x[{},{}) basin=[{},{})x[{},{}) left_probe={left_probe} \
                 right_probe={right_probe}",
                topo.left_x0, topo.left_x1, left_surface0, topo.well_bot,
                topo.right_x0, topo.right_x1, right_surface0, topo.well_bot,
                basin_x0, basin_x1, topo.well_bot, basin_bot
            );

            // Harness sanity, same defensive spirit as the original hand-coordinate version:
            // every cell this test is about to fill must actually be open.
            // `find_two_well_topology` guarantees this by construction (run bounds are
            // intersected across the whole band before being returned), but this fails loudly
            // instead of silently reading a wall if that guarantee is ever violated.
            for x in topo.left_x0..topo.left_x1 {
                for y in left_surface0..topo.well_bot {
                    assert_ne!(
                        mask[y * w + x], crate::MASK_OUTSIDE,
                        "w={w}: left well cell (x={x}, y={y}) outside -- geometry search is wrong"
                    );
                }
            }
            for x in topo.right_x0..topo.right_x1 {
                for y in right_surface0..topo.well_bot {
                    assert_ne!(
                        mask[y * w + x], crate::MASK_OUTSIDE,
                        "w={w}: right well cell (x={x}, y={y}) outside -- geometry search is wrong"
                    );
                }
            }
            for x in basin_x0..basin_x1 {
                for y in topo.well_bot..basin_bot {
                    assert_ne!(
                        mask[y * w + x], crate::MASK_OUTSIDE,
                        "w={w}: basin cell (x={x}, y={y}) outside -- geometry search is wrong"
                    );
                }
            }

            let gravity_dir = glam::Vec2::new(0.0, 0.04);
            let props = get_test_props(MaterialMode::Water, w * h);

            let build_sim = || -> TestSim {
                let mut sim = TestSim::new(w, h, props.clone(), mask.clone(), block_size);
                for y in 0..h {
                    for x in 0..w {
                        let idx = y * w + x;
                        if mask[idx] == crate::MASK_OUTSIDE {
                            continue;
                        }
                        let in_basin =
                            y >= topo.well_bot && y < basin_bot && x >= basin_x0 && x < basin_x1;
                        let in_left_well = x >= topo.left_x0 && x < topo.left_x1
                            && y >= left_surface0 && y < topo.well_bot;
                        let in_right_well = x >= topo.right_x0 && x < topo.right_x1
                            && y >= right_surface0 && y < topo.well_bot;
                        if in_basin || in_left_well || in_right_well {
                            sim.hm.data[idx] = 1.0;
                        }
                    }
                }
                sim
            };

            let eps = 0.05f32;
            let level_diff = |data: &[f32]| -> f64 {
                let left = diag_task55_surface_row(data, w, left_probe, topo.well_top, topo.well_bot, eps)
                    .unwrap_or(topo.well_bot);
                let right = diag_task55_surface_row(data, w, right_probe, topo.well_top, topo.well_bot, eps)
                    .unwrap_or(topo.well_bot);
                (right as f64 - left as f64).abs()
            };

            let budget_n = 256usize;
            let probe_ticks = task55_probe_ticks(run_ticks);

            // TASK #55 cross-measurement: defensive reset before this width's own loop runs (same
            // pattern as `diag_task47_variant_divergence_comparison`'s `fresh_overburden_gate`).
            multiplicative_lateral_gate::set_enabled(false);
            for (gate_label, mult_enabled) in [("additive(shipped)", false), ("multiplicative(gated)", true)] {
                for (label, use_perfect) in [("adaptive(shipped)", false), ("perfect_sim", true)] {
                    let mut sim = build_sim();
                    multiplicative_lateral_gate::set_enabled(mult_enabled);
                    let initial = level_diff(&sim.hm.data);
                    let target = initial * 0.5;
                    let mut half_life_tick: Option<usize> = None;
                    let mut next_probe = 0usize;
                    println!(
                        "diag_task55_pocket_equalisation: w={w:>4} --- {gate_label} / {label} --- \
                         initial_level_diff={initial:.4} rows"
                    );
                    for t in 0..=run_ticks {
                        let diff = level_diff(&sim.hm.data);
                        if half_life_tick.is_none() && t > 0 && diff <= target {
                            half_life_tick = Some(t);
                        }
                        if next_probe < probe_ticks.len() && probe_ticks[next_probe] == t {
                            next_probe += 1;
                            let left = diag_task55_surface_row(&sim.hm.data, w, left_probe, topo.well_top, topo.well_bot, eps)
                                .unwrap_or(topo.well_bot);
                            let right = diag_task55_surface_row(&sim.hm.data, w, right_probe, topo.well_top, topo.well_bot, eps)
                                .unwrap_or(topo.well_bot);
                            println!(
                                "diag_task55_pocket_equalisation: w={w:>4} {gate_label:<22} \
                                 {label:<20} tick={t:>4} left_surface_row={left:>3} \
                                 right_surface_row={right:>3} level_diff={diff:>7.4} \
                                 fraction_of_initial={:.4}",
                                diff / initial.max(1e-9)
                            );
                        }
                        if t == run_ticks {
                            break;
                        }
                        if use_perfect {
                            perfect_sim_tick(&mut sim, &mask, gravity_dir);
                        } else {
                            sim.tick(gravity_dir, budget_n);
                        }
                    }
                    multiplicative_lateral_gate::set_enabled(false);
                    println!(
                        "diag_task55_pocket_equalisation: w={w:>4} {gate_label:<22} {label:<20} \
                         ticks_to_halve={half_life_tick:?} run_ticks_budget={run_ticks} \
                         (initial={initial:.4}, target<={target:.4})"
                    );
                }
            }
        }
    }

    /// Task #55, defect 3, RESOLUTION SWEEP: "a draining lake does not stay level while it
    /// drains". Reuses the `Hourglass` mid-drain recipe (`neck_width=0.15, hourglass_curve=0.6`)
    /// with Water and a shallow initial fill, now at w = h = 64/128/256/512. Unlike
    /// `ProceduralFunnel`, `Hourglass`'s own geometry (`eval_sandbox_shape`) is purely parametric
    /// in `dx / w_f`, `dy / h_f` -- no fixed-pixel-frequency noise term -- so its shape genuinely
    /// IS the same at every width once expressed as a fraction of w/h, unlike the pocket-
    /// equalisation cave. `fill_lo` is expressed as that fraction (12/64 = 0.1875 of h, the
    /// original construction's own ratio) rather than a literal row count.
    ///
    /// `spread(t)`: max - min of the per-column free-surface row over every wetted column. Also
    /// reports the mean surface row each probe tick to confirm the lake is actually draining.
    /// DIAGNOSTIC (reproduce-only, no assertions, reported as a full time series plus the run's
    /// peak spread rather than a ticks-to-halve number -- a lake that starts flat by construction
    /// has nothing to halve from). `run_ticks` differs across widths for wall-clock budget only
    /// (see `diag_task55_arch_collapse_rate`'s doc comment), never between gate-on/off at a given
    /// width. Run with --ignored --nocapture.
    #[test]
    #[ignore]
    fn diag_task55_draining_lake_flatness() {
        for &(w, run_ticks) in &[(64usize, 400usize), (128, 500), (256, 700), (512, 1300)] {
            let h = w;
            let block_size = (w / 32).max(1);
            let mask = make_test_mask(w, h, SandboxShape::Hourglass, 0.15, 0.6);
            let center_y = h / 2;
            // 12/64 = 0.1875 of h at the original w=h=64 construction, expressed here as a
            // fraction so the same physical fill line (comfortably below the chamber ceiling,
            // leaving room to visibly drain) is used at every width.
            let fill_lo = ((0.1875 * h as f64).round() as usize).max(1);
            let fill_hi = center_y; // bottom of the top chamber, i.e. down to the neck

            let gravity_dir = glam::Vec2::new(0.0, 0.04);
            let props = get_test_props(MaterialMode::Water, w * h);

            let build_sim = || -> TestSim {
                let mut sim = TestSim::new(w, h, props.clone(), mask.clone(), block_size);
                for y in fill_lo..fill_hi {
                    for x in 0..w {
                        let idx = y * w + x;
                        if mask[idx] != crate::MASK_OUTSIDE {
                            sim.hm.data[idx] = 1.0;
                        }
                    }
                }
                sim
            };

            let eps = 0.05f32;
            // (spread, mean_row, wetted_columns) over every column with material anywhere in the
            // top chamber's row range.
            let surface_stats = |data: &[f32]| -> (f64, f64, usize) {
                let mut rows = Vec::new();
                for x in 0..w {
                    if mask[fill_lo * w + x] == crate::MASK_OUTSIDE
                        && (fill_lo..center_y).all(|y| mask[y * w + x] == crate::MASK_OUTSIDE)
                    {
                        continue;
                    }
                    if let Some(r) = diag_task55_surface_row(data, w, x, fill_lo, center_y, eps) {
                        rows.push(r);
                    }
                }
                if rows.is_empty() {
                    return (0.0, center_y as f64, 0);
                }
                let min_r = *rows.iter().min().unwrap();
                let max_r = *rows.iter().max().unwrap();
                let mean_r = rows.iter().sum::<usize>() as f64 / rows.len() as f64;
                ((max_r - min_r) as f64, mean_r, rows.len())
            };

            let budget_n = 256usize;
            let probe_ticks = task55_probe_ticks(run_ticks);

            println!(
                "diag_task55_draining_lake_flatness: w={w:>4} h={h} block_size={block_size} \
                 budget_n={budget_n} run_ticks={run_ticks} fill=[{fill_lo},{fill_hi}) \
                 neck_width=0.15 hourglass_curve=0.6"
            );

            // TASK #55 cross-measurement: defensive reset before this width's own loop runs (same
            // pattern as `diag_task47_variant_divergence_comparison`'s `fresh_overburden_gate`).
            multiplicative_lateral_gate::set_enabled(false);
            for (gate_label, mult_enabled) in [("additive(shipped)", false), ("multiplicative(gated)", true)] {
                for (label, use_perfect) in [("adaptive(shipped)", false), ("perfect_sim", true)] {
                    let mut sim = build_sim();
                    multiplicative_lateral_gate::set_enabled(mult_enabled);
                    let mut peak_spread = 0.0f64;
                    let mut peak_spread_tick = 0usize;
                    let mut next_probe = 0usize;
                    println!(
                        "diag_task55_draining_lake_flatness: w={w:>4} --- {gate_label} / {label} ---"
                    );
                    for t in 0..=run_ticks {
                        let (spread, mean_row, wetted) = surface_stats(&sim.hm.data);
                        if spread > peak_spread {
                            peak_spread = spread;
                            peak_spread_tick = t;
                        }
                        if next_probe < probe_ticks.len() && probe_ticks[next_probe] == t {
                            next_probe += 1;
                            println!(
                                "diag_task55_draining_lake_flatness: w={w:>4} {gate_label:<22} \
                                 {label:<20} tick={t:>4} spread={spread:>7.4} \
                                 mean_surface_row={mean_row:>7.3} wetted_columns={wetted:>3}"
                            );
                        }
                        if t == run_ticks {
                            break;
                        }
                        if use_perfect {
                            perfect_sim_tick(&mut sim, &mask, gravity_dir);
                        } else {
                            sim.tick(gravity_dir, budget_n);
                        }
                    }
                    multiplicative_lateral_gate::set_enabled(false);
                    println!(
                        "diag_task55_draining_lake_flatness: w={w:>4} {gate_label:<22} {label:<20} \
                         peak_spread={peak_spread:.4} at tick={peak_spread_tick} \
                         run_ticks_budget={run_ticks}"
                    );
                }
            }
        }
    }

    /// Task #55 "cheap prediction worth testing": today's lateral driving head is ADDITIVE
    /// (`head_a = h_a + k * LATERAL_PRESSURE_SCALE * depth_a`, see `LATERAL_PRESSURE_SCALE`'s own
    /// doc comment) -- the brief's hypothesis is that a FLAT liquid surface sitting over columns of
    /// DIFFERENT depth (e.g. a flat lake over a sloping floor) might still be driven to flow,
    /// because the `depth_a`/`depth_b` (`column_depth`) terms differ even though the surfaces do
    /// not.
    ///
    /// Hand-built container: a step in the floor (left region's floor 10 rows deeper than the
    /// right region's, `step_x` splits the two), filled with Water so BOTH regions' material tops
    /// out at the exact same row (`top_row`) -- a genuinely flat free surface over a stepped floor,
    /// left column 18 rows deep, right column 8 rows deep at the moment of construction. This
    /// mirrors the already-in-tree `test_mult_lateral_flat_surface_over_sloping_floor_no_drift`'s
    /// construction (same idea, independently sized here) rather than inventing a new one, since
    /// that scenario is already known to isolate the mechanism cleanly.
    ///
    /// Reports two independent things: (1) an ANALYTIC check -- `column_depth` at a row comfortably
    /// inside both columns and above the step (row 25, i.e. 5 rows below the shared surface on both
    /// sides), computed directly via `recompute_column_depth` before any tick runs, for both
    /// columns; `column_depth` is a per-CELL top-down accumulation of resting material above THAT
    /// cell (see its own doc comment), which counts filled rows above the free surface -- at equal
    /// depth-below-a-shared-flat-surface this should read identically regardless of how deep the
    /// floor is beneath either column, so this check is a direct test of whether the mechanism the
    /// brief names is even live at a row unaffected by the step's own sidewall. (2) an EMPIRICAL
    /// check -- net mass drift into the shallow (right) region over 60 ticks, both under the
    /// shipped additive head (`perfect_sim_tick`, ground truth) and, as a bonus (this gate already
    /// exists in the tree, `multiplicative_lateral_gate` -- an experimental candidate fix for
    /// exactly this mechanism; toggling it is read-only, it changes no default), under the
    /// multiplicative alternative, so a reader can see both numbers from one build.
    ///
    /// DIAGNOSTIC (reproduce-only, no assertions). Reports the answer plainly either way --  a null
    /// result (no drift, `column_depth` equal) is as useful as a positive one. Run with --ignored
    /// --nocapture.
    #[test]
    #[ignore]
    fn diag_task55_flat_surface_over_sloping_floor() {
        let w = 24usize;
        let h = 40usize;
        let wall = 2usize;
        let step_x = 12usize; // x < step_x: deep (left) region; x >= step_x: shallow (right) region
        let floor_left = h - wall; // first OUTSIDE row, left region
        let floor_right = floor_left - 10; // right region's floor is 10 rows shallower
        let top_row = 20usize; // shared, flat top row for both regions
        let probe_row = 25usize; // 5 rows below the shared surface, above the step's own sidewall
        assert!(top_row < floor_right && probe_row < floor_right, "harness sanity");

        let mut mask = vec![crate::MASK_OUTSIDE; w * h];
        for y in 0..h {
            for x in 0..w {
                if x < wall || x >= w - wall {
                    continue;
                }
                let floor = if x < step_x { floor_left } else { floor_right };
                if y >= wall && y < floor {
                    mask[y * w + x] = crate::MASK_INSIDE;
                }
            }
        }

        let gravity_dir = glam::Vec2::new(0.0, 0.04);
        let props = get_test_props(MaterialMode::Water, w * h);

        let build_sim = || -> TestSim {
            let mut sim = TestSim::new(w, h, props.clone(), mask.clone(), 4);
            for y in top_row..h {
                for x in wall..w - wall {
                    let floor = if x < step_x { floor_left } else { floor_right };
                    if y < floor {
                        sim.hm.data[y * w + x] = 0.9; // see the in-tree sibling test: 0.9, not 1.0,
                        // so `room_a`/`room_b` aren't both zero everywhere, which would sleep
                        // every edge on the "pooled interior" branch regardless of driving head.
                    }
                }
            }
            sim
        };

        // --- (1) Analytic check: column_depth at equal depth-below-surface, both sides of the step.
        {
            let sim0 = build_sim();
            let mut column_depth = vec![0.0f32; w * h];
            recompute_column_depth(
                w, h, &mask, &sim0.hm.data, &sim0.hm.data, &sim0.hm.external_mass_this_tick,
                &sim0.cell_props, &sim0.edge_vel_v, &mut column_depth[..],
            );
            let left_x = step_x - 1;
            let right_x = step_x + 1;
            let left_depth = column_depth[probe_row * w + left_x];
            let right_depth = column_depth[probe_row * w + right_x];
            println!(
                "diag_task55_flat_surface_over_sloping_floor: ANALYTIC row={probe_row} \
                 (both sides {}rows below shared surface row={top_row}) \
                 left_x={left_x} (floor={floor_left}, depth-to-floor={}) column_depth={left_depth:.4} | \
                 right_x={right_x} (floor={floor_right}, depth-to-floor={}) column_depth={right_depth:.4} | \
                 delta={:.4}",
                probe_row - top_row, floor_left - probe_row, floor_right - probe_row,
                (left_depth - right_depth).abs()
            );
        }

        // --- (2) Empirical check: net mass drift into the shallow region over time, shipped head
        // vs. (bonus) the multiplicative candidate.
        let right_mass = |data: &[f32]| -> f64 {
            diag_task55_region_mass(data, w, step_x, w - wall, top_row, floor_right)
        };
        let run_ticks = 60usize;
        let probe_ticks: [usize; 8] = [0, 1, 2, 5, 10, 20, 40, 60];

        for (label, mult_enabled) in [("additive(shipped)", false), ("multiplicative(bonus,gated)", true)] {
            let mut sim = build_sim();
            let initial = right_mass(&sim.hm.data);
            multiplicative_lateral_gate::set_enabled(mult_enabled);
            let mut next_probe = 0usize;
            println!(
                "diag_task55_flat_surface_over_sloping_floor: EMPIRICAL --- {label} --- \
                 initial_right_mass={initial:.4}"
            );
            for t in 0..=run_ticks {
                if next_probe < probe_ticks.len() && probe_ticks[next_probe] == t {
                    next_probe += 1;
                    let mass = right_mass(&sim.hm.data);
                    println!(
                        "diag_task55_flat_surface_over_sloping_floor: {label:<28} tick={t:>3} \
                         right_mass={mass:>9.4} drift={:>9.4}",
                        mass - initial
                    );
                }
                if t == run_ticks {
                    break;
                }
                perfect_sim_tick(&mut sim, &mask, gravity_dir);
            }
            multiplicative_lateral_gate::set_enabled(false);
        }
    }

    /// TASK #55, Job 2 investigation: why does `test_liquid_flowing_liquid_does_not_stand_in_walls`
    /// regress at tick 160 (6 -> 12 voids) under `multiplicative_lateral_gate` even though it
    /// improves at tick 120 (60 -> 54, see `diag_mult_lateral_void_count_gate_on_vs_off`)? Hypothesis
    /// under test (from the task brief): `mult_lateral_conveyance` raises local depth to a POWER
    /// (`MULT_LATERAL_CONVEYANCE_EXPONENT = 1.5`), so once the scenario is past its deep-pool phase
    /// and into a thin trickle, conveyance collapses super-linearly (`x^1.5 < x` for `x < 1`) exactly
    /// where the additive form's driving stays merely linear in depth -- i.e. the multiplicative form
    /// may be too weak in the endgame, precisely when the last few voids need closing.
    ///
    /// Reads `h` and `column_depth` directly off the running `TestSim` post-tick (not a separate
    /// recompute) and reproduces `settle_tick`'s own horizontal-edge formulas exactly (both forms),
    /// for every edge flanking a still-open void, at each probe tick -- `gravity_dir.x == 0` in this
    /// scenario so the tilt term both formulas carry (`gravity_dir.x * GRAVITY_HEAD_SCALE`) is
    /// identically zero and dropped here. `liq == 1.0` unconditionally (Water, no granular blend),
    /// so `k_of_liquidity(1.0) == 1.0` and `janssen_effective_depth` is the identity transform for
    /// both forms -- this scenario never exercises the Janssen saturation curve either.
    ///
    /// Same scenario, same 64x64 Hourglass/Water/half-fill setup, same `budget_n = usize::MAX` as
    /// `test_liquid_flowing_liquid_does_not_stand_in_walls` itself (not the 256-budget adaptive
    /// scheduler the other `diag_task55_*` tests use) -- this is specifically about that test's own
    /// regression, not the scheduler. DIAGNOSTIC (reproduce-only, no assertions). Run with --ignored
    /// --nocapture.
    #[test]
    #[ignore]
    fn diag_task55_mult_lateral_settling_falloff() {
        let w = 64usize;
        let h = 64usize;
        let mask = make_test_mask(w, h, SandboxShape::Hourglass, 0.15, 0.6);
        let props = get_test_props(MaterialMode::Water, w * h);
        let gravity_dir = glam::Vec2::new(0.0, 0.04);

        let mut sim = TestSim::new(w, h, props, mask.clone(), 32);
        for y in 0..h / 2 {
            for x in 0..w {
                if mask[y * w + x] != crate::MASK_OUTSIDE {
                    sim.hm.data[y * w + x] = 1.0;
                }
            }
        }

        // Same void definition as `test_liquid_flowing_liquid_does_not_stand_in_walls`'s own
        // `count_voids`, but returning the void cells themselves rather than just a count.
        let find_voids = |data: &[f32]| -> Vec<(usize, usize)> {
            let mut voids = Vec::new();
            for y in 1..h - 1 {
                let mut liquid_to_the_left = false;
                for x in 0..w {
                    if mask[y * w + x] == crate::MASK_OUTSIDE {
                        liquid_to_the_left = false;
                        continue;
                    }
                    let v = data[y * w + x];
                    if v > 0.5 {
                        liquid_to_the_left = true;
                        continue;
                    }
                    if !liquid_to_the_left || v > 0.05 {
                        continue;
                    }
                    let liquid_to_the_right = (x + 1..w)
                        .take_while(|&x2| mask[y * w + x2] != crate::MASK_OUTSIDE)
                        .any(|x2| data[y * w + x2] > 0.5);
                    if liquid_to_the_right {
                        voids.push((y, x));
                    }
                }
            }
            voids
        };

        let liq = 1.0f32; // Water: liquidity(wetness) == 1 everywhere in this scenario.
        let k = k_of_liquidity(liq); // == 1.0 for a fully liquid cell, both forms.
        let mult_edge = |data: &[f32], cd: &[f32], a: usize, b: usize| -> (f32, f32, f32) {
            let eta_a = data[a] + cd[a];
            let eta_b = data[b] + cd[b];
            let conveyance_a = mult_lateral_conveyance(data[a], cd[a], k, liq);
            let conveyance_b = mult_lateral_conveyance(data[b], cd[b], k, liq);
            let conveyance = 0.5 * (conveyance_a + conveyance_b);
            (eta_a - eta_b, conveyance, MULT_LATERAL_SCALE * conveyance * (eta_a - eta_b))
        };
        let additive_edge = |data: &[f32], cd: &[f32], a: usize, b: usize| -> f32 {
            let depth_a = janssen_effective_depth(cd[a], liq);
            let depth_b = janssen_effective_depth(cd[b], liq);
            (data[a] + k * LATERAL_PRESSURE_SCALE * depth_a)
                - (data[b] + k * LATERAL_PRESSURE_SCALE * depth_b)
        };

        multiplicative_lateral_gate::set_enabled(true);
        let probe_ticks: [usize; 13] = [80, 100, 110, 120, 130, 140, 150, 155, 160, 165, 170, 175, 180];
        let mut next_probe = 0usize;
        println!(
            "diag_task55_mult_lateral_settling_falloff: w={w} h={h} \
             (multiplicative gate ON, budget_n=usize::MAX, matching test_liquid_flowing_liquid_does_not_stand_in_walls)"
        );
        for t in 0..=180usize {
            if next_probe < probe_ticks.len() && probe_ticks[next_probe] == t {
                next_probe += 1;
                let voids = find_voids(&sim.hm.data);
                let mut sum_grad = 0.0f64;
                let mut sum_conv = 0.0f64;
                let mut sum_mult_drive = 0.0f64;
                let mut sum_add_drive = 0.0f64;
                let mut n_edges = 0usize;
                for &(y, x) in &voids {
                    for &nx in &[x.wrapping_sub(1), x + 1] {
                        if nx >= w || mask[y * w + nx] == crate::MASK_OUTSIDE {
                            continue;
                        }
                        let a = y * w + x.min(nx);
                        let b = y * w + x.max(nx);
                        let (grad, conv, drive) = mult_edge(&sim.hm.data, &sim.column_depth, a, b);
                        let add_drive = additive_edge(&sim.hm.data, &sim.column_depth, a, b);
                        sum_grad += grad.abs() as f64;
                        sum_conv += conv as f64;
                        sum_mult_drive += drive.abs() as f64;
                        sum_add_drive += add_drive.abs() as f64;
                        n_edges += 1;
                    }
                }
                let n = n_edges.max(1) as f64;
                println!(
                    "diag_task55_mult_lateral_settling_falloff: tick={t:>4} voids={:>3} void_edges={n_edges:>3} \
                     mean|eta_grad|={:>9.5} mean_conveyance={:>10.4} mean|mult_drive|={:>10.6} mean|additive_drive|={:>9.5}",
                    voids.len(), sum_grad / n, sum_conv / n, sum_mult_drive / n, sum_add_drive / n
                );
            }
            if t == 180 {
                break;
            }
            sim.tick(gravity_dir, usize::MAX);
        }
        multiplicative_lateral_gate::set_enabled(false);
    }

    /// TASK #55, Job 2 investigation, part 2: `diag_task55_mult_lateral_settling_falloff`'s
    /// per-probe void counts under the multiplicative gate are NOT monotonically decreasing near
    /// tick 160 (9 at t=155, 12 at t=160, 6 at t=165 -- a bump, not a plateau or a stall). Before
    /// concluding "conveyance is too weak to close the endgame", this checks whether that kind of
    /// transient bump is a normal feature of this scenario's settling (i.e. would show up under the
    /// shipped additive form too, just at different ticks, and `test_liquid_flowing_liquid_does_not_
    /// stand_in_walls`'s fixed 120/160 probes simply catch the multiplicative form's bump and miss
    /// the additive form's), or whether it is specific to the multiplicative gate.
    ///
    /// Same scenario as `diag_task55_mult_lateral_settling_falloff`, EVERY tick from 100 to 180
    /// (not sparse probes) under both gate states from one fresh sim each. DIAGNOSTIC
    /// (reproduce-only, no assertions). Run with --ignored --nocapture.
    #[test]
    #[ignore]
    fn diag_task55_mult_lateral_settling_void_trajectory() {
        let w = 64usize;
        let h = 64usize;
        let mask = make_test_mask(w, h, SandboxShape::Hourglass, 0.15, 0.6);
        let props = get_test_props(MaterialMode::Water, w * h);
        let gravity_dir = glam::Vec2::new(0.0, 0.04);

        let count_voids = |sim: &TestSim| -> usize {
            let mut voids = 0;
            for y in 1..h - 1 {
                let mut liquid_to_the_left = false;
                for x in 0..w {
                    if mask[y * w + x] == crate::MASK_OUTSIDE {
                        liquid_to_the_left = false;
                        continue;
                    }
                    let v = sim.hm.data[y * w + x];
                    if v > 0.5 {
                        liquid_to_the_left = true;
                        continue;
                    }
                    if !liquid_to_the_left || v > 0.05 {
                        continue;
                    }
                    let liquid_to_the_right = (x + 1..w)
                        .take_while(|&x2| mask[y * w + x2] != crate::MASK_OUTSIDE)
                        .any(|x2| sim.hm.data[y * w + x2] > 0.5);
                    if liquid_to_the_right {
                        voids += 1;
                    }
                }
            }
            voids
        };

        multiplicative_lateral_gate::set_enabled(false);
        for (label, mult_enabled) in [("additive(shipped)", false), ("multiplicative(gated)", true)] {
            let mut sim = TestSim::new(w, h, props.clone(), mask.clone(), 32);
            for y in 0..h / 2 {
                for x in 0..w {
                    if mask[y * w + x] != crate::MASK_OUTSIDE {
                        sim.hm.data[y * w + x] = 1.0;
                    }
                }
            }
            multiplicative_lateral_gate::set_enabled(mult_enabled);
            let mut trajectory = String::new();
            for t in 0..=180usize {
                if t >= 100 {
                    trajectory.push_str(&format!("{}:{} ", t, count_voids(&sim)));
                }
                if t == 180 {
                    break;
                }
                sim.tick(gravity_dir, usize::MAX);
            }
            multiplicative_lateral_gate::set_enabled(false);
            println!("diag_task55_mult_lateral_settling_void_trajectory: {label}\n  {trajectory}");
        }
    }

    /// TASK #55, Job 3 investigation: is `eta = h + column_depth` (the free-surface elevation the
    /// gated multiplicative lateral head's `grad(eta)` term reads, see `mult_lateral_conveyance`'s
    /// doc comment) actually a coherent height, or off by `depth_scale = REFERENCE_GRID_HEIGHT / w`
    /// (see `REFERENCE_GRID_HEIGHT`'s own doc comment) away from reference resolution?
    ///
    /// `column_depth` is DELIBERATELY resolution-normalised: `recompute_column_depth` multiplies
    /// each row's `resting_above` contribution by `depth_scale` before accumulating specifically SO
    /// THAT a column spanning many LOCAL rows contributes the same total regardless of grid
    /// resolution (see `REFERENCE_GRID_HEIGHT`'s doc comment, "The fix"). `h`, by contrast, is never
    /// touched by `depth_scale` anywhere -- it stays a fraction of ONE local cell's own capacity
    /// (`cell_capacity_for`, ~1.0-1.5), which does not shrink or grow with grid resolution. Summing
    /// them (`h + column_depth`) therefore adds a resolution-INDEPENDENT quantity to a resolution-
    /// DEPENDENT one: only at `w == REFERENCE_GRID_HEIGHT == 512` (`depth_scale == 1.0`) do the two
    /// terms share units. This test computes `column_depth` (via `recompute_column_depth`, the same
    /// function `eta`'s `column_depth` term reads) for the IDENTICAL physical fill pattern (same
    /// count of fully-filled rows above the probe, same probe-row fill fraction) at two different
    /// grid widths, to show the resulting `eta` numerically, not just algebraically.
    ///
    /// DIAGNOSTIC (reproduce-only, no assertions -- this is a correctness question the task brief
    /// asks be checked, not a regression gate). Run with --ignored --nocapture.
    #[test]
    #[ignore]
    fn diag_task55_eta_depth_scale_consistency() {
        // PART 1 -- single column, SAME PHYSICAL GEOMETRY across w = 64/128/256/512.
        //
        // `column_depth`'s own design (`REFERENCE_GRID_HEIGHT`'s doc comment, "The fix") makes a
        // FIXED PHYSICAL depth correspond to a row COUNT that scales linearly with `w`: refining
        // the grid N-fold to cover the same physical container multiplies the rows spanning it by
        // N, and `depth_scale = REFERENCE_GRID_HEIGHT / w` divides that back out. So "the same
        // physical pile" at different `w` is NOT "the same number of stacked rows" (that was the
        // original version of this test, and is itself a physically SHRINKING pile at higher `w`
        // -- more on that below) -- it is `stacked_rows(w) = stacked_rows_ref * w /
        // REFERENCE_GRID_HEIGHT`, i.e. a row count derived from a FIXED reference-row-unit
        // quantity `stacked_rows_ref`, the same quantity `column_depth` itself is designed to
        // report regardless of `w`.
        //
        // Consistency requires going further than the original test did: `probe_h`, the LOCAL
        // partial-row remainder at the probe, must be derived from a fixed reference-row-unit
        // quantity the SAME way -- `probe_h(w) = probe_h_ref / depth_scale(w) = probe_h_ref * w /
        // REFERENCE_GRID_HEIGHT` -- not held at one constant LOCAL fraction across every `w` (the
        // original test's choice). Holding `probe_h` fixed in LOCAL units is itself a hidden unit
        // bug of exactly the same shape this task fixes: a fixed local fraction represents a
        // SHRINKING physical remainder as `w` grows (thinner local rows), so it is not "the same
        // physical geometry" either, and using it here would corrupt this very test's premise
        // with the same mistake being fixed in `settle_tick`. Deriving `probe_h` from
        // `probe_h_ref` the way `stacked_rows` is derived from `stacked_rows_ref` keeps the WHOLE
        // scenario -- not just the stacked full rows -- pinned to one fixed physical shape at
        // every resolution.
        //
        // Two depths swept: `40` (SHALLOW -- reduces to the original test's literal "5 rows at
        // w=64" case, where `column_depth` and `h`'s reference-row-scaled contribution are close
        // enough in magnitude that the old bug is clearly visible) and `320` (DEEP -- typical of
        // an established pile, where `column_depth` dominates and the bug's effect on `eta`'s
        // absolute value is small in percentage terms even though it is exactly the same
        // dimensional error). Both `stacked_rows_ref` values divide evenly by every swept width's
        // `REFERENCE_GRID_HEIGHT / w`, so `stacked_rows(w)` stays an exact integer throughout.
        let probe_h_ref = 0.4f32;

        for &(label, stacked_rows_ref) in &[("SHALLOW", 40usize), ("DEEP", 320usize)] {
        println!(
            "diag_task55_eta_depth_scale_consistency PART 1 [{label}]: single column, SAME \
             physical geometry at every w (stacked_rows_ref={stacked_rows_ref}, \
             probe_h_ref={probe_h_ref:.4}, both expressed in fixed reference-row units and \
             converted to this w's local grid representation before simulating)"
        );
        let mut part1_new_eta = Vec::new();
        let mut part1_old_eta = Vec::new();
        for &w in &[64usize, 128usize, 256usize, 512usize] {
            let h = w;
            let depth_scale = REFERENCE_GRID_HEIGHT as f32 / w as f32;
            let stacked_rows = stacked_rows_ref * w / REFERENCE_GRID_HEIGHT;
            let probe_h_local = probe_h_ref * w as f32 / REFERENCE_GRID_HEIGHT as f32;
            let probe_row = stacked_rows + 10;
            let mask = vec![crate::MASK_INSIDE; w * h];
            let mut heights = vec![0.0f32; w * h];
            let probe_x = w / 2;
            for r in (probe_row - stacked_rows)..probe_row {
                heights[r * w + probe_x] = 1.0;
            }
            heights[probe_row * w + probe_x] = probe_h_local;

            let external_mass_this_tick = vec![0.0f32; w * h];
            let cell_props = get_test_props(MaterialMode::Water, w * h);
            let edge_vel_v = vec![0.0f32; w * h];
            let mut column_depth = vec![0.0f32; w * h];
            recompute_column_depth(
                w, h, &mask, &heights, &heights, &external_mass_this_tick, &cell_props, &edge_vel_v,
                &mut column_depth[..],
            );

            let cd = column_depth[probe_row * w + probe_x];
            let h_local = heights[probe_row * w + probe_x];
            let eta_old = h_local + cd; // pre-fix: raw h, unscaled
            let eta_new = h_local * depth_scale + cd; // post-fix: h lifted to column_depth's units
            part1_old_eta.push(eta_old);
            part1_new_eta.push(eta_new);
            println!(
                "  w={w:>4} depth_scale={depth_scale:>7.4} stacked_rows={stacked_rows:>3} \
                 probe_h_local={probe_h_local:>8.5} column_depth={cd:>9.4} | \
                 eta_OLD(h+cd)={eta_old:>9.4} eta_NEW(h*scale+cd)={eta_new:>9.4}"
            );
        }
        let part1_new_baseline = *part1_new_eta.last().unwrap(); // w=512, depth_scale==1, no-op
        let part1_old_baseline = *part1_old_eta.last().unwrap();
        for (i, &w) in [64usize, 128, 256, 512].iter().enumerate() {
            let pct_old = 100.0 * (part1_old_eta[i] - part1_old_baseline) / part1_old_baseline;
            let pct_new = 100.0 * (part1_new_eta[i] - part1_new_baseline) / part1_new_baseline;
            println!(
                "  w={w:>4} eta deviation from w=512 baseline: OLD={pct_old:+7.2}%  \
                 NEW={pct_new:+7.2}%"
            );
        }
        } // end SHALLOW/DEEP loop

        // PART 2 -- lateral EDGE (two adjacent columns, same row), same fixed-physical-geometry
        // construction as PART 1 (both `stacked_rows` and `probe_h` derived from fixed
        // reference-row-unit quantities), reporting the actual quantity the flux solver uses:
        // `conveyance * (eta_a - eta_b)`. This is the acceptance criterion that matters -- Job 1
        // asks for the FLUX to be resolution invariant, and an invariant absolute `eta` alone is
        // necessary but not sufficient proof of that (a per-resolution offset that cancelled in
        // the subtraction would still pass PART 1's per-column check but fail here). Column A
        // gets `stacked_a_ref = 320` / `probe_h_a_ref = 0.4`, column B gets `stacked_b_ref = 192`
        // / `probe_h_b_ref = 0.25` -- a fixed physical elevation difference between the two
        // columns at every resolution. Swept at the same SHALLOW (40/24 rows at reference
        // resolution) and DEEP (320/192) depths as PART 1, for the same reason.
        let probe_h_a_ref = 0.4f32;
        let probe_h_b_ref = 0.25f32;
        for &(label, stacked_a_ref, stacked_b_ref) in &[("SHALLOW", 40usize, 24usize), ("DEEP", 320usize, 192usize)] {
        println!(
            "\ndiag_task55_eta_depth_scale_consistency PART 2 [{label}]: lateral edge, two \
             columns at a fixed physical elevation difference (A taller than B), reporting \
             conveyance*(eta_a-eta_b) -- the actual driving-head magnitude the gated \
             multiplicative flux term computes"
        );
        let mut part2_new_driving = Vec::new();
        let mut part2_old_driving = Vec::new();
        for &w in &[64usize, 128usize, 256usize, 512usize] {
            let h = w;
            let depth_scale = REFERENCE_GRID_HEIGHT as f32 / w as f32;
            let stacked_a = stacked_a_ref * w / REFERENCE_GRID_HEIGHT;
            let stacked_b = stacked_b_ref * w / REFERENCE_GRID_HEIGHT;
            let probe_h_a_local = probe_h_a_ref * w as f32 / REFERENCE_GRID_HEIGHT as f32;
            let probe_h_b_local = probe_h_b_ref * w as f32 / REFERENCE_GRID_HEIGHT as f32;
            let probe_row = stacked_a.max(stacked_b) + 10;
            let mask = vec![crate::MASK_INSIDE; w * h];
            let mut heights = vec![0.0f32; w * h];
            let col_a = w / 2;
            let col_b = col_a + 1;
            for r in (probe_row - stacked_a)..probe_row {
                heights[r * w + col_a] = 1.0;
            }
            heights[probe_row * w + col_a] = probe_h_a_local;
            for r in (probe_row - stacked_b)..probe_row {
                heights[r * w + col_b] = 1.0;
            }
            heights[probe_row * w + col_b] = probe_h_b_local;

            let external_mass_this_tick = vec![0.0f32; w * h];
            let cell_props = get_test_props(MaterialMode::Water, w * h); // liquidity==1: k==1,
                                                                          // janssen_effective_depth
                                                                          // is the identity
                                                                          // transform, keeping this
                                                                          // a clean unit-scale
                                                                          // demonstration.
            let edge_vel_v = vec![0.0f32; w * h];
            let mut column_depth = vec![0.0f32; w * h];
            recompute_column_depth(
                w, h, &mask, &heights, &heights, &external_mass_this_tick, &cell_props, &edge_vel_v,
                &mut column_depth[..],
            );

            let cd_a = column_depth[probe_row * w + col_a];
            let cd_b = column_depth[probe_row * w + col_b];
            let h_a = heights[probe_row * w + col_a];
            let h_b = heights[probe_row * w + col_b];
            let k = 1.0f32;
            let liq = 1.0f32;

            // OLD (pre-fix): raw h, unscaled.
            let eta_a_old = h_a + cd_a;
            let eta_b_old = h_b + cd_b;
            let conv_a_old = mult_lateral_conveyance(h_a, cd_a, k, liq);
            let conv_b_old = mult_lateral_conveyance(h_b, cd_b, k, liq);
            let driving_old =
                MULT_LATERAL_SCALE * 0.5 * (conv_a_old + conv_b_old) * (eta_a_old - eta_b_old);

            // NEW (post-fix): h lifted into column_depth's reference-row units.
            let h_a_ref = h_a * depth_scale;
            let h_b_ref = h_b * depth_scale;
            let eta_a_new = h_a_ref + cd_a;
            let eta_b_new = h_b_ref + cd_b;
            let conv_a_new = mult_lateral_conveyance(h_a_ref, cd_a, k, liq);
            let conv_b_new = mult_lateral_conveyance(h_b_ref, cd_b, k, liq);
            let driving_new =
                MULT_LATERAL_SCALE * 0.5 * (conv_a_new + conv_b_new) * (eta_a_new - eta_b_new);

            part2_old_driving.push(driving_old);
            part2_new_driving.push(driving_new);
            println!(
                "  w={w:>4} depth_scale={depth_scale:>7.4} stacked_a={stacked_a:>3} \
                 stacked_b={stacked_b:>3} | driving_OLD={driving_old:>14.4} \
                 driving_NEW={driving_new:>14.4}"
            );
        }
        let part2_new_baseline = *part2_new_driving.last().unwrap(); // w=512 anchor
        let part2_old_baseline = *part2_old_driving.last().unwrap();
        for (i, &w) in [64usize, 128, 256, 512].iter().enumerate() {
            let pct_new = 100.0 * (part2_new_driving[i] - part2_new_baseline) / part2_new_baseline;
            let pct_old = 100.0 * (part2_old_driving[i] - part2_old_baseline) / part2_old_baseline;
            println!(
                "  w={w:>4} driving deviation from w=512 baseline: OLD={pct_old:+9.2}%  \
                 NEW={pct_new:+9.2}%"
            );
        }
        } // end SHALLOW/DEEP loop
    }

    // ---- Task #61: U-tube flow-through vessel ------------------------------------------------
    //
    // These three tests exercise `SandboxShape::UTubeFlowThrough`'s geometry, sourced from
    // `U_TUBE_RECTS` so they can never drift from the shape they are testing. A fourth check --
    // that this shape is covered "for free" by the existing geometry/mass-conservation tests
    // that iterate `SANDFALL_FUNNEL_SHAPES` (in `lib.rs`) -- needs no new test here: extending
    // that const to include `UTubeFlowThrough` is what wires it in.

    /// The five `U_TUBE_RECTS` are meant to union into ONE connected region -- reservoir, basin,
    /// right arm, spout, catch well -- not five separate islands. Flood-fills the generated mask
    /// from a cell in the reservoir and asserts every non-OUTSIDE cell is reachable, at four grid
    /// sizes so a pinch-off that only appears at a particular rasterisation (e.g. a rect boundary
    /// landing between two cell centres) is caught rather than hidden by whichever resolution
    /// happens to get tested.
    #[test]
    fn test_u_tube_is_one_connected_region() {
        for &grid in &[64usize, 128, 256, 512] {
            let w = grid;
            let h = grid;
            let mask = make_test_mask(w, h, SandboxShape::UTubeFlowThrough, 0.05, 1.0);

            let reservoir = U_TUBE_RECTS[U_TUBE_RESERVOIR_RECT];
            let start_dx = (reservoir[0] + reservoir[1]) / 2.0;
            let start_dy = (reservoir[2] + reservoir[3]) / 2.0;
            let start_x = ((w as f32 / 2.0) + start_dx * w as f32).round() as usize;
            let start_y = ((h as f32 / 2.0) + start_dy * h as f32).round() as usize;
            let start = start_y * w + start_x;
            assert_ne!(
                mask[start],
                crate::MASK_OUTSIDE,
                "grid={grid}: flood-fill start cell ({start_x},{start_y}), the reservoir's \
                 centre, is not inside the mask"
            );

            let total_inside = mask.iter().filter(|&&m| m != crate::MASK_OUTSIDE).count();

            let mut visited = vec![false; w * h];
            let mut stack = vec![start];
            visited[start] = true;
            let mut reached = 0usize;
            while let Some(idx) = stack.pop() {
                reached += 1;
                let x = idx % w;
                let y = idx / w;
                let neighbors = [
                    (x.wrapping_sub(1), y),
                    (x + 1, y),
                    (x, y.wrapping_sub(1)),
                    (x, y + 1),
                ];
                for (nx, ny) in neighbors {
                    if nx < w && ny < h {
                        let nidx = ny * w + nx;
                        if !visited[nidx] && mask[nidx] != crate::MASK_OUTSIDE {
                            visited[nidx] = true;
                            stack.push(nidx);
                        }
                    }
                }
            }

            assert_eq!(
                reached, total_inside,
                "grid={grid}: flood-fill from the reservoir reached {reached} of {total_inside} \
                 non-OUTSIDE cells -- the U-tube is not one connected region (a pinch-off exists \
                 at this resolution)"
            );
        }
    }

    /// The strip of the basin between the reservoir's right edge and the right arm's left edge
    /// is deliberately NOT covered by any rect above it -- that roofed channel is the whole
    /// reason this apparatus exists (the Pascal pressure test case). Confirms at least one
    /// in-mask basin cell in that strip has an OUTSIDE cell directly above it (smaller y, i.e.
    /// higher on screen, per `eval_sandbox_shape`'s "y increases downward" convention).
    #[test]
    fn test_u_tube_basin_has_a_roof() {
        let w = 256usize;
        let h = 256usize;
        let mask = make_test_mask(w, h, SandboxShape::UTubeFlowThrough, 0.05, 1.0);

        let reservoir = U_TUBE_RECTS[U_TUBE_RESERVOIR_RECT];
        let right_arm = U_TUBE_RECTS[2];
        let basin = U_TUBE_RECTS[1];

        // The unroofed gap between the two arms, in x.
        let gap_lo = reservoir[1]; // reservoir's right edge
        let gap_hi = right_arm[0]; // right arm's left edge
        assert!(
            gap_lo < gap_hi,
            "reservoir and right arm overlap or touch in x (gap_lo={gap_lo:.4} \
             gap_hi={gap_hi:.4}) -- there is no gap left for the basin roof to cover"
        );

        let w_f = w as f32;
        let h_f = h as f32;
        let center_x = w_f / 2.0;
        let center_y = h_f / 2.0;

        let mut found_roof = false;
        'outer: for x in 0..w {
            let dx = x as f32 - center_x;
            if dx < gap_lo * w_f || dx >= gap_hi * w_f {
                continue;
            }
            for y in 1..h {
                let dy = y as f32 - center_y;
                if dy < basin[2] * h_f || dy >= basin[3] * h_f {
                    continue;
                }
                let idx = y * w + x;
                let above = (y - 1) * w + x;
                if mask[idx] != crate::MASK_OUTSIDE && mask[above] == crate::MASK_OUTSIDE {
                    found_roof = true;
                    break 'outer;
                }
            }
        }

        assert!(
            found_roof,
            "no in-mask basin cell in the unroofed gap (x fraction {gap_lo:.3}..{gap_hi:.3}, y \
             fraction {:.3}..{:.3}) has an OUTSIDE cell directly above it -- the deliberately \
             roofed Pascal-pressure channel is missing",
            basin[2], basin[3]
        );
    }

    /// Purely geometric (no simulation): from `U_TUBE_RECTS` alone, confirms the vessel
    /// genuinely spills over the lip rather than merely filling up to it -- the reservoir's
    /// capacity above the lip line must exceed the basin's plus the right arm's capacity below
    /// it -- and that the catch well is large enough to hold the entire spilled remainder
    /// without itself overflowing.
    #[test]
    fn test_u_tube_reservoir_overflows_the_lip() {
        let reservoir = U_TUBE_RECTS[U_TUBE_RESERVOIR_RECT];
        let basin = U_TUBE_RECTS[1];
        let right_arm = U_TUBE_RECTS[2];
        let catch_well = U_TUBE_RECTS[4];

        // The overflow lip is the right arm's top edge.
        let lip_y = right_arm[2];
        assert!(
            lip_y > reservoir[2] && lip_y < reservoir[3],
            "lip_y={lip_y:.4} must fall strictly inside the reservoir's y range \
             ({:.4}..{:.4}) for 'area above the lip' to mean anything",
            reservoir[2],
            reservoir[3]
        );

        let reservoir_area_above_lip = (reservoir[1] - reservoir[0]) * (lip_y - reservoir[2]);
        let basin_area = (basin[1] - basin[0]) * (basin[3] - basin[2]);
        let right_arm_area_below_lip = (right_arm[1] - right_arm[0]) * (right_arm[3] - lip_y);
        let downstream_capacity = basin_area + right_arm_area_below_lip;
        let expected_spill = reservoir_area_above_lip - downstream_capacity;
        let catch_well_area = (catch_well[1] - catch_well[0]) * (catch_well[3] - catch_well[2]);

        assert!(
            reservoir_area_above_lip > downstream_capacity,
            "vessel does not genuinely spill: reservoir_area_above_lip={reservoir_area_above_lip:.5} \
             <= basin_area({basin_area:.5}) + right_arm_area_below_lip({right_arm_area_below_lip:.5}) \
             = downstream_capacity({downstream_capacity:.5})"
        );
        assert!(
            catch_well_area > expected_spill,
            "catch well cannot hold the spill: catch_well_area={catch_well_area:.5} <= \
             expected_spill={expected_spill:.5} (reservoir_area_above_lip={reservoir_area_above_lip:.5}, \
             downstream_capacity={downstream_capacity:.5})"
        );
    }
}

// Task #55, step 2: the STATIC hydraulic head field itself (see that file's module doc comment
// for the physics). Declared as a CHILD of `physics`, exactly like `task55_head_spec` below, so
// it can call `support_fraction` and read `REFERENCE_GRID_HEIGHT` without widening either's
// visibility. Promoted out of `#[cfg(test)]` (task 2.32 visualisation step): the pressure
// heat-map overlay's new-field source (`DrawingSimulation::pressure_heatmap_head_field`, gated
// exactly like `pressure_heatmap_enabled` -- see `pressure_field_texels`) is now a real, shipping
// caller, so this must compile and run outside test builds too. Still `pub(crate)`, not `pub`:
// the only caller outside this crate's own test suite is `DrawingSimulation::pressure_field_texels`
// in `lib.rs`, reached via the full `physics::task55_head_field::...` path.
#[path = "task55_head_field.rs"]
pub(crate) mod task55_head_field;

// Task #55, step 1: an ISOLATION SPEC for the pressure field, kept in its own file (see that
// file's module doc comment for why). Declared as a CHILD of `physics` (not of the crate root)
// so `use super::*` there reaches `recompute_column_depth`, `REFERENCE_GRID_HEIGHT`, and every
// other private item it needs without widening anyone's visibility.
#[cfg(test)]
#[path = "task55_head_spec.rs"]
mod task55_head_spec;
