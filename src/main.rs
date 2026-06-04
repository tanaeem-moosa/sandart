mod app;
mod config;
mod pattern;
mod renderer;
mod sim;

use app::SandArtApp;

fn main() -> eframe::Result {
    // Configure log display
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([900.0, 700.0])
            .with_min_inner_size([500.0, 400.0])
            .with_title("Sands of Time: Kinetic Sand Art Simulator"),
        depth_buffer: 24,
        ..Default::default()
    };

    eframe::run_native(
        "sand_art_simulator",
        options,
        Box::new(|cc| {
            // Note: If using wgpu, this ensures the eframe renderer hooks up correctly
            Ok(Box::new(SandArtApp::new(cc)))
        }),
    )
}
