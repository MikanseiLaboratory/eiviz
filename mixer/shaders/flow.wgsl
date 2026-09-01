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

fn load_a(p: vec2<i32>, dims: vec2<i32>) -> f32 {
    return luma(textureLoad(tex_a, clamp(p, vec2<i32>(0), dims - vec2<i32>(1)), 0));
}

fn load_b(p: vec2<i32>, dims: vec2<i32>) -> f32 {
    return luma(textureLoad(tex_b, clamp(p, vec2<i32>(0), dims - vec2<i32>(1)), 0));
}

@compute @workgroup_size(8, 8)
fn cs_main(@builtin(global_invocation_id) id: vec3<u32>) {
    let dst_dims = vec2<i32>(textureDimensions(dst_tex));
    let p = vec2<i32>(id.xy);
    if p.x >= dst_dims.x || p.y >= dst_dims.y {
        return;
    }
    let src_dims = vec2<i32>(textureDimensions(tex_a));
    let center = p * 2;
    var best = vec2<i32>(0);
    var best_err = 1.0e9;
    for (var dy = -4; dy <= 4; dy = dy + 1) {
        for (var dx = -4; dx <= 4; dx = dx + 1) {
            var err = 0.0;
            for (var oy = -1; oy <= 1; oy = oy + 1) {
                for (var ox = -1; ox <= 1; ox = ox + 1) {
                    let pa = center + vec2<i32>(ox, oy);
                    let pb = center + vec2<i32>(dx + ox, dy + oy);
                    err = err + abs(load_a(pa, src_dims) - load_b(pb, src_dims));
                }
            }
            if err < best_err {
                best_err = err;
                best = vec2<i32>(dx, dy);
            }
        }
    }
    let enc = vec2<f32>(best) / 8.0 * 0.5 + 0.5;
    textureStore(dst_tex, p, vec4<f32>(enc.x, enc.y, 0.0, 1.0));
}
