struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

struct ColorParams {
    color: vec4<f32>,
    scroll: f32,
    flags: f32,
    pad: vec2<f32>,
}

@group(0) @binding(0) var<uniform> params: ColorParams;

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
    if params.flags > 0.5 {
        let x = fract(in.uv.x - params.scroll);
        if x < 0.06 {
            return vec4<f32>(1.0, 1.0, 1.0, 1.0);
        }
    }
    return params.color;
}
