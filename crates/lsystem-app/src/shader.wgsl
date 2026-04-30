struct Transform {
    scale: vec2<f32>,
    offset: vec2<f32>,
}

@group(0) @binding(0)
var<uniform> transform: Transform;

@vertex
fn vs_main(@location(0) position: vec2<f32>) -> @builtin(position) vec4<f32> {
    let ndc = position * transform.scale + transform.offset;
    return vec4<f32>(ndc, 0.0, 1.0);
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    return vec4<f32>(0.0, 0.9, 0.5, 1.0);
}
