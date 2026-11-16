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

@group(0) @binding(0) var src_tex: texture_2d<f32>;
@group(0) @binding(1) var dst_tex: texture_storage_2d<rgba8unorm, write>;
@group(0) @binding(2) var<uniform> params: MixParams;

@compute @workgroup_size(8, 8)
fn cs_main(@builtin(global_invocation_id) id: vec3<u32>) {
    let dims = vec2<i32>(textureDimensions(dst_tex));
    let p = vec2<i32>(id.xy);
    if p.x >= dims.x || p.y >= dims.y {
        return;
    }
    let horiz = params.direction == 0u;
    let step = select(vec2<i32>(0, 1), vec2<i32>(1, 0), horiz);
    let w = array<f32, 7>(0.05, 0.09, 0.12, 0.48, 0.12, 0.09, 0.05);
    var acc = vec3<f32>(0.0);
    for (var i = 0; i < 7; i = i + 1) {
        let q = clamp(p + step * (i - 3) * 3, vec2<i32>(0), dims - vec2<i32>(1));
        acc = acc + textureLoad(src_tex, q, 0).rgb * w[i];
    }
    textureStore(dst_tex, p, vec4<f32>(acc, 1.0));
}
