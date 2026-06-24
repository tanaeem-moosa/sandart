use crate::config::AppConfig;
use egui;
use egui_wgpu;
use wgpu;

pub struct SandArtCallback {
    pub uniforms: crate::renderer::LightingUniforms,
    pub camera_uniforms: crate::renderer::CameraUniforms,
}

impl egui_wgpu::CallbackTrait for SandArtCallback {
    fn prepare(
        &self,
        _device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen_descriptor: &egui_wgpu::ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if let Some(res) = resources.get_mut::<crate::renderer::HeightmapRenderer>() {
            res.update_uniforms(queue, &self.uniforms);
            res.update_camera(queue, &self.camera_uniforms);
        }
        vec![]
    }

    fn paint(
        &self,
        info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        if let Some(res) = resources.get::<crate::renderer::HeightmapRenderer>() {
            let pixels_per_point = info.pixels_per_point;
            let rect = info.viewport;

            let target_width = info.screen_size_px[0] as f32;
            let target_height = info.screen_size_px[1] as f32;

            let physical_x = (rect.min.x * pixels_per_point).clamp(0.0, target_width);
            let physical_y = (rect.min.y * pixels_per_point).clamp(0.0, target_height);
            let physical_width = (rect.width() * pixels_per_point).min(target_width - physical_x);
            let physical_height =
                (rect.height() * pixels_per_point).min(target_height - physical_y);

            if physical_width > 0.0 && physical_height > 0.0 {
                render_pass.set_viewport(
                    physical_x,
                    physical_y,
                    physical_width,
                    physical_height,
                    0.0,
                    1.0,
                );

                res.draw(render_pass, &self.camera_uniforms, &self.uniforms);
            }
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
    /// Cumulative elapsed time in seconds.
    pub elapsed_time: f32,
    /// The physics simulation engine.
    pub sim: crate::sim::DrawingSimulation,
    /// Flag indicating that a full heightmap upload is required (e.g. startup or reset).
    pub full_upload_needed: bool,
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
    /// Track the current clock minute for Clock Mode transitions.
    pub clock_minute: u32,
}

impl SandArtApp {
    /// Create a new instance of the application.
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        if let Some(wgpu_state) = &cc.wgpu_render_state {
            let device = &wgpu_state.device;
            let target_format = wgpu_state.target_format;
            let mut resources = crate::renderer::HeightmapRenderer::new(device, target_format);
            let default_color = vec![255u8; 1024 * 1024 * 4];
            resources.update_colormap(&wgpu_state.queue, &default_color);
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
            sim: crate::sim::DrawingSimulation::new(),
            full_upload_needed: true,
            playback: crate::pattern::PlaybackController::new(),
            pattern_error: None,
            sample_patterns,
            settings_open: true,
            camera_azimuth: 0.0,
            camera_elevation: 0.8, // ~45 degrees
            camera_zoom: 2.8,
            clock_minute: 99,
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
                        let mut waypoints = crate::pattern::close_loop_path(waypoints);
                        if self.config.sandbox_shape == crate::config::SandboxShape::Oval {
                            for pt in waypoints.iter_mut() {
                                pt.y *= 0.652;
                            }
                        }
                        if waypoints.is_empty() {
                            self.pattern_error = Some("Parsed pattern contains no waypoints".to_string());
                            return;
                        }
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
        self.playback.loop_pattern = self.config.pattern_mode != crate::config::PatternMode::Clock;
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
            crate::config::PatternMode::Butterfly => {
                crate::pattern::generate_butterfly_curve()
            }
            crate::config::PatternMode::ZenWaves => {
                crate::pattern::generate_zen_waves()
            }
            crate::config::PatternMode::ZenMandala => {
                crate::pattern::generate_zen_mandala()
            }
            crate::config::PatternMode::Clock => {
                use chrono::Timelike;
                let now = chrono::Local::now();
                let h = now.hour();
                let m = now.minute();
                self.clock_minute = m;
                crate::pattern::generate_clock_pattern(h, m, 0.0, 1)
            }
            crate::config::PatternMode::Dinosaur => {
                crate::pattern::generate_dinosaur()
            }
            crate::config::PatternMode::Unicorn => {
                crate::pattern::generate_unicorn()
            }
            crate::config::PatternMode::MultiMarble => {
                let paths = crate::pattern::generate_multi_spiral(
                    self.config.spiral_spacing,
                    self.config.marble_count as usize,
                );
                for j in 0..5 {
                    if j < paths.len() {
                        let mut path = paths[j].clone();
                        if self.config.sandbox_shape == crate::config::SandboxShape::Oval {
                            for pt in path.iter_mut() {
                                pt.y *= 0.652;
                            }
                        }
                        self.playback.waypoints[j] = crate::pattern::close_loop_path(path);
                        self.playback.current_indices[j] = 0;

                        // Snap simulation marble to its starting waypoint position
                        self.playback.waypoints[j] = crate::pattern::close_loop_path(paths[j].clone());
                        if self.config.sandbox_shape == crate::config::SandboxShape::Oval {
                            for pt in self.playback.waypoints[j].iter_mut() {
                                pt.y *= 0.652;
                            }
                        }
                        
                        let start_pos = self.playback.waypoints[j][0];
                        self.sim.marbles[j].pos = start_pos;
                        self.sim.marbles[j].prev_pos = start_pos;
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

        let mut base_waypoints = crate::pattern::close_loop_path(base_waypoints);
        if base_waypoints.is_empty() {
            self.pattern_error = Some("Failed to generate pattern waypoints".to_string());
            return;
        }

        if self.config.sandbox_shape == crate::config::SandboxShape::Oval {
            for pt in base_waypoints.iter_mut() {
                pt.y *= 0.652;
            }
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
        let old_shape = self.config.sandbox_shape;
        self.frame_counter += 1;

        // Track frame delta time for frame-rate-independent physics calculations
        self.dt = ctx.input(|i| i.stable_dt).min(0.1);
        self.elapsed_time += self.dt;

        // Dynamic clock state checking if clock mode is selected
        if self.config.pattern_mode == crate::config::PatternMode::Clock {
            use chrono::Timelike;
            let now = chrono::Local::now();
            let m = now.minute();

            if m != self.clock_minute {
                self.sim.reset();
                self.full_upload_needed = true;
                self.clock_minute = m;
                self.load_selected_pattern();
            }
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
                            changed |= ui
                                .selectable_value(
                                    &mut self.config.pattern_mode,
                                    crate::config::PatternMode::Butterfly,
                                    "Butterfly Curve",
                                )
                                .changed();
                            changed |= ui
                                .selectable_value(
                                    &mut self.config.pattern_mode,
                                    crate::config::PatternMode::ZenWaves,
                                    "Zen Waves",
                                )
                                .changed();
                            changed |= ui
                                .selectable_value(
                                    &mut self.config.pattern_mode,
                                    crate::config::PatternMode::ZenMandala,
                                    "Zen Mandala",
                                )
                                .changed();
                            changed |= ui
                                .selectable_value(
                                    &mut self.config.pattern_mode,
                                    crate::config::PatternMode::Dinosaur,
                                    "Dinosaur Outline",
                                )
                                .changed();
                            changed |= ui
                                .selectable_value(
                                    &mut self.config.pattern_mode,
                                    crate::config::PatternMode::Unicorn,
                                    "Unicorn Outline",
                                )
                                .changed();
                            changed |= ui
                                .selectable_value(
                                    &mut self.config.pattern_mode,
                                    crate::config::PatternMode::Clock,
                                    "Clock Mode",
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
                            crate::config::MaterialMode::Water => "Water (Ripples)",
                            crate::config::MaterialMode::Milk => "Milk (Thick Liquid)",
                            crate::config::MaterialMode::VegetableOil => "Vegetable Oil (Transparent Viscous)",
                            crate::config::MaterialMode::CalmWater => "Water (Calm/Glassy)",
                            crate::config::MaterialMode::Yogurt => "Yogurt (Thick/Creamy)",
                            crate::config::MaterialMode::CoarseSand => "Coarse Sand (Large Grain)",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.config.material_mode, crate::config::MaterialMode::DrySand, "Dry Sand (Granular)");
                            ui.selectable_value(&mut self.config.material_mode, crate::config::MaterialMode::KineticSand, "Kinetic Sand");
                            ui.selectable_value(&mut self.config.material_mode, crate::config::MaterialMode::WetSand, "Wet Sand");
                            ui.selectable_value(&mut self.config.material_mode, crate::config::MaterialMode::CoarseSand, "Coarse Sand (Large Grain)");
                            ui.selectable_value(&mut self.config.material_mode, crate::config::MaterialMode::ButterCream, "Butter-Cream (Viscous)");
                            ui.selectable_value(&mut self.config.material_mode, crate::config::MaterialMode::Snow, "Snow (Cohesive)");
                            ui.selectable_value(&mut self.config.material_mode, crate::config::MaterialMode::FinePowder, "Fine Powder");
                            ui.selectable_value(&mut self.config.material_mode, crate::config::MaterialMode::Oobleck, "Oobleck (Non-Newtonian)");
                            ui.selectable_value(&mut self.config.material_mode, crate::config::MaterialMode::MoonDust, "Moon Dust");
                            ui.selectable_value(&mut self.config.material_mode, crate::config::MaterialMode::Water, "Water (Ripples)");
                            ui.selectable_value(&mut self.config.material_mode, crate::config::MaterialMode::Milk, "Milk (Thick Liquid)");
                            ui.selectable_value(&mut self.config.material_mode, crate::config::MaterialMode::VegetableOil, "Vegetable Oil (Transparent Viscous)");
                            ui.selectable_value(&mut self.config.material_mode, crate::config::MaterialMode::CalmWater, "Water (Calm/Glassy)");
                            ui.selectable_value(&mut self.config.material_mode, crate::config::MaterialMode::Yogurt, "Yogurt (Thick/Creamy)");
                        });

                    ui.add_space(12.0);
                    ui.label("Sandbox Shape");
                    egui::ComboBox::from_label("Shape")
                        .selected_text(match self.config.sandbox_shape {
                            crate::config::SandboxShape::Circle => "Circle",
                            crate::config::SandboxShape::Square => "Square",
                            crate::config::SandboxShape::Oval => "Oval",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.config.sandbox_shape, crate::config::SandboxShape::Circle, "Circle");
                            ui.selectable_value(&mut self.config.sandbox_shape, crate::config::SandboxShape::Square, "Square");
                            ui.selectable_value(&mut self.config.sandbox_shape, crate::config::SandboxShape::Oval, "Oval");
                        });

                    ui.add_space(12.0);
                    ui.label("Lighting Settings");

                    // LED Mode selection
                    egui::ComboBox::from_label("LED Mode")
                        .selected_text(match self.config.led_mode {
                            crate::config::LedMode::Single => "Single Direction",
                            crate::config::LedMode::RainbowRing => "Rainbow Ring",
                            crate::config::LedMode::ColorCycle => "Color Cycle Ring",
                            crate::config::LedMode::OverheadMoon => "Overhead Moon Light",
                            crate::config::LedMode::RainbowMoon => "Rainbow Ring + Moon",
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
                            ui.selectable_value(
                                &mut self.config.led_mode,
                                crate::config::LedMode::OverheadMoon,
                                "Overhead Moon Light",
                            );
                            ui.selectable_value(
                                &mut self.config.led_mode,
                                crate::config::LedMode::RainbowMoon,
                                "Rainbow Ring + Moon",
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

                    ui.horizontal(|ui| {
                        if ui.button("Reset Sand").clicked() {
                            self.sim.reset();
                            self.full_upload_needed = true;
                            self.playback.state = crate::pattern::PlaybackState::Stopped;
                            self.playback.current_indices = [0; 5];
                        }
                        if ui.button("Draw Ripples").clicked() {
                            self.sim.heightmap.generate_ripples();
                            self.full_upload_needed = true;
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
            let z = 0.08; // very low grazing angle for realistic long shadows
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
                    crate::config::LedMode::OverheadMoon => 3,
                    crate::config::LedMode::RainbowMoon => 4,
                },
                time: self.elapsed_time % (2.0 * std::f32::consts::PI * 100.0),
                marble_count: self.config.marble_count,
                material_mode: self.config.material_mode as u32,
                sandbox_shape: self.config.sandbox_shape as u32,
                color_mode: 0,
                marbles: [
                    crate::renderer::MarbleUniform {
                        pos: [self.sim.marbles[0].pos.x, self.sim.marbles[0].pos.y],
                        radius: self.config.marble_size,
                        z_pos: {
                            let (gx, gy) = crate::sim::DrawingSimulation::norm_to_grid(
                                self.sim.marbles[0].pos,
                                self.sim.heightmap.width,
                                self.sim.heightmap.height,
                            );
                            self.sim.heightmap.get(gx, gy)
                        },
                    },
                    crate::renderer::MarbleUniform {
                        pos: [self.sim.marbles[1].pos.x, self.sim.marbles[1].pos.y],
                        radius: self.config.marble_size,
                        z_pos: {
                            let (gx, gy) = crate::sim::DrawingSimulation::norm_to_grid(
                                self.sim.marbles[1].pos,
                                self.sim.heightmap.width,
                                self.sim.heightmap.height,
                            );
                            self.sim.heightmap.get(gx, gy)
                        },
                    },
                    crate::renderer::MarbleUniform {
                        pos: [self.sim.marbles[2].pos.x, self.sim.marbles[2].pos.y],
                        radius: self.config.marble_size,
                        z_pos: {
                            let (gx, gy) = crate::sim::DrawingSimulation::norm_to_grid(
                                self.sim.marbles[2].pos,
                                self.sim.heightmap.width,
                                self.sim.heightmap.height,
                            );
                            self.sim.heightmap.get(gx, gy)
                        },
                    },
                    crate::renderer::MarbleUniform {
                        pos: [self.sim.marbles[3].pos.x, self.sim.marbles[3].pos.y],
                        radius: self.config.marble_size,
                        z_pos: {
                            let (gx, gy) = crate::sim::DrawingSimulation::norm_to_grid(
                                self.sim.marbles[3].pos,
                                self.sim.heightmap.width,
                                self.sim.heightmap.height,
                            );
                            self.sim.heightmap.get(gx, gy)
                        },
                    },
                    crate::renderer::MarbleUniform {
                        pos: [self.sim.marbles[4].pos.x, self.sim.marbles[4].pos.y],
                        radius: self.config.marble_size,
                        z_pos: {
                            let (gx, gy) = crate::sim::DrawingSimulation::norm_to_grid(
                                self.sim.marbles[4].pos,
                                self.sim.heightmap.width,
                                self.sim.heightmap.height,
                            );
                            self.sim.heightmap.get(gx, gy)
                        },
                    },
                ],
            };

            // 3. Draw visuals centered in the allocated space via custom WGPU rendering
            ui.painter().add(egui_wgpu::Callback::new_paint_callback(
                centered_rect,
                SandArtCallback {
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
                    let current_speed = self.config.speed;
                    targets = self.playback.step_playback_all(
                        &marble_positions,
                        self.config.marble_count as usize,
                        current_speed,
                        self.dt,
                    );
                }
            }

            // Run simulation tick
            self.sim.update(self.dt, &targets, self.config.marble_size, self.config.material_mode, self.config.sandbox_shape, self.dt * 1000.0, self.dt * 1000.0);

            // Direct GPU update (zero-mutex, zero-CPU-to-CPU copies)
            if let Some(wgpu_state) = _frame.wgpu_render_state() {
                let mut renderer = wgpu_state.renderer.write();
                if let Some(res) = renderer.callback_resources.get_mut::<crate::renderer::HeightmapRenderer>() {
                    if self.full_upload_needed {
                        res.update_heightmap(&wgpu_state.queue, self.sim.heightmap.as_slice());
                        self.full_upload_needed = false;
                    } else {
                        let bounds = self.sim.active_bounds;
                        let render_bounds = crate::renderer::ActiveBounds {
                            min_x: bounds.min_x,
                            max_x: bounds.max_x,
                            min_y: bounds.min_y,
                            max_y: bounds.max_y,
                            active: bounds.active,
                        };
                        res.update_heightmap_partial(
                            &wgpu_state.queue,
                            self.sim.heightmap.as_slice(),
                            render_bounds,
                        );
                    }
                }
            }
        });

        if self.config.sandbox_shape != old_shape && self.config.pattern_mode != crate::config::PatternMode::Manual {
            self.load_selected_pattern();
        }

        // Keep requesting frames for continuous physics simulation/render
        ctx.request_repaint();
    }
}
