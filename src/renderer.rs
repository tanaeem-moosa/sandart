use wgpu;
use egui;
use egui_wgpu;

pub struct SandArtRenderResources {
    pub pipeline: wgpu::RenderPipeline,
}

impl SandArtRenderResources {
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sand_art_shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(include_str!("shader.wgsl"))),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sand_art_pipeline_layout"),
            bind_group_layouts: &[],
            push_constant_ranges: &[],
        });

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

        Self { pipeline }
    }
}

pub struct SandArtCallback;

impl egui_wgpu::CallbackTrait for SandArtCallback {
    fn prepare(
        &self,
        _device: &wgpu::Device,
        _queue: &wgpu::Queue,
        _screen_descriptor: &egui_wgpu::ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        _resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
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
            let rect = info.viewport; // Use viewport instead of clip_rect to avoid stretching/warping

            let target_width = info.screen_size_px[0] as f32;
            let target_height = info.screen_size_px[1] as f32;

            // Convert logical points to physical pixels, clamping to the physical screen boundaries
            // to prevent out-of-bounds viewports during window resizing.
            let physical_x = (rect.min.x * pixels_per_point).clamp(0.0, target_width);
            let physical_y = (rect.min.y * pixels_per_point).clamp(0.0, target_height);
            let physical_width = (rect.width() * pixels_per_point).min(target_width - physical_x);
            let physical_height = (rect.height() * pixels_per_point).min(target_height - physical_y);

            // Fix: Guard against division-by-zero or empty viewports
            if physical_width > 0.0 && physical_height > 0.0 {
                // Enforce a pixel-perfect viewport mapping matching our centered canvas
                render_pass.set_viewport(
                    physical_x,
                    physical_y,
                    physical_width,
                    physical_height,
                    0.0,
                    1.0,
                );

                render_pass.set_pipeline(&res.pipeline);
                render_pass.draw(0..6, 0..1);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Helper to initialize a device and queue in a headless context
    async fn get_device_and_queue() -> Option<(wgpu::Device, wgpu::Queue)> {
        let instance = wgpu::Instance::default();
        let adapter = match instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::default(),
            compatible_surface: None,
            force_fallback_adapter: false,
        }).await {
            Some(a) => a,
            None => {
                instance.request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::default(),
                    compatible_surface: None,
                    force_fallback_adapter: true,
                }).await?
            }
        };

        let (device, queue) = adapter.request_device(
            &wgpu::DeviceDescriptor::default(),
            None,
        ).await.ok()?;

        Some((device, queue))
    }

    #[test]
    fn test_pipeline_creation_validation() {
        pollster::block_on(async {
            let Some((device, _queue)) = get_device_and_queue().await else {
                eprintln!("Skipping GPU test: No compatible wgpu adapter found.");
                return;
            };

            // Capture pipeline creation validation errors
            device.push_error_scope(wgpu::ErrorFilter::Validation);
            
            let target_format = wgpu::TextureFormat::Rgba8Unorm;
            let _resources = SandArtRenderResources::new(&device, target_format);

            let error = device.pop_error_scope().await;
            assert!(error.is_none(), "Validation error during pipeline creation: {:?}", error);
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

            let resources = SandArtRenderResources::new(&device, target_format);

            // Create offscreen texture
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

            // Create readback buffer (aligned to 256 bytes per row)
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

            // Perform render pass
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
                render_pass.draw(0..6, 0..1);
            }

            // Copy texture to buffer
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

            // Map buffer and read pixels
            let buffer_slice = read_buffer.slice(..);
            let (tx, rx) = std::sync::mpsc::channel();
            buffer_slice.map_async(wgpu::MapMode::Read, move |res| {
                tx.send(res).unwrap();
            });

            device.poll(wgpu::Maintain::Wait);
            rx.recv().unwrap().expect("Failed to map readback buffer");

            let data = buffer_slice.get_mapped_range();
            
            // Check center pixel (128, 128) - should be within the circle (Coral color: ~[229, 102, 76, 255])
            let center_offset = ((128 * width + 128) * 4) as usize;
            let r_center = data[center_offset];
            let g_center = data[center_offset + 1];
            let b_center = data[center_offset + 2];
            let a_center = data[center_offset + 3];

            // Check corner pixel (0, 0) - should be outside the circle (Dark background color: ~[25, 25, 30, 255])
            let corner_offset = 0usize;
            let r_corner = data[corner_offset];
            let g_corner = data[corner_offset + 1];
            let b_corner = data[corner_offset + 2];
            let a_corner = data[corner_offset + 3];

            // Verify center (Coral Sand Color: 0.9, 0.4, 0.3, 1.0)
            assert!(r_center > 200, "Center R should be high (coral), got {}", r_center);
            assert!(g_center > 90 && g_center < 120, "Center G should be moderate, got {}", g_center);
            assert!(b_center > 60 && b_center < 90, "Center B should be low, got {}", b_center);
            assert_eq!(a_center, 255, "Center alpha should be 255");

            // Verify corner (Dark Background: 0.1, 0.1, 0.12, 1.0)
            assert!(r_corner < 40, "Corner R should be low (dark frame), got {}", r_corner);
            assert!(g_corner < 40, "Corner G should be low, got {}", g_corner);
            assert!(b_corner > 20 && b_corner < 40, "Corner B should be low, got {}", b_corner);
            assert_eq!(a_corner, 255, "Corner alpha should be 255");

            drop(data);
            read_buffer.unmap();
        });
    }
}

