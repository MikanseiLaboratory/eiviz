struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

struct MixParams {
    mix: f32,
    kind: u32,
    direction: u32,
    softness: f32,
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

fn sample_pgm(uv: vec2<f32>) -> vec4<f32> {
    return textureSample(pgm_tex, src_samp, uv);
}

fn sample_pvw(uv: vec2<f32>) -> vec4<f32> {
    return textureSample(pvw_tex, src_samp, uv);
}

fn in_bounds(uv: vec2<f32>) -> bool {
    return uv.x >= 0.0 && uv.x <= 1.0 && uv.y >= 0.0 && uv.y <= 1.0;
}

fn wipe_axis(uv: vec2<f32>, t: f32, dir: u32, softness: f32) -> f32 {
    var coord = uv.x;
    var edge = t;
    if dir == 1u {
        coord = 1.0 - uv.x;
    } else if dir == 2u {
        coord = uv.y;
        edge = t;
    } else if dir == 3u {
        coord = 1.0 - uv.y;
    }
    let soft = max(softness, 0.001);
    return smoothstep(edge - soft, edge + soft, coord);
}

fn slide_uv(uv: vec2<f32>, t: f32, dir: u32, incoming: bool) -> vec2<f32> {
    var offset = vec2<f32>(0.0, 0.0);
    if dir == 0u {
        offset = vec2<f32>(select(t, t - 1.0, incoming), 0.0);
    } else if dir == 1u {
        offset = vec2<f32>(select(-t, 1.0 - t, incoming), 0.0);
    } else if dir == 2u {
        offset = vec2<f32>(0.0, select(t, t - 1.0, incoming));
    } else {
        offset = vec2<f32>(0.0, select(-t, 1.0 - t, incoming));
    }
    return uv + offset;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let a = sample_pgm(in.uv);
    let b = sample_pvw(in.uv);
    let t = params.mix;
    let kind = params.kind;
    let dir = params.direction;
    let soft = select(params.softness, 0.02, params.softness <= 0.0);

    if kind == 2u {
        let hold = 0.08;
        let fade = 0.5 - hold * 0.5;
        if t < fade {
            return mix(a, params.dip, t / max(fade, 0.001));
        }
        if t > 1.0 - fade {
            return mix(params.dip, b, (t - (1.0 - fade)) / max(fade, 0.001));
        }
        return params.dip;
    }
    if kind == 3u {
        return mix(b, a, wipe_axis(in.uv, t, dir, soft));
    }
    if kind == 4u {
        let uv_b = slide_uv(in.uv, t, dir, true);
        if in_bounds(uv_b) {
            return sample_pvw(uv_b);
        }
        return a;
    }
    if kind == 5u {
        let uv_a = slide_uv(in.uv, t, dir, false);
        let uv_b = slide_uv(in.uv, t, dir, true);
        if in_bounds(uv_b) {
            return sample_pvw(uv_b);
        }
        if in_bounds(uv_a) {
            return sample_pgm(uv_a);
        }
        return params.dip;
    }
    if kind == 6u {
        let d = distance(in.uv, vec2<f32>(0.5, 0.5));
        let radius = t * 0.75;
        let w = smoothstep(radius - soft, radius + soft, d);
        return mix(b, a, w);
    }
    if kind == 7u {
        let strips = 12.0;
        let coord = select(in.uv.x, in.uv.y, dir != 0u);
        let cell = fract(coord * strips);
        let w = smoothstep(t - soft, t + soft, cell);
        return mix(b, a, w);
    }
    if kind == 8u {
        let zoom_a = mix(1.0, 1.35, t);
        let zoom_b = mix(1.35, 1.0, t);
        let uv_a = (in.uv - vec2<f32>(0.5, 0.5)) / zoom_a + vec2<f32>(0.5, 0.5);
        let uv_b = (in.uv - vec2<f32>(0.5, 0.5)) / zoom_b + vec2<f32>(0.5, 0.5);
        return mix(sample_pgm(uv_a), sample_pvw(uv_b), t);
    }
    if kind == 9u {
        let add = min(a + b * t, vec4<f32>(1.0));
        return mix(a, add, t);
    }
    return mix(a, b, t);
}
