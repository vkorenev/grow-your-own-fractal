struct Mvp {
    matrix: mat4x4<f32>,
}

struct ColorParams {
    mode: u32,
    total_segments: u32,
    max_topological_depth: u32,
    _pad1: u32,
    color_start: vec4<f32>,
    color_end: vec4<f32>,
    hue_start: f32,
    saturation: f32,
    value: f32,
    _pad2: f32,
}

@group(0) @binding(0)
var<uniform> mvp: Mvp;

@group(0) @binding(1)
var<uniform> color_params: ColorParams;

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> vec3<f32> {
    let h6 = (h % 360.0) / 60.0;
    let i = u32(h6) % 6u;
    let f = h6 - floor(h6);
    let p = v * (1.0 - s);
    let q = v * (1.0 - f * s);
    let t = v * (1.0 - (1.0 - f) * s);
    switch i {
        case 0u: { return vec3<f32>(v, t, p); }
        case 1u: { return vec3<f32>(q, v, p); }
        case 2u: { return vec3<f32>(p, v, t); }
        case 3u: { return vec3<f32>(p, q, v); }
        case 4u: { return vec3<f32>(t, p, v); }
        default: { return vec3<f32>(v, p, q); }
    }
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
}

fn color_for_traversal(instance_index: u32) -> vec4<f32> {
    let denom = max(color_params.total_segments, 2u) - 1u;
    let t = f32(instance_index) / f32(denom);

    switch color_params.mode {
        case 1u: {
            return mix(color_params.color_start, color_params.color_end, t);
        }
        case 2u: {
            let hue = color_params.hue_start + t * 360.0;
            return vec4<f32>(
                hsv_to_rgb(hue, color_params.saturation, color_params.value),
                1.0,
            );
        }
        case 3u: {
            return color_for_depth(instance_index);
        }
        default: {
            return color_params.color_start;
        }
    }
}

fn color_for_depth(topological_depth: u32) -> vec4<f32> {
    let t = f32(topological_depth) / f32(max(color_params.max_topological_depth, 1u));
    return mix(color_params.color_start, color_params.color_end, t);
}

fn segment_position(vi: u32, start: vec3<f32>, end: vec3<f32>) -> vec3<f32> {
    if vi == 0u {
        return start;
    }
    return end;
}

@vertex
fn vs_main(
    @builtin(vertex_index) vi: u32,
    @builtin(instance_index) ii: u32,
    @location(0) start: vec3<f32>,
    @location(1) end: vec3<f32>,
) -> VertexOutput {
    let position = segment_position(vi, start, end);

    var out: VertexOutput;
    out.clip_position = mvp.matrix * vec4<f32>(position, 1.0);
    out.color = color_for_traversal(ii);
    return out;
}

@vertex
fn vs_depth_main(
    @builtin(vertex_index) vi: u32,
    @builtin(instance_index) ii: u32,
    @location(0) start: vec3<f32>,
    @location(1) end: vec3<f32>,
    @location(2) topological_depth: u32,
) -> VertexOutput {
    let position = segment_position(vi, start, end);

    var out: VertexOutput;
    out.clip_position = mvp.matrix * vec4<f32>(position, 1.0);
    if color_params.mode == 3u {
        out.color = color_for_depth(topological_depth);
    } else {
        out.color = color_for_traversal(ii);
    }
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return in.color;
}
