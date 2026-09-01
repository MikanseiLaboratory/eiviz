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
    param: f32,
    time: f32,
    resolution: vec2<f32>,
}

@group(0) @binding(0) var pgm_tex: texture_2d<f32>;
@group(0) @binding(1) var pvw_tex: texture_2d<f32>;
@group(0) @binding(2) var src_samp: sampler;
@group(0) @binding(3) var<uniform> params: MixParams;
@group(0) @binding(4) var prev_tex: texture_2d<f32>;
@group(0) @binding(5) var src_samp_n: sampler;

const PI: f32 = 3.14159265;

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

fn sample_prev(uv: vec2<f32>) -> vec4<f32> {
    return textureSample(prev_tex, src_samp, clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0)));
}

fn sample_prev_n(uv: vec2<f32>) -> vec4<f32> {
    return textureSample(prev_tex, src_samp_n, clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0)));
}

fn in_bounds(uv: vec2<f32>) -> bool {
    return uv.x >= 0.0 && uv.x <= 1.0 && uv.y >= 0.0 && uv.y <= 1.0;
}

fn clamp_sample_pgm(uv: vec2<f32>) -> vec4<f32> {
    if in_bounds(uv) {
        return sample_pgm(uv);
    }
    return vec4<f32>(0.0);
}

fn clamp_sample_pvw(uv: vec2<f32>) -> vec4<f32> {
    if in_bounds(uv) {
        return sample_pvw(uv);
    }
    return vec4<f32>(0.0);
}

fn pval(fallback: f32) -> f32 {
    return select(fallback, params.param, params.param > 0.0);
}

fn soft() -> f32 {
    return select(0.02, params.softness, params.softness > 0.0);
}

fn luma(c: vec4<f32>) -> f32 {
    return dot(c.rgb, vec3<f32>(0.299, 0.587, 0.114));
}

fn hash21(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

fn rotate2(p: vec2<f32>, a: f32) -> vec2<f32> {
    let c = cos(a);
    let s = sin(a);
    return vec2<f32>(p.x * c - p.y * s, p.x * s + p.y * c);
}

fn dir_sign(dir: u32) -> vec2<f32> {
    if dir == 1u {
        return vec2<f32>(-1.0, 0.0);
    }
    if dir == 2u {
        return vec2<f32>(0.0, 1.0);
    }
    if dir == 3u {
        return vec2<f32>(0.0, -1.0);
    }
    return vec2<f32>(1.0, 0.0);
}

fn wipe_axis(uv: vec2<f32>, t: f32, dir: u32, softness: f32) -> f32 {
    var coord = uv.x;
    if dir == 1u {
        coord = 1.0 - uv.x;
    } else if dir == 2u {
        coord = uv.y;
    } else if dir == 3u {
        coord = 1.0 - uv.y;
    }
    let edge = t;
    let s = max(softness, 0.001);
    return smoothstep(edge - s, edge + s, coord);
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

fn axis_coord(uv: vec2<f32>, dir: u32) -> f32 {
    if dir == 1u {
        return 1.0 - uv.x;
    }
    if dir == 2u {
        return uv.y;
    }
    if dir == 3u {
        return 1.0 - uv.y;
    }
    return uv.x;
}

fn zoom_uv(uv: vec2<f32>, amount: f32) -> vec2<f32> {
    return (uv - vec2<f32>(0.5)) / amount + vec2<f32>(0.5);
}

fn cube_face_uv(face_u: f32, other: f32, persp: f32, hinge: f32) -> vec2<f32> {
    let v = (other - 0.5) * mix(1.0, persp, hinge) + 0.5;
    return clamp(vec2<f32>(face_u, v), vec2<f32>(0.0), vec2<f32>(1.0));
}

fn cube_mix(uv: vec2<f32>, t: f32, dir: u32, zoom: f32) -> vec4<f32> {
    let theta = clamp(t, 0.0, 1.0) * PI * 0.5;
    let ca = cos(theta);
    let sb = sin(theta);
    let split = ca / max(ca + sb, 0.001);
    var coord = uv.x;
    var other = uv.y;
    let vert = dir == 2u || dir == 3u;
    if vert {
        coord = uv.y;
        other = uv.x;
    }
    if dir == 1u || dir == 3u {
        coord = 1.0 - coord;
    }
    var su = uv;
    var incoming = false;
    var shade = 1.0;
    if split >= 0.999 || (coord < split && split > 0.001) {
        let u = coord / max(split, 0.001);
        let mapped = cube_face_uv(u, other, mix(1.0, 1.22, 1.0 - ca), u);
        su = mapped;
        shade = 0.74 + 0.26 * ca;
    } else {
        let u = (coord - split) / max(1.0 - split, 0.001);
        let mapped = cube_face_uv(u, other, mix(1.22, 1.0, sb), 1.0 - u);
        su = mapped;
        incoming = true;
        shade = 0.74 + 0.26 * sb;
    }
    if vert {
        su = vec2<f32>(su.y, su.x);
    }
    if dir == 1u {
        su.x = 1.0 - su.x;
    }
    if dir == 3u {
        su.y = 1.0 - su.y;
    }
    su = clamp(zoom_uv(su, max(zoom, 0.08)), vec2<f32>(0.0), vec2<f32>(1.0));
    var color = select(sample_pgm(su), sample_pvw(su), incoming);
    return vec4<f32>(color.rgb * shade, color.a);
}

fn flip_mix(uv: vec2<f32>, t: f32, dir: u32) -> vec4<f32> {
    let ang = t * PI;
    let c = cos(ang);
    let shade = 0.65 + 0.35 * abs(c);
    var mapped = uv;
    if dir == 2u || dir == 3u {
        mapped.y = (uv.y - 0.5) / max(abs(c), 0.04) + 0.5;
    } else {
        mapped.x = (uv.x - 0.5) / max(abs(c), 0.04) + 0.5;
    }
    if !in_bounds(mapped) {
        return vec4<f32>(0.0);
    }
    var color = select(sample_pgm(mapped), sample_pvw(mapped), c < 0.0);
    if c < 0.0 {
        if dir == 0u || dir == 1u {
            color = sample_pvw(vec2<f32>(1.0 - mapped.x, mapped.y));
        } else {
            color = sample_pvw(vec2<f32>(mapped.x, 1.0 - mapped.y));
        }
    }
    return vec4<f32>(color.rgb * shade, color.a);
}

fn sd_heart(p0: vec2<f32>) -> f32 {
    var p = p0;
    p.x = abs(p.x);
    if p.y + p.x > 1.0 {
        return length(p - vec2<f32>(0.25, 0.75)) - 0.35355339;
    }
    let d1 = dot(p - vec2<f32>(0.0, 1.0), p - vec2<f32>(0.0, 1.0));
    let h = max(p.x + p.y, 0.0);
    let q = p - vec2<f32>(0.5 * h, 0.5 * h);
    let d2 = dot(q, q);
    return sqrt(min(d1, d2)) * sign(p.x - p.y);
}

fn sd_star(p0: vec2<f32>, radius: f32, points: f32, m: f32) -> f32 {
    let an = PI / points;
    let en = PI / m;
    let acs = vec2<f32>(cos(an), sin(an));
    let ecs = vec2<f32>(cos(en), sin(en));
    let bn = (fract(atan2(p0.x, p0.y) / (2.0 * an)) - 0.5) * 2.0 * an;
    var p = length(p0) * vec2<f32>(cos(bn), abs(sin(bn)));
    p = p - radius * acs;
    p = p + ecs * clamp(-dot(p, ecs), 0.0, radius * acs.y / ecs.y);
    return length(p) * sign(p.x);
}

fn shape_wipe(uv: vec2<f32>, t: f32, shape: u32) -> vec4<f32> {
    let a = sample_pgm(uv);
    let b = sample_pvw(uv);
    let s = max(soft(), 0.001);
    let grow = t * t * (3.0 - 2.0 * t);
    let p = vec2<f32>((uv.x - 0.5) * 2.0, -(uv.y - 0.5) * 2.0);
    var d = 0.0;
    if shape == 0u {
        let scale = mix(0.05, 2.4, grow);
        d = sd_heart(p / max(scale, 0.001) + vec2<f32>(0.0, 0.55));
    } else if shape == 1u {
        d = (abs(p.x) + abs(p.y)) - mix(-0.08, 2.2, grow);
    } else {
        let scale = mix(0.06, 2.5, grow);
        d = sd_star(p / max(scale, 0.001), 0.85, 5.0, 2.6);
    }
    let w = smoothstep(-s, s, d);
    return mix(mix(b, a, w), b, smoothstep(0.9, 1.0, t));
}

fn card_local_uv(uv: vec2<f32>, offset: vec2<f32>, angle: f32, scale: f32) -> vec2<f32> {
    let p = uv - vec2<f32>(0.5) - offset;
    return rotate2(p, -angle) / max(scale, 0.001) + vec2<f32>(0.5);
}

fn fly_rotate(uv: vec2<f32>, t: f32, dir: u32) -> vec4<f32> {
    let spins = pval(1.0);
    let flip = select(-1.0, 1.0, dir == 1u || dir == 3u);
    let travel = dir_sign(dir);
    let ease = t * t * (3.0 - 2.0 * t);
    let scale_b = mix(0.2, 1.0, ease);
    let ang_b = spins * (1.0 - ease) * PI;
    let off_b = travel * mix(0.92, 0.0, ease);
    let uv_b = card_local_uv(uv, off_b, ang_b * flip, scale_b);
    let scale_a = mix(1.0, 0.32, ease);
    let ang_a = spins * ease * PI * 0.45;
    let off_a = -travel * mix(0.0, 0.92, ease);
    let uv_a = card_local_uv(uv, off_a, -ang_a * flip, scale_a);
    if in_bounds(uv_b) {
        return sample_pvw(uv_b);
    }
    if in_bounds(uv_a) {
        return sample_pgm(uv_a);
    }
    return vec4<f32>(0.0, 0.0, 0.0, 1.0);
}

fn lorez_mix(uv: vec2<f32>, t: f32) -> vec4<f32> {
    let peak = 1.0 - abs(t * 2.0 - 1.0);
    let res_x = max(params.resolution.x, 1280.0);
    let max_cell = mix(1.0 / 36.0, 1.0 / 8.0, clamp(params.softness * 16.0, 0.0, 1.0));
    let cell = mix(1.0 / res_x, max_cell, pow(max(peak, 0.0), 0.65));
    let q = (floor(uv / max(cell, 0.0005)) + 0.5) * cell;
    let pix = mix(sample_pgm(q), sample_pvw(q), smoothstep(0.38, 0.62, t));
    let sharp = mix(sample_pgm(uv), sample_pvw(uv), smoothstep(0.45, 0.55, t));
    return mix(sharp, pix, peak);
}

fn metamix(uv: vec2<f32>, t: f32) -> vec4<f32> {
    let copies = max(pval(8.0), 2.0);
    let depth = 1.0 - abs(t * 2.0 - 1.0);
    let z = pow(copies, depth);
    let tile = fract((uv - vec2<f32>(0.5)) * z + vec2<f32>(0.5));
    return mix(sample_pgm(tile), sample_pvw(tile), smoothstep(0.45, 0.55, t));
}

fn tile_mix(uv: vec2<f32>, t: f32, dir: u32) -> vec4<f32> {
    let n = max(pval(8.0), 2.0);
    let cell = floor(uv * n);
    let local = fract(uv * n);
    var idx = cell.x + cell.y * n;
    if dir == 1u {
        idx = (n - 1.0 - cell.x) + cell.y * n;
    } else if dir == 2u {
        idx = cell.y + cell.x * n;
    } else if dir == 3u {
        idx = (n - 1.0 - cell.y) + cell.x * n;
    }
    let copies_amt = smoothstep(0.0, 0.42, t);
    let a_uv = mix(uv, local, copies_amt);
    let thresh = 0.22 + 0.78 * idx / max(n * n, 1.0);
    let reveal = smoothstep(thresh, min(thresh + 0.18, 1.0), t);
    return mix(sample_pgm(a_uv), sample_pvw(uv), reveal);
}

fn parts_mix(uv: vec2<f32>, t: f32, dir: u32) -> vec4<f32> {
    let n = max(pval(6.0), 2.0);
    let cell = floor(uv * n);
    let local = fract(uv * n);
    let order = hash21(cell + vec2<f32>(f32(dir) + 1.3, 4.7));
    let eaten = smoothstep(order, min(order + 0.24, 1.0), t);
    let shrink = max(1.0 - eaten, 0.001);
    let local2 = (local - vec2<f32>(0.5)) / shrink + vec2<f32>(0.5);
    if eaten >= 0.995 || !in_bounds(local2) {
        return sample_pvw(uv);
    }
    return sample_pgm((cell + local2) / n);
}

fn swirl_mix(uv: vec2<f32>, t: f32, dir: u32) -> vec4<f32> {
    let turns = pval(2.0);
    let flip = select(1.0, -1.0, dir == 1u || dir == 3u);
    let p = uv - vec2<f32>(0.5);
    let falloff = 1.0 - length(p);
    let swirl_a = t * turns * 2.0 * PI * falloff * flip;
    let swirl_b = (t - 1.0) * turns * 2.0 * PI * falloff * flip;
    return mix(
        sample_pgm(rotate2(p, swirl_a) + vec2<f32>(0.5)),
        sample_pvw(rotate2(p, swirl_b) + vec2<f32>(0.5)),
        t
    );
}

fn multitask_mix(uv: vec2<f32>, t: f32, dir: u32) -> vec4<f32> {
    let flip = select(1.0, -1.0, dir == 1u || dir == 3u);
    let vert = dir == 2u || dir == 3u;
    let a_c = mix(0.5, 0.5 - 0.72 * flip, t);
    let b_c = mix(0.5 + 0.72 * flip, 0.5, t);
    let a_w = mix(1.0, 0.38, t);
    let b_w = mix(0.38, 1.0, t);
    let a_h = mix(1.0, 0.68, t);
    let b_h = mix(0.68, 1.0, t);
    var coord = uv.x;
    var other = uv.y;
    if vert {
        coord = uv.y;
        other = uv.x;
    }
    let la_axis = (coord - (a_c - a_w * 0.5)) / max(a_w, 0.001);
    let lb_axis = (coord - (b_c - b_w * 0.5)) / max(b_w, 0.001);
    let la_o = (other - 0.5) / max(a_h, 0.001) + 0.5;
    let lb_o = (other - 0.5) / max(b_h, 0.001) + 0.5;
    var la = vec2<f32>(la_axis, la_o);
    var lb = vec2<f32>(lb_axis, lb_o);
    if vert {
        la = vec2<f32>(la_o, la_axis);
        lb = vec2<f32>(lb_o, lb_axis);
    }
    let a_ok = in_bounds(la);
    let b_ok = in_bounds(lb);
    if b_ok && (t > 0.48 || !a_ok) {
        let color = sample_pvw(lb);
        let shade = mix(0.78, 1.0, t);
        return vec4<f32>(color.rgb * shade, color.a);
    }
    if a_ok {
        let color = sample_pgm(la);
        let shade = mix(1.0, 0.78, t);
        return vec4<f32>(color.rgb * shade, color.a);
    }
    return vec4<f32>(0.0, 0.0, 0.0, 1.0);
}

fn page_curl(uv: vec2<f32>, t: f32, dir: u32) -> vec4<f32> {
    var u = uv;
    if dir == 1u {
        u.x = 1.0 - u.x;
    } else if dir == 2u {
        u = vec2<f32>(uv.y, 1.0 - uv.x);
    } else if dir == 3u {
        u = vec2<f32>(1.0 - uv.y, uv.x);
    }
    let radius = 0.03 + 0.15 * sin(t * PI);
    let curl = mix(1.0 + radius, -radius, t);
    let x = u.x - curl;
    let a = sample_pgm(uv);
    let b = sample_pvw(uv);
    if x <= -radius {
        return a;
    }
    let shadow = (1.0 - smoothstep(radius, radius + 0.1, x)) * 0.28 * smoothstep(0.0, 0.08, t);
    if x >= radius {
        return vec4<f32>(b.rgb * (1.0 - shadow), b.a);
    }
    let theta = clamp(x / max(radius, 0.001), -1.0, 1.0) * PI * 0.5;
    let mapped = vec2<f32>(curl + sin(theta) * radius, u.y);
    var src_uv = uv;
    if dir == 0u {
        src_uv.x = mapped.x;
    } else if dir == 1u {
        src_uv.x = 1.0 - mapped.x;
    } else if dir == 2u {
        src_uv = vec2<f32>(1.0 - mapped.y, mapped.x);
    } else {
        src_uv = vec2<f32>(mapped.y, 1.0 - mapped.x);
    }
    let shade = 0.52 + 0.48 * cos(theta);
    let paper = sample_pgm(clamp(src_uv, vec2<f32>(0.0), vec2<f32>(1.0))) * shade;
    let show_paper = smoothstep(radius, radius * 0.15, x);
    return mix(vec4<f32>(b.rgb * (1.0 - shadow), b.a), paper, show_paper);
}

fn axis_offset(amount: f32, dir: u32) -> vec2<f32> {
    return dir_sign(dir) * amount;
}

fn load_bus(p: vec2<i32>, dims: vec2<i32>, incoming: bool) -> vec4<f32> {
    let q = clamp(p, vec2<i32>(0), dims - vec2<i32>(1));
    if incoming {
        return textureLoad(pvw_tex, q, 0);
    }
    return textureLoad(pgm_tex, q, 0);
}

fn sort_luma(colors: ptr<function, array<vec4<f32>, 25>>, n: i32, descending: bool) {
    for (var i = 1; i < 25; i = i + 1) {
        if i >= n {
            break;
        }
        let key = (*colors)[i];
        let key_l = luma(key);
        var j = i - 1;
        loop {
            if j < 0 {
                break;
            }
            let left = luma((*colors)[j]);
            let should_shift = select(left > key_l, left < key_l, descending);
            if !should_shift {
                break;
            }
            (*colors)[j + 1] = (*colors)[j];
            j = j - 1;
        }
        (*colors)[j + 1] = key;
    }
}

fn pixel_sort(uv: vec2<f32>, t: f32, dir: u32) -> vec4<f32> {
    let dims = vec2<i32>(textureDimensions(pgm_tex));
    let p = vec2<i32>(
        i32(clamp(uv.x, 0.0, 0.99999) * f32(dims.x)),
        i32(clamp(uv.y, 0.0, 0.99999) * f32(dims.y))
    );
    let a = load_bus(p, dims, false);
    let b = load_bus(p, dims, true);
    if t <= 0.001 {
        return a;
    }
    if t >= 0.999 {
        return b;
    }
    let horiz = dir == 0u || dir == 1u;
    let descending = dir == 1u || dir == 3u;
    let thresh = max(params.softness, 0.001);
    let step = max(1, i32(round(1.0 + 4.0 * clamp(params.param, 0.0, 1.0))));
    let axis = select(p.x, p.y, !horiz);
    let axis_max = select(dims.x, dims.y, !horiz) - 1;
    if luma(a) < thresh {
        if t < 0.9 {
            return a;
        }
        return b;
    }
    var colors_a: array<vec4<f32>, 25>;
    var colors_b: array<vec4<f32>, 25>;
    colors_a[0] = a;
    colors_b[0] = b;
    var n = 1;
    var start = axis;
    var end = axis;
    for (var i = 1; i <= 12; i = i + 1) {
        let coord = axis - i * step;
        if coord < 0 {
            break;
        }
        var q = p;
        if horiz {
            q.x = coord;
        } else {
            q.y = coord;
        }
        let ca = load_bus(q, dims, false);
        if luma(ca) < thresh {
            break;
        }
        start = coord;
        colors_a[n] = ca;
        colors_b[n] = load_bus(q, dims, true);
        n = n + 1;
    }
    for (var i = 1; i <= 12; i = i + 1) {
        let coord = axis + i * step;
        if coord > axis_max {
            break;
        }
        var q = p;
        if horiz {
            q.x = coord;
        } else {
            q.y = coord;
        }
        let ca = load_bus(q, dims, false);
        if luma(ca) < thresh {
            break;
        }
        end = coord;
        colors_a[n] = ca;
        colors_b[n] = load_bus(q, dims, true);
        n = n + 1;
    }
    sort_luma(&colors_a, n, descending);
    sort_luma(&colors_b, n, descending);
    let local = clamp(f32(axis - start) / f32(max(end - start, 1)), 0.0, 1.0);
    let idx = clamp(i32(local * f32(n - 1) + 0.5), 0, n - 1);
    let sorted_a = colors_a[idx];
    let sorted_b = colors_b[idx];
    let show_a = mix(a, sorted_a, smoothstep(0.0, 0.28, t));
    let show_b = mix(sorted_b, b, smoothstep(0.72, 1.0, t));
    if local < smoothstep(0.24, 0.76, t) {
        return show_b;
    }
    return show_a;
}

fn datamosh(uv: vec2<f32>, t: f32, dir: u32) -> vec4<f32> {
    let peak = pow(max(1.0 - abs(t * 2.0 - 1.0), 0.0), 0.45);
    let chaos = peak * pval(1.0);
    let blocks = mix(48.0, 16.0, chaos);
    let cell = floor(uv * vec2<f32>(blocks, blocks * 0.56));
    let tick = floor(params.time * 10.0);
    let h = hash21(cell + vec2<f32>(tick * 0.15, 1.7));
    let h2 = hash21(cell.yx + vec2<f32>(3.1, 4.4));
    var flow = vec2<f32>((h - 0.5) * 0.14, (h2 - 0.5) * 0.06) * mix(0.15, 1.0, chaos);
    if dir == 2u || dir == 3u {
        flow = flow.yx;
    }
    if dir == 1u || dir == 3u {
        flow = -flow;
    }
    let slide_a = axis_offset(0.12 * t, dir);
    let slide_b = axis_offset(-0.12 * (1.0 - t), dir);
    let uv_a = uv + flow * t + slide_a;
    let uv_b = uv - flow * (1.0 - t) + slide_b;
    let ca = sample_pgm(uv_a);
    let cb = sample_pvw(uv_b);
    let held = sample_prev(uv + flow + slide_a * 0.35);
    let q = (cell + vec2<f32>(0.5)) / vec2<f32>(blocks, blocks * 0.56);
    let blocky = sample_prev_n(q + flow * 0.5);
    let moshed_a = mix(ca, mix(held, blocky, 0.4), chaos * 0.72);
    let torn_a = vec4<f32>(moshed_a.r, mix(moshed_a.g, ca.g, 0.22), moshed_a.b, 1.0);
    let torn_b = vec4<f32>(cb.r, mix(cb.g, sample_pvw(uv_b + flow * 0.25).g, 0.2), cb.b, 1.0);
    let reveal = smoothstep(h * 0.7, h * 0.7 + 0.28, t);
    return mix(torn_a, torn_b, reveal);
}

fn visual_dissolve(uv: vec2<f32>, t: f32) -> vec4<f32> {
    let n0 = hash21(uv * 14.0);
    let n1 = hash21(uv * 6.5 + vec2<f32>(2.3, 0.9));
    let n2 = hash21(floor(uv * 28.0));
    let field = n0 * 0.42 + n1 * 0.38 + n2 * 0.2;
    let gx = hash21(uv * 14.0 + vec2<f32>(0.04, 0.0)) - n0;
    let gy = hash21(uv * 14.0 + vec2<f32>(0.0, 0.04)) - n0;
    let flow = vec2<f32>(gy, -gx) * pval(0.42);
    let s = max(soft(), 0.04);
    let mask = smoothstep(t - s, t + s, field);
    let front = 1.0 - smoothstep(0.0, 0.16, abs(field - t));
    let uv_a = uv + flow * t * (0.55 + front * 0.7);
    let uv_b = uv - flow * (1.0 - t) * (0.55 + front * 0.7);
    let ca = sample_pgm(uv_a);
    let cb = sample_pvw(uv_b);
    let fringe = vec4<f32>(
        sample_pgm(uv_a + flow * 0.03).r,
        ca.g,
        sample_pvw(uv_b - flow * 0.03).b,
        1.0
    );
    let body = mix(cb, ca, mask);
    return mix(body, fringe, front * 0.55);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let a = sample_pgm(in.uv);
    let b = sample_pvw(in.uv);
    let t = params.mix;
    let kind = params.kind;
    let dir = params.direction;
    let s = soft();

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
        return mix(b, a, wipe_axis(in.uv, t, dir, s));
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
        let w = smoothstep(radius - s, radius + s, d);
        return mix(b, a, w);
    }
    if kind == 7u {
        let strips = pval(12.0);
        let coord = select(in.uv.x, in.uv.y, dir != 0u);
        let cell = fract(coord * strips);
        let w = smoothstep(t - s, t + s, cell);
        return mix(b, a, w);
    }
    if kind == 8u {
        let zoom_a = mix(1.0, 1.35, t);
        let zoom_b = mix(1.35, 1.0, t);
        return mix(sample_pgm(zoom_uv(in.uv, zoom_a)), sample_pvw(zoom_uv(in.uv, zoom_b)), t);
    }
    if kind == 9u {
        let add = min(a + b * t, vec4<f32>(1.0));
        return mix(a, add, t);
    }
    if kind == 10u {
        return cube_mix(in.uv, t, dir, 1.0);
    }
    if kind == 11u {
        let zoom_a = mix(1.0, 2.6, t);
        let zoom_b = mix(2.6, 1.0, t);
        let fade = smoothstep(0.18, 0.82, t);
        return mix(sample_pgm(zoom_uv(in.uv, zoom_a)), sample_pvw(zoom_uv(in.uv, zoom_b)), fade);
    }
    if kind == 12u {
        return fly_rotate(in.uv, t, dir);
    }
    if kind == 13u {
        let coord = axis_coord(in.uv, dir);
        let d = abs(coord - 0.5);
        let w = smoothstep(t * 0.5 - s, t * 0.5 + s, d);
        return mix(b, a, w);
    }
    if kind == 14u {
        let p = in.uv - vec2<f32>(0.5);
        var ang = atan2(p.y, p.x);
        if dir == 1u || dir == 3u {
            ang = -ang;
        }
        let n = fract(ang / (2.0 * PI) + 0.25);
        let w = smoothstep(t - s, t + s, n);
        return mix(b, a, w);
    }
    if kind == 15u {
        return lorez_mix(in.uv, t);
    }
    if kind == 16u {
        return metamix(in.uv, t);
    }
    if kind == 17u {
        return tile_mix(in.uv, t, dir);
    }
    if kind == 18u {
        return flip_mix(in.uv, t, dir);
    }
    if kind == 19u {
        let peak = 1.0 - abs(t * 2.0 - 1.0);
        let amp = pval(0.16) * peak;
        let row = floor(in.uv.y * 28.0);
        let h = hash21(vec2<f32>(row, floor(t * 18.0)));
        let shift = (h - 0.5) * amp;
        let band = step(0.55, hash21(vec2<f32>(row + 3.0, floor(t * 11.0))));
        let uv_g = in.uv + vec2<f32>(shift * band, 0.0);
        let split = amp * 0.45;
        let r = sample_pgm(uv_g + vec2<f32>(split, 0.0)).r;
        let g = sample_pgm(uv_g).g;
        let bl = sample_pgm(uv_g - vec2<f32>(split, 0.0)).b;
        let glitch = vec4<f32>(r, g, bl, 1.0);
        let dest = sample_pvw(uv_g);
        let tint = mix(glitch, params.dip, 0.12 * peak);
        return mix(tint, dest, smoothstep(0.42, 0.88, t));
    }
    if kind == 20u {
        return swirl_mix(in.uv, t, dir);
    }
    if kind == 21u {
        let w = smoothstep(t - s, t + s, luma(a));
        return mix(b, a, w);
    }
    if kind == 22u {
        return parts_mix(in.uv, t, dir);
    }
    if kind == 23u {
        let peak = 1.0 - abs(t * 2.0 - 1.0);
        let nse = hash21(in.uv * 380.0 + vec2<f32>(t * 40.0, t * 17.0));
        let grain = mix(a, vec4<f32>(nse, nse, nse, 1.0), peak * pval(1.0));
        return mix(grain, b, smoothstep(0.35 - s, 0.72 + s, t));
    }
    if kind == 24u {
        return vec4<f32>(
            mix(a.r, b.r, smoothstep(0.0, 0.38, t)),
            mix(a.g, b.g, smoothstep(0.28, 0.7, t)),
            mix(a.b, b.b, smoothstep(0.58, 1.0, t)),
            1.0
        );
    }
    if kind == 25u {
        let disp = (luma(a) - 0.5) * (1.0 - t) * pval(0.18);
        return mix(a, sample_pvw(in.uv + vec2<f32>(disp, disp * 0.35)), t);
    }
    if kind == 26u {
        let p = in.uv - vec2<f32>(0.5);
        let r = length(p);
        let wave = sin((r - t) * 36.0) * (1.0 - t) * pval(1.0);
        let dirn = p / max(r, 0.001);
        let uv_a = in.uv + dirn * wave * 0.04;
        return mix(sample_pgm(uv_a), b, t);
    }
    if kind == 27u {
        let n = pval(12.0);
        let cell = floor(in.uv * n);
        let h = hash21(cell);
        return mix(a, b, smoothstep(h - s, h + s, t));
    }
    if kind == 28u {
        let zoom = mix(1.0, 1.28, sin(t * PI));
        return cube_mix(in.uv, t, dir, zoom);
    }
    if kind == 29u {
        return page_curl(in.uv, t, dir);
    }
    if kind == 30u {
        let segs = pval(6.0);
        let p = in.uv - vec2<f32>(0.5);
        let slice = (2.0 * PI) / max(segs, 2.0);
        let ang = atan2(p.y, p.x);
        let a2 = abs(fract(ang / slice) * slice - slice * 0.5);
        let uv_k = vec2<f32>(cos(a2), sin(a2)) * length(p) + vec2<f32>(0.5);
        return mix(sample_pgm(uv_k), b, t);
    }
    if kind == 31u {
        let p = in.uv - vec2<f32>(0.5);
        let suck = mix(1.0, 0.12, t);
        let launch = mix(2.4, 1.0, t);
        let uv_a = p / suck + vec2<f32>(0.5);
        var uv_b = p * launch + vec2<f32>(0.5);
        if dir == 1u {
            uv_b.x = 1.0 - uv_b.x;
        } else if dir == 2u {
            uv_b = vec2<f32>(uv_b.y, 1.0 - uv_b.x);
        } else if dir == 3u {
            uv_b = vec2<f32>(1.0 - uv_b.y, uv_b.x);
        }
        return mix(clamp_sample_pgm(uv_a), clamp_sample_pvw(uv_b), t);
    }
    if kind == 32u {
        let nse = hash21(in.uv * 18.0 + vec2<f32>(t * 6.0, 1.7));
        let edge = in.uv.x + nse * 0.22 - 0.08;
        let w = smoothstep(t - s - 0.08, t + s, edge);
        let glow = smoothstep(0.0, 0.12, abs(edge - t));
        let burn = mix(a, params.dip, (1.0 - glow) * pval(1.0));
        return mix(b, burn, w);
    }
    if kind == 33u {
        let center = vec2<f32>(0.5);
        let peak = 1.0 - abs(t * 2.0 - 1.0);
        let amp = pval(0.55) * peak;
        var acc = vec4<f32>(0.0);
        for (var i = 0; i < 8; i = i + 1) {
            let k = 1.0 + amp * f32(i) / 7.0;
            let uv_s = (in.uv - center) / k + center;
            acc += mix(sample_pgm(uv_s), sample_pvw(uv_s), t);
        }
        return acc / 8.0;
    }
    if kind == 34u {
        return multitask_mix(in.uv, t, dir);
    }
    if kind == 35u {
        return shape_wipe(in.uv, t, 0u);
    }
    if kind == 36u {
        return shape_wipe(in.uv, t, 1u);
    }
    if kind == 37u {
        return shape_wipe(in.uv, t, 2u);
    }
    if kind == 38u {
        let coord = axis_coord(in.uv, dir);
        let edge = 1.0 - t;
        let curl = smoothstep(edge, edge + 0.08, coord);
        if dir == 0u || dir == 1u {
            let wuv = vec2<f32>(in.uv.x, in.uv.y + (1.0 - curl) * 0.04);
            return mix(sample_pgm(wuv), b, step(edge, coord));
        }
        let wuv = vec2<f32>(in.uv.x + (1.0 - curl) * 0.04, in.uv.y);
        return mix(sample_pgm(wuv), b, step(edge, coord));
    }
    if kind == 39u {
        return pixel_sort(in.uv, t, dir);
    }
    if kind == 40u {
        return datamosh(in.uv, t, dir);
    }
    if kind == 41u {
        return visual_dissolve(in.uv, t);
    }
    return mix(a, b, t);
}
