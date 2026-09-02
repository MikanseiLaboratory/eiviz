struct MixParams {
    mix: f32,
    kind: u32,
    direction: u32,
    softness: f32,
    dip: vec4<f32>,
    param: f32,
    time: f32,
    resolution: vec2<f32>,
}

@group(0) @binding(0) var tex_a: texture_2d<f32>;
@group(0) @binding(1) var tex_b: texture_2d<f32>;
@group(0) @binding(2) var dst_tex: texture_storage_2d<rgba8unorm, write>;
@group(0) @binding(3) var<uniform> params: MixParams;

fn luma(c: vec4<f32>) -> f32 {
    return dot(c.rgb, vec3<f32>(0.299, 0.587, 0.114));
}

@compute @workgroup_size(8, 8)
fn cs_main(@builtin(global_invocation_id) id: vec3<u32>) {
    let dst_dims = vec2<u32>(textureDimensions(dst_tex));
    if id.x >= dst_dims.x || id.y >= dst_dims.y {
        return;
    }
    let src_dims = vec2<f32>(textureDimensions(tex_a));
    let uv = (vec2<f32>(id.xy) + 0.5) / vec2<f32>(dst_dims);
    let pa = vec2<i32>(clamp(uv * src_dims, vec2<f32>(0.0), src_dims - vec2<f32>(1.0)));
    let a = textureLoad(tex_a, pa, 0);
    let b = textureLoad(tex_b, pa, 0);
    let peak = pow(max(1.0 - abs(params.mix * 2.0 - 1.0), 0.0), 0.45);
    let c = mix(a, b, smoothstep(0.2, 0.8, params.mix));
    let hi = max(c, max(a, b));
    let thresh = mix(0.62, 0.08, peak) * clamp(params.softness / 0.45, 0.35, 1.8);
    let lift = pow(max(luma(hi) - thresh, 0.0) / max(1.0 - thresh, 0.08), 0.65);
    textureStore(dst_tex, vec2<i32>(id.xy), vec4<f32>(hi.rgb * lift * (1.4 + peak * 2.2), 1.0));
}
