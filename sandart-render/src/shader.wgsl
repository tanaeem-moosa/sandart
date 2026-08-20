@group(0) @binding(0) var heightmap_tex: texture_2d<f32>;
@group(0) @binding(1) var heightmap_sampler: sampler;
@group(0) @binding(4) var colormap_tex: texture_2d<f32>;
@group(0) @binding(5) var shape_mask_tex: texture_2d<u32>;
// Block-simulation heat-map debug overlay. Always a fixed 64x64 texels (see `HEAT_GRID_SIZE` in
// sandart-render/src/lib.rs) regardless of `uniforms.grid_size` -- the LOD scheduler's block
// grid doesn't scale with resolution. Read via `textureLoad` (integer block coords), same as
// `shape_mask_tex`, so no sampler binding is needed for it.
@group(0) @binding(6) var block_heat_tex: texture_2d<f32>;
// Per-cell pressure-field debug overlay. Unlike `block_heat_tex` above, this IS sized
// `uniforms.grid_size` x `uniforms.grid_size` (one texel per simulation cell), holding the
// already log-compressed, normalised [0,1] `column_depth` value produced by
// `sandart_sim::DrawingSimulation::pressure_field_texels` -- see that function's doc comment for
// why a log scale against a fixed reference (not per-frame auto-normalisation) was chosen. Read
// via `textureLoad` (integer cell coords), same as `shape_mask_tex`/`block_heat_tex`, so no
// sampler binding is needed for it either.
@group(0) @binding(7) var pressure_heat_tex: texture_2d<f32>;
// Coarse-level `eta` (hydraulic head) debug overlay. Same fixed 64x64 shape as `block_heat_tex`
// above, NOT `pressure_heat_tex`'s `uniforms.grid_size`-scaled one -- the coarse grid IS the LOD
// block grid (`t = grid_size / 64` in sandart-sim's `coarse::CoarseGeometry`). Holds
// `sandart_sim::DrawingSimulation::coarse_eta_texels`'s per-frame min/max-normalised [0,1] value
// -- see that function's doc comment for why this is a floating per-frame scale rather than a
// fixed one like `pressure_heat_tex` uses. Read via `textureLoad`, no sampler needed.
@group(0) @binding(8) var coarse_eta_tex: texture_2d<f32>;
// Coarse-fine disagreement (`Delta = M - A`) debug overlay. Same shape as `coarse_eta_tex` just
// above. Holds `DrawingSimulation::coarse_delta_texels`'s output: a SYMMETRIC per-frame scale
// where 0.5 is always exactly zero disagreement (see that function's doc comment) -- this is a
// diverging quantity, not a sequential one like `coarse_eta_tex`/`block_heat_tex`, and its colour
// ramp below reflects that.
@group(0) @binding(9) var coarse_delta_tex: texture_2d<f32>;

const PI: f32 = 3.14159265359;
const Z_SCALE: f32 = 0.009; // Unified heightmap displacement scale

struct MarbleUniform {
    pos: vec2<f32>,
    radius: f32,
    z_pos: f32,
};

struct LightingUniforms {
    light_dir: vec4<f32>,
    light_color: vec4<f32>,
    sand_color: vec4<f32>,
    light_brightness: f32,
    shadow_enabled: u32,
    led_mode: u32,
    time: f32,
    marble_count: u32,
    material_mode: u32,
    sandbox_shape: u32,
    color_mode: u32,
    neck_width: f32,
    hourglass_curve: f32,
    quantile_count: u32,
    grid_size: f32,
    // Quantile line positions, normalised 0.0 (top row edge) .. 1.0 (bottom row edge), packed
    // 3 lines per vec4 (12 slots total, only the first `quantile_count` used). Must mirror the
    // Rust-side `[[f32; 4]; 3]` exactly: a plain `array<f32, 9>` here would pad each element to
    // 16 bytes in a uniform buffer block and silently desync every field that follows it.
    quantile_positions: array<vec4<f32>, 3>,
    marbles: array<MarbleUniform, 5>,
    // Block-simulation heat-map debug overlay: 1u = draw it, 0u = off (default). MUST stay the
    // second-to-last field, mirroring the Rust-side `LightingUniforms` in
    // sandart-render/src/lib.rs exactly -- see that field's doc comment for why the trailing
    // `_pad_heatmap*` scalars below exist (an explicit stand-in for what would otherwise be
    // silent struct-end padding, which `derive(Pod)` on the Rust side refuses to allow).
    //
    // Three separate `u32` scalars, NOT `array<u32, 3>` or `vec3<u32>`: WGSL's uniform-address-
    // space layout rules force BOTH of those to 16-byte alignment/stride (an array's per-element
    // stride and a vec3's own alignment each round up to 16 in this address space), which would
    // shove this padding off to a different offset than the Rust side's plain, tightly-packed
    // `[u32; 3]` and desync every byte after it. Bare scalars stay 4-byte aligned, matching Rust.
    heatmap_enabled: u32,
    // Per-cell pressure-field debug overlay: 1u = draw it, 0u = off (default). Repurposes one of
    // the trailing pad scalars above rather than growing the struct -- mirrors the Rust-side
    // `LightingUniforms::pressure_heatmap_enabled` in sandart-render/src/lib.rs exactly; see that
    // field's doc comment for why this must stay a bare `u32` at this exact offset.
    pressure_heatmap_enabled: u32,
    // Coarse-level `eta` (hydraulic head) debug overlay: 1u = draw it, 0u = off (default).
    // Repurposes one of the two remaining trailing pad scalars -- mirrors the Rust-side
    // `LightingUniforms::coarse_eta_enabled` exactly; see that field's doc comment.
    coarse_eta_enabled: u32,
    // Coarse-fine disagreement (`Delta`) debug overlay: 1u = draw it, 0u = off (default). The
    // last of the four trailing flags this struct has room for -- mirrors
    // `LightingUniforms::coarse_delta_enabled` exactly.
    coarse_delta_enabled: u32,
};

struct CameraUniforms {
    view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
};

@group(0) @binding(2) var<uniform> uniforms: LightingUniforms;
@group(0) @binding(3) var<uniform> camera: CameraUniforms;

struct VertexInput {
    @location(0) position: vec2<f32>,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) world_pos: vec3<f32>,
};

// Pseudo-random hash for sand grain highlights
fn hash(p: vec2<f32>) -> f32 {
    let h = dot(p, vec2<f32>(127.1, 311.7));
    return fract(sin(h) * 43758.5453123);
}

// Hue to RGB converter
fn hue_to_rgb(h: f32) -> vec3<f32> {
    let r = abs(h * 6.0 - 3.0) - 1.0;
    let g = 2.0 - abs(h * 6.0 - 2.0);
    let b = 2.0 - abs(h * 6.0 - 4.0);
    return clamp(vec3<f32>(r, g, b), vec3<f32>(0.0), vec3<f32>(1.0));
}

// Manual Bilinear Texture Filtering to support linear height interpolation
// on platforms without float32_filterable extension support
fn sample_height_bilinear(uv: vec2<f32>) -> f32 {
    let tex_size = uniforms.grid_size;
    let texel_coords = uv * tex_size - 0.5;
    let f = fract(texel_coords);
    let index = floor(texel_coords);
    
    let u0 = (index.x + 0.5) / tex_size;
    let v0 = (index.y + 0.5) / tex_size;
    let u1 = (index.x + 1.5) / tex_size;
    let v1 = (index.y + 1.5) / tex_size;
    
    let h00 = textureSampleLevel(heightmap_tex, heightmap_sampler, vec2<f32>(u0, v0), 0.0).r;
    let h10 = textureSampleLevel(heightmap_tex, heightmap_sampler, vec2<f32>(u1, v0), 0.0).r;
    let h01 = textureSampleLevel(heightmap_tex, heightmap_sampler, vec2<f32>(u0, v1), 0.0).r;
    let h11 = textureSampleLevel(heightmap_tex, heightmap_sampler, vec2<f32>(u1, v1), 0.0).r;
    
    let h0 = mix(h00, h10, f.x);
    let h1 = mix(h01, h11, f.x);
    return mix(h0, h1, f.y);
}

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    
    // Convert pos to UV matching original mapping:
    out.uv = vec2<f32>(
        in.position.x * 0.5 + 0.5,
        -in.position.y * 0.5 + 0.5
    );
    
    // Sample heightmap texture using manual bilinear sampling
    let height = sample_height_bilinear(out.uv);
    
    // Construct 3D world position
    out.world_pos = vec3<f32>(in.position.x, in.position.y, height * Z_SCALE);
    
    out.position = camera.view_proj * vec4<f32>(out.world_pos, 1.0);
    return out;
}

fn intersect_marble(
    C: vec3<f32>, D: vec3<f32>, P: vec3<f32>,
    pos: vec2<f32>, radius: f32, z_pos: f32,
    hit_t: ptr<function, f32>,
    hit_S: ptr<function, vec3<f32>>,
    hit_R: ptr<function, f32>,
    sphere_normal: ptr<function, vec3<f32>>,
    hit_marble: ptr<function, bool>
) {
    let S = vec3<f32>(pos.x, pos.y, z_pos * Z_SCALE);
    let V = C - S;
    let b_dot = dot(V, D);
    let c_val = dot(V, V) - radius * radius;
    let disc = b_dot * b_dot - c_val;
    if (disc >= 0.0) {
        let t = -b_dot - sqrt(disc);
        let dist_to_sand = distance(C, P);
        if (t > 0.0 && t < dist_to_sand && t < *hit_t) {
            *hit_marble = true;
            *hit_t = t;
            *hit_S = S;
            *hit_R = radius;
            let I = C + t * D;
            *sphere_normal = normalize(I - S);
        }
    }
}

fn apply_marble_shadow(
    uv: vec2<f32>,
    light_dir: vec3<f32>,
    pos: vec2<f32>,
    radius: f32,
    z_pos: f32,
    shadow_factor: ptr<function, f32>
) {
    let m_uv = vec2<f32>(pos.x * 0.5 + 0.5, -pos.y * 0.5 + 0.5);
    let S_z = z_pos * Z_SCALE;
    let r_uv = radius * 0.5;

    let shadow_offset = select(vec2<f32>(0.0), (S_z / max(light_dir.z, 0.001)) * vec2<f32>(light_dir.x, -light_dir.y), light_dir.z > 0.001);
    let d_to_shadow = distance(uv, m_uv - shadow_offset);
    if (d_to_shadow < r_uv * 1.5) {
        let m_shadow = smoothstep(r_uv * 0.8, r_uv * 1.5, d_to_shadow);
        *shadow_factor = *shadow_factor * (0.35 + 0.65 * m_shadow);
    }
}

@fragment
fn fs_main(
    @location(0) uv: vec2<f32>,
    @location(1) world_pos: vec3<f32>
) -> @location(0) vec4<f32> {
    let center = vec2<f32>(0.5, 0.5);
    let dist = distance(uv, center);
    let m_brightness = select(uniforms.light_brightness, select(uniforms.light_brightness * 0.22, uniforms.light_brightness * 0.05, uniforms.led_mode == 4u), uniforms.led_mode == 3u || uniforms.led_mode == 4u);
    
    // Determine casing and LED channel from the precomputed shape mask texture
    // Mask values: 0 = OUTSIDE (wall/casing), 1 = INSIDE (safe), 2 = BOUNDARY (inside, near wall → LED strip)
    let grid_size_i = i32(uniforms.grid_size);
    let mask_coord = vec2<i32>(i32(uv.x * uniforms.grid_size), i32(uv.y * uniforms.grid_size));
    let mask_val = textureLoad(shape_mask_tex, clamp(mask_coord, vec2<i32>(0), vec2<i32>(grid_size_i - 1)), 0).r;

    var in_casing = mask_val == 0u;
    // LED strip: boundary cells (mask=2) OR outside cells adjacent to inside cells
    var in_led = mask_val == 2u;

    let angle_light = atan2(uniforms.light_dir.y, uniforms.light_dir.x);
    let dir_light = vec2<f32>(cos(angle_light), -sin(angle_light));
    var led_center = dir_light * 0.468 + 0.5;

    if (in_casing) {
        if (in_led) {
            if (uniforms.led_mode == 0u) {
                // Single Light Mode: Draw a single glowing LED spot
                let d_to_led = distance(uv, led_center);
                var led_glow = vec3<f32>(0.08, 0.08, 0.10);
                if (d_to_led < 0.02) {
                    let intensity = smoothstep(0.02, 0.0, d_to_led);
                    led_glow = led_glow + uniforms.light_color.rgb * intensity * 1.5 * uniforms.light_brightness;
                }
                return vec4<f32>(led_glow, 1.0);
            } else if (uniforms.led_mode == 1u || uniforms.led_mode == 4u) {
                // Rainbow Ring Mode: Continuous rotating rainbow ring
                let angle = atan2(-(uv.y - 0.5), uv.x - 0.5);
                let hue = fract(angle / (2.0 * PI) - uniforms.time * 0.05);
                let led_color = hue_to_rgb(hue);
                return vec4<f32>(led_color * 1.5 * uniforms.light_brightness, 1.0);
            } else if (uniforms.led_mode == 2u) {
                // Color Cycle Mode: Continuous single color cycling ring
                let hue = fract(uniforms.time * 0.03);
                let led_color = hue_to_rgb(hue);
                return vec4<f32>(led_color * 1.5 * uniforms.light_brightness, 1.0);
            } else { // led_mode == 3u
                // Overhead Moon Light Mode: Soft glowing cool white ring
                let led_color = vec3<f32>(0.85, 0.90, 0.95);
                return vec4<f32>(led_color * 0.8 * uniforms.light_brightness, 1.0);
            }
        } else {
            // Outer casing. Lifted from 0.07 — at that value the frame read as a hole in the page
            // rather than as the body of an object, and the sand had nothing to sit against. This
            // is still well below any lit sand, so it stays recessive, but it now has a surface.
            return vec4<f32>(0.125, 0.13, 0.145, 1.0);
        }
    }

    // 2. Render shiny silver 3D marble sphere
    let C = camera.camera_pos.xyz;
    let P = world_pos;
    let D = normalize(P - C);
    let view_dir = -D;
    
    var hit_marble = false;
    var sphere_normal = vec3<f32>(0.0, 0.0, 1.0);
    var hit_t = 1e9;
    var hit_S = vec3<f32>(0.0);
    var hit_R = 0.0;
    
    if (uniforms.marble_count > 0u) {
        intersect_marble(C, D, P, uniforms.marbles[0].pos, uniforms.marbles[0].radius, uniforms.marbles[0].z_pos, &hit_t, &hit_S, &hit_R, &sphere_normal, &hit_marble);
    }
    if (uniforms.marble_count > 1u) {
        intersect_marble(C, D, P, uniforms.marbles[1].pos, uniforms.marbles[1].radius, uniforms.marbles[1].z_pos, &hit_t, &hit_S, &hit_R, &sphere_normal, &hit_marble);
    }
    if (uniforms.marble_count > 2u) {
        intersect_marble(C, D, P, uniforms.marbles[2].pos, uniforms.marbles[2].radius, uniforms.marbles[2].z_pos, &hit_t, &hit_S, &hit_R, &sphere_normal, &hit_marble);
    }
    if (uniforms.marble_count > 3u) {
        intersect_marble(C, D, P, uniforms.marbles[3].pos, uniforms.marbles[3].radius, uniforms.marbles[3].z_pos, &hit_t, &hit_S, &hit_R, &sphere_normal, &hit_marble);
    }
    if (uniforms.marble_count > 4u) {
        intersect_marble(C, D, P, uniforms.marbles[4].pos, uniforms.marbles[4].radius, uniforms.marbles[4].z_pos, &hit_t, &hit_S, &hit_R, &sphere_normal, &hit_marble);
    }

    if (hit_marble) {
        var sphere_diffuse = vec3<f32>(0.0);
        var sphere_specular = vec3<f32>(0.0);
        
        if (uniforms.led_mode == 0u || uniforms.led_mode == 3u || uniforms.led_mode == 4u) {
            let light_dir = select(normalize(uniforms.light_dir.xyz), vec3<f32>(0.0, 0.0, 1.0), uniforms.led_mode == 3u || uniforms.led_mode == 4u);
            let diff = max(dot(sphere_normal, light_dir), 0.0);
            
            let m_color = select(uniforms.light_color.rgb, vec3<f32>(0.85, 0.90, 0.95), uniforms.led_mode == 3u || uniforms.led_mode == 4u);
            
            sphere_diffuse = vec3<f32>(0.1) * diff * m_brightness;
            
            let reflect_dir = reflect(-light_dir, sphere_normal);
            let spec = pow(max(dot(reflect_dir, view_dir), 0.0), 128.0);
            sphere_specular = m_color * spec * 2.0 * m_brightness;
            
            if (uniforms.led_mode == 4u) {
                let r_dir = reflect(-view_dir, sphere_normal);
                let r_horizontal_len = length(r_dir.xy);
                if (r_horizontal_len > 0.001) {
                    let ray_slope = r_dir.z / r_horizontal_len;
                    let target_slope = 0.21;
                    let slope_diff = abs(ray_slope - target_slope);
                    let ring_spec = pow(max(1.0 - slope_diff * 4.5, 0.0), 40.0);
                    let angle_reflect = atan2(r_dir.y, r_dir.x);
                    let hue = fract(angle_reflect / (2.0 * PI) - uniforms.time * 0.05);
                    let ring_reflect_color = hue_to_rgb(hue);
                    sphere_specular = sphere_specular + ring_reflect_color * ring_spec * 1.5 * uniforms.light_brightness;
                }
            }
        } else {
            // Compute diffuse from the 8 virtual lights
            let num_leds = 8;
            for (var i = 0; i < num_leds; i = i + 1) {
                let angle_led = f32(i) * (2.0 * PI / f32(num_leds)) + uniforms.time * 0.10;
                let l_dir = normalize(vec3<f32>(cos(angle_led), sin(angle_led), 0.06));
                
                var led_color = vec3<f32>(0.0);
                if (uniforms.led_mode == 1u || uniforms.led_mode == 4u) {
                    let hue = fract(f32(i) / f32(num_leds) - uniforms.time * 0.05);
                    led_color = hue_to_rgb(hue);
                } else {
                    let hue = fract(uniforms.time * 0.03);
                    led_color = hue_to_rgb(hue);
                }
                
                let diff = max(dot(sphere_normal, l_dir), 0.0);
                sphere_diffuse = sphere_diffuse + vec3<f32>(0.08) * diff * led_color;
            }
            sphere_diffuse = sphere_diffuse * (uniforms.light_brightness / f32(num_leds));
            
            // Compute continuous specular reflection of the LED ring
            let reflect_dir = reflect(-view_dir, sphere_normal);
            let r_horizontal_len = length(reflect_dir.xy);
            
            if (r_horizontal_len > 0.001) {
                let ray_slope = reflect_dir.z / r_horizontal_len;
                let target_slope = 0.21; // angle of elevation to LED ring (Z/R ≈ 0.20/0.936)
                let slope_diff = abs(ray_slope - target_slope);
                
                // Sharp Gaussian-like falloff for specular reflection
                let ring_spec = pow(max(1.0 - slope_diff * 4.5, 0.0), 40.0);
                
                let angle_reflect = atan2(reflect_dir.y, reflect_dir.x);
                var ring_reflect_color = vec3<f32>(0.0);
                if (uniforms.led_mode == 1u || uniforms.led_mode == 4u) {
                    let hue = fract(angle_reflect / (2.0 * PI) - uniforms.time * 0.05);
                    ring_reflect_color = hue_to_rgb(hue);
                } else {
                    let hue = fract(uniforms.time * 0.03);
                    ring_reflect_color = hue_to_rgb(hue);
                }
                
                sphere_specular = ring_reflect_color * ring_spec * 3.5 * uniforms.light_brightness;
            }
        }
        
        let ambient_ref = vec3<f32>(0.4, 0.4, 0.42);
        let fresnel = pow(1.0 - max(dot(sphere_normal, view_dir), 0.0), 4.0);
        let rim_light = vec3<f32>(0.8, 0.8, 0.85) * fresnel * 0.8;
        
        let base_metal_color = vec3<f32>(0.92, 0.92, 0.94);
        let final_sphere_color = base_metal_color * (ambient_ref + sphere_diffuse) + sphere_specular + rim_light;
        
        return vec4<f32>(final_sphere_color, 1.0);
    }
    
    // 1. Compute finite difference normal from neighbor heightmap pixels
    // Normal tilting scale (high factor creates visual depth)
    let tex_size = uniforms.grid_size;
    let texel_size = 1.0 / tex_size;
    let texel_coords = uv * tex_size - 0.5;
    let index = floor(texel_coords);
    let f = fract(texel_coords);

    let u0 = clamp((index.x + 0.5) / tex_size, 0.0, 1.0);
    let v0 = clamp((index.y + 0.5) / tex_size, 0.0, 1.0);
    let u1 = clamp((index.x + 1.5) / tex_size, 0.0, 1.0);
    let v1 = clamp((index.y + 1.5) / tex_size, 0.0, 1.0);

    let sample00 = textureSampleLevel(heightmap_tex, heightmap_sampler, vec2<f32>(u0, v0), 0.0);
    let sample10 = textureSampleLevel(heightmap_tex, heightmap_sampler, vec2<f32>(u1, v0), 0.0);
    let sample01 = textureSampleLevel(heightmap_tex, heightmap_sampler, vec2<f32>(u0, v1), 0.0);
    let sample11 = textureSampleLevel(heightmap_tex, heightmap_sampler, vec2<f32>(u1, v1), 0.0);

    let h00 = sample00.r;
    let h10 = sample10.r;
    let h01 = sample01.r;
    let h11 = sample11.r;

    let h_center = mix(mix(h00, h10, f.x), mix(h01, h11, f.x), f.y);
    let props = mix(mix(sample00, sample10, f.x), mix(sample01, sample11, f.x), f.y);
    let wetness = props.g;
    let grain_size = props.b;

    // For water (high wetness), use a wider 3-tap Sobel-like filter for smooth wave normals.
    // For dry materials, use the standard 1-texel finite difference.
    var dh_dx: f32;
    var dh_dy: f32;
    let depth_factor = 14.0;
    let is_water = step(0.85, wetness);
    if (is_water > 0.5) {
        let u_prev = clamp((index.x - 0.5) / tex_size, 0.0, 1.0);
        let u_next = clamp((index.x + 2.5) / tex_size, 0.0, 1.0);
        let v_prev = clamp((index.y - 0.5) / tex_size, 0.0, 1.0);
        let v_next = clamp((index.y + 2.5) / tex_size, 0.0, 1.0);
        let hL0 = textureSampleLevel(heightmap_tex, heightmap_sampler, vec2<f32>(u_prev, v0), 0.0).r;
        let hL1 = textureSampleLevel(heightmap_tex, heightmap_sampler, vec2<f32>(u_prev, v1), 0.0).r;
        let hR0 = textureSampleLevel(heightmap_tex, heightmap_sampler, vec2<f32>(u_next, v0), 0.0).r;
        let hR1 = textureSampleLevel(heightmap_tex, heightmap_sampler, vec2<f32>(u_next, v1), 0.0).r;
        let hB0 = textureSampleLevel(heightmap_tex, heightmap_sampler, vec2<f32>(u0, v_prev), 0.0).r;
        let hB1 = textureSampleLevel(heightmap_tex, heightmap_sampler, vec2<f32>(u1, v_prev), 0.0).r;
        let hT0 = textureSampleLevel(heightmap_tex, heightmap_sampler, vec2<f32>(u0, v_next), 0.0).r;
        let hT1 = textureSampleLevel(heightmap_tex, heightmap_sampler, vec2<f32>(u1, v_next), 0.0).r;
        dh_dx = (mix(hR0, hR1, f.y) - mix(hL0, hL1, f.y)) * 0.5 + (h10 - h00) * 0.5;
        dh_dy = (mix(hT0, hT1, f.x) - mix(hB0, hB1, f.x)) * 0.5 + (h01 - h00) * 0.5;
    } else {
        dh_dx = mix(h10 - h00, h11 - h01, f.y);
        dh_dy = mix(h01 - h00, h11 - h10, f.x);
    }

    var normal = normalize(vec3<f32>(
        -dh_dx * depth_factor,
        dh_dy * depth_factor,
        1.0
    ));

    // Smooth opacity blend (h_center >= 0.003 is 100% opaque sand color)
    let empty_blend = clamp(h_center / 0.003, 0.0, 1.0);
    let sparkles_fade = clamp(1.0 - wetness / 0.3, 0.0, 1.0) * empty_blend;
    let sparkles_intensity = mix(2.0, 18.0, grain_size) * sparkles_fade;
    let sparkles_threshold = clamp(mix(0.998, 0.990, grain_size) + (1.0 - sparkles_fade) * 0.01, 0.990, 1.0);
    let sparkles_power = mix(500.0, 250.0, grain_size);
    let rim_mult = mix(0.40, 0.15, clamp(wetness, 0.0, 1.0));
    let roughness = clamp(mix(1.0 - 0.2 * grain_size, 0.05, wetness), 0.05, 1.0);
    let grain_fade = clamp(1.0 - wetness / 0.5, 0.0, 1.0);
    let grain_strength = mix(0.0, 0.55, grain_size) * grain_fade * empty_blend;
    let grain_scale = mix(3500.0, 300.0, grain_size);
    let is_metallic = 0.0;
    let is_moon_dust = 0.0;

    var dry_color = uniforms.sand_color.rgb;
    if (uniforms.color_mode > 0u) {
        dry_color = textureSampleLevel(colormap_tex, heightmap_sampler, uv, 0.0).rgb;
    } else {
        // Continuous color mapping for solid/preset colors based on wetness & grain_size
        let dry_sand = uniforms.sand_color.rgb;
        let coarse_sand = dry_sand * 0.95;
        let fine_powder = vec3<f32>(0.96, 0.96, 0.96);
        let moon_dust = vec3<f32>(0.35, 0.35, 0.35);

        // Mix dry states based on grain_size
        if (grain_size < 0.15) {
            let t = clamp((grain_size - 0.05) / 0.10, 0.0, 1.0);
            dry_color = mix(fine_powder, moon_dust, t);
        } else if (grain_size < 0.45) {
            let t = clamp((grain_size - 0.10) / 0.35, 0.0, 1.0);
            dry_color = mix(moon_dust, dry_sand, t);
        } else {
            let t = clamp((grain_size - 0.45) / 0.35, 0.0, 1.0);
            dry_color = mix(dry_sand, coarse_sand, t);
        }
    }

    let snow = mix(dry_color, vec3<f32>(0.98, 0.98, 1.0), 0.8);
    let kinetic_sand = dry_color * 0.8;
    let wet_sand = mix(dry_color * 0.55, vec3<f32>(0.5, 0.45, 0.4), 0.3);
    let oobleck = mix(dry_color * 0.8, vec3<f32>(0.75, 0.90, 0.30), 0.6);
    let buttercream = mix(dry_color * 0.9, vec3<f32>(0.95, 0.93, 0.88), 0.7);
    let yogurt = mix(dry_color * 0.9, vec3<f32>(0.96, 0.94, 0.88), 0.75);
    let oil = mix(dry_color * 0.4, vec3<f32>(0.60, 0.45, 0.15), 0.5);
    // Water gets TWO mixes, chosen by whether the user has picked a color scheme at all.
    //
    // `color_mode == 0` is the no-scheme-chosen state: `dry_color` is then derived from the
    // uniform sand color above, not from anything the user painted, so there is no painted
    // color to honour and water should simply be the deep blue it has always been. That blue
    // is a deliberate response to an earlier report that water did not look properly blue, so
    // the default must not be weakened.
    //
    // `color_mode > 0` means a scheme IS active and `dry_color` is the per-cell painted color.
    // The old 0.15/0.85 split gave it only ~2.25% weight in the final mix (0.15 * 0.15) — an
    // outlier far below every neighbouring material in this same wetness progression (oobleck
    // ~32%, oil ~20%, milk ~18%) — which is why liquid rendered as flat blue no matter what was
    // painted on it. 0.6/0.5 restores a comparable ~30% weight, but ONLY in this branch.
    let water_default = mix(dry_color * 0.15, vec3<f32>(0.03, 0.22, 0.58), 0.85);
    let water_painted = mix(dry_color * 0.6, vec3<f32>(0.03, 0.22, 0.58), 0.5);
    let water = select(water_default, water_painted, uniforms.color_mode > 0u);
    let milk = mix(dry_color * 0.9, vec3<f32>(0.95, 0.95, 0.93), 0.8);

    var mat_base_color = dry_color;

    // Blend dry color towards wet colors depending on wetness
    if (wetness < 0.05) {
        let t = clamp(wetness / 0.05, 0.0, 1.0);
        mat_base_color = mix(dry_color, snow, t);
    } else if (wetness < 0.20) {
        let t = clamp((wetness - 0.05) / 0.15, 0.0, 1.0);
        mat_base_color = mix(snow, kinetic_sand, t);
    } else if (wetness < 0.45) {
        let t = clamp((wetness - 0.20) / 0.25, 0.0, 1.0);
        mat_base_color = mix(kinetic_sand, wet_sand, t);
    } else if (wetness < 0.55) {
        let t = clamp((wetness - 0.45) / 0.10, 0.0, 1.0);
        mat_base_color = mix(wet_sand, oobleck, t);
    } else if (wetness < 0.70) {
        let t = clamp((wetness - 0.55) / 0.15, 0.0, 1.0);
        mat_base_color = mix(oobleck, buttercream, t);
    } else if (wetness < 0.75) {
        let t = clamp((wetness - 0.70) / 0.05, 0.0, 1.0);
        mat_base_color = mix(buttercream, yogurt, t);
    } else if (wetness < 0.85) {
        let t = clamp((wetness - 0.75) / 0.10, 0.0, 1.0);
        mat_base_color = mix(yogurt, oil, t);
    } else if (wetness < 0.90) {
        let t = clamp((wetness - 0.85) / 0.05, 0.0, 1.0);
        mat_base_color = mix(oil, water, t);
    } else if (wetness < 0.95) {
        let t = clamp((wetness - 0.90) / 0.05, 0.0, 1.0);
        mat_base_color = mix(water, milk, t);
    } else {
        let t = clamp((wetness - 0.95) / 0.05, 0.0, 1.0);
        mat_base_color = mix(milk, water, t);
    }

    // Empty sandbed reveals dark table bed
    mat_base_color = mix(vec3<f32>(0.05, 0.05, 0.06), mat_base_color, empty_blend);

    // 3. Perturb normal with micro-surface grain noise
    let grain_noise = hash(uv * grain_scale);
    let grain_noise_y = hash(uv * grain_scale + vec2<f32>(17.0, 43.0));
    var perturb = vec3<f32>(
        (grain_noise - 0.5) * grain_strength,
        (grain_noise_y - 0.5) * grain_strength,
        0.0
    );

    normal = normalize(normal + perturb);

    // 4. Lighting Mode evaluation
    let r_norm = clamp(dist / 0.46, 0.0, 1.0);
    let edge_factor = smoothstep(0.5, 1.0, r_norm);
    let radial_boost = 1.0 + 0.3 * edge_factor;

    // Determine the local light color based on LED mode and pixel angle
    var local_light_color = vec3<f32>(1.0, 0.95, 0.85); // Default warm white
    if (uniforms.led_mode == 0u) {
        local_light_color = uniforms.light_color.rgb;
    } else if (uniforms.led_mode == 1u) {
        let angle = atan2(-(uv.y - 0.5), uv.x - 0.5);
        let hue = fract(angle / (2.0 * PI) - uniforms.time * 0.05);
        local_light_color = hue_to_rgb(hue);
    } else if (uniforms.led_mode == 2u) {
        let hue = fract(uniforms.time * 0.03);
        local_light_color = hue_to_rgb(hue);
    } else if (uniforms.led_mode == 3u || uniforms.led_mode == 4u) {
        local_light_color = vec3<f32>(0.85, 0.90, 0.95); // Cool moonlight
    }

    var diffuse = vec3<f32>(0.0);
    var specular_reflect = vec3<f32>(0.0);
    var directional_sparkle = 0.0;

    // A. Directional Light component (Single direction, Overhead Moon, or Rainbow Moon)
    if (uniforms.led_mode == 0u || uniforms.led_mode == 3u || uniforms.led_mode == 4u) {
        let light_dir = select(normalize(uniforms.light_dir.xyz), vec3<f32>(0.0, 0.0, 1.0), uniforms.led_mode == 3u || uniforms.led_mode == 4u);
        
        // Power-wrapped diffuse to simulate multiple scattering of sand grains
        let diff_strength = pow(dot(normal, light_dir) * 0.5 + 0.5, 1.5);
        let diff_color = local_light_color * diff_strength * m_brightness;
        
        // Microfacet Sparkles for quartz highlights
        let half_vec = normalize(light_dir + view_dir);
        let sparkle_noise = hash(floor(uv * 4000.0));
        let m_sparkles_threshold = select(sparkles_threshold, 1.0, uniforms.led_mode == 3u || uniforms.led_mode == 4u); // disable sparkles in moon mode
        if (sparkle_noise > m_sparkles_threshold) {
            let dot_nh = max(dot(normal, half_vec), 0.0);
            let sparkle_intensity = pow(dot_nh, sparkles_power) * sparkles_intensity;
            directional_sparkle = step(0.8, sparkle_intensity) * (sparkle_noise - m_sparkles_threshold) * 50.0;
        }

        var shadow_factor = 1.0;
        if (uniforms.shadow_enabled == 1u) {
            let step_count = 32;
            let step_size = 0.0022;
            let uv_step = vec2<f32>(light_dir.x, -light_dir.y) * step_size;
            let h_step = (2.0 * light_dir.z * step_size) / Z_SCALE;
            
            var curr_uv = uv;
            var curr_h = h_center + 0.0010;
            
            for (var i = 0; i < step_count; i = i + 1) {
                curr_uv = curr_uv + uv_step;
                curr_h = curr_h + h_step;
                
                if (curr_uv.x < 0.0 || curr_uv.x > 1.0 || curr_uv.y < 0.0 || curr_uv.y > 1.0 || curr_h > 1.0) {
                    break;
                }
                
                let sample_h = textureSampleLevel(heightmap_tex, heightmap_sampler, curr_uv, 0.0).r;
                let depth = sample_h - curr_h;
                if (depth > 0.0) {
                    let inst_shadow = clamp(1.0 - depth * 3.5, 0.15, 1.0);
                    shadow_factor = min(shadow_factor, inst_shadow);
                    if (shadow_factor <= 0.15) {
                        break;
                    }
                }
            }
        }
        
        if (uniforms.marble_count > 0u) {
            apply_marble_shadow(uv, light_dir, uniforms.marbles[0].pos, uniforms.marbles[0].radius, uniforms.marbles[0].z_pos, &shadow_factor);
        }
        if (uniforms.marble_count > 1u) {
            apply_marble_shadow(uv, light_dir, uniforms.marbles[1].pos, uniforms.marbles[1].radius, uniforms.marbles[1].z_pos, &shadow_factor);
        }
        if (uniforms.marble_count > 2u) {
            apply_marble_shadow(uv, light_dir, uniforms.marbles[2].pos, uniforms.marbles[2].radius, uniforms.marbles[2].z_pos, &shadow_factor);
        }
        if (uniforms.marble_count > 3u) {
            apply_marble_shadow(uv, light_dir, uniforms.marbles[3].pos, uniforms.marbles[3].radius, uniforms.marbles[3].z_pos, &shadow_factor);
        }
        if (uniforms.marble_count > 4u) {
            apply_marble_shadow(uv, light_dir, uniforms.marbles[4].pos, uniforms.marbles[4].radius, uniforms.marbles[4].z_pos, &shadow_factor);
        }
        
        diffuse = diffuse + diff_color * shadow_factor * radial_boost;

        // Glossy Specular component for directional light
        if (roughness < 0.5) {
            let spec_factor = clamp((roughness - 0.1) / 0.4, 0.0, 1.0);
            let spec_power = mix(256.0, 16.0, spec_factor);
            let spec_int = mix(2.5, 0.5, spec_factor);
            
            let half_vec = normalize(light_dir + view_dir);
            let dot_nh = max(dot(normal, half_vec), 0.0);
            specular_reflect = specular_reflect + local_light_color * pow(dot_nh, spec_power) * spec_int * m_brightness;
        }
    }

    // B. Multi-LED Ring component (Rainbow Ring, Color Cycle, or Rainbow Moon)
    if (uniforms.led_mode == 1u || uniforms.led_mode == 2u || uniforms.led_mode == 4u) {
        let num_leds = 8;
        let step_size = 0.004;
        let step_count = 8;
        
        var diffuse_accum = vec3<f32>(0.0);
        var spec_accum = vec3<f32>(0.0);

        let spec_factor = clamp((roughness - 0.1) / 0.4, 0.0, 1.0);
        let spec_power = mix(256.0, 16.0, spec_factor);
        let spec_int = mix(2.5, 0.5, spec_factor);
        
        for (var i = 0; i < num_leds; i = i + 1) {
            let angle_led = f32(i) * (2.0 * PI / f32(num_leds)) + uniforms.time * 0.10;
            let l_dir = normalize(vec3<f32>(cos(angle_led), sin(angle_led), 0.06));
            
            var led_color = vec3<f32>(0.0);
            if (uniforms.led_mode == 1u || uniforms.led_mode == 4u) {
                let hue = fract(f32(i) / f32(num_leds) - uniforms.time * 0.05);
                led_color = hue_to_rgb(hue);
            } else {
                let hue = fract(uniforms.time * 0.03);
                led_color = hue_to_rgb(hue);
            }
            
            // Power-wrapped diffuse to simulate multiple scattering of sand grains
            let diff_strength = pow(dot(normal, l_dir) * 0.5 + 0.5, 1.5);
            
            // Microfacet Sparkles under this LED light
            let half_vec = normalize(l_dir + view_dir);
            let sparkle_noise = hash(floor(uv * 4000.0));
            var sp = 0.0;
            let led_sparkles_threshold = sparkles_threshold + 0.003;
            if (sparkle_noise > led_sparkles_threshold) {
                let dot_nh = max(dot(normal, half_vec), 0.0);
                let sparkle_intensity = pow(dot_nh, sparkles_power) * sparkles_intensity;
                sp = step(0.85, sparkle_intensity) * (sparkle_noise - led_sparkles_threshold) * 40.0;
            }
            
            var shadow_factor = 1.0;
            if (uniforms.shadow_enabled == 1u) {
                let uv_step = vec2<f32>(l_dir.x, -l_dir.y) * step_size;
                let h_step = (2.0 * l_dir.z * step_size) / Z_SCALE;
                
                var curr_uv = uv;
                var curr_h = h_center + 0.0010;
                
                for (var s = 0; s < step_count; s = s + 1) {
                    curr_uv = curr_uv + uv_step;
                    curr_h = curr_h + h_step;
                    
                    if (curr_uv.x < 0.0 || curr_uv.x > 1.0 || curr_uv.y < 0.0 || curr_uv.y > 1.0 || curr_h > 1.0) {
                        break;
                    }
                    
                    let sample_h = textureSampleLevel(heightmap_tex, heightmap_sampler, curr_uv, 0.0).r;
                    let depth = sample_h - curr_h;
                    if (depth > 0.0) {
                        let inst_shadow = clamp(1.0 - depth * 3.5, 0.15, 1.0);
                        shadow_factor = min(shadow_factor, inst_shadow);
                        if (shadow_factor <= 0.15) {
                            break;
                        }
                    }
                }
            }
            
            if (uniforms.marble_count > 0u) {
                apply_marble_shadow(uv, l_dir, uniforms.marbles[0].pos, uniforms.marbles[0].radius, uniforms.marbles[0].z_pos, &shadow_factor);
            }
            if (uniforms.marble_count > 1u) {
                apply_marble_shadow(uv, l_dir, uniforms.marbles[1].pos, uniforms.marbles[1].radius, uniforms.marbles[1].z_pos, &shadow_factor);
            }
            if (uniforms.marble_count > 2u) {
                apply_marble_shadow(uv, l_dir, uniforms.marbles[2].pos, uniforms.marbles[2].radius, uniforms.marbles[2].z_pos, &shadow_factor);
            }
            if (uniforms.marble_count > 3u) {
                apply_marble_shadow(uv, l_dir, uniforms.marbles[3].pos, uniforms.marbles[3].radius, uniforms.marbles[3].z_pos, &shadow_factor);
            }
            if (uniforms.marble_count > 4u) {
                apply_marble_shadow(uv, l_dir, uniforms.marbles[4].pos, uniforms.marbles[4].radius, uniforms.marbles[4].z_pos, &shadow_factor);
            }
            
            diffuse_accum = diffuse_accum + led_color * (diff_strength + sp * 2.0) * shadow_factor;

            // Specular contribution from this LED
            if (roughness < 0.5) {
                let spec_l_dir = normalize(vec3<f32>(cos(angle_led), sin(angle_led), 0.20));
                let spec_half_vec = normalize(spec_l_dir + view_dir);
                let spec_dot_nh = max(dot(normal, spec_half_vec), 0.0);
                spec_accum = spec_accum + led_color * pow(spec_dot_nh, spec_power) * spec_int;
            }
        }
        
        diffuse = diffuse + diffuse_accum * (uniforms.light_brightness / f32(num_leds)) * radial_boost;
        if (roughness < 0.5) {
            specular_reflect = specular_reflect + spec_accum * (uniforms.light_brightness / f32(num_leds));
        }
    }

    // Base sand color from presets with grain color variation locked to texel resolution (1024)
    // Grain noise locked to actual grid resolution to avoid sub-texel aliasing
    let color_grain = hash(floor(uv * uniforms.grid_size));
    let sand_base_color = mat_base_color * (1.0 + (color_grain - 0.5) * 0.025);

    // Warm ambient reflection for soft sand look (darker charcoal for Moon Dust)
    let ambient_base = mix(vec3<f32>(0.52, 0.52, 0.55), vec3<f32>(0.02, 0.02, 0.025), is_moon_dust);
    
    // Tint ambient light with local light color, increasing towards the edges
    // This simulates indirect light bounce from the nearby LED ring.
    let edge_tint_strength = edge_factor * 0.35; // up to 35% tinting near the edges
    let ambient_tint = mix(vec3<f32>(1.0), local_light_color, edge_tint_strength);
    let ambient_tinted = ambient_base * ambient_tint;
    
    // Procedural Ambient Occlusion based on local height depth relative to the flat bed (0.35)
    let ao = clamp(1.0 - max(0.35 - h_center, 0.0) * 1.8, 0.30, 1.0);
    let ambient = ambient_tinted * ao;
    
    // Fresnel Rim Light to simulate soft rim scattering
    let fresnel = pow(1.0 - max(dot(normal, view_dir), 0.0), 5.0);
    let rim_color = local_light_color * fresnel * rim_mult * m_brightness * radial_boost;
    
    // Combine shading: ambient + diffuse + rim light + sparkles
    let final_lighting = ambient + diffuse + rim_color + vec3<f32>(directional_sparkle * m_brightness);
    
    // Blend with dark table floor based on sand thickness (height)
    // Opacity fades smoothly to 0 for height < 0.005 so tiny residual specs are completely invisible
    let sand_opacity = smoothstep(0.005, 0.035, h_center);
    let table_color = vec3<f32>(0.02, 0.02, 0.03);
    
    var sand_shaded = vec3<f32>(0.0);
    if (is_metallic > 0.5) {
        // Metallic reflection: multiply reflect by base color
        sand_shaded = sand_base_color * final_lighting + specular_reflect * mat_base_color * sand_opacity;
    } else {
        // Dielectric reflection: additive specular
        sand_shaded = sand_base_color * final_lighting + specular_reflect * sand_opacity;
    }

    var final_color = mix(table_color, sand_shaded, sand_opacity);

    if (wetness >= 0.75) {
        let absorption = 1.0 - exp(-h_center * 12.0);
        let liquid_refracted = mix(table_color, mat_base_color, absorption);
        let liquid_factor = (wetness - 0.75) / 0.25;
        let liquid_shaded = mix(final_color, liquid_refracted + specular_reflect * sand_opacity, liquid_factor);
        final_color = mix(table_color, liquid_shaded, sand_opacity);
    }

    // Mass-distribution quantile lines (Sand-fall overlay). This point in the shader is only
    // reached once we're past the `in_casing` early-return above and the marble-hit return, so
    // a line drawn here is guaranteed to be inside the shape and never over the casing/LED ring,
    // and never on top of a marble. `quantile_count` is 0 whenever the feature is off (the
    // default) or the sim isn't in Sand-fall mode, so this costs nothing in the common case.
    if (uniforms.quantile_count > 0u) {
        let row_f = uv.y * uniforms.grid_size;
        let half_width_px = 1.0; // texel half-width — a soft ~2-texel-wide antialiased line
        for (var i = 0u; i < uniforms.quantile_count; i = i + 1u) {
            let target_row = uniforms.quantile_positions[i / 4u][i % 4u] * uniforms.grid_size;
            let d = abs(row_f - target_row);
            if (d < half_width_px) {
                // Recycled-glass green, and quieter than the cyan it replaces. These lines are a
                // reading aid laid over the artwork, so they should be legible without becoming
                // the brightest thing on screen — the old 0.85 alpha on a saturated cyan won every
                // staring contest with the sand underneath it.
                let line_alpha = smoothstep(half_width_px, 0.0, d) * 0.55;
                let line_color = vec3<f32>(0.561, 0.722, 0.631);
                final_color = mix(final_color, line_color, line_alpha);
            }
        }
    }

    // Block-simulation heat-map debug overlay. Same placement contract as the quantile lines
    // just above: past the `in_casing` early-return and the marble-hit return, so it only ever
    // tints actual sand/table, never the casing or a marble, and `heatmap_enabled == 0u` (the
    // default) skips this whole block, costing nothing.
    if (uniforms.heatmap_enabled != 0u) {
        // The block grid is a fixed 64x64 regardless of `uniforms.grid_size` (see
        // `block_heat_tex`'s binding comment -- HEAT_GRID_SIZE in sandart-render/src/lib.rs, was
        // 32 before the LOD block became grid_size/64 instead of grid_size/32), so the block a
        // fragment falls in is just its UV scaled directly by 64, not by the cell-resolution
        // `grid_size`.
        let heat_block_coord = vec2<i32>(vec2<f32>(uv.x, uv.y) * 64.0);
        let heat = textureLoad(block_heat_tex, clamp(heat_block_coord, vec2<i32>(0), vec2<i32>(63)), 0).r;

        // Cold -> hot ramp: dark blue, through teal-green, to a bright orange-red at 1.0. Picked
        // over a single-hue (e.g. all-red) ramp because this overlay sits on top of BOTH the
        // pale tan/cream sand AND the dark casing background in the same frame: a ramp that
        // stayed dark at its cold end would disappear into the casing, and one that stayed
        // bright at its hot end would wash out over pale sand. Blue-to-red moves in both
        // lightness and hue at once, so the cold end reads against light sand (dark blue on
        // cream) and the hot end reads against the dark casing (bright red-orange on near-black)
        // instead of either extreme depending on luck to have contrast.
        let cold = vec3<f32>(0.06, 0.10, 0.45);
        let mid = vec3<f32>(0.10, 0.65, 0.55);
        let hot = vec3<f32>(1.0, 0.30, 0.05);
        var heat_color: vec3<f32>;
        if (heat < 0.5) {
            heat_color = mix(cold, mid, heat * 2.0);
        } else {
            heat_color = mix(mid, hot, (heat - 0.5) * 2.0);
        }
        // A flat, fairly strong blend rather than an intensity-scaled one: this is a debug
        // instrument meant to be read at a glance, including over a completely cold (heat = 0.0)
        // block, which needs to visibly differ from "no overlay at all" or the coldest blocks
        // would be indistinguishable from the overlay being off.
        final_color = mix(final_color, heat_color, 0.55);
    }

    // Per-cell pressure-field debug overlay. Same placement contract as the two overlays above:
    // past the `in_casing` early-return and the marble-hit return, so it only ever tints actual
    // sand/table, and `pressure_heatmap_enabled == 0u` (the default) skips this whole block,
    // costing nothing.
    if (uniforms.pressure_heatmap_enabled != 0u) {
        // Unlike `heat_block_coord` above, this texture IS sized `grid_size` x `grid_size` (one
        // texel per simulation cell, not per 64x64 LOD block -- see `pressure_heat_tex`'s binding
        // comment), so the coordinate is the fragment's cell index, same math as `mask_coord`
        // near the top of this function.
        let grid_size_i2 = i32(uniforms.grid_size);
        let pressure_coord = vec2<i32>(i32(uv.x * uniforms.grid_size), i32(uv.y * uniforms.grid_size));
        let pressure = textureLoad(pressure_heat_tex, clamp(pressure_coord, vec2<i32>(0), vec2<i32>(grid_size_i2 - 1)), 0).r;

        // Deep violet -> hot magenta -> pale warm yellow. Deliberately a different hue path from
        // the block heat-map's blue/teal/orange-red above (270deg->320deg->50deg here vs.
        // 230deg->160deg->20deg there) so the two overlays are never confusable mid-comparison
        // when flipping between them -- exactly the use case this instrument exists for (A/B'ing
        // the "Fresh pressure field" toggle). Like that ramp it moves in both lightness and hue at
        // once (dark, saturated violet up to a pale, warm yellow), so the cold end still reads
        // against pale sand and the hot end still reads against the near-black casing/table,
        // rather than either extreme depending on luck for contrast. `pressure` already IS the
        // log-compressed, normalised [0,1] value (see `pressure_field_texels`'s doc comment for
        // the scaling rationale) -- this ramp only maps that scalar to colour, it does no further
        // compression itself.
        let p_cold = vec3<f32>(0.20, 0.05, 0.35);
        let p_mid = vec3<f32>(0.85, 0.10, 0.55);
        let p_hot = vec3<f32>(1.0, 0.92, 0.55);
        var pressure_color: vec3<f32>;
        if (pressure < 0.5) {
            pressure_color = mix(p_cold, p_mid, pressure * 2.0);
        } else {
            pressure_color = mix(p_mid, p_hot, (pressure - 0.5) * 2.0);
        }
        // Same flat, strong blend as the block heat-map overlay, for the same reason: a debug
        // instrument meant to be read at a glance, including over a genuinely-zero-pressure void
        // (pressure = 0.0), which needs to visibly differ from "no overlay at all".
        final_color = mix(final_color, pressure_color, 0.55);
    }

    // Coarse-level `eta` (hydraulic head) debug overlay. Same placement contract as the three
    // overlays above: past the `in_casing` early-return and the marble-hit return, and
    // `coarse_eta_enabled == 0u` (the default) skips this whole block, costing nothing. This
    // answers "is the coarse level flat across a connected body" -- the whole point of the
    // instrument -- so it must show STRUCTURE (a resting pool as one flat hue, a U-tube's two
    // arms as two visibly different hues until they equalise), not a washed-out fixed scale.
    if (uniforms.coarse_eta_enabled != 0u) {
        // The coarse grid is a fixed 64x64 regardless of `uniforms.grid_size`, same block-coord
        // math as `block_heat_tex` above -- the coarse tile IS the LOD block (`t = grid_size /
        // 64`), not a texel-per-cell field like `pressure_heat_tex`.
        let eta_block_coord = vec2<i32>(vec2<f32>(uv.x, uv.y) * 64.0);
        let eta = textureLoad(coarse_eta_tex, clamp(eta_block_coord, vec2<i32>(0), vec2<i32>(63)), 0).r;

        // Deep forest green -> mid green -> pale warm cream. A third, distinct hue arc (roughly
        // 150deg -> 125deg -> 60deg) from BOTH sequential ramps already in this shader --
        // `block_heat_tex`'s blue/teal/orange-red (230deg->160deg->20deg) and
        // `pressure_heat_tex`'s violet/magenta/yellow (270deg->320deg->50deg) -- so this overlay
        // is never confusable with either when flipping between debug instruments. Like those two
        // it moves in both lightness and hue at once (dark, saturated green up to a pale, warm
        // cream), so the low end still reads against pale sand and the high end still reads
        // against the near-black casing/table.
        let eta_cold = vec3<f32>(0.04, 0.22, 0.10);
        let eta_mid = vec3<f32>(0.18, 0.55, 0.22);
        let eta_hot = vec3<f32>(0.92, 0.95, 0.68);
        var eta_color: vec3<f32>;
        if (eta < 0.5) {
            eta_color = mix(eta_cold, eta_mid, eta * 2.0);
        } else {
            eta_color = mix(eta_mid, eta_hot, (eta - 0.5) * 2.0);
        }
        // Same flat, strong blend as the other overlays, for the same reason: a debug instrument
        // meant to be read at a glance, including over dry land / the exterior (eta = 0.0 there,
        // see `coarse_eta_texels`'s doc comment), which needs to visibly differ from "no overlay
        // at all".
        final_color = mix(final_color, eta_color, 0.55);
    }

    // Coarse-fine disagreement (`Delta`) debug overlay. Same placement contract as the overlays
    // above; `coarse_delta_enabled == 0u` (the default) skips this whole block. This answers "how
    // much work has the fine level not done yet, and in which direction" -- the future
    // scheduler's clock signal -- so unlike every other overlay in this shader it is DIVERGING,
    // not sequential: zero disagreement is a real, meaningful value that must land on a fixed,
    // recognisable point of the ramp, not an arbitrary end of it.
    if (uniforms.coarse_delta_enabled != 0u) {
        let delta_block_coord = vec2<i32>(vec2<f32>(uv.x, uv.y) * 64.0);
        let delta = textureLoad(coarse_delta_tex, clamp(delta_block_coord, vec2<i32>(0), vec2<i32>(63)), 0).r;

        // Vivid blue (coarse behind fine, M < A) -> neutral mid-grey (agreement, delta = 0, at
        // EXACTLY 0.5) -> vivid orange (coarse ahead of fine, M > A). Structurally different from
        // every sequential ramp in this shader (dark-to-pale) on purpose: a diverging quantity's
        // zero must be visually distinct from BOTH its extremes, which a monotone-lightness ramp
        // cannot do (its midpoint is just "medium", indistinguishable from a small nonzero
        // reading). The neutral sits at MID lightness rather than white or near-black specifically
        // so it keeps contrast against both the pale sand and the dark casing at once -- an
        // all-white or all-black "zero" would vanish into one of the two backgrounds this overlay
        // has to render over.
        let delta_neg = vec3<f32>(0.15, 0.35, 0.85);
        let delta_zero = vec3<f32>(0.50, 0.50, 0.53);
        let delta_pos = vec3<f32>(0.95, 0.55, 0.10);
        var delta_color: vec3<f32>;
        if (delta < 0.5) {
            delta_color = mix(delta_neg, delta_zero, delta * 2.0);
        } else {
            delta_color = mix(delta_zero, delta_pos, (delta - 0.5) * 2.0);
        }
        // Same flat, strong blend as the other overlays. Dry land / the exterior reads as 0.5,
        // i.e. exactly the neutral "no disagreement" colour (see `coarse_delta_texels`'s doc
        // comment) -- NOT one of the saturated extremes, which would otherwise falsely flag empty
        // space as the tile most starved by the coarse level.
        final_color = mix(final_color, delta_color, 0.55);
    }

    return vec4<f32>(final_color, 1.0);
}
