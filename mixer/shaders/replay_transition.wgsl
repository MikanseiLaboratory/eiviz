// Replay clip transition, composited on the GPU.
//
// The CPU path blended two decoded frames in memory and uploaded the result; this pass
// takes both frames as textures and writes the combined picture, so the picture never
// travels back through the CPU. The kinds are the same ones the mixing unit offers that
// a two-frame blend can express: cut, dissolve, dip through black, wipe, slide and push.

struct TransitionParams {
    // 0 cut, 1 dissolve, 2 dip, 3 wipe, 4 slide, 5 push.
    kind: u32,
    // 0 left, 1 right, 2 up, 3 down.
    direction: u32,
    // 0 shows only the outgoing frame, 1 only the incoming one.
    mix: f32,
    // Unused padding so the uniform block is 16-byte aligned.
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
    // Clip space y points up, the picture's y points down.
    out.uv = vec2<f32>((positions[index].x + 1.0) * 0.5, (1.0 - positions[index].y) * 0.5);
    return out;
}

fn outgoing(uv: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(outgoing_tex, linear_sampler, clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0)), 0.0);
}

fn incoming(uv: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(incoming_tex, linear_sampler, clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0)), 0.0);
}

/// How far along the outgoing frame is across the travel axis, 0..1.
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
        // Out through black over the first half, in from black over the second.
        if m < 0.5 {
            return a * (1.0 - m * 2.0);
        }
        return b * ((m - 0.5) * 2.0);
    }
    // The three directional kinds all use the same edge; they differ in what the
    // outgoing half shows.
    let edge = 1.0 - m;
    let axis = travel(in.uv);
    if params.kind == 3u {
        // Wipe: a hard edge, the outgoing picture stays put.
        return select(a, b, axis >= edge);
    }
    if params.kind == 4u {
        // Slide: the incoming frame comes in over the outgoing one, which stays put.
        if axis >= edge {
            return b;
        }
        return a;
    }
    // Push: both frames travel. The incoming enters from the far edge while the
    // outgoing leaves by the same distance, so the two move together.
    if axis >= edge {
        var shifted = in.uv;
        if params.direction == 0u {
            shifted.x = in.uv.x - edge;
        } else if params.direction == 1u {
            shifted.x = in.uv.x + edge;
        } else if params.direction == 2u {
            shifted.y = in.uv.y - edge;
        } else {
            shifted.y = in.uv.y + edge;
        }
        return incoming(shifted);
    }
    var pushed = in.uv;
    if params.direction == 0u {
        pushed.x = in.uv.x - m;
    } else if params.direction == 1u {
        pushed.x = in.uv.x + m;
    } else if params.direction == 2u {
        pushed.y = in.uv.y - m;
    } else {
        pushed.y = in.uv.y + m;
    }
    return outgoing(pushed);
}
