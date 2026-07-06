import package::common::{color_params, color_for_traversal, color_for_depth, VertexOutput};

struct Transform {
    scale: vec2<f32>,
    offset: vec2<f32>,
}

@group(0) @binding(1)
var<uniform> transform: Transform;

struct Segment2D {
    @location(0) start: vec2<f32>,
    @location(1) end: vec2<f32>,
}

struct TopologicalDepthSegment2D {
    @location(0) start: vec2<f32>,
    @location(1) end: vec2<f32>,
    @location(2) topological_depth: u32,
}

fn segment_position_2d(vi: u32, start: vec2<f32>, end: vec2<f32>) -> vec2<f32> {
    if vi == 0u {
        return start;
    }
    return end;
}

@vertex
fn vs_main_2d(
    @builtin(vertex_index) vi: u32,
    @builtin(instance_index) ii: u32,
    segment: Segment2D,
) -> VertexOutput {
    let position = segment_position_2d(vi, segment.start, segment.end);

    var out: VertexOutput;
    out.clip_position = vec4<f32>(
        position * transform.scale + transform.offset,
        0.0,
        1.0,
    );
    out.color = color_for_traversal(ii);
    return out;
}

@vertex
fn vs_depth_main_2d(
    @builtin(vertex_index) vi: u32,
    @builtin(instance_index) ii: u32,
    segment: TopologicalDepthSegment2D,
) -> VertexOutput {
    let position = segment_position_2d(vi, segment.start, segment.end);

    var out: VertexOutput;
    out.clip_position = vec4<f32>(
        position * transform.scale + transform.offset,
        0.0,
        1.0,
    );
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
