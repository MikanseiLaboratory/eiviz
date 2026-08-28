struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@group(0) @binding(0) var y_tex: texture_2d<f32>;
@group(0) @binding(1) var uv_tex: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;

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
    var out: VsOut;
    out.clip = vec4<f32>(local.x * 2.0 - 1.0, (1.0 - local.y) * 2.0 - 1.0, 0.0, 1.0);
    out.uv = local;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let y = (textureSample(y_tex, samp, in.uv).r - 16.0 / 255.0) * (255.0 / 219.0);
    let chroma = textureSample(uv_tex, samp, in.uv).rg - vec2<f32>(128.0 / 255.0);
    let u = chroma.x;
    let v = chroma.y;
    let r = clamp(y + 1.5748 * v, 0.0, 1.0);
    let g = clamp(y - 0.1873 * u - 0.4681 * v, 0.0, 1.0);
    let b = clamp(y + 1.8556 * u, 0.0, 1.0);
    return vec4<f32>(r, g, b, 1.0);
}
