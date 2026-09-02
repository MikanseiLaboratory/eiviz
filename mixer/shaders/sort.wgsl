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

fn luma(c: vec4<f32>) -> f32 {
    return dot(c.rgb, vec3<f32>(0.299, 0.587, 0.114));
}

fn load_px(p: vec2<i32>, dims: vec2<i32>) -> vec4<f32> {
    return textureLoad(src_tex, clamp(p, vec2<i32>(0), dims - vec2<i32>(1)), 0);
}

@compute @workgroup_size(64)
fn cs_main(@builtin(global_invocation_id) id: vec3<u32>) {
    let dims = vec2<i32>(textureDimensions(src_tex));
    let horiz = params.direction == 0u || params.direction == 1u;
    let descending = params.direction == 1u || params.direction == 3u;
    let thresh = max(params.softness, 0.001);
    let line = i32(id.x);
    let line_max = select(dims.y, dims.x, !horiz);
    if line >= line_max {
        return;
    }
    let len = select(dims.x, dims.y, !horiz);
    var i = 0;
    loop {
        if i >= len {
            break;
        }
        var p = vec2<i32>(i, line);
        if !horiz {
            p = vec2<i32>(line, i);
        }
        let c = load_px(p, dims);
        if luma(c) < thresh {
            textureStore(dst_tex, p, c);
            i = i + 1;
            continue;
        }
        var colors: array<vec4<f32>, 64>;
        var n = 0;
        var j = i;
        loop {
            if j >= len || n >= 64 {
                break;
            }
            var q = vec2<i32>(j, line);
            if !horiz {
                q = vec2<i32>(line, j);
            }
            let s = load_px(q, dims);
            if luma(s) < thresh {
                break;
            }
            colors[n] = s;
            n = n + 1;
            j = j + 1;
        }
        for (var a = 1; a < 64; a = a + 1) {
            if a >= n {
                break;
            }
            let key = colors[a];
            let key_l = luma(key);
            var b = a - 1;
            loop {
                if b < 0 {
                    break;
                }
                let left = luma(colors[b]);
                let should_shift = select(left > key_l, left < key_l, descending);
                if !should_shift {
                    break;
                }
                colors[b + 1] = colors[b];
                b = b - 1;
            }
            colors[b + 1] = key;
        }
        for (var k = 0; k < n; k = k + 1) {
            var q = vec2<i32>(i + k, line);
            if !horiz {
                q = vec2<i32>(line, i + k);
            }
            textureStore(dst_tex, q, colors[k]);
        }
        i = j;
    }
}
