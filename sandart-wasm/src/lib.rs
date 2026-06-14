#![cfg(target_arch = "wasm32")]

use wasm_bindgen::prelude::*;
use wgpu;
use js_sys;
use web_sys;
use wasm_bindgen::JsCast;

use sandart_sim::{DrawingSimulation, MaterialMode, SandboxShape};
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
    full_upload_needed: bool,

    // Config state
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

        let renderer = HeightmapRenderer::new(&device, target_format);
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
            full_upload_needed: true,
            marble_count: 1,
            material_mode: MaterialMode::ButterCream,
            sandbox_shape: SandboxShape::Circle,
            marble_size: 0.018,
            speed: 1.0,
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

    pub fn step(&mut self, dt: f32, cursor_x: f32, cursor_y: f32, shift_pressed: bool) {
        self.elapsed_time += dt;
        let mut targets = [None; 5];

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

        self.sim.update(
            dt,
            &targets,
            self.marble_size,
            self.material_mode,
            self.sandbox_shape,
        );
    }

    pub fn reset(&mut self) {
        self.sim.reset();
        self.full_upload_needed = true;
        self.playback.state = PlaybackState::Stopped;
        self.playback.current_indices = [0; 5];
    }

    pub fn draw_ripples(&mut self) {
        self.sim.heightmap.generate_ripples();
        self.full_upload_needed = true;
    }

    pub fn set_material_mode(&mut self, mode: u32) {
        self.material_mode = match mode {
            0 => MaterialMode::ButterCream,
            1 => MaterialMode::DrySand,
            2 => MaterialMode::Snow,
            3 => MaterialMode::KineticSand,
            4 => MaterialMode::WetSand,
            5 => MaterialMode::FinePowder,
            6 => MaterialMode::Oobleck,
            7 => MaterialMode::MoonDust,
            8 => MaterialMode::IronFilings,
            9 => MaterialMode::Water,
            10 => MaterialMode::Milk,
            11 => MaterialMode::Ferrofluid,
            12 => MaterialMode::VegetableOil,
            13 => MaterialMode::CalmWater,
            14 => MaterialMode::Yogurt,
            15 => MaterialMode::CoarseSand,
            _ => MaterialMode::ButterCream,
        };
    }

    pub fn set_sandbox_shape(&mut self, shape: u32) {
        self.sandbox_shape = match shape {
            0 => SandboxShape::Circle,
            1 => SandboxShape::Square,
            2 => SandboxShape::Oval,
            _ => SandboxShape::Circle,
        };
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
                let order = self.hilbert_order.clamp(1, 4);
                sandart_pattern::generate_gosper_curve(order)
            }
            "sierpinski" => {
                let order = self.hilbert_order.clamp(1, 6);
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
            _ => return false,
        };

        let arms = self.marble_count.clamp(1, 5) as usize;
        for j in 0..arms {
            let angle_offset = (j as f32 / arms as f32) * 2.0 * std::f32::consts::PI;
            let mut rotated = Vec::with_capacity(base_waypoints.len());
            for p in &base_waypoints {
                let cos_a = angle_offset.cos();
                let sin_a = angle_offset.sin();
                let rx = p.x * cos_a - p.y * sin_a;
                let ry = p.x * sin_a + p.y * cos_a;
                let mut p_rot = glam::Vec2::new(rx, ry);
                if self.sandbox_shape == SandboxShape::Oval {
                    p_rot.y *= 0.652;
                }
                rotated.push(p_rot);
            }
            self.playback.waypoints[j] = rotated;
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

        // Update GPU heightmap
        if self.full_upload_needed {
            self.renderer.update_heightmap(&self.queue, self.sim.heightmap.as_slice());
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
            self.renderer.update_heightmap_partial(
                &self.queue,
                self.sim.heightmap.as_slice(),
                render_bounds,
            );
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
                GRID_SIZE,
                GRID_SIZE,
            );
            let z = self.sim.heightmap.get(gx, gy);
            current_marbles[j] = MarbleUniform {
                pos: [self.sim.marbles[j].pos.x, self.sim.marbles[j].pos.y],
                radius: self.marble_size,
                z_pos: z,
            };
        }

        let current_uniforms = LightingUniforms {
            light_dir: current_light_dir,
            light_color: self.led_color,
            sand_color: self.sand_color,
            light_brightness: 1.2,
            shadow_enabled: if self.shadows_enabled { 1 } else { 0 },
            led_mode: self.led_mode,
            time: self.elapsed_time,
            marble_count: self.marble_count,
            material_mode: self.material_mode as u32,
            sandbox_shape: self.sandbox_shape as u32,
            _padding: 0,
            marbles: current_marbles,
        };
        self.renderer.update_uniforms(&self.queue, &current_uniforms);

        // Calculate view projection matrix
        let aspect = self.surface_config.width as f32 / self.surface_config.height as f32;
        let projection = glam::Mat4::perspective_lh(0.785, aspect, 0.1, 100.0);
        let cam_x = self.camera_zoom * self.camera_elevation.cos() * self.camera_azimuth.sin();
        let cam_y = self.camera_zoom * self.camera_elevation.cos() * self.camera_azimuth.cos();
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
}

pub const GRID_SIZE: usize = 1024;
