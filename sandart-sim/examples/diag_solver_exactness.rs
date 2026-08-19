use sandart_sim::physics::{cell_potential, overfill_equilibrium_transfer};
fn reference(h_a: f32, cap_a: f32, h_b: f32, cap_b: f32, cap_a_eff: f32, cap_b_eff: f32,
             g: f32, tau: f32, ga: f32, gb: f32, ratio: f32, unit: f32, tension: f32) -> f32 {
    let phi = |h: f32, cap: f32, gain: f32| cell_potential(h, cap, ratio, unit, tension, gain) as f64;
    let stress = |d: f64| phi(h_a - d as f32, cap_a, ga) + g as f64 - phi(h_b + d as f32, cap_b, gb);
    let tau = tau.max(0.0) as f64;
    let s0 = stress(0.0);
    let (mut lo, mut hi, target) = if s0 > tau {
        let limit = h_a.min((cap_b_eff - h_b).max(0.0)) as f64;
        if limit <= 0.0 || stress(limit) >= tau { return limit.max(0.0) as f32; }
        (0.0, limit, tau)
    } else if s0 < -tau {
        let limit = h_b.min((cap_a_eff - h_a).max(0.0)) as f64;
        if limit <= 0.0 || stress(-limit) <= -tau { return -limit.max(0.0) as f32; }
        (-limit, 0.0, -tau)
    } else { return 0.0 };
    for _ in 0..200 { let m = 0.5*(lo+hi); if stress(m) > target { lo = m } else { hi = m } }
    (0.5*(lo+hi)) as f32
}
fn main() {
    let mut seed = 0x12345678u32;
    let mut rnd = || { seed ^= seed << 13; seed ^= seed >> 17; seed ^= seed << 5; (seed >> 8) as f32 / 16777216.0 };
    let (ratio, unit) = (0.9f32, 125.0f32);
    let mut worst = 0.0f32; let mut wc = String::new();
    let mut worst_lat = 0.0f32;
    for tension in [0.0f32, 1.0, 2.0] {
        for _ in 0..200000 {
            let equal = rnd() < 0.5; // half the samples on the equal-cap/equal-gain lateral shape
            let cap_a = if rnd() < 0.5 { 1.0 } else { 1.5 };
            let cap_b = if equal { cap_a } else if rnd() < 0.5 { 1.0 } else { 1.5 };
            let over = rnd() < 0.5;
            let h_a = if over { cap_a * (1.0 + ratio * rnd()) } else { cap_a * rnd() };
            let h_b = if over { cap_b * (1.0 + ratio * rnd()) } else { cap_b * rnd() };
            let (ca, cb) = (cap_a * (1.0 + ratio), cap_b * (1.0 + ratio));
            let g = if equal { (rnd() - 0.5) * 0.2 } else { (rnd() - 0.5) * 2.0 };
            let tau = if equal { 0.0 } else if rnd() < 0.5 { 0.0 } else { rnd() * 0.3 };
            let ga = rnd(); let gb = if equal { ga } else { rnd() };
            let got = overfill_equilibrium_transfer(h_a, cap_a, h_b, cap_b, ca, cb, g, 0.0, tau, ga, gb, ratio, unit, tension);
            let want = reference(h_a, cap_a, h_b, cap_b, ca, cb, g, tau, ga, gb, ratio, unit, tension);
            let e = (got - want).abs();
            if equal && e > worst_lat { worst_lat = e; }
            if e > worst { worst = e; wc = format!("t={tension} h_a={h_a:.4}/{cap_a} h_b={h_b:.4}/{cap_b} g={g:.4} tau={tau:.4} ga={ga:.3} gb={gb:.3} got={got:.6} want={want:.6}"); }
        }
    }
    println!("600k samples: worst |got-want| = {worst:.3e}\n  {wc}");
    println!("equal-cap/equal-gain (lateral) subset: worst = {worst_lat:.3e}");
}
