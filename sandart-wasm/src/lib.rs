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
    // Block-simulation heat-map debug overlay (see `set_heatmap_overlay`). Off by default, like
    // `shadows_enabled` and `quantile_mode` — a pure render-side toggle, doesn't affect `sim` at
    // all (the underlying per-block counter it displays, `sim.block_heat_buckets`, is always
    // maintained regardless of whether this is on; this field only gates whether `render()`
    // bothers uploading/tinting with it).
    heatmap_enabled: bool,
    // Per-cell pressure-field debug overlay (see `set_pressure_heatmap_overlay`). Same shape as
    // `heatmap_enabled` just above: a pure render-side toggle, doesn't affect `sim` at all (the
    // underlying `sim.column_depth` it displays is always maintained regardless of whether this
    // is on; this field only gates whether `render()` bothers uploading/tinting with it).
    pressure_heatmap_enabled: bool,
    // Coarse-level `eta` (hydraulic head) debug overlay (see `set_coarse_eta_overlay`). Same
    // shape as `heatmap_enabled`/`pressure_heatmap_enabled` above -- a pure render-side toggle;
    // the underlying `sim.coarse_state.eta` runs unconditionally whenever `sim.coarse.available`
    // (OVERCLOCKING.md: no longer gated on `coarse_pressure_coupling`, which now only gates the
    // driving-potential coupling into the fine solver), this only gates whether `render()`
    // bothers uploading/tinting with it. Off by default, like every other Debug-group overlay.
    coarse_eta_enabled: bool,
    // Coarse-fine disagreement (`Delta`) debug overlay (see `set_coarse_delta_overlay`). Same
    // shape as `coarse_eta_enabled` just above -- independent toggle, independent texture, so
    // both can be viewed at once the same way the block and pressure heat-maps already can.
    coarse_delta_enabled: bool,
    // Cache key for the per-cell pressure overlay upload below: the `(tick_count, source)` the
    // currently-uploaded texture was built from, or `None` if nothing has been uploaded yet.
    //
    // Exists for the PAUSED case, which is the one this overlay is actually for (see the pause /
    // step controls in demo.js -- they were added so a pressure distribution could be read while
    // nothing moves). Pause stops `step()` but deliberately keeps `render()` running every frame,
    // so without this the new head-field source would recompute a bit-identical field ~60 times a
    // second at ~11.4ms a call (w=512, release) and make the paused inspection sluggish for no
    // reason at all. `tick_count` does not advance while paused, so the whole recompute-and-upload
    // is skipped after the first frame.
    //
    // Keyed on the SOURCE as well as the tick so flipping `pressure_heatmap_head_field` while
    // paused -- exactly the A/B this overlay was built for -- redraws immediately instead of
    // showing a stale field from the other source.
    pressure_heat_cache_key: Option<(u32, bool)>,
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
            heatmap_enabled: false,
            pressure_heatmap_enabled: false,
            coarse_eta_enabled: false,
            coarse_delta_enabled: false,
            pressure_heat_cache_key: None,
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
        let perfect_simulation = self.sim.perfect_simulation;
        let fresh_pressure_field = self.sim.fresh_pressure_field;
        let pressure_heatmap_head_field = self.sim.pressure_heatmap_head_field;
        let head_field_transport = self.sim.head_field_transport;
        let pressure_sensitive_flow = self.sim.pressure_sensitive_flow;
        let overfill_pressure = self.sim.overfill_pressure;
        let overfill_capacity = self.sim.overfill_capacity;
        let overfill_stiffness = self.sim.overfill_stiffness;
        let pressure_heatmap_overlay = self.sim.pressure_heatmap_overlay;
        let coarse_pressure_coupling = self.sim.coarse_pressure_coupling;
        let overclocking_enabled = self.sim.overclocking_enabled;
        let max_clock_rate = self.sim.max_clock_rate;
        let rank_clock_rates = self.sim.rank_clock_rates;
        let rate_gated_reps = self.sim.rate_gated_reps;
        let clock_band_log_falloff = self.sim.clock_band_log_falloff;
        let grade_clock_rates = self.sim.grade_clock_rates;

        let mut sim = DrawingSimulation::new_with_size(size);
        sim.material_mode = self.material_mode;
        sim.sandbox_shape = self.sandbox_shape;
        sim.gravity_dir = gravity_dir;
        sim.neck_width = neck_width;
        sim.hourglass_curve = hourglass_curve;
        sim.multistage_chambers = multistage_chambers;
        // Carried over like the geometry params above, not left to reset to the fresh sim's
        // default -- a resolution change is a full sim teardown/rebuild (see this function's
        // doc comment), but the "perfect simulation" debug toggle is a UI setting, same
        // category as `quantile_mode` below, and shouldn't silently flip off just because the
        // grid resized underneath it.
        sim.perfect_simulation = perfect_simulation;
        // Same reasoning as `perfect_simulation` just above: a UI debug toggle, not simulation
        // state, so it should survive a resolution rebuild rather than silently reset to default.
        sim.fresh_pressure_field = fresh_pressure_field;
        // Same reasoning again: a UI debug toggle (which pressure heat-map source is selected),
        // not simulation state -- must survive a resolution rebuild like its siblings above.
        sim.pressure_heatmap_head_field = pressure_heatmap_head_field;
        // Same reasoning again -- and if this were dropped the decile legend would silently stop
        // refreshing after any resolution change, leaving a stale scale on screen.
        sim.pressure_heatmap_overlay = pressure_heatmap_overlay;
        // Same reasoning again: a UI debug toggle, not simulation state.
        sim.head_field_transport = head_field_transport;
        // Same reasoning again: a UI debug toggle, not simulation state.
        sim.pressure_sensitive_flow = pressure_sensitive_flow;
        // Same reasoning again: a UI debug toggle, not simulation state.
        sim.overfill_pressure = overfill_pressure;
        sim.overfill_capacity = overfill_capacity;
        sim.overfill_stiffness = overfill_stiffness;
        // Same reasoning again: a UI debug toggle, not simulation state -- must survive a
        // resolution rebuild like its siblings above rather than reverting to the fresh sim's
        // (also `true`) default and silently discarding an explicit user choice to turn it off.
        sim.coarse_pressure_coupling = coarse_pressure_coupling;
        // Same reasoning again: a UI debug toggle, not simulation state.
        sim.overclocking_enabled = overclocking_enabled;
        // Same reasoning again: a UI slider position, not simulation state.
        sim.max_clock_rate = max_clock_rate;
        // Same reasoning again: a UI toggle, not simulation state.
        sim.rank_clock_rates = rank_clock_rates;
        // Same reasoning again: a UI toggle, not simulation state.
        sim.rate_gated_reps = rate_gated_reps;
        // Same reasoning again: a UI toggle, not simulation state.
        sim.clock_band_log_falloff = clock_band_log_falloff;
        // Same reasoning again: a UI toggle, not simulation state.
        sim.grade_clock_rates = grade_clock_rates;
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
            9 => SandboxShape::UTubeFlowThrough,
            _ => SandboxShape::Circle,
        };

        // Selecting the shape that is ALREADY selected is a no-op, and has to be, because this
        // function is not only called when the user picks a shape. `syncSettings()` in demo.js
        // pushes the ENTIRE settings panel down on every change to any control in it -- every
        // slider `input`, every checkbox `change` -- and re-reads `shape-select` each time. So
        // without this guard, dragging the speed slider, toggling shadows, or switching on either
        // of the LOD debug overlays all ran the `sim.reset()` below and wiped whatever material
        // layout was in the container back to a default single-material fill.
        //
        // That is how it surfaced: "setting heat map overlay overrides multi material selection.
        // redoing material selection after selecting heat map works though" -- re-selecting the
        // material simply re-applied what the reset had just discarded. The debug checkboxes did
        // not introduce this; they only added two more triggers to a path every other control in
        // the panel was already firing.
        //
        // Guarding here rather than in `syncSettings` deliberately: the invariant belongs to this
        // function (a shape CHANGE resets, a shape non-change does not), and fixing it JS-side
        // would leave the same trap armed for the next caller.
        //
        // Scoped to `sim.reset()` alone rather than an early return, because an early return
        // would ALSO skip `generate_shape_mask()` and `full_upload_needed`, and the very first
        // `syncSettings()` at startup is a same-shape call: the constructor initialises
        // `sandbox_shape` to `Circle` and `shape-select`'s default option is Circle (value 0), so
        // that first call sets nothing new and would have returned before generating the initial
        // mask. Mask regeneration is idempotent and non-destructive; only the reset is not.
        let shape_changed = new_shape != self.sandbox_shape;

        self.sandbox_shape = new_shape;
        self.sim.sandbox_shape = new_shape;
        if shape_changed
            && matches!(
                new_shape,
                SandboxShape::Hourglass
                    | SandboxShape::MultiStageHourglass
                    | SandboxShape::GaltonBoard
                    | SandboxShape::StaircaseCascade
                    | SandboxShape::ProceduralFunnel
                    | SandboxShape::MultiNeckHourglass
                    | SandboxShape::UTubeFlowThrough
            )
        {
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

    /// "Perfect simulation" debug toggle: forwarded straight to the sim, which force-admits
    /// every non-trivial (in-mask, holding material) block into its unconditional simulate tier
    /// every tick instead of letting the adaptive budget skip any of them. See
    /// `DrawingSimulation::perfect_simulation`'s doc comment in sandart-sim/src/lib.rs. Slow by
    /// design — it exists to A/B the scheduler's approximation against the ground truth, not to
    /// be left on.
    pub fn set_perfect_simulation(&mut self, enabled: bool) {
        self.sim.perfect_simulation = enabled;
    }

    /// "Fresh pressure field" debug toggle: forwarded straight to the sim, a plain field write
    /// (same shape as `set_perfect_simulation` just above — no reset, no reinitialisation). See
    /// `DrawingSimulation::fresh_pressure_field`'s doc comment in sandart-sim/src/lib.rs for what
    /// it switches between. Experimental — it exists to A/B the standalone `column_depth` pass
    /// against the shipped default live, not because it is known to be an improvement.
    pub fn set_fresh_pressure_field(&mut self, enabled: bool) {
        self.sim.fresh_pressure_field = enabled;
    }

    /// Selects which quantity feeds the pressure heat-map overlay (see
    /// `set_pressure_heatmap_overlay` for the overlay's own on/off switch): forwarded straight to
    /// the sim, a plain field write (same shape as `set_fresh_pressure_field` just above — no
    /// reset, no reinitialisation, safe to call every frame from `syncSettings()`). See
    /// `DrawingSimulation::pressure_heatmap_head_field`'s doc comment in sandart-sim/src/lib.rs
    /// for what it switches between: `false` (default) is today's shipped `column_depth`; `true`
    /// is task #55 step 2's static hydraulic head field, converted to a pressure-like quantity so
    /// it renders on the same colour scale.
    pub fn set_pressure_heatmap_head_field(&mut self, enabled: bool) {
        self.sim.pressure_heatmap_head_field = enabled;
    }

    /// "Drive transport from the head field" debug toggle (task #55 step 3): forwarded straight
    /// to the sim, a plain field write (same shape as `set_fresh_pressure_field` above — no
    /// reset, no reinitialisation, safe to call every frame from `syncSettings()`). See
    /// `DrawingSimulation::head_field_transport`'s doc comment in sandart-sim/src/lib.rs for what
    /// it switches on: `false` (default) is today's shipped `column_depth`/`GRAVITY_HEAD_SCALE`
    /// driving head, bit-identical; `true` makes LIQUID-ONLY lateral and vertical edges use the
    /// unified hydraulic head field instead. Granular material and mixed liquid/granular edges
    /// are unaffected.
    pub fn set_head_field_transport(&mut self, enabled: bool) {
        self.sim.head_field_transport = enabled;
    }

    /// "Pressure-sensitive flow rate" debug toggle (task #63): forwarded straight to the sim, a
    /// plain field write (same shape as `set_head_field_transport` just above — no reset, no
    /// reinitialisation, safe to call every frame from `syncSettings()`). See
    /// `DrawingSimulation::pressure_sensitive_flow`'s doc comment in sandart-sim/src/lib.rs for
    /// what it switches on: `false` (default) is today's head-independent conveyance rate,
    /// bit-identical; `true` slows LIQUID-ONLY edges whose donor carries less than one cell of
    /// hydrostatic head. Free-falling material and granular material are unaffected.
    pub fn set_pressure_sensitive_flow(&mut self, enabled: bool) {
        self.sim.pressure_sensitive_flow = enabled;
    }

    /// "Per-cell overfill pressure simulation" toggle (task #70): forwarded straight to the sim.
    pub fn set_overfill_pressure(&mut self, enabled: bool) {
        self.sim.overfill_pressure = enabled;
    }

    /// "Coarse pressure coupling" debug toggle (HIERARCHICAL-PRESSURE.md, split by
    /// OVERCLOCKING.md): forwarded straight to the sim, a plain field write (same shape as
    /// `set_overfill_pressure` just above -- no reset, no reinitialisation, safe to call every
    /// frame from `syncSettings()`). See `DrawingSimulation::coarse_pressure_coupling`'s doc
    /// comment in sandart-sim/src/lib.rs for what it switches: this now gates ONLY the
    /// driving-potential coupling into the fine solver (`phi`/`gravity_head`) -- the coarse
    /// level's own per-tick work runs unconditionally regardless of this flag, since the
    /// scheduler (`set_overclocking`) needs `|Delta|` either way. **Defaults `false`**, per the
    /// user's own words: "let's leave the coupling behind a flag until we are happy with
    /// overclocking."
    pub fn set_coarse_pressure_coupling(&mut self, enabled: bool) {
        self.sim.coarse_pressure_coupling = enabled;
    }

    /// "Overclocking" (multi-rate block scheduler) debug toggle (OVERCLOCKING.md): forwarded
    /// straight to the sim, a plain field write -- no reset, no reinitialisation, safe to call
    /// every frame from `syncSettings()`, same shape as `set_coarse_pressure_coupling` above.
    /// Independent of that toggle: this drives per-block clock rate from `|Delta|` regardless of
    /// whether the driving-potential coupling is also on. See
    /// `DrawingSimulation::overclocking_enabled`'s doc comment in sandart-sim/src/lib.rs.
    pub fn set_overclocking(&mut self, enabled: bool) {
        self.sim.overclocking_enabled = enabled;
    }

    /// "Rank-based allocation" checkbox (EARLY-STOP.md): selects between allocating clock rates
    /// by absolute threshold and allocating them by rank against a fixed ladder. Plain field
    /// write, safe every frame; inert while the overclocking toggle is off. See
    /// `DrawingSimulation::rank_clock_rates`.
    pub fn set_rank_clock_rates(&mut self, rank: bool) {
        self.sim.rank_clock_rates = rank;
    }

    /// "Rates gate repetitions" checkbox (EARLY-STOP.md): whether a block's clock rate decides
    /// who RUNS in the extra repetitions, or only who is force-woken on top of everyone else.
    /// Plain field write, safe every frame; inert while overclocking is off. See
    /// `DrawingSimulation::rate_gated_reps`.
    pub fn set_rate_gated_reps(&mut self, gated: bool) {
        self.sim.rate_gated_reps = gated;
    }

    /// "Gentle band falloff" checkbox (EARLY-STOP.md): band sizes `∝ 1/lg(1+r)` instead of
    /// `∝ 1/r`, so the fast bands are ~3x wider and more likely to be contiguous. See
    /// `DrawingSimulation::clock_band_log_falloff`.
    pub fn set_clock_band_log_falloff(&mut self, log_falloff: bool) {
        self.sim.clock_band_log_falloff = log_falloff;
    }

    /// "Grade neighbouring rates" checkbox (EARLY-STOP.md): cap the rate field's GRADIENT so
    /// adjacent blocks differ by at most one repetition, enforced by pulling fast blocks down.
    /// See `DrawingSimulation::grade_clock_rates`.
    pub fn set_grade_clock_rates(&mut self, grade: bool) {
        self.sim.grade_clock_rates = grade;
    }

    /// "Max clock rate" slider (EARLY-STOP.md): the ceiling of the per-block clock-rate range,
    /// `1.0..=8.0`. A frame costs at most `round(max_clock_rate)` `settle_tick` repetitions, so
    /// this is the direct frame-time/settling-rate trade -- `1.0` is "no overclocking" while
    /// leaving underclocking in place, `8.0` is the default. Plain field write like
    /// `set_overclocking` above: no reset, no reinitialisation, safe to call every frame from
    /// `syncSettings()`. Has no effect while the overclocking toggle is off, since
    /// `update_block_clock_rates` does not run then.
    pub fn set_max_clock_rate(&mut self, rate: f32) {
        self.sim.max_clock_rate = rate;
    }

    pub fn get_max_clock_rate(&self) -> f32 {
        self.sim.max_clock_rate
    }

    /// "Overfill capacity multiplier" (task #70): 1.00..=2.00, forwarded straight to the sim.
    /// Task #70: the fluid's bulk stiffness — how hard a column resists compressing under its own
    /// weight. This replaced the "overfill capacity" control, which is now DERIVED from it:
    /// stiffness decides how much compression a column wants and the ceiling has to clear that, so
    /// exposing both let the user set a ceiling below what their own fluid demanded and pin every
    /// cell against it. Setting one and computing the other makes that unreachable.
    pub fn set_overfill_stiffness(&mut self, stiffness: f32) {
        self.sim.overfill_stiffness = stiffness;
        self.sim.overfill_capacity = sandart_sim::physics::overfill_ceiling_for(stiffness);
    }

    pub fn get_overfill_stiffness(&self) -> f32 {
        self.sim.overfill_stiffness
    }

    pub fn set_overfill_capacity(&mut self, capacity: f32) {
        self.sim.overfill_capacity = capacity;
    }

    pub fn get_overfill_capacity(&self) -> f32 {
        self.sim.overfill_capacity
    }

    /// Block-simulation heat-map debug overlay: purely a render-side toggle (see
    /// `heatmap_enabled`'s field doc comment) — the underlying per-block counter runs
    /// unconditionally in `sim`, this only gates whether `render()` uploads/draws it.
    pub fn set_heatmap_overlay(&mut self, enabled: bool) {
        self.heatmap_enabled = enabled;
    }

    /// Per-cell pressure-field debug overlay: plumbed exactly like `set_heatmap_overlay` just
    /// above -- a plain field write, no reset/reinitialisation path. Purely a render-side toggle
    /// (see `pressure_heatmap_enabled`'s field doc comment); the underlying `sim.column_depth`
    /// runs unconditionally in `sim`, this only gates whether `render()` uploads/draws it. Shows
    /// whichever pass currently populates `column_depth`, so it tracks the "Fresh pressure field"
    /// toggle (`set_fresh_pressure_field`) automatically and can be used to compare the two.
    pub fn set_pressure_heatmap_overlay(&mut self, enabled: bool) {
        self.pressure_heatmap_enabled = enabled;
        // Mirrored into the sim purely as a cost gate on the saturation-decile refresh, which
        // only exists to produce this overlay's legend.
        self.sim.pressure_heatmap_overlay = enabled;
    }

    /// Coarse-level `eta` (hydraulic head) debug overlay: plumbed exactly like
    /// `set_heatmap_overlay` -- a plain field write, no reset/reinitialisation path. Purely a
    /// render-side toggle; `sim.coarse_state.eta` is maintained by `sim.update()` unconditionally
    /// whenever `sim.coarse.available` (OVERCLOCKING.md: no longer tied to
    /// `coarse_pressure_coupling`, see that field's own doc comment), this only gates whether
    /// `render()` bothers uploading/tinting with it.
    pub fn set_coarse_eta_overlay(&mut self, enabled: bool) {
        self.coarse_eta_enabled = enabled;
    }

    /// Coarse-fine disagreement (`Delta`) debug overlay: plumbed exactly like
    /// `set_coarse_eta_overlay` just above -- an independent toggle for an independent texture,
    /// so both this and the `eta` overlay can be viewed at once.
    pub fn set_coarse_delta_overlay(&mut self, enabled: bool) {
        self.coarse_delta_enabled = enabled;
    }

    /// Numeric readout for the coarse `eta` overlay's console-footer entry: `[min, max, mean,
    /// base_head_reference]` over `inside` coarse tiles, or empty when nothing is coarse-coupled
    /// on screen (same empty-when-unavailable convention `get_saturation_deciles` already uses).
    /// Exists because a colour ramp alone can't distinguish "the field truly is flat" from "I
    /// can't tell" -- see `DrawingSimulation::coarse_eta_stats`'s doc comment for what each
    /// element means, and `coarse_eta_texels`'s for why `base_head_reference` (not the frame's own
    /// spread) is the scale the colour ramp itself is built against.
    pub fn get_coarse_eta_stats(&self) -> Vec<f32> {
        match self.sim.coarse_eta_stats() {
            Some((min, max, mean, reference)) => vec![min, max, mean, reference],
            None => Vec::new(),
        }
    }

    /// Numeric readout for the coarse `Delta` overlay's console-footer entry: `[max(|Delta|)]` in
    /// raw mass units over `inside` coarse tiles, or empty when nothing is coarse-coupled on
    /// screen. Deliberately the plain absolute number, not normalised against anything -- the
    /// point is to let the user read "how big is the worst disagreement right now" directly, which
    /// is exactly the number `coarse_delta_texels`' per-tile `capacity[C]` normalisation does NOT
    /// surface on its own.
    pub fn get_coarse_delta_max_abs(&self) -> Vec<f32> {
        match self.sim.coarse_delta_max_abs() {
            Some(max_abs) => vec![max_abs],
            None => Vec::new(),
        }
    }

    /// The overfill heat-map's legend: nine saturation decile boundaries (D1..D9), where
    /// saturation is `height / capacity`, so 1.0 is exactly full and above that is overfill.
    /// Empty until the overlay has been on long enough for the first refresh, and empty whenever
    /// the overfill model is off (the overlay then shows an absolute pressure scale, which needs
    /// no legend of its own). Read this to answer "how saturated are we" -- under decile
    /// colouring the hue tells you a cell's RANK, and only these numbers tell you its magnitude.
    pub fn get_saturation_deciles(&self) -> Vec<f32> {
        self.sim.saturation_deciles.clone()
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
                let wetness = self.sim.cell_props[i * 4 + sandart_sim::PROP_WETNESS];
                let cap = sandart_sim::physics::cell_capacity_for(wetness);
                interleaved[i * 4 + 0] = self.sim.heightmap.data[i].min(cap);
                interleaved[i * 4 + 1] = wetness;
                interleaved[i * 4 + 2] = self.sim.cell_props[i * 4 + sandart_sim::PROP_GRAIN_SIZE];
                interleaved[i * 4 + 3] = 1.0;
            }
            self.renderer.update_heightmap(&self.queue, &interleaved);
            self.renderer.update_colormap(&self.queue, &self.sim.cell_colors);
            self.full_upload_needed = false;
            // Anything that forces a full heightmap re-upload -- reset, resolution change, shape
            // change, material change -- has also invalidated whatever the pressure overlay last
            // drew. Hanging the invalidation off this flag rather than off each of those call
            // sites individually is deliberate: `full_upload_needed` is already set at every one
            // of them, so a future one gets this for free instead of silently keeping a stale
            // field. It also covers the case the tick-count key alone cannot -- `sim.reset()`
            // sets `tick_count` back to 0, which could otherwise collide with a cached key from
            // an earlier tick 0 and leave the pre-reset image on screen.
            self.pressure_heat_cache_key = None;
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
                        let wetness = self.sim.cell_props[src_idx * 4 + sandart_sim::PROP_WETNESS];
                        let cap = sandart_sim::physics::cell_capacity_for(wetness);
                        interleaved[dest_idx * 4 + 0] = self.sim.heightmap.data[src_idx].min(cap);
                        interleaved[dest_idx * 4 + 1] = wetness;
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
            heatmap_enabled: if self.heatmap_enabled { 1 } else { 0 },
            pressure_heatmap_enabled: if self.pressure_heatmap_enabled { 1 } else { 0 },
            coarse_eta_enabled: if self.coarse_eta_enabled { 1 } else { 0 },
            coarse_delta_enabled: if self.coarse_delta_enabled { 1 } else { 0 },
        };
        self.renderer.update_uniforms(&self.queue, &current_uniforms);

        // Block-simulation heat-map debug overlay: `sim.block_heat_buckets` is always
        // maintained (see its doc comment), but building the byte array and uploading it to the
        // GPU only happens while the overlay is actually switched on — this is the "costs
        // nothing when disabled" half of the contract, the shader-side `heatmap_enabled == 0u`
        // early-out (see shader.wgsl) is the other half.
        if self.heatmap_enabled {
            let heat_texels = self.sim.block_heat_texels();
            self.renderer.update_block_heat(&self.queue, &heat_texels);
        }

        // Per-cell pressure-field debug overlay: same "costs nothing when disabled" contract as
        // the block heat-map above -- `sim.column_depth` is always maintained, but building the
        // per-cell byte array and uploading it only happens while this overlay is switched on;
        // the shader-side `pressure_heatmap_enabled == 0u` early-out is the other half.
        if self.pressure_heatmap_enabled {
            // Skip the rebuild entirely when neither the simulation nor the chosen source has
            // moved since the last upload -- see `pressure_heat_cache_key` for why (the paused
            // case this overlay exists to serve). The uploaded texture persists on the GPU, so
            // skipping the upload leaves the correct image on screen rather than a blank one.
            let cache_key = (self.sim.tick_count, self.sim.pressure_heatmap_head_field);
            if self.pressure_heat_cache_key != Some(cache_key) {
                let pressure_texels = self.sim.pressure_field_texels();
                self.renderer.update_pressure_heat(&self.queue, &pressure_texels);
                self.pressure_heat_cache_key = Some(cache_key);
            }
        }

        // Coarse-level debug overlays: same "costs nothing when disabled" contract as the two
        // above. No cache key like the pressure heat-map's -- `coarse_eta_texels`/
        // `coarse_delta_texels` are a single pass over the fixed 4096-tile coarse grid (same cost
        // order as `block_heat_texels` above, which also uploads unconditionally every frame
        // while on), not the per-cell `O(grid_size^2)` scan that made the pressure heat-map's
        // cache worth having.
        if self.coarse_eta_enabled {
            let eta_texels = self.sim.coarse_eta_texels();
            self.renderer.update_coarse_eta(&self.queue, &eta_texels);
        }
        if self.coarse_delta_enabled {
            let delta_texels = self.sim.coarse_delta_texels();
            self.renderer.update_coarse_delta(&self.queue, &delta_texels);
        }

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

    /// EARLY-STOP.md: `[executed_block_steps, block_count]` for the console footer's "steps"
    /// readout. `executed_block_steps` is `DrawingSimulation::last_frame_block_steps` -- the
    /// ACTUAL count of per-block interior sweeps the most recent `update()` call ran, summed
    /// across every repetition of its rep loop, not the rate-implied budget. `block_count` is
    /// `active_blocks.len()`, the fixed denominator the ratio is read against. Both are `i32`
    /// (never negative at these grid sizes) so the pair fits a plain `Int32Array`, same pattern as
    /// `get_active_block_counts` just above.
    pub fn get_block_step_stats(&self) -> js_sys::Int32Array {
        let executed = self.sim.last_frame_block_steps as i32;
        let block_count = self.sim.active_blocks.len() as i32;
        js_sys::Int32Array::from(&[executed, block_count][..])
    }

    pub fn get_budget_n(&self) -> usize {
        self.sim.budget_n
    }

    pub fn get_ema_frame_ms(&self) -> f32 {
        self.sim.ema_frame_ms
    }
}

pub const GRID_SIZE: usize = 512;
