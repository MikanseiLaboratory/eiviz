struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

struct BlitParams {
    dst: vec4<f32>,
    src: vec4<f32>,
    opacity: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
}

@group(0) @binding(0) var packed_tex: texture_2d<f32>;
@group(0) @binding(1) var packed_samp: sampler;
@group(0) @binding(2) var<uniform> params: BlitParams;

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VsOut {
    var verts = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(1.0, 1.0),
    );
    let local = verts[index];
    let dest_uv = params.dst.xy + local * params.dst.zw;
    var out: VsOut;
    out.clip = vec4<f32>(dest_uv.x * 2.0 - 1.0, (1.0 - dest_uv.y) * 2.0 - 1.0, 0.0, 1.0);
    out.uv = params.src.xy + local * params.src.zw;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let dims = vec2<f32>(textureDimensions(packed_tex));
    let x = in.uv.x * dims.x * 2.0;
    let packed_u = (floor(x * 0.5) + 0.5) / dims.x;
    let sample = textureSample(packed_tex, packed_samp, vec2<f32>(packed_u, in.uv.y));
    let u = sample.r * 255.0 - 128.0;
    let y0 = sample.g * 255.0;
    let v = sample.b * 255.0 - 128.0;
    let y1 = sample.a * 255.0;
    let y = select(y1, y0, (u32(floor(x)) & 1u) == 0u);
    let r = clamp(y + 1.402 * v, 0.0, 255.0) / 255.0;
    let g = clamp(y - 0.344136 * u - 0.714136 * v, 0.0, 255.0) / 255.0;
    let b = clamp(y + 1.772 * u, 0.0, 255.0) / 255.0;
    return vec4<f32>(r * params.opacity, g * params.opacity, b * params.opacity, params.opacity);
}
