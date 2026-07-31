#![cfg(target_arch = "wasm32")]

use wasm_bindgen::prelude::*;
use wgpu;
use js_sys;
use web_sys;
use wasm_bindgen::JsCast;

use sandart_sim::{DrawingSimulation, MaterialMode, QuantileMode, SandboxShape, SimulatorMode, MAX_QUANTILE_LINES};
use sandart_render::{HeightmapRenderer, CameraUniforms, LightingUniforms, MarbleUniform};
use sandart_pattern::{PlaybackController, PlaybackState, parse_gcode, parse_thr};

#[wasm_bindgen]
pub struct WasmSimulationState {
    sim: DrawingSimulation,
    playback: PlaybackController,
    renderer: HeightmapRenderer,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface_config: wgpu::SurfaceConfiguration,
    /// Surface/target color format, kept around so `set_grid_size` can rebuild `renderer` from
    /// scratch (GPU textures can't be resized in place — a resolution change is a full
    /// teardown/rebuild of both `sim` and `renderer`, not a mutation of either).
    target_format: wgpu::TextureFormat,
    /// Current simulation grid resolution (64/128/256/512, default 512). Single source of truth
    /// for sizing CPU-side upload buffers in `render()` — replaces the old hardcoded module-level
    /// `GRID_SIZE` constant at every read site that depends on the *current* grid, not the
    /// compile-time default.
    grid_size: usize,
    full_upload_needed: bool,

    // Config state
    simulator_mode: SimulatorMode,
    marble_count: u32,
    material_mode: MaterialMode,
    sandbox_shape: SandboxShape,
    marble_size: f32,
    speed: f32,
    pattern_mode: String, // "Manual" or "Pattern"

    // Pattern config parameters
    spiral_spacing: f32,
    lissajous_a: f32,
    lissajous_b: f32,
    rose_k: f32,
    hypotrochoid_r: f32,
    hypotrochoid_d: f32,
    random_walk_steps: u32,
    random_walk_step_size: f32,
    hilbert_order: u32,

    // Lighting/Camera configs
    camera_azimuth: f32,
    camera_elevation: f32,
    camera_zoom: f32,
    led_mode: u32,
    led_color: [f32; 4],
    sand_color: [f32; 4],
    light_angle: f32,
    shadows_enabled: bool,
    elapsed_time: f32,
    clock_minute: u32,
    color_mode: u32,

    // Quantile mass-distribution overlay (UI-facing setting; only actually applied to `sim`
    // while `simulator_mode == SandFall` — see `set_quantile_mode`/`set_simulator_mode`).
    quantile_mode: QuantileMode,
    // Last dt seen by `step()`, used to make the per-frame easing below frame-rate independent.
    last_dt: f32,
    // Displayed (eased) quantile line positions, plus whether they hold a meaningful previous
    // value yet — false right after the feature is (re-)enabled, so the first frame snaps
    // straight to the target instead of easing in from stale/zeroed state.
    quantile_eased: [f32; MAX_QUANTILE_LINES],
    quantile_eased_valid: bool,
}

#[wasm_bindgen]
impl WasmSimulationState {
    pub async fn create(canvas_id: String, width: u32, height: u32, force_webgl: bool) -> Result<WasmSimulationState, JsValue> {
        console_error_panic_hook::set_once();
        let window = web_sys::window().ok_or_else(|| JsValue::from_str("No global window"))?;
        let document = window.document().ok_or_else(|| JsValue::from_str("No global document"))?;
        let canvas = document
            .get_element_by_id(&canvas_id)
            .ok_or_else(|| JsValue::from_str(&format!("Canvas with id {} not found", canvas_id)))?
            .dyn_into::<web_sys::HtmlCanvasElement>()
            .map_err(|_| JsValue::from_str("Element is not a canvas"))?;

        // 1. Try to request a WebGPU adapter first *without* creating a surface
        // to avoid locking/polluting the HTML Canvas with a WebGPU context.
        let webgpu_instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::BROWSER_WEBGPU,
            ..Default::default()
        });

        let webgpu_adapter = if force_webgl {
            None
        } else {
            webgpu_instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::default(),
                    compatible_surface: None,
                    force_fallback_adapter: false,
                })
                .await
        };

        let (_instance, surface, adapter) = if let Some(adapter) = webgpu_adapter {
            // WebGPU adapter is available and working!
            // Now we can safely associate the canvas with a WebGPU surface.
            let surface = webgpu_instance
                .create_surface(wgpu::SurfaceTarget::Canvas(canvas.clone()))
                .map_err(|e| JsValue::from_str(&format!("Failed to create WebGPU surface: {:?}", e)))?;
            (webgpu_instance, surface, adapter)
        } else {
            // WebGPU is not supported/blocked.
            // Fall back to WebGL2 completely before touching the canvas.
            let gl_instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
                backends: wgpu::Backends::GL,
                ..Default::default()
            });
            let surface = gl_instance
                .create_surface(wgpu::SurfaceTarget::Canvas(canvas))
                .map_err(|e| JsValue::from_str(&format!("Failed to create WebGL2 surface: {:?}", e)))?;
            let adapter = gl_instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::default(),
                    compatible_surface: Some(&surface),
                    force_fallback_adapter: false,
                })
                .await
                .ok_or_else(|| JsValue::from_str("No compatible WebGL2 adapter found"))?;
            (gl_instance, surface, adapter)
        };

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: None,
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::downlevel_webgl2_defaults(),
                    memory_hints: Default::default(),
                },
                None,
            )
            .await
            .map_err(|e| JsValue::from_str(&format!("Failed to request device: {:?}", e)))?;

        let target_format = surface
            .get_capabilities(&adapter)
            .formats
            .first()
            .copied()
            .unwrap_or(wgpu::TextureFormat::Rgba8Unorm);

        let max_dim = device.limits().max_texture_dimension_2d;
        let clamped_width = width.min(max_dim);
        let clamped_height = height.min(max_dim);

        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: target_format,
            width: clamped_width,
            height: clamped_height,
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &surface_config);

        let renderer = HeightmapRenderer::new(&device, target_format, GRID_SIZE);
        let sim = DrawingSimulation::new();
        let playback = PlaybackController::new();

        Ok(Self {
            sim,
            playback,
            renderer,
            surface,
            device,
            queue,
            surface_config,
            target_format,
            grid_size: GRID_SIZE,
            full_upload_needed: true,
            simulator_mode: SimulatorMode::Sandbox,
            marble_count: 1,
            material_mode: MaterialMode::DrySand,
            sandbox_shape: SandboxShape::Circle,
            marble_size: 0.018,
            speed: 0.5,
            pattern_mode: "Manual".to_string(),
            spiral_spacing: 0.030,
            lissajous_a: 3.0,
            lissajous_b: 4.0,
            rose_k: 5.0,
            hypotrochoid_r: 0.28,
            hypotrochoid_d: 0.20,
            random_walk_steps: 1000,
            random_walk_step_size: 0.02,
            hilbert_order: 5,
            camera_azimuth: 0.0,
            camera_elevation: 0.8,
            camera_zoom: 2.8,
            led_mode: 1,
            led_color: [0.85, 0.90, 0.95, 1.0],
            sand_color: [0.92, 0.89, 0.82, 1.0],
            light_angle: 0.0,
            shadows_enabled: true,
            elapsed_time: 0.0,
            clock_minute: 99,
            color_mode: 0,
            quantile_mode: QuantileMode::Off,
            last_dt: 1.0 / 60.0,
            quantile_eased: [0.0; MAX_QUANTILE_LINES],
            quantile_eased_valid: false,
        })
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width > 0 && height > 0 {
            let max_dim = self.device.limits().max_texture_dimension_2d;
            self.surface_config.width = width.min(max_dim);
            self.surface_config.height = height.min(max_dim);
            self.surface.configure(&self.device, &self.surface_config);
        }
    }

    pub fn step(&mut self, dt: f32, cursor_x: f32, cursor_y: f32, shift_pressed: bool, last_frame_time_ms: f32, target_frame_time_ms: f32) {
        self.elapsed_time += dt;
        // Recorded for the quantile-line easing in `render()`, which runs after `step()` every
        // frame but doesn't otherwise receive dt.
        self.last_dt = dt;

        let shape = if self.simulator_mode == SimulatorMode::SandFall {
            SandboxShape::Hourglass
        } else {
            self.sandbox_shape
        };

        let mut targets = [None; 5];

        if self.simulator_mode == SimulatorMode::Sandbox {
            if self.pattern_mode == "Clock" {
                let date = js_sys::Date::new_0();
                let m = date.get_minutes();

                if m != self.clock_minute {
                    self.sim.reset();
                    self.full_upload_needed = true;
                    self.clock_minute = m;
                    self.load_preset_pattern("clock");
                }
            }

            if self.pattern_mode == "Manual" {
                let mut target_pos = None;
                if shift_pressed {
                    target_pos = Some(glam::Vec2::new(cursor_x, cursor_y));
                }
                targets[0] = target_pos;
            } else {
                if self.playback.state == PlaybackState::Playing {
                    let positions = [
                        self.sim.marbles[0].pos,
                        self.sim.marbles[1].pos,
                        self.sim.marbles[2].pos,
                        self.sim.marbles[3].pos,
                        self.sim.marbles[4].pos,
                    ];
                    targets = self.playback.step_playback_all(
                        &positions,
                        self.marble_count as usize,
                        self.speed,
                        dt,
                    );
                }
            }
        }

        self.sim.update(
            dt,
            &targets,
            self.marble_size,
            self.material_mode,
            shape,
            last_frame_time_ms,
            target_frame_time_ms,
        );
    }

    pub fn reset(&mut self) {
        self.sim.sandbox_shape = self.sandbox_shape;
        self.sim.reset();
        self.full_upload_needed = true;
        self.playback.state = PlaybackState::Stopped;
        self.playback.current_indices = [0; 5];
    }

    /// Change the simulation/render grid resolution to 64, 128, 256, or 512 (`GRID_SIZE`, the
    /// shipped default, is unchanged by this feature). This is a debugging/perf instrument, not
    /// just a performance knob: the test suite and the shipped app used to run at different,
    /// never-compared resolutions, which is exactly how a lateral-pressure term that scaled with
    /// grid resolution instead of physical depth went unnoticed — see `docs/ARCHITECTURE.md`.
    /// Comparing behaviour across resolutions is the point, so this is meant to be switched at
    /// will while the user is looking at the sim, not just read once on startup.
    ///
    /// This is a full teardown/rebuild of both `sim` and `renderer`, never a partial resize:
    /// every CPU buffer inside `DrawingSimulation` (and every GPU texture inside
    /// `HeightmapRenderer`) is a fixed-size allocation made at construction time, so there is no
    /// in-place "resize" — only "replace with a freshly constructed one of the right size". That
    /// necessarily discards the current sand/water contents (same as any other reset), but current
    /// material, shape, gravity, neck width, chamber curvature and multistage chamber count
    /// survive via `sim.reset()`'s normal contract (it never touches those fields) rather than
    /// reverting to defaults.
    pub fn set_grid_size(&mut self, size: u32) -> Result<(), JsValue> {
        let size = size as usize;
        if !matches!(size, 64 | 128 | 256 | 512) {
            return Err(JsValue::from_str(&format!(
                "Unsupported grid size: {} (must be 64, 128, 256, or 512)",
                size
            )));
        }
        if size == self.grid_size {
            return Ok(());
        }

        let gravity_dir = self.sim.gravity_dir;
        let neck_width = self.sim.neck_width;
        let hourglass_curve = self.sim.hourglass_curve;
        let multistage_chambers = self.sim.multistage_chambers;

        let mut sim = DrawingSimulation::new_with_size(size);
        sim.material_mode = self.material_mode;
        sim.sandbox_shape = self.sandbox_shape;
        sim.gravity_dir = gravity_dir;
        sim.neck_width = neck_width;
        sim.hourglass_curve = hourglass_curve;
        sim.multistage_chambers = multistage_chambers;
        sim.reset();
        sim.set_quantile_mode(self.effective_quantile_mode());
        self.sim = sim;

        self.grid_size = size;
        self.renderer = HeightmapRenderer::new(&self.device, self.target_format, size);
        self.full_upload_needed = true;
        self.playback.state = PlaybackState::Stopped;
        self.playback.current_indices = [0; 5];
        Ok(())
    }

    /// Current simulation grid resolution (64/128/256/512). Lets the web UI display the actual
    /// backing value rather than assuming its `<select>`'s own default matches Rust's.
    pub fn get_grid_size(&self) -> u32 {
        self.grid_size as u32
    }

    pub fn set_simulator_mode(&mut self, mode: u32) {
        self.simulator_mode = match mode {
            0 => SimulatorMode::Sandbox,
            1 => SimulatorMode::SandFall,
            _ => SimulatorMode::Sandbox,
        };
        // The quantile overlay only makes sense in Sand-fall (Sandbox has zero gravity and a
        // top-down heightmap, so "how far down has mass fallen" measures nothing). Re-derive
        // the effective mode on every simulator-mode switch rather than only in
        // `set_quantile_mode`, so leaving Sand-fall always turns the sim-side bookkeeping back
        // off even though the UI's own remembered selection (`self.quantile_mode`) is left
        // alone, ready to be restored if the user switches back.
        self.sim.set_quantile_mode(self.effective_quantile_mode());
        self.quantile_eased_valid = false;
        self.reset();
    }

    /// The quantile-line mode actually applied to the simulation: the UI's selection while in
    /// Sand-fall mode, forced to `Off` otherwise. Kept as a single source of truth so
    /// `set_simulator_mode` and `set_quantile_mode` can't disagree about when the feature is
    /// really active.
    fn effective_quantile_mode(&self) -> QuantileMode {
        if self.simulator_mode == SimulatorMode::SandFall {
            self.quantile_mode
        } else {
            QuantileMode::Off
        }
    }

    /// UI setter for the quantile-line overlay: 0 = off, 1 = quartiles, 2 = deciles. Stores the
    /// selection even while in Sandbox (so it's remembered if the user switches back to
    /// Sand-fall) but only actually enables the sim-side computation while in Sand-fall.
    pub fn set_quantile_mode(&mut self, mode: u32) {
        self.quantile_mode = match mode {
            0 => QuantileMode::Off,
            1 => QuantileMode::Quartiles,
            2 => QuantileMode::Deciles,
            _ => QuantileMode::Off,
        };
        self.sim.set_quantile_mode(self.effective_quantile_mode());
        // Next render should snap straight to the fresh targets rather than easing in from
        // whatever (possibly stale, possibly zeroed) values were last displayed.
        self.quantile_eased_valid = false;
    }

    pub fn set_gravity(&mut self, x: f32, y: f32) {
        self.sim.gravity_dir = glam::Vec2::new(x, y);
    }

    pub fn flip_hourglass(&mut self) {
        if self.simulator_mode == SimulatorMode::SandFall {
            self.sim.flip_hourglass();
            self.full_upload_needed = true;
        }
    }

    pub fn set_neck_width(&mut self, width: f32) {
        self.sim.neck_width = width;
        self.sim.generate_shape_mask();
    }

    pub fn set_hourglass_curve(&mut self, curve: f32) {
        self.sim.hourglass_curve = curve;
        self.sim.generate_shape_mask();
    }

    /// The widest (top) tier's chamber count for `SandboxShape::MultiStageHourglass`'s
    /// merging cascade -- user-selectable 5..=16, default 8. Clamped defensively even though
    /// the UI slider (`chambers-slider` in `index.html`) already enforces the range, matching
    /// `set_marble_count`'s pattern below. Follows the exact same contract as
    /// `set_neck_width`/`set_hourglass_curve`: only regenerates the mask, does not reset the
    /// sim (the caller in `demo.js` resets explicitly afterward, same as it does for those
    /// two, since changing the chamber count changes the boundary as much as they do).
    pub fn set_multistage_chambers(&mut self, chambers: u32) {
        self.sim.multistage_chambers = chambers.clamp(5, 16);
        self.sim.generate_shape_mask();
    }

    /// Current `multistage_chambers` value, so the web UI can initialise its slider/readout
    /// from the actual backing value rather than assuming its own hard-coded default matches.
    pub fn get_multistage_chambers(&self) -> u32 {
        self.sim.multistage_chambers
    }

    /// The rasterised neck HALF-width, in cells, that `eval_sandbox_shape` actually uses for
    /// the current shape/neck_width/multistage_chambers/grid-size combination -- i.e. after
    /// the per-tier cap and floor (and, for MultiStageHourglass, the anti-merge ceiling) have
    /// been applied, not just the raw slider fraction. Exists purely for the UI cell-count
    /// readout next to the neck-width slider: the floor/cap logic means the slider's fraction
    /// alone is a poor guide to what actually rasterises, especially at small grid sizes,
    /// which is exactly what prompted adding this readout in the first place. Display-only;
    /// does not affect geometry.
    pub fn neck_half_width_cells(&self) -> f32 {
        sandart_sim::physics::effective_neck_half_width_cells(
            self.grid_size,
            self.sandbox_shape,
            self.sim.neck_width,
            self.sim.multistage_chambers,
        )
    }

    pub fn draw_ripples(&mut self) {
        self.sim.heightmap.generate_ripples();
        self.full_upload_needed = true;
    }

    pub fn update_colormap(&mut self, data: &[u8]) {
        self.sim.set_cell_colors(data);
        self.full_upload_needed = true;
    }

    pub fn set_cell_props(&mut self, data: &[f32]) {
        self.sim.set_cell_props(data);
        self.full_upload_needed = true;
    }

    /// Select a material by its stable string id (see `MaterialMode::as_str`/`from_str`).
    /// Returns an `Err` for an unrecognized id instead of silently falling back to a default —
    /// a caller passing a stale/wrong id should see it fail loudly rather than quietly select
    /// the wrong material.
    pub fn set_material_preset(&mut self, name: &str) -> Result<(), JsValue> {
        let preset = MaterialMode::from_str(name)
            .ok_or_else(|| JsValue::from_str(&format!("Unknown material preset: {}", name)))?;
        self.material_mode = preset;
        self.sim.apply_preset(preset);
        self.full_upload_needed = true;
        Ok(())
    }

    /// List every material as `[id, label, wetness, threshold, flow_rate, grain_size]` rows, in
    /// menu order. The web UI should build its material `<select>` options from this at
    /// startup rather than hardcoding its own copy of the material list — that hardcoded-copy
    /// pattern is exactly what caused the material selection to silently point at the wrong
    /// material after a past UI rewrite.
    pub fn list_materials() -> js_sys::Array {
        let out = js_sys::Array::new();
        for mode in MaterialMode::ALL {
            let (wetness, threshold, flow_rate, grain_size) = mode.preset_props();
            let row = js_sys::Array::new();
            row.push(&JsValue::from_str(mode.as_str()));
            row.push(&JsValue::from_str(mode.label()));
            row.push(&JsValue::from_f64(wetness as f64));
            row.push(&JsValue::from_f64(threshold as f64));
            row.push(&JsValue::from_f64(flow_rate as f64));
            row.push(&JsValue::from_f64(grain_size as f64));
            out.push(&row);
        }
        out
    }

    /// Short git SHA of the commit this wasm bundle was built from, baked in at compile time by
    /// `build.rs` (see `SANDART_GIT_SHA` there). Falls back to `"unknown"` if the build ran
    /// without a git checkout available — never a fabricated or stale-looking value. Paired with
    /// `build_timestamp_epoch` to render the diagnostic "Build" readout in the panel footer, so
    /// the user can tell a fresh deploy apart from a stale cached page.
    pub fn build_git_sha() -> String {
        env!("SANDART_GIT_SHA").to_string()
    }

    /// Wall-clock time this wasm bundle was compiled, as seconds since the Unix epoch, baked in
    /// at compile time by `build.rs` (see `SANDART_BUILD_EPOCH` there). Left as a raw epoch
    /// rather than formatted here so the JS side can render it with `Date`, which already knows
    /// how to do that correctly.
    pub fn build_timestamp_epoch() -> f64 {
        env!("SANDART_BUILD_EPOCH").parse::<u64>().unwrap_or(0) as f64
    }

    pub fn set_sandbox_shape(&mut self, shape: u32) {
        let new_shape = match shape {
            0 => SandboxShape::Circle,
            1 => SandboxShape::Square,
            2 => SandboxShape::Oval,
            3 => SandboxShape::Hourglass,
            4 => SandboxShape::MultiStageHourglass,
            5 => SandboxShape::GaltonBoard,
            6 => SandboxShape::StaircaseCascade,
            7 => SandboxShape::ProceduralFunnel,
            8 => SandboxShape::MultiNeckHourglass,
            _ => SandboxShape::Circle,
        };
        self.sandbox_shape = new_shape;
        self.sim.sandbox_shape = new_shape;
        if matches!(
            new_shape,
            SandboxShape::Hourglass
                | SandboxShape::MultiStageHourglass
                | SandboxShape::GaltonBoard
                | SandboxShape::StaircaseCascade
                | SandboxShape::ProceduralFunnel
                | SandboxShape::MultiNeckHourglass
        ) {
            self.sim.reset();
        } else {
            self.sim.generate_shape_mask();
        }
        self.full_upload_needed = true;
    }

    pub fn set_marble_count(&mut self, count: u32) {
        self.marble_count = count.clamp(1, 5);
    }

    pub fn set_speed(&mut self, speed: f32) {
        self.speed = speed;
    }

    pub fn set_marble_size(&mut self, size: f32) {
        self.marble_size = size;
    }

    pub fn set_pattern_mode(&mut self, mode: String) {
        self.pattern_mode = mode;
    }

    pub fn set_spiral_spacing(&mut self, spacing: f32) {
        self.spiral_spacing = spacing;
    }

    pub fn set_lissajous_params(&mut self, a: f32, b: f32) {
        self.lissajous_a = a;
        self.lissajous_b = b;
    }

    pub fn set_rose_k(&mut self, k: f32) {
        self.rose_k = k;
    }

    pub fn set_hypotrochoid_params(&mut self, r: f32, d: f32) {
        self.hypotrochoid_r = r;
        self.hypotrochoid_d = d;
    }

    pub fn set_random_walk_params(&mut self, steps: u32, step_size: f32) {
        self.random_walk_steps = steps;
        self.random_walk_step_size = step_size;
    }

    pub fn set_hilbert_order(&mut self, order: u32) {
        self.hilbert_order = order;
    }

    pub fn set_camera(&mut self, azimuth: f32, elevation: f32, zoom: f32) {
        self.camera_azimuth = azimuth;
        self.camera_elevation = elevation;
        self.camera_zoom = zoom;
    }

    pub fn set_led_mode(&mut self, mode: u32) {
        self.led_mode = mode;
    }

    pub fn set_led_color(&mut self, r: f32, g: f32, b: f32) {
        self.led_color = [r, g, b, 1.0];
    }

    pub fn set_sand_color(&mut self, r: f32, g: f32, b: f32) {
        self.sand_color = [r, g, b, 1.0];
    }

    pub fn set_color_mode(&mut self, mode: u32) {
        self.color_mode = mode;
    }

    pub fn set_light_angle(&mut self, angle: f32) {
        self.light_angle = angle;
    }

    pub fn set_shadows_enabled(&mut self, enabled: bool) {
        self.shadows_enabled = enabled;
    }

    pub fn load_pattern_gcode(&mut self, content: &str) -> bool {
        if let Ok(points) = parse_gcode(content) {
            self.playback.clear_waypoints();
            self.playback.waypoints[0] = points;
            self.playback.randomize_speeds(1, self.sim.seed);
            self.playback.state = PlaybackState::Playing;
            true
        } else {
            false
        }
    }

    pub fn load_pattern_thr(&mut self, content: &str) -> bool {
        if let Ok(points) = parse_thr(content) {
            self.playback.clear_waypoints();
            self.playback.waypoints[0] = points;
            self.playback.randomize_speeds(1, self.sim.seed);
            self.playback.state = PlaybackState::Playing;
            true
        } else {
            false
        }
    }

    pub fn load_multi_pattern(&mut self, format: &str, file_content: &str, arms: u32) -> bool {
        let arms = arms.clamp(1, 5) as usize;
        let points_res = if format == "thr" {
            parse_thr(file_content)
        } else {
            parse_gcode(file_content)
        };

        if let Ok(base_points) = points_res {
            self.playback.clear_waypoints();
            for j in 0..arms {
                let angle_offset = (j as f32 / arms as f32) * 2.0 * std::f32::consts::PI;
                let mut rotated = Vec::with_capacity(base_points.len());
                for p in &base_points {
                    let cos_a = angle_offset.cos();
                    let sin_a = angle_offset.sin();
                    let rx = p.x * cos_a - p.y * sin_a;
                    let ry = p.x * sin_a + p.y * cos_a;
                    rotated.push(glam::Vec2::new(rx, ry));
                }
                self.playback.waypoints[j] = rotated;
            }
            self.playback.randomize_speeds(arms, self.sim.seed);
            self.playback.state = PlaybackState::Playing;
            true
        } else {
            false
        }
    }

    pub fn load_preset_pattern(&mut self, pattern_type: &str) -> bool {
        self.playback.clear_waypoints();
        self.playback.loop_pattern = pattern_type != "clock";

        let base_waypoints = match pattern_type {
            "spiral" => {
                sandart_pattern::generate_spiral(self.spiral_spacing)
            }
            "lissajous" => {
                sandart_pattern::generate_lissajous(
                    self.lissajous_a,
                    self.lissajous_b,
                    1.5707963,
                )
            }
            "rose" => {
                sandart_pattern::generate_rose(self.rose_k)
            }
            "spirograph" | "hypotrochoid" => {
                sandart_pattern::generate_hypotrochoid(
                    self.hypotrochoid_r,
                    self.hypotrochoid_d,
                )
            }
            "fermat" => {
                sandart_pattern::generate_fermat_spiral(self.rose_k)
            }
            "hilbert" => {
                sandart_pattern::generate_hilbert_curve(self.hilbert_order)
            }
            "gosper" => {
                // Keep order in bounds for web performance
                let order = self.hilbert_order.clamp(1, 5);
                sandart_pattern::generate_gosper_curve(order)
            }
            "sierpinski" => {
                let order = self.hilbert_order.clamp(1, 7);
                sandart_pattern::generate_sierpinski_curve(order)
            }
            "random_walk" => {
                sandart_pattern::generate_random_walk(
                    self.random_walk_steps as usize,
                    self.random_walk_step_size,
                )
            }
            "lemniscate" => {
                sandart_pattern::generate_lemniscate(0.8)
            }
            "butterfly" => {
                sandart_pattern::generate_butterfly_curve()
            }
            "zen_waves" => {
                sandart_pattern::generate_zen_waves()
            }
            "zen_mandala" => {
                sandart_pattern::generate_zen_mandala()
            }
            "dinosaur" => {
                sandart_pattern::generate_dinosaur()
            }
            "unicorn" => {
                sandart_pattern::generate_unicorn()
            }
            "clock" => {
                let date = js_sys::Date::new_0();
                let h = date.get_hours();
                let m = date.get_minutes();
                self.clock_minute = m;
                sandart_pattern::generate_clock_pattern(h, m, 0.0, 1)
            }
            _ => return false,
        };

        let base_waypoints = sandart_pattern::close_loop_path(base_waypoints);
        if base_waypoints.is_empty() {
            return false;
        }

        let arms = self.marble_count.clamp(1, 5) as usize;
        for j in 0..5 {
            if j < arms {
                let mut path = base_waypoints.clone();
                if self.sandbox_shape == SandboxShape::Oval {
                    for p in &mut path {
                        p.y *= 0.652;
                    }
                }
                self.playback.waypoints[j] = path;
                let start_idx = (j * base_waypoints.len() / arms) % base_waypoints.len();
                self.playback.current_indices[j] = start_idx;

                // Snap simulation marble to its starting waypoint position
                let start_pos = self.playback.waypoints[j][start_idx];
                self.sim.marbles[j].pos = start_pos;
                self.sim.marbles[j].prev_pos = start_pos;
                self.sim.marbles[j].vel = glam::Vec2::ZERO;
                self.sim.marbles[j].was_active = true;
            } else {
                self.sim.marbles[j].was_active = false;
            }
        }
        if arms > 0 {
            self.sim.marble_pos = self.sim.marbles[0].pos;
            self.sim.prev_marble_pos = self.sim.marbles[0].prev_pos;
            self.sim.marble_vel = self.sim.marbles[0].vel;
            self.sim.was_active = self.sim.marbles[0].was_active;
        }

        self.playback.randomize_speeds(arms, self.sim.seed);
        self.playback.state = PlaybackState::Playing;
        true
    }

    pub fn render(&mut self) -> Result<(), JsValue> {
        // 1. Prepare uniform data
        let surface_texture = self
            .surface
            .get_current_texture()
            .map_err(|e| JsValue::from_str(&format!("Failed to acquire surface texture: {:?}", e)))?;

        let view = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        if self.sim.shape_mask_dirty {
            self.renderer.update_shape_mask(&self.queue, &self.sim.shape_mask);
            self.sim.shape_mask_dirty = false;
        }

        // Update GPU heightmap and colormap
        let grid_size = self.grid_size;
        if self.full_upload_needed {
            let mut interleaved = vec![0.0f32; grid_size * grid_size * 4];
            for i in 0..grid_size * grid_size {
                interleaved[i * 4 + 0] = self.sim.heightmap.data[i];
                interleaved[i * 4 + 1] = self.sim.cell_props[i * 4 + sandart_sim::PROP_WETNESS];
                interleaved[i * 4 + 2] = self.sim.cell_props[i * 4 + sandart_sim::PROP_GRAIN_SIZE];
                interleaved[i * 4 + 3] = 1.0;
            }
            self.renderer.update_heightmap(&self.queue, &interleaved);
            self.renderer.update_colormap(&self.queue, &self.sim.cell_colors);
            self.full_upload_needed = false;
        } else {
            let bounds = self.sim.active_bounds;
            let render_bounds = sandart_render::ActiveBounds {
                min_x: bounds.min_x,
                max_x: bounds.max_x,
                min_y: bounds.min_y,
                max_y: bounds.max_y,
                active: bounds.active,
            };
            if render_bounds.active {
                let sub_width = bounds.max_x - bounds.min_x + 1;
                let sub_height = bounds.max_y - bounds.min_y + 1;
                
                let mut interleaved = vec![0.0f32; sub_width * sub_height * 4];
                for y in bounds.min_y..=bounds.max_y {
                    let src_row_offset = y * grid_size;
                    let dest_row_offset = (y - bounds.min_y) * sub_width;
                    for x in bounds.min_x..=bounds.max_x {
                        let src_idx = src_row_offset + x;
                        let dest_idx = dest_row_offset + (x - bounds.min_x);
                        interleaved[dest_idx * 4 + 0] = self.sim.heightmap.data[src_idx];
                        interleaved[dest_idx * 4 + 1] = self.sim.cell_props[src_idx * 4 + sandart_sim::PROP_WETNESS];
                        interleaved[dest_idx * 4 + 2] = self.sim.cell_props[src_idx * 4 + sandart_sim::PROP_GRAIN_SIZE];
                        interleaved[dest_idx * 4 + 3] = 1.0;
                    }
                }
                self.renderer.update_heightmap_partial(
                    &self.queue,
                    &interleaved,
                    render_bounds,
                );

                let mut colormap_sub = vec![0u8; sub_width * sub_height * 4];
                for y in bounds.min_y..=bounds.max_y {
                    let src_row_offset = y * grid_size * 4;
                    let dest_row_offset = (y - bounds.min_y) * sub_width * 4;
                    let bytes_to_copy = sub_width * 4;
                    colormap_sub[dest_row_offset..(dest_row_offset + bytes_to_copy)].copy_from_slice(
                        &self.sim.cell_colors[src_row_offset + bounds.min_x * 4..src_row_offset + (bounds.max_x + 1) * 4]
                    );
                }
                self.renderer.update_colormap_partial(
                    &self.queue,
                    &colormap_sub,
                    render_bounds,
                );
            }
        }

        // Calculate uniforms
        let current_light_dir = [
            self.light_angle.cos(),
            self.light_angle.sin(),
            0.15,
            0.0,
        ];

        let mut current_marbles = [MarbleUniform {
            pos: [0.0, 0.0],
            radius: 0.018,
            z_pos: 0.35,
        }; 5];

        for j in 0..5 {
            let (gx, gy) = DrawingSimulation::norm_to_grid(
                self.sim.marbles[j].pos,
                grid_size,
                grid_size,
            );
            let z = self.sim.heightmap.get(gx, gy);
            current_marbles[j] = MarbleUniform {
                pos: [self.sim.marbles[j].pos.x, self.sim.marbles[j].pos.y],
                radius: self.marble_size,
                z_pos: z,
            };
        }

        let (render_shape, render_marble_count) = if self.simulator_mode == SimulatorMode::SandFall {
            (self.sandbox_shape as u32, 0u32)
        } else {
            (self.sandbox_shape as u32, self.marble_count)
        };

        // --- Quantile mass-distribution lines (Sand-fall only) ---
        //
        // `sim.quantile_positions()` gives the *raw* targets, recomputed at most every 5 ticks
        // (see `DrawingSimulation::update`), so displaying them directly would visibly step
        // every 5th frame. We ease the displayed value toward the target every render instead,
        // decoupling visual smoothness from the compute cadence.
        //
        // The easing is an EMA (the same shape as `EMA_ALPHA` in sandart-sim for frame time),
        // not a fixed lerp fraction, so it looks the same regardless of frame rate: alpha is
        // derived from actual dt and a time constant, rather than being a fixed per-frame step
        // that would visibly speed up or slow down as the adaptive frame budget reacts to load.
        let effective_quantile_mode = self.effective_quantile_mode();
        let quantile_count = effective_quantile_mode.line_count() as u32;

        if quantile_count > 0 {
            let targets = self.sim.quantile_positions();
            const TAU_SECONDS: f32 = 0.20;
            // A jump bigger than this fraction of the grid height (reset, hourglass flip, mode
            // just switched on) snaps instead of easing — otherwise a discontinuity plays back
            // as a slow crawl across the whole screen instead of an instant re-place.
            const SNAP_FRACTION: f32 = 0.25;
            let alpha = 1.0 - (-self.last_dt.max(0.0) / TAU_SECONDS).exp();

            for i in 0..MAX_QUANTILE_LINES {
                let target = targets.get(i).copied().unwrap_or(0.0);
                let next = if !self.quantile_eased_valid {
                    target
                } else {
                    let prev = self.quantile_eased[i];
                    if (target - prev).abs() > SNAP_FRACTION {
                        target
                    } else {
                        prev + alpha * (target - prev)
                    }
                };
                self.quantile_eased[i] = next;
            }
            self.quantile_eased_valid = true;
        } else {
            // Feature off/hidden this frame: next time it turns on, snap rather than ease in
            // from stale state.
            self.quantile_eased_valid = false;
        }

        let mut quantile_positions_uniform = [[0.0f32; 4]; 3];
        if quantile_count > 0 {
            for i in 0..MAX_QUANTILE_LINES {
                quantile_positions_uniform[i / 4][i % 4] = self.quantile_eased[i];
            }
        }

        let current_uniforms = LightingUniforms {
            light_dir: current_light_dir,
            light_color: self.led_color,
            sand_color: self.sand_color,
            light_brightness: 1.2,
            shadow_enabled: if self.shadows_enabled { 1 } else { 0 },
            led_mode: self.led_mode,
            time: self.elapsed_time,
            marble_count: render_marble_count,
            material_mode: self.material_mode as u32,
            sandbox_shape: render_shape,
            color_mode: self.color_mode,
            neck_width: self.sim.neck_width,
            hourglass_curve: self.sim.hourglass_curve,
            quantile_count,
            grid_size: self.grid_size as f32,
            quantile_positions: quantile_positions_uniform,
            marbles: current_marbles,
        };
        self.renderer.update_uniforms(&self.queue, &current_uniforms);

        // Calculate view projection matrix
        let aspect = self.surface_config.width as f32 / self.surface_config.height as f32;
        let projection = glam::Mat4::perspective_lh(0.785, aspect, 0.1, 100.0);
        let cam_x = self.camera_zoom * self.camera_elevation.cos() * self.camera_azimuth.cos();
        let cam_y = self.camera_zoom * self.camera_elevation.cos() * self.camera_azimuth.sin();
        let cam_z = self.camera_zoom * self.camera_elevation.sin();
        let eye = glam::Vec3::new(cam_x, cam_y, cam_z);
        let target = glam::Vec3::ZERO;
        let up = glam::Vec3::Z;
        let view_proj = projection * glam::Mat4::look_at_lh(eye, target, up);

        let camera_uniforms = CameraUniforms {
            view_proj: view_proj.to_cols_array(),
            camera_pos: [eye.x, eye.y, eye.z, 0.0],
        };
        self.renderer.update_camera(&self.queue, &camera_uniforms);

        // 2. Perform rendering
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("WASM Render Encoder"),
            });

        // Depth texture view
        let depth_texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("WASM Depth Texture"),
            size: wgpu::Extent3d {
                width: self.surface_config.width,
                height: self.surface_config.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth24Plus,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let depth_view = depth_texture.create_view(&wgpu::TextureViewDescriptor::default());

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("WASM Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            render_pass.set_viewport(
                0.0,
                0.0,
                self.surface_config.width as f32,
                self.surface_config.height as f32,
                0.0,
                1.0,
            );

            self.renderer
                .draw(&mut render_pass, &camera_uniforms, &current_uniforms);
        }

        self.queue.submit(Some(encoder.finish()));
        surface_texture.present();

        Ok(())
    }

    pub fn get_heightmap(&self) -> js_sys::Float32Array {
        unsafe { js_sys::Float32Array::view(self.sim.heightmap.as_slice()) }
    }

    pub fn get_active_block_counts(&self) -> js_sys::Int32Array {
        let mut inactive = 0;
        let mut slow = 0;
        let mut medium = 0;
        let mut fast = 0;
        for &activity in &self.sim.active_blocks {
            match activity {
                sandart_sim::BlockActivity::Inactive => inactive += 1,
                sandart_sim::BlockActivity::Slow => slow += 1,
                sandart_sim::BlockActivity::Medium => medium += 1,
                sandart_sim::BlockActivity::Fast => fast += 1,
            }
        }
        js_sys::Int32Array::from(&[inactive, slow, medium, fast][..])
    }

    pub fn get_budget_n(&self) -> usize {
        self.sim.budget_n
    }

    pub fn get_ema_frame_ms(&self) -> f32 {
        self.sim.ema_frame_ms
    }
}

pub const GRID_SIZE: usize = 512;
