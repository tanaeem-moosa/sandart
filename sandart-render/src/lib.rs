use wgpu;

/// Default/shipped grid resolution. `HeightmapRenderer::new` now takes an explicit `grid_size`
/// so the renderer's textures can be sized for 64/128/256/512 (see the resolution selector in
/// `sandart-wasm`); this constant remains the value used wherever a caller doesn't need a
/// different size (tests, the desktop app).
pub const GRID_SIZE: usize = 512;

/// The LOD scheduler's block grid edge length -- always 32x32 blocks regardless of simulation
/// resolution (see `sandart-sim`'s `DrawingSimulation::new_with_size` doc comment on
/// `block_size`). The block-simulation heat-map overlay's texture is sized to this, not to
/// `GRID_SIZE`/the current resolution.
pub const HEAT_GRID_SIZE: usize = 32;

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub pos: [f32; 2],
}

impl Vertex {
    pub fn desc() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x2,
                },
            ],
        }
    }
}

#[repr(C, align(16))]
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable, PartialEq)]
pub struct MarbleUniform {
    pub pos: [f32; 2],     // x, y coordinate
    pub radius: f32,        // radius in normalized coordinates
    pub z_pos: f32,         // z height from heightmap
}

#[repr(C, align(16))]
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable, PartialEq)]
pub struct LightingUniforms {
    pub light_dir: [f32; 4],   // xyz direction + padding
    pub light_color: [f32; 4], // rgb color + padding
    pub sand_color: [f32; 4],  // rgb color + padding
    pub light_brightness: f32, // intensity
    pub shadow_enabled: u32,   // 1 = enabled, 0 = disabled
    pub led_mode: u32,         // 0 = Single, 1 = RainbowRing, 2 = ColorCycle
    pub time: f32,             // elapsed animation time
    pub marble_count: u32,     // active marbles count (1 to 5)
    pub material_mode: u32,    // active material preset (0 to 8)
    pub sandbox_shape: u32,    // active sandbox shape (0 = Circle, 1 = Square, 2 = Oval)
    pub color_mode: u32,       // active color mode (0 = Solid, 1 = Gradient/Pattern)
    pub neck_width: f32,       // user-controlled neck width
    pub hourglass_curve: f32,  // user-controlled hourglass shape curvature
    pub quantile_count: u32,  // active quantile lines: 0 = off, 3 = quartiles, 9 = deciles
    /// Current simulation grid resolution (64/128/256/512), as f32 for direct use in shader
    /// texel-coordinate math. Replaces what used to be a pure alignment-padding field
    /// (`_pad2`) that sat here purely to 16-byte-align `quantile_positions` below — repurposing
    /// it costs no layout change (still 4 bytes at the same offset) and lets the shader stop
    /// hardcoding `512.0` for texture size, LOD grain scale, and the quantile-line row math.
    pub grid_size: f32,
    // Quantile line positions, normalised 0.0 (top row edge) .. 1.0 (bottom row edge).
    // Packed as 3x vec4 (12 slots, only the first `quantile_count` used) rather than
    // `[f32; 9]` because WGSL pads array-of-f32 elements to 16 bytes each in a uniform buffer
    // block (144 bytes wasted for 9 floats), whereas array<vec4<f32>, 3> has zero padding.
    pub quantile_positions: [[f32; 4]; 3],
    pub marbles: [MarbleUniform; 5], // array of up to 5 marbles
    /// Block-simulation heat-map debug overlay: 1 = draw it, 0 = off (default). Appended as the
    /// LAST field rather than interleaved among the scalars above (`shadow_enabled` et al.), so
    /// only the tail of the struct needs to change.
    ///
    /// The struct's overall alignment is 16 (forced by `quantile_positions`/`marbles`), so its
    /// size must land on a 16-byte multiple; before this field the struct was exactly 224 bytes
    /// (`224 / 16 == 14`, no trailing pad). A single `u32` here would land at 228, and Rust
    /// would silently insert 12 bytes of TRAILING padding to round back up to 240 -- which
    /// `derive(Pod)` correctly refuses to allow, since padding bytes are uninitialized and Pod
    /// promises every byte is defined. `_pad_heatmap` makes that padding explicit data instead,
    /// the same fix `grid_size` above already used once for a mid-struct gap (see its doc
    /// comment) -- here there's nothing useful to repurpose the slack for, so it stays padding.
    pub heatmap_enabled: u32,
    /// Per-cell pressure-field debug overlay (see `HeightmapRenderer::pressure_heat_texture`'s
    /// doc comment and `sandart_sim::DrawingSimulation::pressure_field_texels`): 1 = draw it,
    /// 0 = off (default). Repurposes one of `heatmap_enabled`'s three trailing pad slots below
    /// (the same move `grid_size` above made once already for a mid-struct gap -- see its doc
    /// comment) rather than growing the struct, so this costs no layout change and the
    /// `size_of::<LightingUniforms>() == 240` assert further down stays untouched.
    pub pressure_heatmap_enabled: u32,
    pub _pad_heatmap: [u32; 2],
}

#[repr(C, align(16))]
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct CameraUniforms {
    pub view_proj: [f32; 16], // column-major 4x4 matrix
    pub camera_pos: [f32; 4], // xyz + padding
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActiveBounds {
    pub min_x: usize,
    pub max_x: usize,
    pub min_y: usize,
    pub max_y: usize,
    pub active: bool,
}

pub struct HeightmapRenderer {
    pub pipeline: wgpu::RenderPipeline,
    pub heightmap_texture: wgpu::Texture,
    pub colormap_texture: wgpu::Texture,
    pub shape_mask_texture: wgpu::Texture,
    /// Block-simulation heat-map debug overlay texture. Fixed 32x32 (`HEAT_GRID_SIZE`) rather
    /// than `grid_size` x `grid_size` like the textures above -- the LOD scheduler's block grid
    /// is always exactly 32x32 regardless of simulation resolution (see the `block_size`
    /// derivation note in sandart-sim's `DrawingSimulation::new_with_size`), so this is the one
    /// GPU resource in this struct that does NOT need rebuilding when resolution changes.
    pub block_heat_texture: wgpu::Texture,
    /// Per-cell pressure-field debug overlay texture. Unlike `block_heat_texture` above, this IS
    /// sized `grid_size` x `grid_size` (one texel per simulation cell, not per 32x32 LOD block --
    /// see `set_grid_size`'s doc comment in sandart-wasm for why grid_size can be 64/128/256/512),
    /// so it must be rebuilt on a resolution change along with `heightmap_texture` et al., not
    /// left alone the way `block_heat_texture` is. R8Unorm holding the log-compressed,
    /// already-normalised [0,1] `column_depth` value produced by
    /// `sandart_sim::DrawingSimulation::pressure_field_texels` -- see that function's doc comment
    /// for the scaling rationale.
    pub pressure_heat_texture: wgpu::Texture,
    pub bind_group: wgpu::BindGroup,
    pub uniform_buffer: wgpu::Buffer,
    pub camera_buffer: wgpu::Buffer,
    pub vertex_buffer: wgpu::Buffer,
    pub index_buffer: wgpu::Buffer,
    pub num_indices: u32,
    /// Resolution the GPU textures above were allocated at. Changing resolution requires a full
    /// `HeightmapRenderer::new` teardown/rebuild (textures cannot be resized in place), not a
    /// mutation of this field alone — see `sandart-wasm`'s `set_grid_size`.
    pub grid_size: usize,
}

impl HeightmapRenderer {
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat, grid_size: usize) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sand_art_shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(include_str!(
                "shader.wgsl"
            ))),
        });

        // Generate 1024x1024 vertex grid and index buffer
        let resolution = 1024;
        let mut vertices = Vec::with_capacity(resolution * resolution);
        for y in 0..resolution {
            let fy = (y as f32 / (resolution - 1) as f32) * 2.0 - 1.0;
            for x in 0..resolution {
                let fx = (x as f32 / (resolution - 1) as f32) * 2.0 - 1.0;
                vertices.push(Vertex { pos: [fx, fy] });
            }
        }

        let mut indices = Vec::with_capacity((resolution - 1) * (resolution - 1) * 6);
        for y in 0..resolution - 1 {
            for x in 0..resolution - 1 {
                let idx0 = y * resolution + x;
                let idx1 = idx0 + 1;
                let idx2 = (y + 1) * resolution + x;
                let idx3 = idx2 + 1;

                indices.push(idx0 as u32);
                indices.push(idx1 as u32);
                indices.push(idx2 as u32);

                indices.push(idx1 as u32);
                indices.push(idx3 as u32);
                indices.push(idx2 as u32);
            }
        }

        use wgpu::util::DeviceExt;
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("grid_vertex_buffer"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("grid_index_buffer"),
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        let num_indices = indices.len() as u32;

        // 1. Create heightmap texture (grid_size x grid_size R8Unorm)
        let texture_size = wgpu::Extent3d {
            width: grid_size as u32,
            height: grid_size as u32,
            depth_or_array_layers: 1,
        };

        let heightmap_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("heightmap_texture"),
            size: texture_size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let heightmap_texture_view =
            heightmap_texture.create_view(&wgpu::TextureViewDescriptor::default());

        // Create colormap texture (GRID_SIZE x GRID_SIZE Rgba8Unorm)
        let colormap_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("colormap_texture"),
            size: texture_size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let colormap_texture_view =
            colormap_texture.create_view(&wgpu::TextureViewDescriptor::default());

        // Create shape mask texture (GRID_SIZE x GRID_SIZE R8Uint)
        let shape_mask_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("shape_mask_texture"),
            size: texture_size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Uint,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let shape_mask_texture_view =
            shape_mask_texture.create_view(&wgpu::TextureViewDescriptor::default());

        // Create block-simulation heat-map texture (HEAT_GRID_SIZE x HEAT_GRID_SIZE R8Unorm).
        // R8Unorm (not R8Uint like shape_mask_texture) because the shader reads it as a
        // normalised [0,1] intensity to feed straight into a colour ramp, not as a small set of
        // discrete tags to branch on. Sized to the fixed 32x32 block grid, not `texture_size`
        // (which scales with `grid_size`) -- see `HEAT_GRID_SIZE`'s doc comment.
        let heat_texture_size = wgpu::Extent3d {
            width: HEAT_GRID_SIZE as u32,
            height: HEAT_GRID_SIZE as u32,
            depth_or_array_layers: 1,
        };
        let block_heat_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("block_heat_texture"),
            size: heat_texture_size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let block_heat_texture_view =
            block_heat_texture.create_view(&wgpu::TextureViewDescriptor::default());

        // Create per-cell pressure-field heat-map texture (grid_size x grid_size R8Unorm).
        // Sized to `texture_size` (tracks `grid_size`), unlike `block_heat_texture` above which
        // stays fixed at 32x32 -- this overlay tints individual simulation cells, not LOD blocks.
        // R8Unorm for the same reason as `block_heat_texture`: the shader reads it as a
        // normalised [0,1] intensity to feed a colour ramp, not as discrete tags.
        let pressure_heat_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("pressure_heat_texture"),
            size: texture_size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let pressure_heat_texture_view =
            pressure_heat_texture.create_view(&wgpu::TextureViewDescriptor::default());

        // 2. Create heightmap sampler (using Nearest filtering for portable R32Float manual bilinear interpolation in shader)
        let heightmap_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("heightmap_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
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
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    // Was FRAGMENT-only; `vs_main` now reads `uniforms.grid_size` too (via
                    // `sample_height_bilinear`'s `tex_size`, formerly a hardcoded 512.0), so the
                    // vertex stage needs visibility into this binding as well or pipeline
                    // creation fails validation ("Invisible" binding error).
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
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
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Uint,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        // Read via `textureLoad` in the shader (integer block coords, no
                        // sampler), same as `shape_mask_tex` above -- `filterable: false` costs
                        // nothing since nothing ever samples this with a filtering sampler.
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        // Read via `textureLoad` in the shader, same as `block_heat_tex` -- no
                        // sampler needed, so `filterable: false` costs nothing.
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
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
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&colormap_texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(&shape_mask_texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(&block_heat_texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: wgpu::BindingResource::TextureView(&pressure_heat_texture_view),
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
                buffers: &[Vertex::desc()],
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
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth24Plus,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        Self {
            pipeline,
            heightmap_texture,
            colormap_texture,
            shape_mask_texture,
            block_heat_texture,
            pressure_heat_texture,
            bind_group,
            uniform_buffer,
            camera_buffer,
            vertex_buffer,
            index_buffer,
            num_indices,
            grid_size,
        }
    }

    /// Upload CPU float heightmap data directly to the WGPU texture.
    pub fn update_heightmap(&mut self, queue: &wgpu::Queue, data: &[f32]) {
        let grid_size = self.grid_size as u32;
        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &self.heightmap_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(data),
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(grid_size * 16),
                rows_per_image: Some(grid_size),
            },
            wgpu::Extent3d {
                width: grid_size,
                height: grid_size,
                depth_or_array_layers: 1,
            },
        );
    }

    /// Upload the shape mask (R8Uint) to GPU. Call when shape changes.
    pub fn update_shape_mask(&mut self, queue: &wgpu::Queue, data: &[u8]) {
        let grid_size = self.grid_size as u32;
        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &self.shape_mask_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(grid_size), // 1 byte per pixel for R8Uint
                rows_per_image: Some(grid_size),
            },
            wgpu::Extent3d {
                width: grid_size,
                height: grid_size,
                depth_or_array_layers: 1,
            },
        );
    }

    /// Upload the block-simulation heat-map (R8Unorm) to GPU. `data` must be
    /// `HEAT_GRID_SIZE * HEAT_GRID_SIZE` bytes, row-major -- see
    /// `DrawingSimulation::block_heat_texels` in sandart-sim, which produces exactly that.
    /// Callers are expected to only call this while the overlay is switched on (see
    /// `sandart-wasm`'s `render()`), since the whole point of gating the upload behind the
    /// toggle -- not just the shader read -- is to cost nothing when it's off.
    pub fn update_block_heat(&mut self, queue: &wgpu::Queue, data: &[u8]) {
        let size = HEAT_GRID_SIZE as u32;
        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &self.block_heat_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(size), // 1 byte per pixel for R8Unorm
                rows_per_image: Some(size),
            },
            wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
        );
    }

    /// Upload the per-cell pressure-field heat-map (R8Unorm) to GPU. `data` must be
    /// `grid_size * grid_size` bytes, row-major -- see
    /// `DrawingSimulation::pressure_field_texels` in sandart-sim, which produces exactly that.
    /// Same "only call while the overlay is on" contract as `update_block_heat`: gating the
    /// upload behind the toggle (not just the shader read) is the "costs nothing when off" half
    /// that the shader-side `pressure_heatmap_enabled == 0u` early-out doesn't cover on its own.
    pub fn update_pressure_heat(&mut self, queue: &wgpu::Queue, data: &[u8]) {
        let grid_size = self.grid_size as u32;
        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &self.pressure_heat_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(grid_size), // 1 byte per pixel for R8Unorm
                rows_per_image: Some(grid_size),
            },
            wgpu::Extent3d {
                width: grid_size,
                height: grid_size,
                depth_or_array_layers: 1,
            },
        );
    }

    /// Upload a sub-rectangle of CPU float heightmap data directly to the WGPU texture.
    pub fn update_heightmap_partial(&mut self, queue: &wgpu::Queue, data: &[f32], bounds: ActiveBounds) {
        if !bounds.active {
            return;
        }

        let sub_width = (bounds.max_x - bounds.min_x + 1) as u32;
        let sub_height = (bounds.max_y - bounds.min_y + 1) as u32;

        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &self.heightmap_texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: bounds.min_x as u32,
                    y: bounds.min_y as u32,
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(data),
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some((sub_width * 16) as u32),
                rows_per_image: Some(sub_height),
            },
            wgpu::Extent3d {
                width: sub_width,
                height: sub_height,
                depth_or_array_layers: 1,
            },
        );
    }

    /// Upload CPU RGBA colormap data directly to the WGPU texture.
    pub fn update_colormap(&mut self, queue: &wgpu::Queue, data: &[u8]) {
        let grid_size = self.grid_size as u32;
        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &self.colormap_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(grid_size * 4),
                rows_per_image: Some(grid_size),
            },
            wgpu::Extent3d {
                width: grid_size,
                height: grid_size,
                depth_or_array_layers: 1,
            },
        );
    }

    /// Upload a sub-rectangle of CPU RGBA colormap data directly to the WGPU texture.
    pub fn update_colormap_partial(&mut self, queue: &wgpu::Queue, data: &[u8], bounds: ActiveBounds) {
        if !bounds.active {
            return;
        }

        let sub_width = (bounds.max_x - bounds.min_x + 1) as u32;
        let sub_height = (bounds.max_y - bounds.min_y + 1) as u32;

        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &self.colormap_texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: bounds.min_x as u32,
                    y: bounds.min_y as u32,
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some((sub_width * 4) as u32),
                rows_per_image: Some(sub_height),
            },
            wgpu::Extent3d {
                width: sub_width,
                height: sub_height,
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

    /// Perform a draw call on the render pass using the renderer's resources.
    pub fn draw<'pass>(
        &self,
        render_pass: &mut wgpu::RenderPass<'pass>,
        _camera: &CameraUniforms,
        _light: &LightingUniforms,
    ) {
        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_bind_group(0, &self.bind_group, &[]);
        render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        render_pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        render_pass.draw_indexed(0..self.num_indices, 0, 0..1);
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
            let _resources = HeightmapRenderer::new(&device, target_format, GRID_SIZE);

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

            let mut resources = HeightmapRenderer::new(&device, target_format, GRID_SIZE);

            let mut heightmap_data = vec![0.0f32; GRID_SIZE * GRID_SIZE * 4];
            for y in 0..256 {
                for x in 0..GRID_SIZE {
                    let idx = y * GRID_SIZE + x;
                    heightmap_data[idx * 4 + 0] = 1.0;
                    heightmap_data[idx * 4 + 1] = 0.0;
                    heightmap_data[idx * 4 + 2] = 0.45;
                    heightmap_data[idx * 4 + 3] = 1.0;
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
                marble_count: 1,
                material_mode: 0,
                sandbox_shape: 0,
                color_mode: 0,
                neck_width: 0.005,
                hourglass_curve: 0.6,
                quantile_count: 0,
                grid_size: GRID_SIZE as f32,
                quantile_positions: [[0.0; 4]; 3],
                marbles: [
                    MarbleUniform { pos: [0.0, 0.0], radius: 0.025, z_pos: 0.0 },
                    MarbleUniform { pos: [0.0, 0.0], radius: 0.025, z_pos: 0.0 },
                    MarbleUniform { pos: [0.0, 0.0], radius: 0.025, z_pos: 0.0 },
                    MarbleUniform { pos: [0.0, 0.0], radius: 0.025, z_pos: 0.0 },
                    MarbleUniform { pos: [0.0, 0.0], radius: 0.025, z_pos: 0.0 },
                ],
                heatmap_enabled: 0,
                pressure_heatmap_enabled: 0,
                _pad_heatmap: [0; 2],
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

            let depth_texture_desc = wgpu::TextureDescriptor {
                label: Some("test_depth_texture"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Depth24Plus,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            };
            let depth_texture = device.create_texture(&depth_texture_desc);
            let depth_view = depth_texture.create_view(&wgpu::TextureViewDescriptor::default());

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

                render_pass.set_viewport(0.0, 0.0, width as f32, height as f32, 0.0, 1.0);
                resources.draw(&mut render_pass, &camera_uniforms, &uniforms);
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

            // Verify render has run (RGBA values are populated and RGB contains rendered color, not cleared black)
            let top_offset = ((64 * width + 128) * 4) as usize;
            let r_top = data[top_offset];
            let g_top = data[top_offset + 1];
            let b_top = data[top_offset + 2];
            let a_top = data[top_offset + 3];
            assert_eq!(a_top, 255);
            assert!(r_top > 0 || g_top > 0 || b_top > 0, "Rendered pixel color is pure black; rasterization may have failed!");

            drop(data);
            read_buffer.unmap();
        });
    }

    #[test]
    #[ignore] // Run this test explicitly to save a render to disk
    fn test_save_render() {
        pollster::block_on(async {
            let Some((device, queue)) = get_device_and_queue().await else {
                eprintln!("Skipping GPU test: No compatible wgpu adapter found.");
                return;
            };

            let width = 512;
            let height = 512;
            let target_format = wgpu::TextureFormat::Rgba8Unorm;

            let mut resources = HeightmapRenderer::new(&device, target_format, GRID_SIZE);

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

            let depth_texture_desc = wgpu::TextureDescriptor {
                label: Some("test_depth_texture"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Depth24Plus,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            };

            let buffer_desc = wgpu::BufferDescriptor {
                label: Some("test_readback_buffer"),
                size: (width * height * 4) as u64,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            };

            let configs = [
                // (suffix, material_mode, led_mode, marble_count, marble_pos_x, marble_pos_y, marble_z)
                ("water_moonlight_circle", 9, 3, 1, 0.0, 0.0, 0.35),
                ("vegetable_oil_moonlight", 12, 3, 1, 0.0, 0.0, 0.35),
                ("ferrofluid_moonlight", 11, 3, 1, 0.0, 0.0, 0.35),
                ("water_rainbowmoon", 9, 4, 1, 0.0, 0.0, 0.35),
                ("calm_water_moonlight", 13, 3, 1, 0.0, 0.0, 0.35),
                ("yogurt_moonlight", 14, 3, 1, 0.0, 0.0, 0.35),
                ("coarse_sand_moonlight", 15, 3, 1, 0.0, 0.0, 0.35),
            ];

            for (suffix, mat_mode, led_mode, marble_count, m_x, m_y, m_z) in configs {
                let mut heightmap_data = vec![0.0f32; GRID_SIZE * GRID_SIZE * 4];
                let (wetness, grain_size) = match mat_mode {
                    9 => (1.00, 0.00), // Water
                    11 => (0.00, 0.45), // Ferrofluid (which is magnetism-less dry sand now, since magnetism was removed as part of this)
                    12 => (0.85, 0.00), // VegetableOil
                    13 => (0.90, 0.00), // CalmWater
                    14 => (0.75, 0.08), // Yogurt
                    15 => (0.00, 0.80), // CoarseSand
                    _ => (0.00, 0.45), // Default
                };

                for y in 0..GRID_SIZE {
                    for x in 0..GRID_SIZE {
                        let dx = (x as f32 - (GRID_SIZE / 2) as f32) / (GRID_SIZE as f32);
                        let dy = (y as f32 - (GRID_SIZE / 2) as f32) / (GRID_SIZE as f32);
                        let r = (dx * dx + dy * dy).sqrt();

                        let h = if mat_mode == 11 { // Ferrofluid
                            let mut spike_pattern = 0.0f32;
                            if r < 0.22 {
                                let weight = (1.0 - r / 0.22).max(0.0);
                                let base_pull = weight * 0.25;
                                let angle = dy.atan2(dx);
                                let radial_spikes = (angle * 24.0).cos();
                                let concentric_spikes = (r * 2.0 * std::f32::consts::PI / 0.012).cos();
                                let pattern = 0.5 + 0.5 * radial_spikes * concentric_spikes;
                                spike_pattern = base_pull * (0.3 + 0.7 * pattern);
                            }
                            (0.35 + spike_pattern).clamp(0.0, 1.0)
                        } else {
                            let decay = if mat_mode == 13 { 20.0 } else { 6.0 };
                            let ripple = 0.045 * (r * 75.0).cos() * (-r * decay).exp();
                            (0.35 + ripple).clamp(0.0, 1.0)
                        };

                        let idx = y * GRID_SIZE + x;
                        heightmap_data[idx * 4 + 0] = h;
                        heightmap_data[idx * 4 + 1] = wetness;
                        heightmap_data[idx * 4 + 2] = grain_size;
                        heightmap_data[idx * 4 + 3] = 1.0;
                    }
                }
                resources.update_heightmap(&queue, &heightmap_data);

                let uniforms = LightingUniforms {
                    light_dir: [0.0, 0.0, 1.0, 0.0],
                    light_color: [0.85, 0.90, 0.95, 1.0],
                    sand_color: [0.92, 0.89, 0.82, 1.0],
                    light_brightness: 1.4,
                    shadow_enabled: 1,
                    led_mode,
                    time: 0.0,
                    marble_count,
                    material_mode: mat_mode,
                    sandbox_shape: 0, // Circle
                    color_mode: 0,
                    neck_width: 0.005,
                    hourglass_curve: 0.6,
                    quantile_count: 0,
                    grid_size: GRID_SIZE as f32,
                    quantile_positions: [[0.0; 4]; 3],
                    marbles: [
                        MarbleUniform { pos: [m_x, m_y], radius: 0.018, z_pos: m_z },
                        MarbleUniform { pos: [0.0, 0.0], radius: 0.018, z_pos: 0.35 },
                        MarbleUniform { pos: [0.0, 0.0], radius: 0.018, z_pos: 0.35 },
                        MarbleUniform { pos: [0.0, 0.0], radius: 0.018, z_pos: 0.35 },
                        MarbleUniform { pos: [0.0, 0.0], radius: 0.018, z_pos: 0.35 },
                    ],
                    heatmap_enabled: 0,
                    pressure_heatmap_enabled: 0,
                    _pad_heatmap: [0; 2],
                };
                resources.update_uniforms(&queue, &uniforms);

                let texture = device.create_texture(&texture_desc);
                let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
                let depth_texture = device.create_texture(&depth_texture_desc);
                let depth_view = depth_texture.create_view(&wgpu::TextureViewDescriptor::default());
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

                    render_pass.set_viewport(0.0, 0.0, width as f32, height as f32, 0.0, 1.0);
                    resources.draw(&mut render_pass, &camera_uniforms, &uniforms);
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
                let raw_data = data.to_vec();
                drop(data);
                read_buffer.unmap();

                let filename = format!("target/{}.raw", suffix);
                std::fs::write(filename, raw_data).unwrap();
            }
        });
    }
}

// Compile-time layout/size verification assertions for WebGPU uniform alignments
const _: () = assert!(std::mem::size_of::<LightingUniforms>() == 240);
const _: () = assert!(std::mem::size_of::<CameraUniforms>() == 80);
