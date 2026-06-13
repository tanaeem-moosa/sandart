@group(0) @binding(0) var heightmap_tex: texture_2d<f32>;
@group(0) @binding(1) var heightmap_sampler: sampler;

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
    padding1: u32,
    padding2: u32,
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
    let tex_size = 1024.0;
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
    
    // 1. Draw outer frame and emissive LED ring
    if (dist >= 0.46) {
        if (dist >= 0.46 && dist < 0.475) {
            // Emissive LED strip channel
            if (uniforms.led_mode == 0u) {
                // Single Light Mode: Draw a single glowing LED spot at the angle of uniforms.light_dir
                let angle_light = atan2(uniforms.light_dir.y, uniforms.light_dir.x);
                let led_center = vec2<f32>(
                    cos(angle_light) * 0.468 + 0.5,
                    -sin(angle_light) * 0.468 + 0.5
                );
                let d_to_led = distance(uv, led_center);
                var led_glow = vec3<f32>(0.08, 0.08, 0.10);
                if (d_to_led < 0.02) {
                    let intensity = smoothstep(0.02, 0.0, d_to_led);
                    led_glow = led_glow + uniforms.light_color.rgb * intensity * 1.5 * uniforms.light_brightness;
                }
                return vec4<f32>(led_glow, 1.0);
            } else if (uniforms.led_mode == 1u) {
                // Rainbow Ring Mode: Continuous rotating rainbow ring
                let angle = atan2(-(uv.y - 0.5), uv.x - 0.5);
                let hue = fract(angle / (2.0 * PI) - uniforms.time * 0.05);
                let led_color = hue_to_rgb(hue);
                return vec4<f32>(led_color * 1.5 * uniforms.light_brightness, 1.0);
            } else {
                // Color Cycle Mode: Continuous single color cycling ring
                let hue = fract(uniforms.time * 0.03);
                let led_color = hue_to_rgb(hue);
                return vec4<f32>(led_color * 1.5 * uniforms.light_brightness, 1.0);
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
        
        if (uniforms.led_mode == 0u) {
            let light_dir = normalize(uniforms.light_dir.xyz);
            let diff = max(dot(sphere_normal, light_dir), 0.0);
            sphere_diffuse = vec3<f32>(0.1) * diff * uniforms.light_brightness;
            
            let reflect_dir = reflect(-light_dir, sphere_normal);
            let spec = pow(max(dot(reflect_dir, view_dir), 0.0), 128.0);
            sphere_specular = uniforms.light_color.rgb * spec * 2.0 * uniforms.light_brightness;
        } else {
            // Compute diffuse from the 8 virtual lights
            let num_leds = 8;
            for (var i = 0; i < num_leds; i = i + 1) {
                let angle_led = f32(i) * (2.0 * PI / f32(num_leds)) + uniforms.time * 0.10;
                let l_dir = normalize(vec3<f32>(cos(angle_led), sin(angle_led), 0.06));
                
                var led_color = vec3<f32>(0.0);
                if (uniforms.led_mode == 1u) {
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
                if (uniforms.led_mode == 1u) {
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
    let depth_factor = 28.0;
    let tex_size = 1024.0;
    let texel_coords = uv * tex_size - 0.5;
    let index = floor(texel_coords);
    let f = fract(texel_coords);

    let u0 = clamp((index.x + 0.5) / tex_size, 0.0, 1.0);
    let v0 = clamp((index.y + 0.5) / tex_size, 0.0, 1.0);
    let u1 = clamp((index.x + 1.5) / tex_size, 0.0, 1.0);
    let v1 = clamp((index.y + 1.5) / tex_size, 0.0, 1.0);

    let h00 = textureSampleLevel(heightmap_tex, heightmap_sampler, vec2<f32>(u0, v0), 0.0).r;
    let h10 = textureSampleLevel(heightmap_tex, heightmap_sampler, vec2<f32>(u1, v0), 0.0).r;
    let h01 = textureSampleLevel(heightmap_tex, heightmap_sampler, vec2<f32>(u0, v1), 0.0).r;
    let h11 = textureSampleLevel(heightmap_tex, heightmap_sampler, vec2<f32>(u1, v1), 0.0).r;

    let h_center = mix(mix(h00, h10, f.x), mix(h01, h11, f.x), f.y);
    let dh_dx = mix(h10 - h00, h11 - h01, f.y);
    let dh_dy = mix(h01 - h00, h11 - h10, f.x);

    var normal = normalize(vec3<f32>(
        -dh_dx * depth_factor,
        dh_dy * depth_factor,
        1.0
    ));

    // 2. Define material presets and grain configurations
    var mat_base_color = uniforms.sand_color.rgb;
    var sparkles_threshold = 0.996;
    var sparkles_intensity = 8.0;
    var sparkles_power = 500.0;
    var rim_mult = 0.45;
    var roughness = 0.9;
    var is_metallic = 0.0;
    var is_moon_dust = 0.0;
    var grain_scale = 1500.0;
    var grain_strength = 0.28;

    if (uniforms.material_mode == 0u) { // ButterCream
        mat_base_color = vec3<f32>(0.95, 0.93, 0.88);
        sparkles_threshold = 1.0;
        sparkles_intensity = 0.0;
        rim_mult = 0.10;
        roughness = 0.8;
        grain_scale = 2200.0;
        grain_strength = 0.08;
    } else if (uniforms.material_mode == 2u) { // Snow
        mat_base_color = vec3<f32>(0.98, 0.98, 1.0);
        sparkles_threshold = 0.990;
        sparkles_intensity = 20.0;
        sparkles_power = 400.0;
        rim_mult = 0.90;
        roughness = 0.6;
        grain_scale = 1200.0;
        grain_strength = 0.38;
    } else if (uniforms.material_mode == 3u) { // KineticSand
        mat_base_color = vec3<f32>(0.85, 0.82, 0.77);
        sparkles_threshold = 1.0;
        sparkles_intensity = 0.0;
        rim_mult = 0.20;
        roughness = 1.0;
        grain_scale = 900.0;
        grain_strength = 0.42;
    } else if (uniforms.material_mode == 4u) { // WetSand
        mat_base_color = vec3<f32>(0.68, 0.62, 0.53);
        sparkles_threshold = 0.999;
        sparkles_intensity = 1.0;
        rim_mult = 0.10;
        roughness = 0.3;
        grain_scale = 1600.0;
        grain_strength = 0.15;
    } else if (uniforms.material_mode == 5u) { // FinePowder
        mat_base_color = vec3<f32>(0.96, 0.96, 0.96);
        sparkles_threshold = 1.0;
        sparkles_intensity = 0.0;
        rim_mult = 0.15;
        roughness = 1.0;
        grain_scale = 3000.0;
        grain_strength = 0.05;
    } else if (uniforms.material_mode == 6u) { // Oobleck
        mat_base_color = vec3<f32>(0.75, 0.90, 0.30);
        sparkles_threshold = 1.0;
        sparkles_intensity = 0.0;
        rim_mult = 0.60;
        roughness = 0.15;
        grain_scale = 1.0;
        grain_strength = 0.00;
    } else if (uniforms.material_mode == 7u) { // MoonDust
        mat_base_color = vec3<f32>(0.35, 0.35, 0.35);
        sparkles_threshold = 0.997;
        sparkles_intensity = 4.0;
        sparkles_power = 450.0;
        rim_mult = 0.05;
        roughness = 0.95;
        is_moon_dust = 1.0;
        grain_scale = 1400.0;
        grain_strength = 0.32;
    } else if (uniforms.material_mode == 8u) { // IronFilings
        mat_base_color = vec3<f32>(0.20, 0.20, 0.22);
        sparkles_threshold = 0.992;
        sparkles_intensity = 12.0;
        sparkles_power = 450.0;
        rim_mult = 0.20;
        roughness = 0.4;
        is_metallic = 1.0;
        grain_scale = 1000.0;
        grain_strength = 0.35;
    }

    // 3. Perturb normal with micro-surface grain noise
    let grain_noise = hash(uv * grain_scale);
    let grain_noise_y = hash(uv * grain_scale + vec2<f32>(17.0, 43.0));
    var perturb = vec3<f32>(
        (grain_noise - 0.5) * grain_strength,
        (grain_noise_y - 0.5) * grain_strength,
        0.0
    );

    // Apply procedural magnetic spike deformation for Iron Filings
    if (uniforms.material_mode == 8u) {
        var mag_offset = vec2<f32>(0.0, 0.0);
        for (var j = 0u; j < uniforms.marble_count; j = j + 1u) {
            let m_pos = uniforms.marbles[j].pos;
            let m_uv = vec2<f32>(m_pos.x * 0.5 + 0.5, -m_pos.y * 0.5 + 0.5);
            let to_m = uv - m_uv;
            let dist = length(to_m);
            if (dist < 0.22) {
                let weight = clamp((0.22 - dist) / 0.22, 0.0, 1.0);
                let w_steep = weight * weight * weight;
                
                // Concentric magnetic ripples
                let conc = cos(dist * 2.0 * PI / 0.015);
                
                // Radial spike needles
                let angle = atan2(to_m.y, to_m.x);
                let rad = cos(angle * 28.0);
                
                let dir = to_m / (dist + 0.0001);
                let perp = vec2<f32>(-dir.y, dir.x);
                
                // Perturb normal along radial field lines and transverse spikes
                mag_offset = mag_offset + dir * conc * w_steep * 0.55 + perp * rad * conc * w_steep * 0.40;
            }
        }
        perturb = perturb + vec3<f32>(mag_offset.x, mag_offset.y, 0.0);
    }

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
    }

    var diffuse = vec3<f32>(0.0);
    var specular = vec3<f32>(0.0);
    var directional_sparkle = 0.0;

    if (uniforms.led_mode == 0u) {
        // Single Directional Light mode
        let light_dir = normalize(uniforms.light_dir.xyz);
        
        // Power-wrapped diffuse to simulate multiple scattering of sand grains
        let diff_strength = pow(dot(normal, light_dir) * 0.5 + 0.5, 1.5);
        let diff_color = local_light_color * diff_strength * uniforms.light_brightness;
        
        // Microfacet Sparkles for quartz highlights
        let half_vec = normalize(light_dir + view_dir);
        let sparkle_noise = hash(floor(uv * 4000.0));
        if (sparkle_noise > sparkles_threshold) {
            let dot_nh = max(dot(normal, half_vec), 0.0);
            let sparkle_intensity = pow(dot_nh, sparkles_power) * sparkles_intensity;
            directional_sparkle = step(0.8, sparkle_intensity) * (sparkle_noise - sparkles_threshold) * 50.0;
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
        
        diffuse = diff_color * shadow_factor * radial_boost;
    } else {
        // Rainbow LED Ring Mode
        let num_leds = 8;
        let step_size = 0.004;
        let step_count = 8;
        
        var diffuse_accum = vec3<f32>(0.0);
        
        for (var i = 0; i < num_leds; i = i + 1) {
            let angle_led = f32(i) * (2.0 * PI / f32(num_leds)) + uniforms.time * 0.10;
            let l_dir = normalize(vec3<f32>(cos(angle_led), sin(angle_led), 0.06));
            
            var led_color = vec3<f32>(0.0);
            if (uniforms.led_mode == 1u) {
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
        }
        
        diffuse = diffuse_accum * (uniforms.light_brightness / f32(num_leds)) * radial_boost;
    }

    // B. Glossy Specular Reflection for wet/liquid/metallic materials
    var specular_reflect = vec3<f32>(0.0);
    if (roughness < 0.5) {
        let spec_power = mix(256.0, 16.0, (roughness - 0.1) / 0.4);
        let spec_int = mix(2.5, 0.5, (roughness - 0.1) / 0.4);
        
        if (uniforms.led_mode == 0u) {
            let light_dir = normalize(uniforms.light_dir.xyz);
            let half_vec = normalize(light_dir + view_dir);
            let dot_nh = max(dot(normal, half_vec), 0.0);
            specular_reflect = local_light_color * pow(dot_nh, spec_power) * spec_int * uniforms.light_brightness;
        } else {
            let num_leds = 8;
            var spec_accum = vec3<f32>(0.0);
            for (var i = 0; i < num_leds; i = i + 1) {
                let angle_led = f32(i) * (2.0 * PI / f32(num_leds)) + uniforms.time * 0.10;
                let l_dir = normalize(vec3<f32>(cos(angle_led), sin(angle_led), 0.20));
                let half_vec = normalize(l_dir + view_dir);
                let dot_nh = max(dot(normal, half_vec), 0.0);
                
                var led_color = vec3<f32>(0.0);
                if (uniforms.led_mode == 1u) {
                    let hue = fract(f32(i) / f32(num_leds) - uniforms.time * 0.05);
                    led_color = hue_to_rgb(hue);
                } else {
                    let hue = fract(uniforms.time * 0.03);
                    led_color = hue_to_rgb(hue);
                }
                spec_accum = spec_accum + led_color * pow(dot_nh, spec_power) * spec_int;
            }
            specular_reflect = spec_accum * (uniforms.light_brightness / f32(num_leds));
        }
    }

    // Base sand color from presets with grain color variation locked to texel resolution (1024)
    let color_grain = hash(floor(uv * 1024.0));
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
    let rim_color = local_light_color * fresnel * rim_mult * uniforms.light_brightness * radial_boost;
    
    // Combine shading: ambient + diffuse + rim light + sparkles
    let final_lighting = ambient + diffuse + rim_color + vec3<f32>(directional_sparkle * uniforms.light_brightness);
    
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
    let final_color = mix(table_color, sand_shaded, sand_opacity);

    return vec4<f32>(final_color, 1.0);
}
