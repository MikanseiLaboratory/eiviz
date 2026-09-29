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

// Span pixel sort of one bus. `params.mix` is the sort amount (0 = untouched, 1 = full look),
// `softness` the luma threshold and `param` the span length.
// One workgroup sorts one SEG-pixel segment of one line with a bitonic sort in shared memory.
// Spans never cross a break (dark pixel or random cut), so the key is (span id, luma, index).
// Every workgroup variable is written before it is read, so the pipeline skips zero-init.
@group(0) @binding(0) var src_tex: texture_2d<f32>;
@group(0) @binding(1) var dst_tex: texture_storage_2d<rgba8unorm, write>;
@group(0) @binding(2) var<uniform> params: MixParams;

// Must match SORT_SEGMENT in compose/mod.rs.
const SEG: u32 = 1024u;
const THREADS: u32 = 256u;
const PER: u32 = 4u;

var<workgroup> keys: array<u32, 1024>;
var<workgroup> vals: array<u32, 1024>;
var<workgroup> sums: array<u32, 256>;

fn pcg(v: u32) -> u32 {
    let state = v * 747796405u + 2891336453u;
    let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}

fn hash01(v: u32) -> f32 {
    return f32(pcg(v) >> 8u) / 16777216.0;
}

fn luma(c: vec4<f32>) -> f32 {
    return dot(c.rgb, vec3<f32>(0.299, 0.587, 0.114));
}

@compute @workgroup_size(256)
fn cs_main(
    @builtin(workgroup_id) wg: vec3<u32>,
    @builtin(local_invocation_index) lid: u32,
) {
    let dims = vec2<i32>(textureDimensions(src_tex));
    let horiz = params.direction == 0u || params.direction == 1u;
    let descending = params.direction == 1u || params.direction == 3u;
    let len = select(dims.y, dims.x, horiz);
    let lines = select(dims.x, dims.y, horiz);
    let line = i32(wg.y);
    // Random segment phase per line keeps segment seams from lining up.
    let phase = i32(pcg(u32(line) * 9781u + params.direction * 6271u + 17u) % SEG);
    let seg_start = i32(wg.x * SEG) - phase;
    if line >= lines || seg_start >= len || seg_start + i32(SEG) <= 0 {
        return;
    }

    let amount = clamp(params.mix, 0.0, 1.0);
    let ease = amount * amount * (3.0 - 2.0 * amount);
    let thresh = mix(1.0, params.softness, ease);
    let span_max = mix(16.0, f32(SEG), clamp(params.param, 0.0, 1.0));
    let cut = 1.0 / max(mix(1.0, span_max, ease * ease), 1.0);

    let base = lid * PER;
    var breaks = array<u32, 4>(0u, 0u, 0u, 0u);
    var lum = array<f32, 4>(0.0, 0.0, 0.0, 0.0);
    var total = 0u;
    var prev_ok = false;
    for (var k = 0u; k < PER + 1u; k = k + 1u) {
        // k == 0 is the pixel just before this thread's first element.
        let local = i32(base) + i32(k) - 1;
        let axis = seg_start + local;
        var ok = false;
        var c = vec4<f32>(0.0);
        if local >= 0 && axis >= 0 && axis < len {
            let p = select(vec2<i32>(line, axis), vec2<i32>(axis, line), horiz);
            c = textureLoad(src_tex, p, 0);
            ok = luma(c) > thresh;
        }
        if k > 0u {
            let i = k - 1u;
            let cut_here = hash01(u32(axis) * 7919u + u32(line) * 104729u + params.direction) < cut;
            breaks[i] = select(0u, 1u, local == 0 || !ok || !prev_ok || cut_here);
            total = total + breaks[i];
            lum[i] = luma(c);
            vals[base + i] = pack4x8unorm(c);
        }
        prev_ok = ok;
    }

    sums[lid] = total;
    workgroupBarrier();
    for (var off = 1u; off < THREADS; off = off << 1u) {
        var v = sums[lid];
        if lid >= off {
            v = v + sums[lid - off];
        }
        workgroupBarrier();
        sums[lid] = v;
        workgroupBarrier();
    }
    var span_id = sums[lid] - total;
    for (var i = 0u; i < PER; i = i + 1u) {
        span_id = span_id + breaks[i];
        let q = u32(clamp(lum[i], 0.0, 1.0) * 1023.0);
        let lk = select(q, 1023u - q, descending);
        keys[base + i] = (span_id << 20u) | (lk << 10u) | (base + i);
    }

    for (var size = 2u; size <= SEG; size = size << 1u) {
        for (var stride = size >> 1u; stride > 0u; stride = stride >> 1u) {
            workgroupBarrier();
            for (var r = 0u; r < SEG / 2u / THREADS; r = r + 1u) {
                let pair = lid + r * THREADS;
                let i = 2u * stride * (pair / stride) + (pair % stride);
                let j = i + stride;
                let up = (i & size) == 0u;
                let ki = keys[i];
                let kj = keys[j];
                if (ki > kj) == up {
                    keys[i] = kj;
                    keys[j] = ki;
                    let vi = vals[i];
                    vals[i] = vals[j];
                    vals[j] = vi;
                }
            }
        }
    }
    workgroupBarrier();

    for (var i = 0u; i < PER; i = i + 1u) {
        let axis = seg_start + i32(base + i);
        if axis >= 0 && axis < len {
            let p = select(vec2<i32>(line, axis), vec2<i32>(axis, line), horiz);
            textureStore(dst_tex, p, unpack4x8unorm(vals[base + i]));
        }
    }
}
