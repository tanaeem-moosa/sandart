use crate::config::AppConfig;
use egui;

pub struct SandArtApp {
    /// Active configuration parameters.
    pub config: AppConfig,
    /// Static counter to verify update loops.
    pub frame_counter: u64,
    /// Delta time since the last frame.
    pub dt: f32,
    /// Cumulative elapsed time in seconds.
    pub elapsed_time: f32,
    /// The physics simulation engine.
    pub sim: crate::sim::Simulation,
    /// Shared heightmap data for zero-allocation rendering transfer.
    pub shared_heightmap: std::sync::Arc<std::sync::Mutex<Vec<f32>>>,
    /// Playback controller for custom files and mathematical paths.
    pub playback: crate::pattern::PlaybackController,
    /// Error message for pattern loading issues.
    pub pattern_error: Option<String>,
    /// List of sample pattern file paths scanned on startup.
    pub sample_patterns: Vec<std::path::PathBuf>,
}

impl SandArtApp {
    /// Create a new instance of the application.
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        if let Some(wgpu_state) = &cc.wgpu_render_state {
            let device = &wgpu_state.device;
            let target_format = wgpu_state.target_format;
            let resources = crate::renderer::SandArtRenderResources::new(device, target_format);
            wgpu_state
                .renderer
                .write()
                .callback_resources
                .insert(resources);
        }

        // Scan patterns directory on startup
        let mut sample_patterns = Vec::new();
        if let Ok(entries) = std::fs::read_dir("patterns") {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                    if ext == "thr" || ext == "gcode" || ext == "nc" {
                        sample_patterns.push(path);
                    }
                }
            }
        }
        // Sort patterns alphabetically for neat dropdown display
        sample_patterns.sort_by(|a, b| a.file_name().cmp(&b.file_name()));

        Self {
            config: AppConfig::default(),
            frame_counter: 0,
            dt: 0.0,
            elapsed_time: 0.0,
            sim: crate::sim::Simulation::new(),
            shared_heightmap: std::sync::Arc::new(std::sync::Mutex::new(vec![0.8; 512 * 512])),
            playback: crate::pattern::PlaybackController::new(),
            pattern_error: None,
            sample_patterns,
        }
    }

    /// Helper to load the custom pattern file specified in config
    fn load_custom_pattern(&mut self) {
        if self.config.custom_file_path.is_empty() {
            self.pattern_error = Some("No path specified".to_string());
            return;
        }
        match std::fs::read_to_string(&self.config.custom_file_path) {
            Ok(content) => {
                let path_lower = self.config.custom_file_path.to_lowercase();
                let res = if path_lower.ends_with(".gcode") || path_lower.ends_with(".nc") {
                    crate::pattern::parse_gcode(&content)
                } else {
                    crate::pattern::parse_thr(&content)
                };

                match res {
                    Ok(waypoints) => {
                        self.playback.waypoints = waypoints;
                        self.playback.current_idx = 0;
                        self.playback.state = crate::pattern::PlaybackState::Playing;
                        self.pattern_error = None;
                    }
                    Err(e) => {
                        self.pattern_error = Some(format!("Parse error: {}", e));
                    }
                }
            }
            Err(e) => {
                self.pattern_error = Some(format!("File error: {}", e));
            }
        }
    }
}

impl eframe::App for SandArtApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.frame_counter += 1;

        // Track frame delta time for frame-rate-independent physics calculations
        self.dt = ctx.input(|i| i.stable_dt).min(0.1);
        self.elapsed_time += self.dt;

        // Copy simulation heights to the shared rendering buffer (non-allocating copy)
        if let Ok(mut shared) = self.shared_heightmap.lock() {
            shared.copy_from_slice(self.sim.heightmap.as_slice());
        }

        // Draw the top panel for basic info
        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Sands of Time: Kinetic Sand Simulator");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(format!(
                        "Frames: {} | dt: {:.4}s",
                        self.frame_counter, self.dt
                    ));
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
                    ui.add(
                        egui::Slider::new(&mut self.config.speed, 0.01..=2.0)
                            .text("Speed (R/s)")
                            .show_value(true),
                    );
                    ui.add(
                        egui::Slider::new(&mut self.config.marble_size, 0.005..=0.1)
                            .text("Radius (R)")
                            .show_value(true),
                    );

                    ui.add_space(12.0);
                    ui.label("Pattern Settings");

                    // Pattern Mode selection dropdown
                    egui::ComboBox::from_label("Mode")
                        .selected_text(format!("{:?}", self.config.pattern_mode))
                        .show_ui(ui, |ui| {
                            let mut changed = false;
                            changed |= ui
                                .selectable_value(
                                    &mut self.config.pattern_mode,
                                    crate::config::PatternMode::Manual,
                                    "Manual (Mouse)",
                                )
                                .changed();
                            changed |= ui
                                .selectable_value(
                                    &mut self.config.pattern_mode,
                                    crate::config::PatternMode::Spiral,
                                    "Archimedean Spiral",
                                )
                                .changed();
                            changed |= ui
                                .selectable_value(
                                    &mut self.config.pattern_mode,
                                    crate::config::PatternMode::CustomFile,
                                    "Custom File (.thr/.gcode)",
                                )
                                .changed();
                            if changed {
                                self.playback.state = crate::pattern::PlaybackState::Stopped;
                                self.playback.waypoints.clear();
                                self.playback.current_idx = 0;
                                self.pattern_error = None;
                            }
                        });

                    if self.config.pattern_mode == crate::config::PatternMode::Spiral {
                        ui.add_space(8.0);
                        ui.add(
                            egui::Slider::new(&mut self.config.spiral_spacing, 0.005..=0.2)
                                .text("Spiral Spacing (R)")
                                .show_value(true),
                        );

                        if ui.button("Load Spiral Pattern").clicked() {
                            let waypoints =
                                crate::pattern::generate_spiral(self.config.spiral_spacing);
                            self.playback.waypoints = waypoints;
                            self.playback.current_idx = 0;
                            self.playback.state = crate::pattern::PlaybackState::Playing;
                            self.pattern_error = None;
                        }
                    }

                    if self.config.pattern_mode == crate::config::PatternMode::CustomFile {
                        ui.add_space(8.0);

                        // Dropdown for sample patterns if patterns directory has files
                        if !self.sample_patterns.is_empty() {
                            ui.horizontal(|ui| {
                                ui.label("Samples:");
                                egui::ComboBox::from_id_salt("samples_combo")
                                    .selected_text(
                                        std::path::Path::new(&self.config.custom_file_path)
                                            .file_name()
                                            .and_then(|f| f.to_str())
                                            .unwrap_or("Select a sample..."),
                                    )
                                    .show_ui(ui, |ui| {
                                        let mut selected_path = None;
                                        for path in &self.sample_patterns {
                                            let filename = path
                                                .file_name()
                                                .and_then(|f| f.to_str())
                                                .unwrap_or("");
                                            if ui
                                                .selectable_label(
                                                    self.config.custom_file_path
                                                        == path.display().to_string(),
                                                    filename,
                                                )
                                                .clicked()
                                            {
                                                selected_path = Some(path.display().to_string());
                                            }
                                        }
                                        if let Some(path_str) = selected_path {
                                            self.config.custom_file_path = path_str;
                                            self.load_custom_pattern();
                                        }
                                    });
                            });
                            ui.add_space(4.0);
                        }

                        ui.horizontal(|ui| {
                            ui.label("File Path:");
                            ui.text_edit_singleline(&mut self.config.custom_file_path);

                            if ui.button("Browse...").clicked() {
                                if let Some(path) = rfd::FileDialog::new()
                                    .add_filter(
                                        "Pattern Files (*.thr, *.gcode, *.nc)",
                                        &["thr", "gcode", "nc"],
                                    )
                                    .pick_file()
                                {
                                    self.config.custom_file_path = path.display().to_string();
                                    self.load_custom_pattern();
                                }
                            }
                        });

                        if ui.button("Load Pattern File").clicked() {
                            self.load_custom_pattern();
                        }
                    }

                    if let Some(err) = &self.pattern_error {
                        ui.add_space(4.0);
                        ui.colored_label(egui::Color32::RED, err);
                    }

                    if self.config.pattern_mode != crate::config::PatternMode::Manual {
                        ui.add_space(12.0);
                        ui.label("Playback Controls");
                        ui.horizontal(|ui| {
                            let label = match self.playback.state {
                                crate::pattern::PlaybackState::Playing => "Pause",
                                _ => "Play",
                            };
                            if ui.button(label).clicked() {
                                if self.playback.state == crate::pattern::PlaybackState::Playing {
                                    self.playback.state = crate::pattern::PlaybackState::Paused;
                                } else {
                                    if self.playback.waypoints.is_empty() {
                                        if self.config.pattern_mode
                                            == crate::config::PatternMode::Spiral
                                        {
                                            self.playback.waypoints =
                                                crate::pattern::generate_spiral(
                                                    self.config.spiral_spacing,
                                                );
                                        }
                                    }
                                    self.playback.state = crate::pattern::PlaybackState::Playing;
                                }
                            }
                            if ui.button("Stop").clicked() {
                                self.playback.state = crate::pattern::PlaybackState::Stopped;
                                self.playback.current_idx = 0;
                            }
                            ui.checkbox(&mut self.playback.loop_pattern, "Loop");
                        });

                        if !self.playback.waypoints.is_empty() {
                            ui.label(format!(
                                "Progress: {} / {}",
                                self.playback.current_idx,
                                self.playback.waypoints.len()
                            ));
                        }
                    }

                    ui.add_space(12.0);
                    ui.label("Lighting Settings");

                    // LED Mode selection
                    egui::ComboBox::from_label("LED Mode")
                        .selected_text(match self.config.led_mode {
                            crate::config::LedMode::Single => "Single Direction",
                            crate::config::LedMode::RainbowRing => "Rainbow Ring",
                            crate::config::LedMode::ColorCycle => "Color Cycle Ring",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.config.led_mode,
                                crate::config::LedMode::Single,
                                "Single Direction",
                            );
                            ui.selectable_value(
                                &mut self.config.led_mode,
                                crate::config::LedMode::RainbowRing,
                                "Rainbow Ring",
                            );
                            ui.selectable_value(
                                &mut self.config.led_mode,
                                crate::config::LedMode::ColorCycle,
                                "Color Cycle Ring",
                            );
                        });

                    ui.add(
                        egui::Slider::new(&mut self.config.light_brightness, 0.0..=3.0)
                            .text("Brightness"),
                    );

                    ui.horizontal(|ui| {
                        ui.label("Sand Color:");
                        ui.color_edit_button_rgb(&mut self.config.sand_color);
                    });

                    if self.config.led_mode == crate::config::LedMode::Single {
                        ui.horizontal(|ui| {
                            ui.label("LED Color:");
                            ui.color_edit_button_rgb(&mut self.config.light_color);
                        });

                        ui.add(
                            egui::Slider::new(
                                &mut self.config.light_angle,
                                0.0..=std::f32::consts::TAU,
                            )
                            .text("LED Angle")
                            .show_value(false),
                        );
                    }

                    ui.checkbox(
                        &mut self.config.shadows_enabled,
                        "Enable Raymarched Shadows",
                    );

                    ui.add_space(20.0);
                    ui.horizontal(|ui| {
                        if ui.button("Reset Sand").clicked() {
                            self.sim.reset();
                            self.playback.state = crate::pattern::PlaybackState::Stopped;
                            self.playback.current_idx = 0;
                        }
                        if ui.button("Draw Ripples").clicked() {
                            crate::pattern::generate_ripples(&mut self.sim.heightmap);
                        }
                    });
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
                egui::pos2(
                    available_rect.min.x + offset_x,
                    available_rect.min.y + offset_y,
                ),
                egui::vec2(square_side, square_side),
            );

            // 2. Allocate the exact centered rect to align mouse interaction with the visuals
            let response = ui.allocate_rect(centered_rect, egui::Sense::drag());

            // Calculate current lighting uniforms
            let angle = self.config.light_angle;
            let x = angle.cos();
            let y = angle.sin();
            let z = 0.25; // low height angle for long shadows
            let light_dir_vec = glam::Vec3::new(x, y, z).normalize();

            let current_uniforms = crate::renderer::LightingUniforms {
                light_dir: [light_dir_vec.x, light_dir_vec.y, light_dir_vec.z, 0.0],
                light_color: [
                    self.config.light_color[0],
                    self.config.light_color[1],
                    self.config.light_color[2],
                    1.0,
                ],
                sand_color: [
                    self.config.sand_color[0],
                    self.config.sand_color[1],
                    self.config.sand_color[2],
                    1.0,
                ],
                light_brightness: self.config.light_brightness,
                shadow_enabled: if self.config.shadows_enabled { 1 } else { 0 },
                led_mode: match self.config.led_mode {
                    crate::config::LedMode::Single => 0,
                    crate::config::LedMode::RainbowRing => 1,
                    crate::config::LedMode::ColorCycle => 2,
                },
                time: self.elapsed_time % (2.0 * std::f32::consts::PI * 100.0),
                marble_pos: [self.sim.marble_pos.x, self.sim.marble_pos.y],
                marble_radius: self.config.marble_size,
                _padding2: 0,
            };

            // 3. Draw visuals centered in the allocated space via custom WGPU rendering
            ui.painter().add(egui_wgpu::Callback::new_paint_callback(
                centered_rect,
                crate::renderer::SandArtCallback {
                    heightmap_data: self.shared_heightmap.clone(),
                    uniforms: current_uniforms,
                },
            ));

            // 4. Capture target position from mouse or playback controller
            let mut target_pos = None;

            if self.config.pattern_mode == crate::config::PatternMode::Manual {
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
                        let clamped_pos = if len > max_r { pos / len * max_r } else { pos };
                        target_pos = Some(glam::Vec2::new(clamped_pos.x, clamped_pos.y));
                    }
                }
            } else {
                // Feed coordinates from PlaybackController when playing
                if self.playback.state == crate::pattern::PlaybackState::Playing {
                    if let Some(next_pos) =
                        self.playback
                            .step_playback(self.sim.marble_pos, self.config.speed, self.dt)
                    {
                        target_pos = Some(next_pos);
                    }
                }
            }

            // Run simulation tick
            self.sim
                .update(self.dt, target_pos, self.config.marble_size);
        });

        // Keep requesting frames for continuous physics simulation/render
        ctx.request_repaint();
    }
}
