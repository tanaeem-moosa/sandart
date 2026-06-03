@group(0) @binding(0) var heightmap_tex: texture_2d<f32>;
@group(0) @binding(1) var heightmap_sampler: sampler;

struct LightingUniforms {
    light_dir: vec4<f32>,
    light_color: vec4<f32>,
    sand_color: vec4<f32>,
    light_brightness: f32,
    shadow_enabled: u32,
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
    let h_center = textureSample(heightmap_tex, heightmap_sampler, uv).r;
    let h_left   = textureSample(heightmap_tex, heightmap_sampler, uv + vec2<f32>(-texel_size, 0.0)).r;
    let h_right  = textureSample(heightmap_tex, heightmap_sampler, uv + vec2<f32>(texel_size, 0.0)).r;
    let h_up     = textureSample(heightmap_tex, heightmap_sampler, uv + vec2<f32>(0.0, -texel_size)).r;
    let h_down   = textureSample(heightmap_tex, heightmap_sampler, uv + vec2<f32>(0.0, texel_size)).r;

    // Normal tilting scale (high factor creates visual depth)
    let depth_factor = 28.0;
    var normal = normalize(vec3<f32>(
        (h_left - h_right) * depth_factor,
        (h_up - h_down) * depth_factor,
        1.0
    ));

    // 2. Perturb normal with micro-surface grain noise
    let noise_scale = 1200.0;
    let grain_noise = hash(uv * noise_scale);
    let grain_noise_y = hash(uv * noise_scale + vec2<f32>(17.0, 43.0));
    normal = normalize(normal + vec3<f32>(
        (grain_noise - 0.5) * 0.05,
        (grain_noise_y - 0.5) * 0.05,
        0.0
    ));

    // 3. Lighting direction & view vectors
    let light_dir = normalize(uniforms.light_dir.xyz);
    let view_dir = vec3<f32>(0.0, 0.0, 1.0);

    // 4. Raymarched Shadows (32 steps along light ray in heightmap space)
    var shadow_factor = 1.0;
    if (uniforms.shadow_enabled == 1u) {
        let step_count = 32;
        // z_scale maps float height [0, 1] relative to the UV step size
        let z_scale = 0.06; 
        let step_size = 0.0022; 
        
        let uv_step = light_dir.xy * step_size;
        let h_step = light_dir.z * step_size * z_scale;
        
        var curr_uv = uv;
        // Bias to avoid self-shadowing on steep slopes
        var curr_h = h_center + 0.0035; 
        
        for (var i = 0; i < step_count; i = i + 1) {
            curr_uv = curr_uv + uv_step;
            curr_h = curr_h + h_step;
            
            if (curr_uv.x < 0.0 || curr_uv.x > 1.0 || curr_uv.y < 0.0 || curr_uv.y > 1.0 || curr_h > 1.0) {
                break;
            }
            
            // Use textureSampleLevel inside dynamic loops to compile correctly in WGSL
            let sample_h = textureSampleLevel(heightmap_tex, heightmap_sampler, curr_uv, 0.0).r;
            if (curr_h < sample_h) {
                shadow_factor = 0.28; // Soft shadow ambient factor
                break;
            }
        }
    }

    // 5. Lighting Model (Phong)
    let ambient = vec3<f32>(0.12, 0.12, 0.14); // faint blue-gray ambient shadows
    
    // Diffuse
    let diff_strength = max(dot(normal, light_dir), 0.0);
    let diffuse = uniforms.light_color.rgb * diff_strength * uniforms.light_brightness;

    // Specular (sand glimmers)
    let reflect_dir = reflect(-light_dir, normal);
    let spec_strength = pow(max(dot(view_dir, reflect_dir), 0.0), 32.0);
    let specular = vec3<f32>(1.0) * spec_strength * 0.15 * uniforms.light_brightness;

    // Base sand color from uniforms
    let sand_base_color = uniforms.sand_color.rgb;

    // Combine shading: ambient + shadow-occluded diffuse/specular
    let final_lighting = ambient + (diffuse + specular) * shadow_factor;
    let final_color = sand_base_color * final_lighting;

    return vec4<f32>(final_color, 1.0);
}
