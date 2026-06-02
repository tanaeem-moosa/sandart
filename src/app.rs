use egui;
use serde::{Deserialize, Serialize};

/// Application configuration and simulation parameters in normalized space.
/// Normalized space scales from 0.0 to 1.0 relative to the sand table radius.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppConfig {
    /// Speed of the marble in units of radius per second (0.01 to 2.0).
    pub speed: f32,
    /// Size (radius) of the marble as a fraction of the table radius (0.005 to 0.1).
    pub marble_size: f32,
    /// Spacing between spiral turns as a fraction of the table radius (0.005 to 0.2).
    pub spiral_spacing: f32,
    /// Flag to enable auto-play of the spiral pattern.
    pub auto_play: bool,
    /// Light brightness slider (0.0 to 1.0).
    pub light_brightness: f32,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            speed: 0.15,
            marble_size: 0.025,
            spiral_spacing: 0.030,
            auto_play: false,
            light_brightness: 0.8,
        }
    }
}

pub struct SandArtApp {
    /// Active configuration parameters.
    pub config: AppConfig,
    /// Static counter to verify update loops.
    pub frame_counter: u64,
    /// Delta time since the last frame.
    pub dt: f32,
    /// The physics simulation engine.
    pub sim: crate::sim::Simulation,
    /// Shared heightmap data for zero-allocation rendering transfer.
    pub shared_heightmap: std::sync::Arc<std::sync::Mutex<Vec<f32>>>,
}

impl SandArtApp {
    /// Create a new instance of the application.
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        if let Some(wgpu_state) = &cc.wgpu_render_state {
            let device = &wgpu_state.device;
            let target_format = wgpu_state.target_format;
            let resources = crate::renderer::SandArtRenderResources::new(device, target_format);
            wgpu_state.renderer.write().callback_resources.insert(resources);
        }
        Self {
            config: AppConfig::default(),
            frame_counter: 0,
            dt: 0.0,
            sim: crate::sim::Simulation::new(),
            shared_heightmap: std::sync::Arc::new(std::sync::Mutex::new(vec![0.8; 512 * 512])),
        }
    }
}

impl eframe::App for SandArtApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.frame_counter += 1;
        
        // Track frame delta time for frame-rate-independent physics calculations
        self.dt = ctx.input(|i| i.stable_dt).min(0.1);

        // Copy simulation heights to the shared rendering buffer (non-allocating copy)
        if let Ok(mut shared) = self.shared_heightmap.lock() {
            shared.copy_from_slice(self.sim.heightmap.as_slice());
        }

        // Draw the top panel for basic info
        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Sands of Time: Kinetic Sand Simulator");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(format!("Frames: {} | dt: {:.4}s", self.frame_counter, self.dt));
                });
            });
        });

        // Draw the left control panel
        egui::SidePanel::left("control_panel")
            .resizable(true)
            .default_width(280.0)
            .min_width(200.0)
            .show(ctx, |ui| {
                ui.vertical(|ui| {
                    ui.add_space(8.0);
                    ui.heading("Controls");
                    ui.separator();
                    ui.add_space(8.0);

                    ui.label("Marble Controls");
                    // Sliders now work in normalized spaces with user-friendly scaling percentages
                    ui.add(egui::Slider::new(&mut self.config.speed, 0.01..=2.0)
                        .text("Speed (R/s)")
                        .show_value(true));
                    ui.add(egui::Slider::new(&mut self.config.marble_size, 0.005..=0.1)
                        .text("Radius (R)")
                        .show_value(true));

                    ui.add_space(12.0);
                    ui.label("Pattern Settings");
                    ui.add(egui::Slider::new(&mut self.config.spiral_spacing, 0.005..=0.2)
                        .text("Spiral Spacing (R)")
                        .show_value(true));
                    ui.checkbox(&mut self.config.auto_play, "Auto-play Spiral");

                    ui.add_space(12.0);
                    ui.label("Lighting Settings");
                    ui.add(egui::Slider::new(&mut self.config.light_brightness, 0.0..=1.0).text("Brightness"));

                    ui.add_space(20.0);
                    if ui.button("Reset Sand").clicked() {
                        self.sim.reset();
                        // Generate a concentric ripple pattern to visually verify height rendering
                        for y in 0..512 {
                            for x in 0..512 {
                                let dx = x as f32 - 256.0;
                                let dy = y as f32 - 256.0;
                                let dist = (dx * dx + dy * dy).sqrt();
                                let val = (dist * 0.1).sin() * 0.3 + 0.5;
                                self.sim.heightmap.set(x, y, val);
                            }
                        }
                    }
                });
            });

        // Draw the central canvas
        egui::CentralPanel::default().show(ctx, |ui| {
            // 1. Calculate centered square rect based on available space BEFORE allocating
            let available_rect = ui.available_rect_before_wrap();
            let square_side = available_rect.width().min(available_rect.height()).max(0.0);
            let radius = square_side / 2.0;
            
            let offset_x = (available_rect.width() - square_side) / 2.0;
            let offset_y = (available_rect.height() - square_side) / 2.0;
            let centered_rect = egui::Rect::from_min_size(
                egui::pos2(available_rect.min.x + offset_x, available_rect.min.y + offset_y),
                egui::vec2(square_side, square_side),
            );

            // 2. Allocate the exact centered rect to align mouse interaction with the visuals
            let response = ui.allocate_rect(centered_rect, egui::Sense::drag());

            // 3. Draw visuals centered in the allocated space via custom WGPU rendering
            ui.painter().add(egui_wgpu::Callback::new_paint_callback(
                centered_rect,
                crate::renderer::SandArtCallback {
                    heightmap_data: self.shared_heightmap.clone(),
                },
            ));

            // 4. Capture mouse interaction (ignoring input on sliders/panels)
            let mut target_pos = None;
            if (response.dragged() || response.clicked()) && radius > 1e-4 {
                if let Some(pointer_pos) = response.interact_pointer_pos() {
                    // Normalize relative pointer pos to [-1.0, 1.0] from center of table
                    let rel_x = (pointer_pos.x - centered_rect.center().x) / radius;
                    // Flip y for standard cartesian coordinate mapping (positive y is up)
                    let rel_y = -(pointer_pos.y - centered_rect.center().y) / radius;
                    
                    let pos = egui::vec2(rel_x, rel_y);
                    let len = pos.length();
                    // Constrain marble center to visual sand circle bounds (0.92 radius in Cartesian)
                    let max_r = (0.92 - self.config.marble_size).max(0.0);
                    let clamped_pos = if len > max_r {
                        pos / len * max_r
                    } else {
                        pos
                    };
                    target_pos = Some(glam::Vec2::new(clamped_pos.x, clamped_pos.y));
                }
            }

            // Run simulation tick
            self.sim.update(self.dt, target_pos, self.config.marble_size);
        });

        // Keep requesting frames for continuous physics simulation/render
        ctx.request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = AppConfig::default();
        assert_eq!(config.speed, 0.15);
        assert_eq!(config.marble_size, 0.025);
        assert_eq!(config.spiral_spacing, 0.030);
        assert_eq!(config.light_brightness, 0.8);
        assert!(!config.auto_play);
    }

    #[test]
    fn test_serialization() {
        let config = AppConfig::default();
        let serialized = serde_json::to_string(&config).unwrap();
        let deserialized: AppConfig = serde_json::from_str(&serialized).unwrap();
        assert_eq!(config, deserialized);
    }

    #[test]
    fn test_json_schema_stability() {
        let json_str = r#"{"speed":0.15,"marble_size":0.025,"spiral_spacing":0.03,"auto_play":false,"light_brightness":0.8}"#;
        let deserialized: AppConfig = serde_json::from_str(json_str).unwrap();
        assert_eq!(deserialized, AppConfig::default());
    }
}
