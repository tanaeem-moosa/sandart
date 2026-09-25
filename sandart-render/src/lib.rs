use wgpu;

/// Default/shipped grid resolution. `HeightmapRenderer::new` takes explicit `sim_size` and
/// `render_size` arguments so the renderer's textures can be sized for 64/128/256/512 (and, for
/// `render_size`, 1024) -- see the resolution and simulation-downscale selectors in
/// `sandart-wasm`; this constant remains the value used wherever a caller doesn't need a
/// different size (tests, the desktop app), passed for both.
pub const GRID_SIZE: usize = 512;

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
    // `material_mode`, `sandbox_shape`, `neck_width` and `hourglass_curve` used to sit here:
    // uploaded by every caller (sandart-wasm, the desktop app, this crate's own tests) but never
    // read by shader.wgsl -- the vessel outline and any material-dependent colouring come from
    // `shape_mask_tex`/`cell_colors` instead, not from these scalars. Removed 2026-09-24; see
    // this struct's byte-count comments below for how the removal (16 bytes, 4 whole u32/f32
    // scalars) kept every later field's offset a clean multiple of 4 without new padding.
    pub color_mode: u32,       // active color mode (0 = Solid, 1 = Gradient/Pattern)
    pub quantile_count: u32,  // active quantile lines: 0 = off, 3 = quartiles, 9 = deciles
    /// Simulation grid resolution `S` (64/128/256/512), as f32 for direct use in shader
    /// texel-coordinate math. Replaces what used to be a pure alignment-padding field
    /// (`_pad2`) that sat here purely to 16-byte-align `quantile_positions` below — repurposing
    /// it costs no layout change (still 4 bytes at the same offset) and lets the shader stop
    /// hardcoding `512.0` for texture size, LOD grain scale, and the quantile-line row math.
    ///
    /// Was named `grid_size` before the sim-downscale feature (`sandart-wasm`'s
    /// `set_sim_downscale`) split simulation resolution from render/display resolution `n` (see
    /// `render_size` below): every texel-per-cell texture in `HeightmapRenderer` (heightmap,
    /// pressure heat-map) and the quantile-line row math are all sim-sized, so they still read
    /// this field, unchanged. The shape mask ALSO stays sim-sized (uploaded straight from
    /// `sim.shape_mask` -- see `HeightmapRenderer::sim_size`'s doc comment for why it is not
    /// re-rasterized at `n`), so `mask_coord` below reads this field too, not `render_size`.
    pub sim_size: f32,
    // Quantile line positions, normalised 0.0 (top row edge) .. 1.0 (bottom row edge).
    // Packed as 3x vec4 (12 slots, only the first `quantile_count` used) rather than
    // `[f32; 9]` because WGSL pads array-of-f32 elements to 16 bytes each in a uniform buffer
    // block (144 bytes wasted for 9 floats), whereas array<vec4<f32>, 3> has zero padding.
    pub quantile_positions: [[f32; 4]; 3],
    pub marbles: [MarbleUniform; 5], // array of up to 5 marbles
    /// Explicit padding. These four `u32` slots held, in order: the block-simulation heat-map
    /// overlay flag (`heatmap_enabled`), the per-cell pressure-field heat-map overlay flag
    /// (`pressure_heatmap_enabled`), and the coarse-level eta/disagreement debug overlay flags
    /// (`coarse_eta_enabled`/`coarse_delta_enabled`, removed 2026-09-17 when `coarse.rs` was
    /// found to have no surviving producer). All four were removed together 2026-09-17: none of
    /// the four textures/bindings they gated (`block_heat_tex`, `pressure_heat_tex`, and the two
    /// already-removed coarse textures) had any producer or upload caller left in the tree --
    /// `update_block_heat`/`update_pressure_heat` had zero callers anywhere, no wasm setter for
    /// either flag was ever implemented, and no UI checkbox or `demo.js` wiring referenced them,
    /// so both flags could only ever be 0. `_pad_heatmap_tail2`/`_pad_heatmap_tail3` are the
    /// (already-explicit) former `coarse_eta_enabled`/`coarse_delta_enabled` slots; this comment
    /// on `_pad_heatmap_tail0` is the one to read for the layout history of all four.
    ///
    /// The struct's overall alignment is 16 (forced by `quantile_positions`/`marbles`), so its
    /// size must land on a 16-byte multiple; before `heatmap_enabled` was added the struct was
    /// exactly 224 bytes (`224 / 16 == 14`, no trailing pad). A single `u32` there would have
    /// landed at 228, and Rust would silently insert 12 bytes of TRAILING padding to round back
    /// up to 240 -- which `derive(Pod)` correctly refuses to allow, since padding bytes are
    /// uninitialized and Pod promises every byte is defined. Explicit padding fields make that
    /// padding explicit data instead, the same fix `sim_size` above already used once for a
    /// mid-struct gap (see its doc comment).
    pub _pad_heatmap_tail0: u32,
    pub _pad_heatmap_tail1: u32,
    pub _pad_heatmap_tail2: u32,
    pub _pad_heatmap_tail3: u32,
    /// Render/display grid resolution `n` (the resolution `<select>`'s value; can be 1024, unlike
    /// `sim_size`) -- the sim-downscale feature's second size, added when simulation resolution
    /// `S` split from `n` (`S = n / m`, `sandart-wasm`'s `set_sim_downscale`). Unlike `sim_size`
    /// this is NOT a repurposed padding slot: at the time this field was added every one of
    /// `_pad_heatmap_tail0..3`'s slots was already spent by the four now-removed debug overlay
    /// flags (see `_pad_heatmap_tail0`'s doc comment above), so this grows the struct from 224 to
    /// 240 bytes -- the next 16-byte multiple, since the struct's overall alignment is 16 (forced
    /// by `quantile_positions`/`marbles`). Used only where a shader quantity is genuinely
    /// per-render-pixel rather than per-simulation-cell -- currently just the grain hash
    /// (`hash(floor(uv * render_size))`), which is meant to look like fixed-size sand grains on
    /// screen regardless of how coarsely the interior is being simulated. At `m == 1` this equals
    /// `sim_size` exactly.
    pub render_size: f32,
    /// Explicit trailing padding, added alongside `render_size` above for the same reason
    /// `_pad_heatmap_tail0..3` originally existed (see that field's doc comment): `render_size`
    /// lands the struct at 228 bytes, and Rust would otherwise silently insert 12 bytes of
    /// TRAILING padding to round back up to the next 16-byte multiple (240) -- which
    /// `derive(Pod)` correctly refuses to allow, since padding bytes are uninitialized and Pod
    /// promises every byte is defined. Three bare `u32` fields, NOT `[u32; 3]`: WGSL's
    /// uniform-address-space layout rules force an array's per-element stride to 16 bytes (see
    /// `quantile_positions`'s doc comment above), which would desync this padding's size from
    /// this tightly-packed Rust side. Nothing to repurpose the slack for yet, so it stays padding
    /// -- the next debug overlay flag or shader scalar should spend these before growing the
    /// struct again.
    pub _pad_uniform_tail0: u32,
    pub _pad_uniform_tail1: u32,
    pub _pad_uniform_tail2: u32,
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
    /// Sized `sim_size` x `sim_size`, uploaded straight from `sim.shape_mask`. This is the
    /// PHYSICS mask, and stays the authority `fs_main` falls back to whenever the fine mask below
    /// disagrees with it by more than `TAU` (a neck open on screen but closed at `S`, or vice
    /// versa, is never drawn contrary to what the sim itself thinks is inside the vessel) -- see
    /// `fine_mask_texture`'s doc comment for the mask that now supplies the everyday outline.
    pub shape_mask_texture: wgpu::Texture,
    /// The vessel outline re-rasterized at RENDER resolution (`render_size` x `render_size`, or a
    /// tiny unused placeholder at `m == 1` when `render_size == sim_size`) via
    /// `sandart_sim::DrawingSimulation::rasterize_shape_mask` -- the SAME shape function
    /// `shape_mask_texture` above is built from, just sampled at a finer lattice, so there is no
    /// second copy of the shape math anywhere and no way for the two to drift apart on a new shape
    /// parameter. This is what lets `fs_main` draw curves and corners finer than one sim cell (the
    /// whole point of this texture: user feedback at `m > 1` was that curves weren't smooth and
    /// edges weren't pointy, which a mask with one bit per SIM cell cannot carry) while
    /// `shape_mask_texture` still vetoes it
    /// wherever the two disagree enough to matter. Only uploaded (`update_fine_mask`) when `m > 1`
    /// -- see that method's doc comment.
    pub fine_mask_texture: wgpu::Texture,
    pub bind_group: wgpu::BindGroup,
    pub uniform_buffer: wgpu::Buffer,
    pub camera_buffer: wgpu::Buffer,
    pub vertex_buffer: wgpu::Buffer,
    pub index_buffer: wgpu::Buffer,
    pub num_indices: u32,
    /// Simulation grid resolution every per-sim-cell GPU texture in this struct (heightmap,
    /// colormap, `shape_mask_texture`) was allocated at -- `S` in the
    /// sim-downscale scheme (`sandart-wasm`'s `set_sim_downscale`/`set_grid_size`):
    /// `S = render_size / m`. Renamed from `grid_size` when the render resolution `n` and the sim
    /// resolution `S` decoupled.
    pub sim_size: usize,
    /// Render/display grid resolution `fine_mask_texture` was allocated at -- `n` in the
    /// sim-downscale scheme, i.e. `render_size >= sim_size` always, with equality at `m == 1`.
    /// Unlike `sim_size` above, `render_size` sizes exactly one GPU resource in this crate
    /// (`fine_mask_texture`); every other texture stays `sim_size`-scaled. Changing either field
    /// requires a full `HeightmapRenderer::new` teardown/rebuild (textures cannot be resized in
    /// place), not a mutation of this field alone.
    pub render_size: usize,
}

impl HeightmapRenderer {
    /// `render_size` is the display/render resolution `n` (>= `sim_size`, equal to it at `m ==
    /// 1`) -- see `fine_mask_texture`'s doc comment for what it sizes and why. Callers that never
    /// downscale (the desktop app, most tests) pass `sim_size` for both.
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat, sim_size: usize, render_size: usize) -> Self {
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

        // 1. Create heightmap texture (sim_size x sim_size R8Unorm)
        let texture_size = wgpu::Extent3d {
            width: sim_size as u32,
            height: sim_size as u32,
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

        // Create the fine (render-resolution) shape mask texture -- same R8Uint format and
        // 0/1/2 semantics as `shape_mask_texture` above, just sized `render_size` x `render_size`
        // instead of `sim_size` x `sim_size`. At `m == 1` (`render_size == sim_size`) this is
        // never uploaded to or read by the shader (`smooth_mask` in `fs_main` gates both), so it
        // is sized 1x1 rather than wasting a full `sim_size`-sized allocation on a texture nothing
        // touches -- see `update_fine_mask`'s doc comment.
        let fine_mask_size = if render_size > sim_size { render_size } else { 1 };
        let fine_mask_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("fine_mask_texture"),
            size: wgpu::Extent3d {
                width: fine_mask_size as u32,
                height: fine_mask_size as u32,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Uint,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let fine_mask_texture_view =
            fine_mask_texture.create_view(&wgpu::TextureViewDescriptor::default());

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
                    // Was FRAGMENT-only; `vs_main` now reads `uniforms.sim_size` too (via
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
                    // Was FRAGMENT-only. `vs_main`'s `sample_height_bilinear` now reads this too
                    // (mask-aware height sampling at `m > 1`, matching the fragment stage's own
                    // sim-mask-renormalised taps) -- same reason binding 2 above went
                    // VERTEX | FRAGMENT when `sim_size` moved into the vertex stage.
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Uint,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 10,
                    // FRAGMENT-only, like `shape_mask_tex` -- nothing in `vs_main` needs the
                    // render-resolution outline (vertex displacement stays sim-mask/heightmap
                    // driven, unchanged by this feature).
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        // Read via `textureLoad` (integer texel coords, no sampler), same as
                        // `shape_mask_tex`.
                        sample_type: wgpu::TextureSampleType::Uint,
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
                    binding: 10,
                    resource: wgpu::BindingResource::TextureView(&fine_mask_texture_view),
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
            fine_mask_texture,
            bind_group,
            uniform_buffer,
            camera_buffer,
            vertex_buffer,
            index_buffer,
            num_indices,
            sim_size,
            render_size,
        }
    }

    /// Upload CPU float heightmap data directly to the WGPU texture.
    pub fn update_heightmap(&mut self, queue: &wgpu::Queue, data: &[f32]) {
        let sim_size = self.sim_size as u32;
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
                bytes_per_row: Some(sim_size * 16),
                rows_per_image: Some(sim_size),
            },
            wgpu::Extent3d {
                width: sim_size,
                height: sim_size,
                depth_or_array_layers: 1,
            },
        );
    }

    /// Upload the shape mask (R8Uint) to GPU. Call when shape changes.
    pub fn update_shape_mask(&mut self, queue: &wgpu::Queue, data: &[u8]) {
        let sim_size = self.sim_size as u32;
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
                bytes_per_row: Some(sim_size), // 1 byte per pixel for R8Uint
                rows_per_image: Some(sim_size),
            },
            wgpu::Extent3d {
                width: sim_size,
                height: sim_size,
                depth_or_array_layers: 1,
            },
        );
    }

    /// Upload the fine (render-resolution) shape mask (R8Uint) to GPU. `data` must be
    /// `render_size * render_size` bytes, row-major -- `sandart_sim::DrawingSimulation::
    /// rasterize_shape_mask(render_size)` produces exactly that.
    ///
    /// Callers must only call this at `m > 1` (`render_size > sim_size`): at `m == 1`
    /// `fine_mask_texture` is a 1x1 placeholder (see `new`'s doc comment on that field) that this
    /// write would overrun, and the shader never reads this binding at `m == 1` anyway
    /// (`smooth_mask` in `fs_main` gates it).
    pub fn update_fine_mask(&mut self, queue: &wgpu::Queue, data: &[u8]) {
        let render_size = self.render_size as u32;
        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &self.fine_mask_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(render_size), // 1 byte per pixel for R8Uint
                rows_per_image: Some(render_size),
            },
            wgpu::Extent3d {
                width: render_size,
                height: render_size,
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
        let sim_size = self.sim_size as u32;
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
                bytes_per_row: Some(sim_size * 4),
                rows_per_image: Some(sim_size),
            },
            wgpu::Extent3d {
                width: sim_size,
                height: sim_size,
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
            let _resources = HeightmapRenderer::new(&device, target_format, GRID_SIZE, GRID_SIZE);

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

            let mut resources = HeightmapRenderer::new(&device, target_format, GRID_SIZE, GRID_SIZE);

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
                color_mode: 0,
                quantile_count: 0,
                sim_size: GRID_SIZE as f32,
                quantile_positions: [[0.0; 4]; 3],
                marbles: [
                    MarbleUniform { pos: [0.0, 0.0], radius: 0.025, z_pos: 0.0 },
                    MarbleUniform { pos: [0.0, 0.0], radius: 0.025, z_pos: 0.0 },
                    MarbleUniform { pos: [0.0, 0.0], radius: 0.025, z_pos: 0.0 },
                    MarbleUniform { pos: [0.0, 0.0], radius: 0.025, z_pos: 0.0 },
                    MarbleUniform { pos: [0.0, 0.0], radius: 0.025, z_pos: 0.0 },
                ],
                _pad_heatmap_tail0: 0,
                _pad_heatmap_tail1: 0,
                _pad_heatmap_tail2: 0,
                _pad_heatmap_tail3: 0,
                render_size: GRID_SIZE as f32,
                _pad_uniform_tail0: 0,
                _pad_uniform_tail1: 0,
                _pad_uniform_tail2: 0,
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
}

// Compile-time layout/size verification assertions for WebGPU uniform alignments
const _: () = assert!(std::mem::size_of::<LightingUniforms>() == 240);
const _: () = assert!(std::mem::size_of::<CameraUniforms>() == 80);
