@group(0) @binding(0) var heightmap_tex: texture_2d<f32>;
@group(0) @binding(1) var heightmap_sampler: sampler;
@group(0) @binding(4) var colormap_tex: texture_2d<f32>;

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
    marbles: array<MarbleUniform, 5>,
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
    let tex_size = 512.0;
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
    
    // Determine casing and LED channel based on the shape
    var in_casing = false;
    var in_led = false;
    var led_center = vec2<f32>(0.5, 0.5);
    
    let angle_light = atan2(uniforms.light_dir.y, uniforms.light_dir.x);
    let dir_light = vec2<f32>(cos(angle_light), -sin(angle_light));

    if (uniforms.sandbox_shape == 1u) { // Square
        let d_max = max(abs(uv.x - 0.5), abs(uv.y - 0.5));
        if (d_max >= 0.46) {
            in_casing = true;
            if (d_max < 0.475) {
                in_led = true;
            }
        }
        // Project light dir onto square perimeter (half-width 0.468)
        let scale = 0.468 / max(abs(dir_light.x), abs(dir_light.y));
        led_center = dir_light * scale + 0.5;
    } else if (uniforms.sandbox_shape == 2u) { // Oval
        let u = uv.x - 0.5;
        let v = uv.y - 0.5;
        let d_oval = sqrt((u * u) / (0.46 * 0.46) + (v * v) / (0.30 * 0.30));
        if (d_oval >= 1.0) {
            in_casing = true;
            if (d_oval < 1.032) {
                in_led = true;
            }
        }
        // Project light dir onto ellipse perimeter (a = 0.468, b = 0.305)
        let scale = 1.0 / sqrt((dir_light.x * dir_light.x) / (0.468 * 0.468) + (dir_light.y * dir_light.y) / (0.305 * 0.305));
        led_center = dir_light * scale + 0.5;
    } else if (uniforms.sandbox_shape == 3u) { // Hourglass
        let u = uv.x - 0.5;
        let v = uv.y - 0.5;
        let chamber_r = 0.28;
        let chamber_offset = 0.32;
        let neck_hw = 0.04;

        let d_upper = sqrt(u * u + (v + chamber_offset) * (v + chamber_offset));
        let d_lower = sqrt(u * u + (v - chamber_offset) * (v - chamber_offset));

        let in_neck_region = abs(u) < neck_hw && abs(v) < chamber_offset;
        let inside = (d_upper < chamber_r) || (d_lower < chamber_r) || in_neck_region;

        if (!inside) {
            in_casing = true;
            let in_upper_led = d_upper >= chamber_r && d_upper < (chamber_r + 0.015);
            let in_lower_led = d_lower >= chamber_r && d_lower < (chamber_r + 0.015);
            let in_neck_led = abs(u) >= neck_hw && abs(u) < (neck_hw + 0.015) && abs(v) < chamber_offset;

            let is_near_upper_circle = in_upper_led && (v + chamber_offset < 0.0 || abs(u) >= neck_hw);
            let is_near_lower_circle = in_lower_led && (v - chamber_offset > 0.0 || abs(u) >= neck_hw);
            let is_near_neck_wall = in_neck_led;

            if (is_near_upper_circle || is_near_lower_circle || is_near_neck_wall) {
                in_led = true;
            }
        }
        led_center = dir_light * 0.468 + 0.5;
    } else { // Circle (0u)
        let d_circle = distance(uv, vec2<f32>(0.5, 0.5));
        if (d_circle >= 0.46) {
            in_casing = true;
            if (d_circle < 0.475) {
                in_led = true;
            }
        }
        led_center = dir_light * 0.468 + 0.5;
    }

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
            // Outer casing
            return vec4<f32>(0.07, 0.07, 0.08, 1.0);
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
    let tex_size = 512.0;
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
    let depth_factor = 28.0;
    let is_water = step(0.85, wetness);
    if (is_water > 0.5) {
        // 3-tap Sobel over 2 texels for smoother water wave normals
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
        // Weighted Sobel: center pair weighted 2x
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

    // 2. Define material presets and grain configurations using continuous property mapping
    let sparkles_fade = clamp(1.0 - wetness / 0.3, 0.0, 1.0);
    let sparkles_intensity = mix(2.0, 18.0, grain_size) * sparkles_fade;
    let sparkles_threshold = clamp(mix(0.998, 0.990, grain_size) + (1.0 - sparkles_fade) * 0.01, 0.990, 1.0);
    let sparkles_power = mix(500.0, 250.0, grain_size);
    let rim_mult = mix(0.40, 0.15, clamp(wetness, 0.0, 1.0));
    let roughness = clamp(mix(1.0 - 0.2 * grain_size, 0.05, wetness), 0.05, 1.0);
    let grain_fade = clamp(1.0 - wetness / 0.5, 0.0, 1.0);
    let grain_strength = mix(0.0, 0.55, grain_size) * grain_fade;
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
    // Rich, deep blue water — more saturated and vivid
    let water = mix(dry_color * 0.15, vec3<f32>(0.03, 0.22, 0.58), 0.85);
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
    // Grain noise locked to actual grid resolution (512) to avoid sub-texel aliasing
    let color_grain = hash(floor(uv * 512.0));
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
    
    var sand_shaded = vec3<f32>(0.0);
    if (is_metallic > 0.5) {
        // Metallic reflection: multiply reflect by base color
        sand_shaded = sand_base_color * final_lighting + specular_reflect * mat_base_color;
    } else {
        // Dielectric reflection: additive specular
        sand_shaded = sand_base_color * final_lighting + specular_reflect;
    }

    // Blend with dark table floor based on sand thickness (height)
    // Opacity rises quickly so a thin sand layer (>= 0.05 height) is fully opaque sand.
    let sand_opacity = smoothstep(0.0, 0.05, h_center);
    let table_color = vec3<f32>(0.02, 0.02, 0.03);
    var final_color = mix(table_color, sand_shaded, sand_opacity);

    if (wetness >= 0.75) {
        let absorption = 1.0 - exp(-h_center * 12.0);
        let liquid_refracted = mix(table_color, mat_base_color, absorption);
        let liquid_factor = (wetness - 0.75) / 0.25;
        final_color = mix(final_color, liquid_refracted + specular_reflect, liquid_factor);
    }

    return vec4<f32>(final_color, 1.0);
}
