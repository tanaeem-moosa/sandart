use sandart_sim::{DrawingSimulation, MaterialMode, SandboxShape};
use glam::Vec2;

fn main() {
    println!("Initializing profiling simulation...");
    let mut sim = DrawingSimulation::new();
    
    // Set up active marbles
    let mut targets = [None; 5];
    targets[0] = Some(Vec2::ZERO);
    
    // Warm up step
    sim.update(0.016, &targets, 0.018, MaterialMode::DrySand, SandboxShape::Circle);

    println!("Starting profiling session (sampling at 250Hz)...");
    let guard = pprof::ProfilerGuard::new(250).unwrap(); // 250Hz sample rate

    // Run 2000 simulation steps
    for i in 0..2000 {
        // Move the marble in a spiral to sweep across the sand bed and trigger CA settling
        let angle = i as f32 * 0.03;
        let radius = 0.1 + (i as f32 * 0.00035).min(0.7);
        let x = angle.cos() * radius;
        let y = angle.sin() * radius;
        targets[0] = Some(Vec2::new(x, y));
        
        sim.update(0.016, &targets, 0.018, MaterialMode::DrySand, SandboxShape::Circle);
    }

    println!("Profiling finished. Generating flamegraph...");
    if let Ok(report) = guard.report().build() {
        let file = std::fs::File::create("flamegraph.svg").unwrap();
        report.flamegraph(file).unwrap();
        println!("Successfully generated flamegraph.svg!");
    } else {
        println!("Failed to generate profiling report.");
    }
}
