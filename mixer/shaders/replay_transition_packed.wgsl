// Replay clip transition over packed UYVY frames.
//
// The replay decoder hands back packed frames: one texel is two pixels (Y0, U, Y1, V).
// Both frames of a transition use the same layout, so a texel-wise blend is the same
// picture as a pixel-wise one — the two bytes of luma travel with the chroma they
// belong to. Horizontal offsets are counted in whole texels for that reason: half a
// texel would take a neighbouring pixel's colour with it, which is why the CPU blend
// rounds the same way.
//
// Cut, dissolve and dip are pure blends. The directional kinds move whole columns, so
// they copy texels rather than mixing them.

struct TransitionParams {
    // 0 cut, 1 dissolve, 2 dip, 3 wipe, 4 slide, 5 push.
    kind: u32,
    // 0 left, 1 right, 2 up, 3 down.
    direction: u32,
    // 0 shows only the outgoing frame, 1 only the incoming one.
    mix: f32,
    _pad: f32,
}

@group(0) @binding(0) var outgoing_tex: texture_2d<f32>;
@group(0) @binding(1) var incoming_tex: texture_2d<f32>;
@group(0) @binding(2) var linear_sampler: sampler;
@group(0) @binding(3) var<uniform> params: TransitionParams;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VsOut {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    var out: VsOut;
    out.clip = vec4<f32>(positions[index], 0.0, 1.0);
    out.uv = vec2<f32>((positions[index].x + 1.0) * 0.5, (1.0 - positions[index].y) * 0.5);
    return out;
}

fn outgoing(uv: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(outgoing_tex, linear_sampler, clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0)), 0.0);
}

fn incoming(uv: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(incoming_tex, linear_sampler, clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0)), 0.0);
}

fn travel(uv: vec2<f32>) -> f32 {
    if params.direction == 0u {
        return uv.x;
    }
    if params.direction == 1u {
        return 1.0 - uv.x;
    }
    if params.direction == 2u {
        return uv.y;
    }
    return 1.0 - uv.y;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let m = clamp(params.mix, 0.0, 1.0);
    let a = outgoing(in.uv);
    let b = incoming(in.uv);
    if params.kind == 0u {
        return select(a, b, m >= 1.0);
    }
    if params.kind == 1u {
        return mix(a, b, m);
    }
    if params.kind == 2u {
        if m < 0.5 {
            // Towards black: luma falls to the 16 floor, chroma to neutral 128.
            let t = m * 2.0;
            return vec4<f32>(mix(a.r, 16.0 / 255.0, t), mix(a.g, 128.0 / 255.0, t),
                            mix(a.b, 16.0 / 255.0, t), mix(a.a, 128.0 / 255.0, t));
        }
        let t = (m - 0.5) * 2.0;
        return vec4<f32>(mix(16.0 / 255.0, b.r, t), mix(128.0 / 255.0, b.g, t),
                        mix(16.0 / 255.0, b.b, t), mix(128.0 / 255.0, b.a, t));
    }
    let edge = 1.0 - m;
    let axis = travel(in.uv);
    if params.kind == 3u {
        // Wipe: a hard edge; the outgoing picture stays where it is.
        return select(a, b, axis >= edge);
    }
    if params.kind == 4u {
        // Slide: the incoming frame comes in over the outgoing one.
        return select(a, b, axis >= edge);
    }
    // Push: both travel. The incoming enters from the far edge while the outgoing
    // leaves by the same distance.
    var shifted = in.uv;
    var pushed = in.uv;
    if params.direction == 0u {
        shifted.x = in.uv.x - edge;
        pushed.x = in.uv.x - m;
    } else if params.direction == 1u {
        shifted.x = in.uv.x + edge;
        pushed.x = in.uv.x + m;
    } else if params.direction == 2u {
        shifted.y = in.uv.y - edge;
        pushed.y = in.uv.y - m;
    } else {
        shifted.y = in.uv.y + edge;
        pushed.y = in.uv.y + m;
    }
    if axis >= edge {
        return incoming(shifted);
    }
    return outgoing(pushed);
}
