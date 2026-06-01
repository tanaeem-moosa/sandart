@group(0) @binding(0) var heightmap_tex: texture_2d<f32>;
@group(0) @binding(1) var heightmap_sampler: sampler;

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

@fragment
fn fs_main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    let center = vec2<f32>(0.5, 0.5);
    let dist = distance(uv, center);
    
    // Sample height value from texture
    let height = textureSample(heightmap_tex, heightmap_sampler, uv).r;
    
    if (dist < 0.46) {
        // Render height directly as grayscale to verify transfer
        return vec4<f32>(height, height, height, 1.0);
    } else {
        // Outer dark frame
        return vec4<f32>(0.1, 0.1, 0.12, 1.0);
    }
}
