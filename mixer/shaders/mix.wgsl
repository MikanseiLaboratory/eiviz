struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

struct MixParams {
    mix: f32,
    kind: u32,
    _pad: vec2<f32>,
    dip: vec4<f32>,
}

@group(0) @binding(0) var pgm_tex: texture_2d<f32>;
@group(0) @binding(1) var pvw_tex: texture_2d<f32>;
@group(0) @binding(2) var src_samp: sampler;
@group(0) @binding(3) var<uniform> params: MixParams;

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VsOut {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    let pos = positions[index];
    var out: VsOut;
    out.clip = vec4<f32>(pos, 0.0, 1.0);
    out.uv = vec2<f32>(pos.x * 0.5 + 0.5, 1.0 - (pos.y * 0.5 + 0.5));
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let a = textureSample(pgm_tex, src_samp, in.uv);
    let b = textureSample(pvw_tex, src_samp, in.uv);
    if params.kind == 2u {
        if params.mix < 0.5 {
            return mix(a, params.dip, params.mix * 2.0);
        }
        return mix(params.dip, b, (params.mix - 0.5) * 2.0);
    }
    return mix(a, b, params.mix);
}
