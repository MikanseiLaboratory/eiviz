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
    _pad: vec3<f32>,
}

@group(0) @binding(0) var pgm_tex: texture_2d<f32>;
@group(0) @binding(1) var pvw_tex: texture_2d<f32>;
@group(0) @binding(2) var src_samp: sampler;
@group(0) @binding(3) var<uniform> params: MixParams;

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

fn cube_uv(uv: vec2<f32>, t: f32, dir: u32, incoming: bool) -> vec2<f32> {
    let ang = t * PI * 0.5;
    let z = select(cos(ang), sin(ang), incoming);
    let slide = select(-sin(ang), 1.0 - cos(ang), incoming);
    let p = uv - vec2<f32>(0.5);
    let persp = 1.0 / max(z, 0.08);
    if dir == 2u || dir == 3u {
        let sign = select(1.0, -1.0, dir == 3u);
        return vec2<f32>(p.x * persp, p.y * persp + slide * sign) + vec2<f32>(0.5);
    }
    let sign = select(1.0, -1.0, dir == 1u);
    return vec2<f32>(p.x * persp + slide * sign, p.y * persp) + vec2<f32>(0.5);
}

fn cube_mix(uv: vec2<f32>, t: f32, dir: u32, zoom: f32) -> vec4<f32> {
    let uv_a = zoom_uv(cube_uv(uv, t, dir, false), zoom);
    let uv_b = zoom_uv(cube_uv(uv, t, dir, true), zoom);
    let a_ok = in_bounds(uv_a);
    let b_ok = in_bounds(uv_b);
    if b_ok && (!a_ok || t > 0.5) {
        return sample_pvw(uv_b);
    }
    if a_ok {
        return sample_pgm(uv_a);
    }
    if b_ok {
        return sample_pvw(uv_b);
    }
    return vec4<f32>(0.0);
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
    p.y = -p.y;
    p.y -= 0.3;
    let xx = p.x * p.x;
    return pow(xx + p.y * p.y - 1.0, 3.0) - xx * p.y * p.y * p.y;
}

fn sd_star(p: vec2<f32>, n: f32, rf: f32) -> f32 {
    let an = PI / n;
    let m = min(fract(atan2(p.y, p.x) / (2.0 * an) + 1.0), 2.0 - fract(atan2(p.y, p.x) / (2.0 * an) + 1.0));
    let en = vec2<f32>(cos(an), sin(an));
    let q = vec2<f32>(length(p) * cos(m * an), length(p) * sin(m * an));
    return length(q - en * clamp(dot(q, en), 0.0, rf)) * sign(q.y * en.x - q.x * en.y);
}

fn shape_wipe(uv: vec2<f32>, t: f32, shape: u32) -> vec4<f32> {
    let a = sample_pgm(uv);
    let b = sample_pvw(uv);
    let s = soft();
    let p = (uv - vec2<f32>(0.5)) * 2.0;
    var d = 0.0;
    if shape == 0u {
        d = sd_heart(p * mix(4.2, 0.55, t));
    } else if shape == 1u {
        d = (abs(p.x) + abs(p.y)) - mix(-0.05, 1.55, t);
    } else {
        d = sd_star(p * mix(3.4, 0.35, t), 5.0, 0.45);
    }
    let w = smoothstep(-s, s, d);
    return mix(b, a, w);
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
    let curl = 1.0 - t;
    let radius = 0.18;
    let x = u.x - curl;
    if x < -radius {
        return sample_pgm(uv);
    }
    if x > radius {
        return sample_pvw(uv);
    }
    let theta = clamp(x / radius, -1.0, 1.0) * PI * 0.5;
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
    let shade = 0.55 + 0.45 * cos(theta);
    let paper = sample_pgm(src_uv) * shade;
    let reveal = sample_pvw(uv);
    return mix(reveal, paper, smoothstep(radius, -radius * 0.2, x));
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
        let spin = pval(1.0) * (1.0 - t) * 1.15;
        let sign = select(-1.0, 1.0, dir == 1u || dir == 3u);
        let scale = mix(0.18, 1.0, t);
        let travel = dir_sign(dir) * (1.0 - t);
        let uv_b = rotate2((in.uv - vec2<f32>(0.5)) / scale, spin * sign) + vec2<f32>(0.5) - travel;
        let uv_a = rotate2(in.uv - vec2<f32>(0.5), -spin * 0.25 * sign) + vec2<f32>(0.5) + travel * 0.15;
        if in_bounds(uv_b) {
            return sample_pvw(uv_b);
        }
        return sample_pgm(uv_a);
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
        let peak = 1.0 - abs(t * 2.0 - 1.0);
        let cell = max(mix(0.002, max(params.softness, 0.01) * 12.0, peak), 0.002);
        let q = floor(in.uv / cell) * cell + cell * 0.5;
        if t < 0.5 {
            return sample_pgm(q);
        }
        return sample_pvw(q);
    }
    if kind == 16u {
        let copies = pval(6.0);
        let n = mix(1.0, copies, t);
        let tile = fract(in.uv * n);
        let src = mix(sample_pgm(tile), sample_pvw(tile), smoothstep(0.35, 0.75, t));
        return mix(src, b, smoothstep(0.72, 1.0, t));
    }
    if kind == 17u {
        let n = pval(8.0);
        let cell = floor(in.uv * n);
        var idx = cell.x + cell.y * n;
        if dir == 1u {
            idx = (n - 1.0 - cell.x) + cell.y * n;
        } else if dir == 2u {
            idx = cell.y + cell.x * n;
        } else if dir == 3u {
            idx = (n - 1.0 - cell.y) + cell.x * n;
        }
        let thresh = idx / max(n * n, 1.0);
        let w = step(thresh, t);
        return mix(a, b, w);
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
        let turns = pval(2.0);
        let sign = select(1.0, -1.0, dir == 1u || dir == 3u);
        let p = in.uv - vec2<f32>(0.5);
        let r = length(p);
        let swirl = (1.0 - t) * turns * 2.0 * PI * (1.0 - r) * sign;
        let uv_a = rotate2(p, swirl) + vec2<f32>(0.5);
        return mix(sample_pgm(uv_a), b, t);
    }
    if kind == 21u {
        let w = smoothstep(t - s, t + s, luma(a));
        return mix(b, a, w);
    }
    if kind == 22u {
        let n = pval(6.0);
        let cell = floor(in.uv * n);
        var order = (cell.x + cell.y * n) / max(n * n, 1.0);
        if dir != 0u {
            order = hash21(cell + vec2<f32>(f32(dir), 2.0));
        }
        return mix(a, b, step(order, t));
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
        let sign = select(1.0, -1.0, dir == 1u || dir == 3u);
        let old_x = (in.uv.x - 0.5) / mix(1.0, 0.62, t) + 0.5 + 0.38 * t * sign;
        let new_x = (in.uv.x - 0.5) / mix(0.62, 1.0, t) + 0.5 - 0.38 * (1.0 - t) * sign;
        let uv_a = vec2<f32>(old_x, in.uv.y);
        let uv_b = vec2<f32>(new_x, in.uv.y);
        if in_bounds(uv_b) && (t > 0.42 || !in_bounds(uv_a)) {
            return sample_pvw(uv_b);
        }
        if in_bounds(uv_a) {
            return sample_pgm(uv_a) * mix(1.0, 0.7, t);
        }
        return vec4<f32>(0.0);
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
    return mix(a, b, t);
}
