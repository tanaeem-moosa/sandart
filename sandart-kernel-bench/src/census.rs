//! Sparsity/subnormal census shared by `kernel_a::census_pass` and `kernel_e2::census_pass`
//! (hypotheses 1 and 2 of the vectorisation-slowdown investigation -- see the report). Not used
//! by any timed path; these functions duplicate the relevant kernel's own math so the counting
//! itself never perturbs `run_pass`'s measured cost.

#[derive(Default, Debug, Clone, Copy)]
pub struct Census {
    /// Owned lateral edges visited (`n_edges` summed over every span).
    pub edges_total: u64,
    /// Of those, edges whose PRE-ARBITRATION candidate flux is nonzero (i.e. not asleep, locked,
    /// or masked out by the `edge_ok`/`active_e` test).
    pub edges_nonzero_candidate: u64,
    /// Of those, edges whose POST-ARBITRATION final flux exceeds `MIN_FLUX` in magnitude -- the
    /// same bar `flux_edge_apply`'s `total_out_flow`/`total_in_flow`/`total_flow` accumulation
    /// uses to decide a flow "really happened".
    pub edges_nonzero_final: u64,
    /// Owned cells visited (`n_data` summed over every span; each span's own `+1` acceptor column
    /// IS counted here, matching A's/E2's own stage4+5 loop bound).
    pub cells_total: u64,
    /// Of those, cells with any nonzero realised out-flow or in-flow this pass (the exact
    /// condition A's stage4+5 branches on: `out_flow == 0.0 && in_flow == 0.0` skips the cell).
    pub cells_with_flow: u64,
    /// Nonzero-but-subnormal (`0 < |x| < f32::MIN_POSITIVE`) values seen among pre-arbitration
    /// candidates.
    pub subnormal_candidate: u64,
    /// Nonzero-but-subnormal values seen among post-arbitration final fluxes.
    pub subnormal_final: u64,
    /// Nonzero-but-subnormal values seen among stage4+5's mixing scalars (`out2`/`in2`/
    /// `own_amount`/`left_amt`/`right_amt`).
    pub subnormal_mix_amounts: u64,
    /// Nonzero-but-subnormal values seen among stage1's `head`/`avail`/`freecap` arrays.
    pub subnormal_head_avail_freecap: u64,
    /// Nonzero-but-subnormal values seen among stage3's `pos`/`neg`/`jit`-weighted arrays.
    pub subnormal_pos_neg: u64,
}

impl Census {
    pub fn merge(&mut self, o: &Census) {
        self.edges_total += o.edges_total;
        self.edges_nonzero_candidate += o.edges_nonzero_candidate;
        self.edges_nonzero_final += o.edges_nonzero_final;
        self.cells_total += o.cells_total;
        self.cells_with_flow += o.cells_with_flow;
        self.subnormal_candidate += o.subnormal_candidate;
        self.subnormal_final += o.subnormal_final;
        self.subnormal_mix_amounts += o.subnormal_mix_amounts;
        self.subnormal_head_avail_freecap += o.subnormal_head_avail_freecap;
        self.subnormal_pos_neg += o.subnormal_pos_neg;
    }

    pub fn report(&self, label: &str) {
        let pct = |n: u64, d: u64| if d == 0 { 0.0 } else { 100.0 * n as f64 / d as f64 };
        println!("  [{label}] edges: {}/{} nonzero-candidate ({:.2}%), {}/{} nonzero-final>MIN_FLUX ({:.2}%)",
            self.edges_nonzero_candidate, self.edges_total, pct(self.edges_nonzero_candidate, self.edges_total),
            self.edges_nonzero_final, self.edges_total, pct(self.edges_nonzero_final, self.edges_total));
        println!("  [{label}] cells: {}/{} with any in/out flow ({:.2}%)",
            self.cells_with_flow, self.cells_total, pct(self.cells_with_flow, self.cells_total));
        println!("  [{label}] subnormals: candidate={} final={} mix_amounts={} head/avail/freecap={} pos/neg={}",
            self.subnormal_candidate, self.subnormal_final, self.subnormal_mix_amounts,
            self.subnormal_head_avail_freecap, self.subnormal_pos_neg);
    }
}

/// `true` for a nonzero value strictly below the smallest normal `f32` -- the range x86 hits with
/// microcode assists unless FTZ/DAZ is set.
#[inline]
pub fn is_subnormal(x: f32) -> bool {
    x != 0.0 && x.abs() < f32::MIN_POSITIVE
}
