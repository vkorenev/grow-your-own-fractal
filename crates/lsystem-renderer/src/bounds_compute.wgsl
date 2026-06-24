struct BoundsParams {
    count: u32,
    dimensions: u32,
    stride_words: u32,
    _pad: u32,
}

struct BoundsOutput {
    min_x: atomic<u32>,
    min_y: atomic<u32>,
    min_z: atomic<u32>,
    max_x: atomic<u32>,
    max_y: atomic<u32>,
    max_z: atomic<u32>,
}

@group(0) @binding(0)
var<storage, read> segment_words: array<u32>;

@group(0) @binding(1)
var<storage, read_write> bounds: BoundsOutput;

@group(0) @binding(2)
var<uniform> params: BoundsParams;

fn float_to_ordered_u32(f: f32) -> u32 {
    let bits = bitcast<u32>(f);
    return select(bits | 0x80000000u, ~bits, (bits & 0x80000000u) != 0u);
}

fn ordered_word(index: u32) -> u32 {
    return float_to_ordered_u32(bitcast<f32>(segment_words[index]));
}

@compute @workgroup_size(64)
fn bounds_segments(@builtin(global_invocation_id) id: vec3<u32>) {
    let index = id.x;
    if index >= params.count {
        return;
    }

    let base = index * params.stride_words;
    let ax = ordered_word(base);
    let ay = ordered_word(base + 1u);
    let bx_offset = select(3u, 2u, params.dimensions == 2u);
    let bx = ordered_word(base + bx_offset);
    let by = ordered_word(base + bx_offset + 1u);

    atomicMin(&bounds.min_x, min(ax, bx));
    atomicMin(&bounds.min_y, min(ay, by));
    atomicMax(&bounds.max_x, max(ax, bx));
    atomicMax(&bounds.max_y, max(ay, by));

    if params.dimensions == 3u {
        let az = ordered_word(base + 2u);
        let bz = ordered_word(base + 5u);
        atomicMin(&bounds.min_z, min(az, bz));
        atomicMax(&bounds.max_z, max(az, bz));
    }
}
