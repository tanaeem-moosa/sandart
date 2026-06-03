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
};

@group(0) @binding(2) var<uniform> uniforms: LightingUniforms;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) in_vertex_index: u32) -> VertexOutput {
    var out: VertexOutput;
    
    // Map vertex_index to a quad (-1.0 to 1.0)
    var positions = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(1.0, -1.0),
        vec2<f32>(-1.0, 1.0),
        vec2<f32>(-1.0, 1.0),
        vec2<f32>(1.0, -1.0),
        vec2<f32>(1.0, 1.0)
    );
    
    out.position = vec4<f32>(positions[in_vertex_index], 0.0, 1.0);
    out.uv = vec2<f32>(
        positions[in_vertex_index].x * 0.5 + 0.5,
        -positions[in_vertex_index].y * 0.5 + 0.5
    );
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
fn fs_main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    let center = vec2<f32>(0.5, 0.5);
    let dist = distance(uv, center);
    
    if (dist >= 0.46) {
        // Outer dark frame (table rim)
        return vec4<f32>(0.07, 0.07, 0.08, 1.0);
    }
    
    // 1. Compute finite difference normal from neighbor heightmap pixels
    let texel_size = 1.0 / 512.0;
    let h_center = textureSampleLevel(heightmap_tex, heightmap_sampler, uv, 0.0).r;
    let h_left   = textureSampleLevel(heightmap_tex, heightmap_sampler, uv + vec2<f32>(-texel_size, 0.0), 0.0).r;
    let h_right  = textureSampleLevel(heightmap_tex, heightmap_sampler, uv + vec2<f32>(texel_size, 0.0), 0.0).r;
    let h_up     = textureSampleLevel(heightmap_tex, heightmap_sampler, uv + vec2<f32>(0.0, -texel_size), 0.0).r;
    let h_down   = textureSampleLevel(heightmap_tex, heightmap_sampler, uv + vec2<f32>(0.0, texel_size), 0.0).r;

    // Normal tilting scale (high factor creates visual depth)
    let depth_factor = 28.0;
    var normal = normalize(vec3<f32>(
        (h_left - h_right) * depth_factor,
        (h_up - h_down) * depth_factor,
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
        let view_dir = vec3<f32>(0.0, 0.0, 1.0);
        
        let diff_strength = max(dot(normal, light_dir), 0.0);
        let diff_color = uniforms.light_color.rgb * diff_strength * uniforms.light_brightness;
        
        let spec_color = vec3<f32>(0.0);
        
        var shadow_factor = 1.0;
        if (uniforms.shadow_enabled == 1u) {
            let step_count = 32;
            let z_scale = 0.06;
            let step_size = 0.0022;
            let uv_step = light_dir.xy * step_size;
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
        
        diffuse = diff_color * shadow_factor;
        specular = spec_color * shadow_factor;
    } else {
        // Rainbow LED Ring Mode
        let view_dir = vec3<f32>(0.0, 0.0, 1.0);
        let num_leds = 8;
        let z_scale = 0.06;
        let step_size = 0.004;
        let step_count = 8;
        
        var diffuse_accum = vec3<f32>(0.0);
        
        for (var i = 0; i < num_leds; i = i + 1) {
            let angle_led = f32(i) * (2.0 * PI / f32(num_leds)) + uniforms.time * 0.10;
            let l_dir = normalize(vec3<f32>(cos(angle_led), sin(angle_led), 0.20));
            
            let hue = fract(f32(i) / f32(num_leds) - uniforms.time * 0.05);
            let led_color = hue_to_rgb(hue);
            
            let diff_strength = max(dot(normal, l_dir), 0.0);
            

            
            var shadow_factor = 1.0;
            if (uniforms.shadow_enabled == 1u) {
                let uv_step = l_dir.xy * step_size;
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
