import package::common::{color_params, color_for_traversal, color_for_depth, VertexOutput};

struct Mvp {
    matrix: mat4x4<f32>,
}

@group(0) @binding(1)
var<uniform> mvp: Mvp;

struct Segment3D {
    @location(0) start: vec3<f32>,
    @location(1) end: vec3<f32>,
}

struct TopologicalDepthSegment3D {
    @location(0) start: vec3<f32>,
    @location(1) end: vec3<f32>,
    @location(2) topological_depth: u32,
}

fn segment_position_3d(vi: u32, start: vec3<f32>, end: vec3<f32>) -> vec3<f32> {
    if vi == 0u {
        return start;
    }
    return end;
}

@vertex
fn vs_main_3d(
    @builtin(vertex_index) vi: u32,
    @builtin(instance_index) ii: u32,
    segment: Segment3D,
) -> VertexOutput {
    let position = segment_position_3d(vi, segment.start, segment.end);

    var out: VertexOutput;
    out.clip_position = mvp.matrix * vec4<f32>(position, 1.0);
    out.color = color_for_traversal(ii);
    return out;
}

@vertex
fn vs_depth_main_3d(
    @builtin(vertex_index) vi: u32,
    @builtin(instance_index) ii: u32,
    segment: TopologicalDepthSegment3D,
) -> VertexOutput {
    let position = segment_position_3d(vi, segment.start, segment.end);

    var out: VertexOutput;
    out.clip_position = mvp.matrix * vec4<f32>(position, 1.0);
    if color_params.mode == 3u {
        out.color = color_for_depth(segment.topological_depth);
    } else {
        out.color = color_for_traversal(ii);
    }
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return in.color;
}
