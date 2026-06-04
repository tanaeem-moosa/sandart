@group(0) @binding(0) var heightmap_tex: texture_2d<f32>;
@group(0) @binding(1) var heightmap_sampler: sampler;

const PI: f32 = 3.14159265359;

struct LightingUniforms {
    light_dir: vec4<f32>,
    light_color: vec4<f32>,
    sand_color: vec4<f32>,
    light_brightness: f32,
    shadow_enabled: u32,
    led_mode: u32,
    time: f32,
    marble_pos: vec2<f32>,
    marble_radius: f32,
    material_mode: u32,
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

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    
    // Convert pos to UV matching original mapping:
    out.uv = vec2<f32>(
        in.position.x * 0.5 + 0.5,
        -in.position.y * 0.5 + 0.5
    );
    
    // Sample heightmap texture
    let height = textureSampleLevel(heightmap_tex, heightmap_sampler, out.uv, 0.0).r;
    
    // Z displacement amplitude
    let z_scale = 0.018;
    
    // Construct 3D world position
    out.world_pos = vec3<f32>(in.position.x, in.position.y, height * z_scale);
    
    // Project position
    out.position = camera.view_proj * vec4<f32>(out.world_pos, 1.0);
    
    return out;
}

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
    
    // Sample height at marble position to find sphere Z center
    let marble_uv = vec2<f32>(
        uniforms.marble_pos.x * 0.5 + 0.5,
        -uniforms.marble_pos.y * 0.5 + 0.5
    );
    let h_marble = textureSampleLevel(heightmap_tex, heightmap_sampler, marble_uv, 0.0).r;
    let S = vec3<f32>(uniforms.marble_pos.x, uniforms.marble_pos.y, h_marble * 0.018);
    let R = uniforms.marble_radius;
    let marble_r_uv = uniforms.marble_radius * 0.5; // for shadow math compat
    
    let V = C - S;
    let b = dot(V, D);
    let c = dot(V, V) - R * R;
    let disc = b * b - c;
    
    var hit_marble = false;
    var sphere_normal = vec3<f32>(0.0, 0.0, 1.0);
    var view_dir = vec3<f32>(0.0, 0.0, 1.0);
    
    if (disc >= 0.0) {
        let t = -b - sqrt(disc);
        let dist_to_sand = distance(C, P);
        if (t > 0.0 && t < dist_to_sand) {
            hit_marble = true;
            let I = C + t * D;
            sphere_normal = normalize(I - S);
            view_dir = -D;
        }
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
                let l_dir = normalize(vec3<f32>(cos(angle_led), sin(angle_led), 0.20));
                
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
    let texel_size = 1.0 / 1024.0;
    let h_center = textureSampleLevel(heightmap_tex, heightmap_sampler, uv, 0.0).r;
    let h_left   = textureSampleLevel(heightmap_tex, heightmap_sampler, uv + vec2<f32>(-texel_size, 0.0), 0.0).r;
    let h_right  = textureSampleLevel(heightmap_tex, heightmap_sampler, uv + vec2<f32>(texel_size, 0.0), 0.0).r;
    let h_up     = textureSampleLevel(heightmap_tex, heightmap_sampler, uv + vec2<f32>(0.0, -texel_size), 0.0).r;
    let h_down   = textureSampleLevel(heightmap_tex, heightmap_sampler, uv + vec2<f32>(0.0, texel_size), 0.0).r;

    // Normal tilting scale (high factor creates visual depth)
    let depth_factor = 28.0;
    var normal = normalize(vec3<f32>(
        (h_left - h_right) * depth_factor,
        (h_down - h_up) * depth_factor,
        1.0
    ));

    // 2. Perturb normal with micro-surface grain noise (larger perturbation for stronger sparkling glimmers)
    let noise_scale = 1500.0;
    let grain_noise = hash(uv * noise_scale);
    let grain_noise_y = hash(uv * noise_scale + vec2<f32>(17.0, 43.0));
    normal = normalize(normal + vec3<f32>(
        (grain_noise - 0.5) * 0.12,
        (grain_noise_y - 0.5) * 0.12,
        0.0
    ));

    // 3. Lighting Mode evaluation
    var diffuse = vec3<f32>(0.0);
    var specular = vec3<f32>(0.0);

    if (uniforms.led_mode == 0u) {
        // Single Directional Light mode
        let light_dir = normalize(uniforms.light_dir.xyz);
        let view_dir = normalize(camera.camera_pos.xyz - world_pos);
        
        let diff_strength = max(dot(normal, light_dir), 0.0);
        let diff_color = uniforms.light_color.rgb * diff_strength * uniforms.light_brightness;
        
        let spec_color = vec3<f32>(0.0);
        
        var shadow_factor = 1.0;
        if (uniforms.shadow_enabled == 1u) {
            let step_count = 32;
            let z_scale = 0.015;
            let step_size = 0.0022;
            let uv_step = vec2<f32>(light_dir.x, -light_dir.y) * step_size;
            let h_step = light_dir.z * step_size * z_scale;
            
            var curr_uv = uv;
            var curr_h = h_center + 0.0035;
            
            for (var i = 0; i < step_count; i = i + 1) {
                curr_uv = curr_uv + uv_step;
                curr_h = curr_h + h_step;
                
                if (curr_uv.x < 0.0 || curr_uv.x > 1.0 || curr_uv.y < 0.0 || curr_uv.y > 1.0 || curr_h > 1.0) {
                    break;
                }
                
                let sample_h = textureSampleLevel(heightmap_tex, heightmap_sampler, curr_uv, 0.0).r;
                if (curr_h < sample_h) {
                    shadow_factor = 0.28;
                    break;
                }
            }
        }
        
        // Add soft circular shadow from the marble sphere
        let light_offset = vec2<f32>(light_dir.x, -light_dir.y) * 0.022;
        let d_to_marble_shadow = distance(uv, marble_uv - light_offset);
        if (d_to_marble_shadow < marble_r_uv * 1.5) {
            let m_shadow = smoothstep(marble_r_uv * 0.8, marble_r_uv * 1.5, d_to_marble_shadow);
            shadow_factor = shadow_factor * (0.35 + 0.65 * m_shadow);
        }
        
        diffuse = diff_color * shadow_factor;
        specular = spec_color * shadow_factor;
    } else {
        // Rainbow LED Ring Mode
        let view_dir = normalize(camera.camera_pos.xyz - world_pos);
        let num_leds = 8;
        let z_scale = 0.015;
        let step_size = 0.004;
        let step_count = 8;
        
        var diffuse_accum = vec3<f32>(0.0);
        
        for (var i = 0; i < num_leds; i = i + 1) {
            let angle_led = f32(i) * (2.0 * PI / f32(num_leds)) + uniforms.time * 0.10;
            let l_dir = normalize(vec3<f32>(cos(angle_led), sin(angle_led), 0.20));
            
            var led_color = vec3<f32>(0.0);
            if (uniforms.led_mode == 1u) {
                let hue = fract(f32(i) / f32(num_leds) - uniforms.time * 0.05);
                led_color = hue_to_rgb(hue);
            } else {
                let hue = fract(uniforms.time * 0.03);
                led_color = hue_to_rgb(hue);
            }
            
            let diff_strength = max(dot(normal, l_dir), 0.0);
            

            
            var shadow_factor = 1.0;
            if (uniforms.shadow_enabled == 1u) {
                let uv_step = vec2<f32>(l_dir.x, -l_dir.y) * step_size;
                let h_step = l_dir.z * step_size * z_scale;
                
                var curr_uv = uv;
                var curr_h = h_center + 0.0035;
                
                for (var s = 0; s < step_count; s = s + 1) {
                    curr_uv = curr_uv + uv_step;
                    curr_h = curr_h + h_step;
                    
                    if (curr_uv.x < 0.0 || curr_uv.x > 1.0 || curr_uv.y < 0.0 || curr_uv.y > 1.0 || curr_h > 1.0) {
                        break;
                    }
                    
                    let sample_h = textureSampleLevel(heightmap_tex, heightmap_sampler, curr_uv, 0.0).r;
                    if (curr_h < sample_h) {
                        shadow_factor = 0.25;
                        break;
                    }
                }
            }
            
            // Add soft circular shadow from the marble for this LED
            let led_offset = vec2<f32>(l_dir.x, -l_dir.y) * 0.018;
            let d_to_marble_shadow = distance(uv, marble_uv - led_offset);
            if (d_to_marble_shadow < marble_r_uv * 1.5) {
                let m_shadow = smoothstep(marble_r_uv * 0.8, marble_r_uv * 1.5, d_to_marble_shadow);
                shadow_factor = shadow_factor * (0.35 + 0.65 * m_shadow);
            }
            
            diffuse_accum = diffuse_accum + led_color * diff_strength * shadow_factor;
        }
        
        diffuse = diffuse_accum * (uniforms.light_brightness / f32(num_leds));
        specular = vec3<f32>(0.0);
    }

    // Base sand color from uniforms with subtle grain color variation (creates realistic sand texture even when flat)
    let color_grain = hash(uv * 1800.0);
    let sand_base_color = uniforms.sand_color.rgb * (1.0 + (color_grain - 0.5) * 0.08);

    // Brighter, warmer ambient reflection to make sand look soft, diffuse and matte (less metallic)
    let ambient = vec3<f32>(0.45, 0.45, 0.48);
    
    // Combine shading: ambient + diffuse + specular
    let final_lighting = ambient + diffuse + specular;
    let final_color = sand_base_color * final_lighting;

    return vec4<f32>(final_color, 1.0);
}
