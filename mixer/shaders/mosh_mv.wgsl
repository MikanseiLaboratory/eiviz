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

// Per-macroblock motion of the incoming bus between its previous and current frame.
// The stored vector points from a current block to where its content was last frame,
// so the mix pass can drag the held picture along the incoming motion like a P-frame
// decoded against the wrong reference.
@group(0) @binding(0) var cur_tex: texture_2d<f32>;
@group(0) @binding(1) var ref_tex: texture_2d<f32>;
@group(0) @binding(2) var dst_tex: texture_storage_2d<rgba8unorm, write>;
@group(0) @binding(3) var<uniform> params: MixParams;

const BLOCK: i32 = 16;
const RANGE: i32 = 16;
const COARSE: i32 = 4;
// Keeps flat or static blocks at zero instead of latching onto an arbitrary tie.
const MOTION_PENALTY: f32 = 0.004;

fn luma(c: vec4<f32>) -> f32 {
    return dot(c.rgb, vec3<f32>(0.299, 0.587, 0.114));
}

fn cur_luma(p: vec2<i32>, dims: vec2<i32>) -> f32 {
    return luma(textureLoad(cur_tex, clamp(p, vec2<i32>(0), dims - vec2<i32>(1)), 0));
}

fn ref_luma(p: vec2<i32>, dims: vec2<i32>) -> f32 {
    return luma(textureLoad(ref_tex, clamp(p, vec2<i32>(0), dims - vec2<i32>(1)), 0));
}

fn block_cost(origin: vec2<i32>, d: vec2<i32>, dims: vec2<i32>) -> f32 {
    var err = 0.0;
    for (var sy = 0; sy < 4; sy = sy + 1) {
        for (var sx = 0; sx < 4; sx = sx + 1) {
            let p = origin + vec2<i32>(sx * 4 + 2, sy * 4 + 2);
            err = err + abs(cur_luma(p, dims) - ref_luma(p + d, dims));
        }
    }
    return err + MOTION_PENALTY * f32(abs(d.x) + abs(d.y));
}

@compute @workgroup_size(8, 8)
fn cs_main(@builtin(global_invocation_id) id: vec3<u32>) {
    let grid = vec2<i32>(textureDimensions(dst_tex));
    let cell = vec2<i32>(id.xy);
    if cell.x >= grid.x || cell.y >= grid.y {
        return;
    }
    let dims = vec2<i32>(textureDimensions(cur_tex));
    let origin = cell * BLOCK;
    var best = vec2<i32>(0);
    var best_cost = block_cost(origin, best, dims);
    for (var dy = -RANGE; dy <= RANGE; dy = dy + COARSE) {
        for (var dx = -RANGE; dx <= RANGE; dx = dx + COARSE) {
            let d = vec2<i32>(dx, dy);
            let cost = block_cost(origin, d, dims);
            if cost < best_cost {
                best_cost = cost;
                best = d;
            }
        }
    }
    let coarse = best;
    for (var dy = -COARSE / 2; dy <= COARSE / 2; dy = dy + 1) {
        for (var dx = -COARSE / 2; dx <= COARSE / 2; dx = dx + 1) {
            let d = clamp(coarse + vec2<i32>(dx, dy), vec2<i32>(-RANGE), vec2<i32>(RANGE));
            let cost = block_cost(origin, d, dims);
            if cost < best_cost {
                best_cost = cost;
                best = d;
            }
        }
    }
    let enc = (vec2<f32>(best) + f32(RANGE)) / f32(RANGE * 2);
    textureStore(dst_tex, cell, vec4<f32>(enc, 0.0, 1.0));
}
