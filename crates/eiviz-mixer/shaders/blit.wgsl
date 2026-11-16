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

@group(0) @binding(0) var src_tex: texture_2d<f32>;
@group(0) @binding(1) var src_samp: sampler;
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
    let color = textureSample(src_tex, src_samp, in.uv);
    return vec4<f32>(color.rgb * color.a * params.opacity, color.a * params.opacity);
}
