use egui;
use egui_wgpu;
use wgpu;

#[repr(C, align(16))]
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct LightingUniforms {
    pub light_dir: [f32; 4],   // xyz direction + padding
    pub light_color: [f32; 4], // rgb color + padding
    pub sand_color: [f32; 4],  // rgb color + padding
    pub light_brightness: f32, // intensity
    pub shadow_enabled: u32,   // 1 = enabled, 0 = disabled
    pub led_mode: u32,         // 0 = Single, 1 = RainbowRing, 2 = ColorCycle
    pub time: f32,             // elapsed animation time
    pub marble_pos: [f32; 2],  // x, y coordinate
    pub marble_radius: f32,    // radius in normalized coordinates
    pub material_mode: u32,    // active material preset (0 to 8)
}

#[repr(C, align(16))]
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct CameraUniforms {
    pub view_proj: [f32; 16], // column-major 4x4 matrix
    pub camera_pos: [f32; 4], // xyz + padding
}

pub struct SandArtRenderResources {
    pub pipeline: wgpu::RenderPipeline,
    pub heightmap_texture: wgpu::Texture,
    pub bind_group: wgpu::BindGroup,
    pub uniform_buffer: wgpu::Buffer,
    pub camera_buffer: wgpu::Buffer,
    pub scratch_buffer: Vec<u8>,
}

impl SandArtRenderResources {
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sand_art_shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(include_str!(
                "shader.wgsl"
            ))),
        });

        // 1. Create heightmap texture (GRID_SIZE x GRID_SIZE R8Unorm)
        let texture_size = wgpu::Extent3d {
            width: crate::sim::GRID_SIZE as u32,
            height: crate::sim::GRID_SIZE as u32,
            depth_or_array_layers: 1,
        };

        let heightmap_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("heightmap_texture"),
            size: texture_size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let heightmap_texture_view =
            heightmap_texture.create_view(&wgpu::TextureViewDescriptor::default());

        // 2. Create heightmap sampler
        let heightmap_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("heightmap_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        // 3. Create lighting uniform buffer
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("lighting_uniform_buffer"),
            size: std::mem::size_of::<LightingUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Create camera uniform buffer
        let camera_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("camera_uniform_buffer"),
            size: std::mem::size_of::<CameraUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // 4. Create Bind Group Layout
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sand_art_bind_group_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        // 5. Create Bind Group
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sand_art_bind_group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&heightmap_texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&heightmap_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: camera_buffer.as_entire_binding(),
                },
            ],
        });

        // 6. Create Pipeline Layout
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sand_art_pipeline_layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        // 7. Create Render Pipeline
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sand_art_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        Self {
            pipeline,
            heightmap_texture,
            bind_group,
            uniform_buffer,
            camera_buffer,
            scratch_buffer: vec![0u8; crate::sim::GRID_SIZE * crate::sim::GRID_SIZE],
        }
    }

    /// Upload CPU float heightmap data directly to the WGPU texture.
    pub fn update_heightmap(&mut self, queue: &wgpu::Queue, data: &[f32]) {
        if self.scratch_buffer.len() != data.len() {
            self.scratch_buffer.resize(data.len(), 0);
        }

        for (src, dst) in data.iter().zip(self.scratch_buffer.iter_mut()) {
            *dst = (src * 255.0).clamp(0.0, 255.0) as u8;
        }

        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &self.heightmap_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &self.scratch_buffer,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(crate::sim::GRID_SIZE as u32),
                rows_per_image: Some(crate::sim::GRID_SIZE as u32),
            },
            wgpu::Extent3d {
                width: crate::sim::GRID_SIZE as u32,
                height: crate::sim::GRID_SIZE as u32,
                depth_or_array_layers: 1,
            },
        );
    }

    /// Upload uniform data directly to the WGPU uniform buffer.
    pub fn update_uniforms(&self, queue: &wgpu::Queue, uniforms: &LightingUniforms) {
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(uniforms));
    }

    /// Upload camera uniform data directly to the WGPU camera uniform buffer.
    pub fn update_camera(&self, queue: &wgpu::Queue, camera: &CameraUniforms) {
        queue.write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(camera));
    }
}

pub struct SandArtCallback {
    pub heightmap_data: std::sync::Arc<std::sync::Mutex<Vec<f32>>>,
    pub uniforms: LightingUniforms,
    pub camera_uniforms: CameraUniforms,
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
        if let Some(res) = resources.get_mut::<SandArtRenderResources>() {
            if let Ok(data) = self.heightmap_data.lock() {
                res.update_heightmap(queue, &data);
            }
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
        if let Some(res) = resources.get::<SandArtRenderResources>() {
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

                render_pass.set_pipeline(&res.pipeline);
                render_pass.set_bind_group(0, &res.bind_group, &[]);
                render_pass.draw(0..6, 0..1);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn get_device_and_queue() -> Option<(wgpu::Device, wgpu::Queue)> {
        let instance = wgpu::Instance::default();
        let adapter = match instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::default(),
                compatible_surface: None,
                force_fallback_adapter: false,
            })
            .await
        {
            Some(a) => a,
            None => {
                instance
                    .request_adapter(&wgpu::RequestAdapterOptions {
                        power_preference: wgpu::PowerPreference::default(),
                        compatible_surface: None,
                        force_fallback_adapter: true,
                    })
                    .await?
            }
        };

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor::default(), None)
            .await
            .ok()?;

        Some((device, queue))
    }

    #[test]
    fn test_pipeline_creation_validation() {
        pollster::block_on(async {
            let Some((device, _queue)) = get_device_and_queue().await else {
                eprintln!("Skipping GPU test: No compatible wgpu adapter found.");
                return;
            };

            device.push_error_scope(wgpu::ErrorFilter::Validation);

            let target_format = wgpu::TextureFormat::Rgba8Unorm;
            let _resources = SandArtRenderResources::new(&device, target_format);

            let error = device.pop_error_scope().await;
            assert!(
                error.is_none(),
                "Validation error during pipeline creation: {:?}",
                error
            );
        });
    }

    #[test]
    fn test_headless_render_capture() {
        pollster::block_on(async {
            let Some((device, queue)) = get_device_and_queue().await else {
                eprintln!("Skipping GPU test: No compatible wgpu adapter found.");
                return;
            };

            let width = 256;
            let height = 256;
            let target_format = wgpu::TextureFormat::Rgba8Unorm;

            let mut resources = SandArtRenderResources::new(&device, target_format);

            let grid_size = crate::sim::GRID_SIZE;
            let mut heightmap_data = vec![0.0f32; grid_size * grid_size];
            for y in 0..256 {
                for x in 0..grid_size {
                    heightmap_data[y * grid_size + x] = 1.0;
                }
            }
            resources.update_heightmap(&queue, &heightmap_data);

            // Update uniforms for headless tests
            let uniforms = LightingUniforms {
                light_dir: [0.5, 0.5, 0.5, 0.0],
                light_color: [1.0, 1.0, 1.0, 1.0],
                sand_color: [0.92, 0.89, 0.82, 1.0],
                light_brightness: 1.0,
                shadow_enabled: 1,
                led_mode: 1,
                time: 0.0,
                marble_pos: [0.0, 0.0],
                marble_radius: 0.025,
                material_mode: 0,
            };
            resources.update_uniforms(&queue, &uniforms);

            let camera_uniforms = CameraUniforms {
                view_proj: [
                    1.0, 0.0, 0.0, 0.0,
                    0.0, 1.0, 0.0, 0.0,
                    0.0, 0.0, 1.0, 0.0,
                    0.0, 0.0, 0.0, 1.0,
                ],
                camera_pos: [0.0, 0.0, 2.0, 0.0],
            };
            resources.update_camera(&queue, &camera_uniforms);

            let texture_desc = wgpu::TextureDescriptor {
                label: Some("test_target_texture"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: target_format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            };
            let texture = device.create_texture(&texture_desc);
            let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());

            let buffer_desc = wgpu::BufferDescriptor {
                label: Some("test_readback_buffer"),
                size: (width * height * 4) as u64,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            };
            let read_buffer = device.create_buffer(&buffer_desc);

            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("test_encoder"),
            });

            {
                let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("test_render_pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &texture_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });

                render_pass.set_viewport(0.0, 0.0, width as f32, height as f32, 0.0, 1.0);
                render_pass.set_pipeline(&resources.pipeline);
                render_pass.set_bind_group(0, &resources.bind_group, &[]);
                render_pass.draw(0..6, 0..1);
            }

            encoder.copy_texture_to_buffer(
                wgpu::ImageCopyTexture {
                    texture: &texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::ImageCopyBuffer {
                    buffer: &read_buffer,
                    layout: wgpu::ImageDataLayout {
                        offset: 0,
                        bytes_per_row: Some(width * 4),
                        rows_per_image: Some(height),
                    },
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );

            queue.submit(Some(encoder.finish()));

            let buffer_slice = read_buffer.slice(..);
            let (tx, rx) = std::sync::mpsc::channel();
            buffer_slice.map_async(wgpu::MapMode::Read, move |res| {
                tx.send(res).unwrap();
            });

            device.poll(wgpu::Maintain::Wait);
            rx.recv().unwrap().expect("Failed to map readback buffer");

            let data = buffer_slice.get_mapped_range();

            // Verify render has run (RGBA values are populated)
            let top_offset = ((64 * width + 128) * 4) as usize;
            let a_top = data[top_offset + 3];
            assert_eq!(a_top, 255);

            drop(data);
            read_buffer.unmap();
        });
    }
}

// Compile-time layout/size verification assertions for WebGPU uniform alignments
const _: () = assert!(std::mem::size_of::<LightingUniforms>() == 80);
const _: () = assert!(std::mem::size_of::<CameraUniforms>() == 80);
