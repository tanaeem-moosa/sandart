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
    /// Toggle state for collapsible side panel UI.
    pub settings_open: bool,
    /// Camera azimuth (yaw) angle in radians.
    pub camera_azimuth: f32,
    /// Camera elevation (pitch) angle in radians.
    pub camera_elevation: f32,
    /// Camera zoom (distance from origin).
    pub camera_zoom: f32,
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
            shared_heightmap: std::sync::Arc::new(std::sync::Mutex::new(vec![
                0.8;
                crate::sim::GRID_SIZE * crate::sim::GRID_SIZE
            ])),
            playback: crate::pattern::PlaybackController::new(),
            pattern_error: None,
            sample_patterns,
            settings_open: true,
            camera_azimuth: 0.0,
            camera_elevation: 0.8, // ~45 degrees
            camera_zoom: 2.8,
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
                        self.playback.clear_waypoints();
                        let count = self.config.marble_count.clamp(1, 5) as usize;
                        for j in 0..5 {
                            if j < count {
                                self.playback.waypoints[j] = waypoints.clone();
                                let start_idx = (j * waypoints.len() / count) % waypoints.len();
                                self.playback.current_indices[j] = start_idx;
                                
                                // Snap simulation marble to its starting waypoint position
                                self.sim.marbles[j].pos = waypoints[start_idx];
                                self.sim.marbles[j].prev_pos = waypoints[start_idx];
                                self.sim.marbles[j].vel = glam::Vec2::ZERO;
                                self.sim.marbles[j].was_active = true;
                            } else {
                                self.sim.marbles[j].was_active = false;
                            }
                        }
                        if count > 0 {
                            self.sim.marble_pos = self.sim.marbles[0].pos;
                            self.sim.prev_marble_pos = self.sim.marbles[0].prev_pos;
                            self.sim.marble_vel = self.sim.marbles[0].vel;
                            self.sim.was_active = self.sim.marbles[0].was_active;
                        }
                        self.playback.randomize_speeds(count, self.sim.seed);
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

    /// Helper to load the selected mathematical pattern based on config parameters
    fn load_selected_pattern(&mut self) {
        self.playback.clear_waypoints();
        self.pattern_error = None;

        let base_waypoints = match self.config.pattern_mode {
            crate::config::PatternMode::Spiral => {
                crate::pattern::generate_spiral(self.config.spiral_spacing)
            }
            crate::config::PatternMode::Lissajous => {
                crate::pattern::generate_lissajous(
                    self.config.lissajous_a,
                    self.config.lissajous_b,
                    1.5707963,
                )
            }
            crate::config::PatternMode::Rose => {
                crate::pattern::generate_rose(self.config.rose_k)
            }
            crate::config::PatternMode::Hypotrochoid => {
                crate::pattern::generate_hypotrochoid(
                    self.config.hypotrochoid_r,
                    self.config.hypotrochoid_d,
                )
            }
            crate::config::PatternMode::FermatSpiral => {
                crate::pattern::generate_fermat_spiral(self.config.rose_k)
            }
            crate::config::PatternMode::HilbertCurve => {
                crate::pattern::generate_hilbert_curve(self.config.hilbert_order)
            }
            crate::config::PatternMode::GosperCurve => {
                crate::pattern::generate_gosper_curve(self.config.hilbert_order)
            }
            crate::config::PatternMode::SierpinskiCurve => {
                crate::pattern::generate_sierpinski_curve(self.config.hilbert_order)
            }
            crate::config::PatternMode::RandomWalk => {
                crate::pattern::generate_random_walk(
                    self.config.random_walk_steps as usize,
                    self.config.random_walk_step_size,
                )
            }
            crate::config::PatternMode::Lemniscate => {
                crate::pattern::generate_lemniscate(0.8)
            }
            crate::config::PatternMode::MultiMarble => {
                let paths = crate::pattern::generate_multi_spiral(
                    self.config.spiral_spacing,
                    self.config.marble_count as usize,
                );
                for j in 0..5 {
                    if j < paths.len() {
                        self.playback.waypoints[j] = paths[j].clone();
                        self.playback.current_indices[j] = 0;

                        // Snap simulation marble to its starting waypoint position
                        self.sim.marbles[j].pos = paths[j][0];
                        self.sim.marbles[j].prev_pos = paths[j][0];
                        self.sim.marbles[j].vel = glam::Vec2::ZERO;
                        self.sim.marbles[j].was_active = true;
                    } else {
                        self.sim.marbles[j].was_active = false;
                    }
                }
                if !paths.is_empty() {
                    self.sim.marble_pos = self.sim.marbles[0].pos;
                    self.sim.prev_marble_pos = self.sim.marbles[0].prev_pos;
                    self.sim.marble_vel = self.sim.marbles[0].vel;
                    self.sim.was_active = self.sim.marbles[0].was_active;
                }
                self.playback.randomize_speeds(paths.len(), self.sim.seed);
                self.playback.state = crate::pattern::PlaybackState::Playing;
                return;
            }
            crate::config::PatternMode::CustomFile => {
                self.load_custom_pattern();
                return;
            }
            _ => return,
        };

        if base_waypoints.is_empty() {
            self.pattern_error = Some("Failed to generate pattern waypoints".to_string());
            return;
        }

        let count = self.config.marble_count.clamp(1, 5) as usize;
        for j in 0..5 {
            if j < count {
                self.playback.waypoints[j] = base_waypoints.clone();
                let start_idx = (j * base_waypoints.len() / count) % base_waypoints.len();
                self.playback.current_indices[j] = start_idx;

                // Snap simulation marble to its starting waypoint position
                self.sim.marbles[j].pos = base_waypoints[start_idx];
                self.sim.marbles[j].prev_pos = base_waypoints[start_idx];
                self.sim.marbles[j].vel = glam::Vec2::ZERO;
                self.sim.marbles[j].was_active = true;
            } else {
                self.sim.marbles[j].was_active = false;
            }
        }
        if count > 0 {
            self.sim.marble_pos = self.sim.marbles[0].pos;
            self.sim.prev_marble_pos = self.sim.marbles[0].prev_pos;
            self.sim.marble_vel = self.sim.marbles[0].vel;
            self.sim.was_active = self.sim.marbles[0].was_active;
        }
        self.playback.randomize_speeds(count, self.sim.seed);
        self.playback.state = crate::pattern::PlaybackState::Playing;
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

                ui.add_space(16.0);
                let btn_text = if self.settings_open { "Hide Controls ⚙" } else { "Show Controls ⚙" };
                if ui.button(btn_text).clicked() {
                    self.settings_open = !self.settings_open;
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(format!(
                        "Frames: {} | dt: {:.4}s",
                        self.frame_counter, self.dt
                    ));
                });
            });
        });

        // Draw the left control panel
        if self.settings_open {
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
                        egui::Slider::new(&mut self.config.marble_size, 0.015..=0.045)
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
                            changed |= ui
                                .selectable_value(
                                    &mut self.config.pattern_mode,
                                    crate::config::PatternMode::Lissajous,
                                    "Lissajous Curve",
                                )
                                .changed();
                            changed |= ui
                                .selectable_value(
                                    &mut self.config.pattern_mode,
                                    crate::config::PatternMode::Rose,
                                    "Rose Curve",
                                )
                                .changed();
                            changed |= ui
                                .selectable_value(
                                    &mut self.config.pattern_mode,
                                    crate::config::PatternMode::Hypotrochoid,
                                    "Hypotrochoid (Spirograph)",
                                )
                                .changed();
                            changed |= ui
                                .selectable_value(
                                    &mut self.config.pattern_mode,
                                    crate::config::PatternMode::FermatSpiral,
                                    "Fermat Spiral",
                                )
                                .changed();
                            changed |= ui
                                .selectable_value(
                                    &mut self.config.pattern_mode,
                                    crate::config::PatternMode::HilbertCurve,
                                    "Hilbert Curve (Space-filling)",
                                )
                                .changed();
                            changed |= ui
                                .selectable_value(
                                    &mut self.config.pattern_mode,
                                    crate::config::PatternMode::GosperCurve,
                                    "Gosper Curve (Hexagonal)",
                                )
                                .changed();
                            changed |= ui
                                .selectable_value(
                                    &mut self.config.pattern_mode,
                                    crate::config::PatternMode::SierpinskiCurve,
                                    "Sierpinski Curve (Arrowhead)",
                                )
                                .changed();
                            changed |= ui
                                .selectable_value(
                                    &mut self.config.pattern_mode,
                                    crate::config::PatternMode::RandomWalk,
                                    "Random Walk (Brownian)",
                                )
                                .changed();
                            changed |= ui
                                .selectable_value(
                                    &mut self.config.pattern_mode,
                                    crate::config::PatternMode::Lemniscate,
                                    "Lemniscate (Infinity)",
                                )
                                .changed();
                            changed |= ui
                                .selectable_value(
                                    &mut self.config.pattern_mode,
                                    crate::config::PatternMode::MultiMarble,
                                    "Multi-Marble Drawing",
                                )
                                .changed();
                            if changed {
                                self.playback.state = crate::pattern::PlaybackState::Stopped;
                                self.playback.clear_waypoints();
                                self.pattern_error = None;
                            }
                        });

                    if self.config.pattern_mode != crate::config::PatternMode::Manual {
                        ui.add_space(8.0);
                        ui.add(
                            egui::Slider::new(&mut self.config.marble_count, 1..=5)
                                .text("Marble Count")
                                .show_value(true),
                        );

                        match self.config.pattern_mode {
                            crate::config::PatternMode::Spiral | crate::config::PatternMode::MultiMarble => {
                                ui.add_space(8.0);
                                ui.add(
                                    egui::Slider::new(&mut self.config.spiral_spacing, 0.005..=0.20)
                                        .text("Spiral Spacing (R)")
                                        .show_value(true),
                                );
                            }
                            crate::config::PatternMode::Lissajous => {
                                ui.add_space(8.0);
                                ui.add(
                                    egui::Slider::new(&mut self.config.lissajous_a, 1.0..=10.0)
                                        .text("Frequency a")
                                        .show_value(true),
                                );
                                ui.add(
                                    egui::Slider::new(&mut self.config.lissajous_b, 1.0..=10.0)
                                        .text("Frequency b")
                                        .show_value(true),
                                );
                            }
                            crate::config::PatternMode::Rose => {
                                ui.add_space(8.0);
                                ui.add(
                                    egui::Slider::new(&mut self.config.rose_k, 1.0..=12.0)
                                        .text("Petal Factor k")
                                        .show_value(true),
                                );
                            }
                            crate::config::PatternMode::FermatSpiral => {
                                ui.add_space(8.0);
                                ui.add(
                                    egui::Slider::new(&mut self.config.rose_k, 1.0..=20.0)
                                        .text("Turns")
                                        .show_value(true),
                                );
                            }
                            crate::config::PatternMode::Hypotrochoid => {
                                ui.add_space(8.0);
                                ui.add(
                                    egui::Slider::new(&mut self.config.hypotrochoid_r, 0.05..=0.90)
                                        .text("Inner Radius r")
                                        .show_value(true),
                                );
                                ui.add(
                                    egui::Slider::new(&mut self.config.hypotrochoid_d, 0.01..=0.80)
                                        .text("Pen Distance d")
                                        .show_value(true),
                                );
                            }
                            crate::config::PatternMode::HilbertCurve => {
                                ui.add_space(8.0);
                                ui.add(
                                    egui::Slider::new(&mut self.config.hilbert_order, 1..=6)
                                        .text("Recursion Order")
                                        .show_value(true),
                                );
                            }
                            crate::config::PatternMode::GosperCurve => {
                                ui.add_space(8.0);
                                ui.add(
                                    egui::Slider::new(&mut self.config.hilbert_order, 1..=5)
                                        .text("Recursion Order")
                                        .show_value(true),
                                );
                            }
                            crate::config::PatternMode::SierpinskiCurve => {
                                ui.add_space(8.0);
                                ui.add(
                                    egui::Slider::new(&mut self.config.hilbert_order, 1..=7)
                                        .text("Recursion Order")
                                        .show_value(true),
                                );
                            }
                            crate::config::PatternMode::RandomWalk => {
                                ui.add_space(8.0);
                                ui.add(
                                    egui::Slider::new(&mut self.config.random_walk_steps, 100..=3000)
                                        .text("Steps")
                                        .show_value(true),
                                );
                                ui.add(
                                    egui::Slider::new(&mut self.config.random_walk_step_size, 0.005..=0.10)
                                        .text("Step Size")
                                        .show_value(true),
                                );
                            }
                            crate::config::PatternMode::CustomFile => {
                                ui.add_space(8.0);
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
                            }
                            _ => {}
                        }

                        ui.add_space(8.0);
                        if ui.button("Load / Restart Pattern").clicked() {
                            self.load_selected_pattern();
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
                                    if self.playback.waypoints[0].is_empty() {
                                        self.load_selected_pattern();
                                    }
                                    self.playback.state = crate::pattern::PlaybackState::Playing;
                                }
                            }
                            if ui.button("Stop").clicked() {
                                self.playback.state = crate::pattern::PlaybackState::Stopped;
                                self.playback.current_indices = [0; 5];
                            }
                            ui.checkbox(&mut self.playback.loop_pattern, "Loop");
                        });

                        if !self.playback.waypoints[0].is_empty() {
                            ui.label(format!(
                                "Progress: {} / {}",
                                self.playback.current_indices[0],
                                self.playback.waypoints[0].len()
                            ));
                        }
                    }
                    ui.add_space(12.0);
                    ui.label("Material Physics");
                    egui::ComboBox::from_label("Preset")
                        .selected_text(match self.config.material_mode {
                            crate::config::MaterialMode::ButterCream => "Butter-Cream (Viscous)",
                            crate::config::MaterialMode::DrySand => "Dry Sand (Granular)",
                            crate::config::MaterialMode::Snow => "Snow (Cohesive)",
                            crate::config::MaterialMode::KineticSand => "Kinetic Sand",
                            crate::config::MaterialMode::WetSand => "Wet Sand",
                            crate::config::MaterialMode::FinePowder => "Fine Powder",
                            crate::config::MaterialMode::Oobleck => "Oobleck (Non-Newtonian)",
                            crate::config::MaterialMode::MoonDust => "Moon Dust",
                            crate::config::MaterialMode::IronFilings => "Iron Filings",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.config.material_mode, crate::config::MaterialMode::ButterCream, "Butter-Cream (Viscous)");
                            ui.selectable_value(&mut self.config.material_mode, crate::config::MaterialMode::DrySand, "Dry Sand (Granular)");
                            ui.selectable_value(&mut self.config.material_mode, crate::config::MaterialMode::Snow, "Snow (Cohesive)");
                            ui.selectable_value(&mut self.config.material_mode, crate::config::MaterialMode::KineticSand, "Kinetic Sand");
                            ui.selectable_value(&mut self.config.material_mode, crate::config::MaterialMode::WetSand, "Wet Sand");
                            ui.selectable_value(&mut self.config.material_mode, crate::config::MaterialMode::FinePowder, "Fine Powder");
                            ui.selectable_value(&mut self.config.material_mode, crate::config::MaterialMode::Oobleck, "Oobleck (Non-Newtonian)");
                            ui.selectable_value(&mut self.config.material_mode, crate::config::MaterialMode::MoonDust, "Moon Dust");
                            ui.selectable_value(&mut self.config.material_mode, crate::config::MaterialMode::IronFilings, "Iron Filings");
                        });

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
                            self.playback.current_indices = [0; 5];
                        }
                        if ui.button("Draw Ripples").clicked() {
                            crate::pattern::generate_ripples(&mut self.sim.heightmap);
                        }
                    });
                });
            });
        }

        // Draw the central canvas
        egui::CentralPanel::default().show(ctx, |ui| {
            // Add a small helper text at the top of the canvas
            ui.vertical_centered(|ui| {
                ui.label(egui::RichText::new("💡 Drag to rotate camera • Scroll to zoom • Shift + Drag to move marble").small());
            });

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

            // 2.5. Camera Controller updates (scrolling zooms, dragging orbits)
            let scroll_delta = ctx.input(|i| i.smooth_scroll_delta.y);
            if scroll_delta.abs() > 0.0 {
                self.camera_zoom = (self.camera_zoom - scroll_delta * 0.005).clamp(1.2, 5.0);
            }

            if response.dragged() && !ctx.input(|i| i.modifiers.shift) {
                let delta = response.drag_delta();
                self.camera_azimuth += delta.x * 0.005;
                self.camera_elevation += delta.y * 0.005;
                self.camera_elevation = self.camera_elevation.clamp(0.05, std::f32::consts::FRAC_PI_2 - 0.05);
            }

            let cos_elev = self.camera_elevation.cos();
            let sin_elev = self.camera_elevation.sin();
            let cos_az = self.camera_azimuth.cos();
            let sin_az = self.camera_azimuth.sin();

            let eye_x = self.camera_zoom * cos_elev * cos_az;
            let eye_y = self.camera_zoom * cos_elev * sin_az;
            let eye_z = self.camera_zoom * sin_elev;
            let eye = glam::Vec3::new(eye_x, eye_y, eye_z);

            let view = glam::Mat4::look_at_lh(eye, glam::Vec3::ZERO, glam::Vec3::Z);
            let proj = glam::Mat4::perspective_lh(45.0f32.to_radians(), 1.0, 0.01, 100.0);
            let view_proj = proj * view;

            let camera_uniforms = crate::renderer::CameraUniforms {
                view_proj: view_proj.to_cols_array(),
                camera_pos: [eye.x, eye.y, eye.z, 0.0],
            };

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
                marble_count: self.config.marble_count,
                material_mode: self.config.material_mode as u32,
                _padding: [0, 0],
                marbles: [
                    crate::renderer::MarbleUniform {
                        pos: [self.sim.marbles[0].pos.x, self.sim.marbles[0].pos.y],
                        radius: self.config.marble_size,
                        padding: 0.0,
                    },
                    crate::renderer::MarbleUniform {
                        pos: [self.sim.marbles[1].pos.x, self.sim.marbles[1].pos.y],
                        radius: self.config.marble_size,
                        padding: 0.0,
                    },
                    crate::renderer::MarbleUniform {
                        pos: [self.sim.marbles[2].pos.x, self.sim.marbles[2].pos.y],
                        radius: self.config.marble_size,
                        padding: 0.0,
                    },
                    crate::renderer::MarbleUniform {
                        pos: [self.sim.marbles[3].pos.x, self.sim.marbles[3].pos.y],
                        radius: self.config.marble_size,
                        padding: 0.0,
                    },
                    crate::renderer::MarbleUniform {
                        pos: [self.sim.marbles[4].pos.x, self.sim.marbles[4].pos.y],
                        radius: self.config.marble_size,
                        padding: 0.0,
                    },
                ],
            };

            // 3. Draw visuals centered in the allocated space via custom WGPU rendering
            ui.painter().add(egui_wgpu::Callback::new_paint_callback(
                centered_rect,
                crate::renderer::SandArtCallback {
                    heightmap_data: self.shared_heightmap.clone(),
                    uniforms: current_uniforms,
                    camera_uniforms,
                },
            ));

            // 4. Capture target position from mouse or playback controller
            let mut targets = [None; 5];

            if self.config.pattern_mode == crate::config::PatternMode::Manual {
                let mut target_pos = None;
                if (response.dragged() || response.clicked()) && ctx.input(|i| i.modifiers.shift) && radius > 1e-4 {
                    if let Some(pointer_pos) = response.interact_pointer_pos() {
                        // Project screen space mouse coordinate to XY plane
                        let ndc_x = (pointer_pos.x - centered_rect.center().x) / radius;
                        let ndc_y = -(pointer_pos.y - centered_rect.center().y) / radius;

                        let ndc_near = glam::Vec4::new(ndc_x, ndc_y, 0.0, 1.0);
                        let ndc_far = glam::Vec4::new(ndc_x, ndc_y, 1.0, 1.0);

                        let view_proj_inv = view_proj.inverse();
                        let w_near_h = view_proj_inv * ndc_near;
                        let w_far_h = view_proj_inv * ndc_far;

                        let w_near = glam::Vec3::new(
                            w_near_h.x / w_near_h.w,
                            w_near_h.y / w_near_h.w,
                            w_near_h.z / w_near_h.w,
                        );
                        let w_far = glam::Vec3::new(
                            w_far_h.x / w_far_h.w,
                            w_far_h.y / w_far_h.w,
                            w_far_h.z / w_far_h.w,
                        );

                        let ray_dir = (w_far - w_near).normalize();

                        if ray_dir.z.abs() > 1e-6 {
                            let t = -w_near.z / ray_dir.z;
                            let intersect = w_near + t * ray_dir;

                            let pos = egui::vec2(intersect.x, intersect.y);
                            let len = pos.length();
                            // Constrain marble center to visual sand circle bounds (0.92 radius in Cartesian)
                            let max_r = (0.92 - self.config.marble_size).max(0.0);
                            let clamped_pos = if len > max_r { pos / len * max_r } else { pos };
                            target_pos = Some(glam::Vec2::new(clamped_pos.x, clamped_pos.y));
                        }
                    }
                }
                targets[0] = target_pos;
            } else {
                // Feed coordinates from PlaybackController when playing
                if self.playback.state == crate::pattern::PlaybackState::Playing {
                    let marble_positions = [
                        self.sim.marbles[0].pos,
                        self.sim.marbles[1].pos,
                        self.sim.marbles[2].pos,
                        self.sim.marbles[3].pos,
                        self.sim.marbles[4].pos,
                    ];
                    targets = self.playback.step_playback_all(
                        &marble_positions,
                        self.config.marble_count as usize,
                        self.config.speed,
                        self.dt,
                    );
                }
            }

            // Run simulation tick
            self.sim.update(self.dt, &targets, self.config.marble_size, self.config.material_mode);
        });

        // Keep requesting frames for continuous physics simulation/render
        ctx.request_repaint();
    }
}
