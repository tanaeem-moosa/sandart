use sandart_sim::{DrawingSimulation, MaterialMode, SandboxShape};
use sandart_pattern::{PlaybackController, PlaybackState, generate_gosper_curve};
use glam::Vec2;

fn main() {
    println!("Initializing profiling simulation under exact conditions (Gosper curve depth 3, 3 marbles, MoonDust, SandboxShape::Circle)...");
    let mut sim = DrawingSimulation::new();
    
    // Generate Gosper curve waypoints
    let base_waypoints = generate_gosper_curve(3);
    println!("Generated Gosper curve waypoints: {}", base_waypoints.len());
    
    // Set up active marbles / PlaybackController
    let mut playback = PlaybackController::new();
    let arms = 3;
    for j in 0..arms {
        let angle_offset = (j as f32 / arms as f32) * 2.0 * std::f32::consts::PI;
        let mut rotated = Vec::with_capacity(base_waypoints.len());
        for p in &base_waypoints {
            let cos_a = angle_offset.cos();
            let sin_a = angle_offset.sin();
            let rx = p.x * cos_a - p.y * sin_a;
            let ry = p.x * sin_a + p.y * cos_a;
            let p_rot = Vec2::new(rx, ry);
            rotated.push(p_rot);
        }
        playback.waypoints[j] = rotated;
    }
    
    playback.randomize_speeds(arms, sim.seed);
    playback.state = PlaybackState::Playing;
    
    // Warm up step
    let positions = [
        sim.marbles[0].pos,
        sim.marbles[1].pos,
        sim.marbles[2].pos,
        sim.marbles[3].pos,
        sim.marbles[4].pos,
    ];
    let targets = playback.step_playback_all(&positions, arms, 0.4, 0.016);
    sim.update(0.016, &targets, 0.018, MaterialMode::ButterCream, SandboxShape::Circle, 33.3, 33.3);

    println!("Starting profiling session (sampling at 250Hz)...");
    let guard = pprof::ProfilerGuard::new(250).unwrap(); // 250Hz sample rate

    let start_time = std::time::Instant::now();
    // Run 3000 simulation steps
    for _ in 0..3000 {
        let positions = [
            sim.marbles[0].pos,
            sim.marbles[1].pos,
            sim.marbles[2].pos,
            sim.marbles[3].pos,
            sim.marbles[4].pos,
        ];
        let targets = playback.step_playback_all(&positions, arms, 0.4, 0.016);
        sim.update(0.016, &targets, 0.018, MaterialMode::ButterCream, SandboxShape::Circle, 33.3, 33.3);
    }
    let elapsed = start_time.elapsed();
    println!("Simulation of 3000 steps took: {:?}", elapsed);

    println!("Profiling finished. Generating flamegraph...");
    if let Ok(report) = guard.report().build() {
        let file = std::fs::File::create("flamegraph_gosper.svg").unwrap();
        report.flamegraph(file).unwrap();
        println!("Successfully generated flamegraph_gosper.svg!");
    } else {
        println!("Failed to generate profiling report.");
    }
}

