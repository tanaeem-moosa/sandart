struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) in_vertex_index: u32) -> VertexOutput {
    var out: VertexOutput;
    
    // Map vertex_index to a quad (-1.0 to 1.0)
    // 0: (-1, -1), 1: (1, -1), 2: (-1, 1), 3: (-1, 1), 4: (1, -1), 5: (1, 1)
    var positions = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(1.0, -1.0),
        vec2<f32>(-1.0, 1.0),
        vec2<f32>(-1.0, 1.0),
        vec2<f32>(1.0, -1.0),
        vec2<f32>(1.0, 1.0)
    );
    
    out.position = vec4<f32>(positions[in_vertex_index], 0.0, 1.0);
    out.uv = positions[in_vertex_index] * 0.5 + vec2<f32>(0.5, 0.5);
    return out;
}

@fragment
fn fs_main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    let center = vec2<f32>(0.5, 0.5);
    let dist = distance(uv, center);
    
    // Render a simple flat colored circle to verify WGPU rendering
    if (dist < 0.46) {
        // Coral/sand circle
        return vec4<f32>(0.9, 0.4, 0.3, 1.0);
    } else {
        // Outer dark frame
        return vec4<f32>(0.1, 0.1, 0.12, 1.0);
    }
}
