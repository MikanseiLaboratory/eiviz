struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@group(0) @binding(0) var src_tex: texture_2d<f32>;
@group(0) @binding(1) var src_samp: sampler;

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

fn rgb_to_yuv(rgb: vec3<f32>) -> vec3<f32> {
    let y = 0.299 * rgb.r + 0.587 * rgb.g + 0.114 * rgb.b;
    let u = -0.169 * rgb.r - 0.331 * rgb.g + 0.500 * rgb.b + 0.5;
    let v = 0.500 * rgb.r - 0.419 * rgb.g - 0.081 * rgb.b + 0.5;
    return clamp(vec3<f32>(y, u, v), vec3<f32>(0.0), vec3<f32>(1.0));
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // One output texel covers two source pixels. The left and right source pixel centers sit a
    // quarter of an output texel either side of this texel's center, whatever the source size.
    let quarter = abs(dpdx(in.uv.x)) * 0.25;
    let a = rgb_to_yuv(textureSample(src_tex, src_samp, vec2<f32>(in.uv.x - quarter, in.uv.y)).rgb);
    let b = rgb_to_yuv(textureSample(src_tex, src_samp, vec2<f32>(in.uv.x + quarter, in.uv.y)).rgb);
    return vec4<f32>(a.y, a.x, a.z, b.x);
}
