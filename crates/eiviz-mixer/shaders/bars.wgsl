struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

struct ColorParams {
    color: vec4<f32>,
    scroll: f32,
    flags: f32,
    scroll_y: f32,
    pad: f32,
}

@group(0) @binding(0) var<uniform> params: ColorParams;

const WIDTH: f32 = 1920.0;
const HEIGHT: f32 = 1080.0;

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

fn ycbcr_to_rgb(ycbcr: vec3<f32>) -> vec4<f32> {
    let y = (ycbcr.x - 16.0) / 219.0;
    let cb = (ycbcr.y - 128.0) / 224.0;
    let cr = (ycbcr.z - 128.0) / 224.0;
    let r = y + 1.5748 * cr;
    let g = y - 0.1873 * cb - 0.4681 * cr;
    let b = y + 1.8556 * cb;
    return vec4<f32>(clamp(r, 0.0, 1.0), clamp(g, 0.0, 1.0), clamp(b, 0.0, 1.0), 1.0);
}

fn hd_yuv(x: f32, y: f32) -> vec3<f32> {
    let d_w = max(floor(WIDTH / 8.0), 1.0);
    let r_w = max(floor(floor((WIDTH + 3.0) / 4.0) * 3.0 / 7.0), 1.0);
    let p1_h = max(floor(HEIGHT * 7.0 / 12.0), 1.0);
    let strip_h = max(floor(HEIGHT / 12.0), 1.0);
    let p2_y0 = p1_h;
    let p3_y0 = p1_h + strip_h;
    let p4_y0 = p1_h + 2.0 * strip_h;
    let l_w = d_w + 7.0 * r_w;

    if y < p2_y0 {
        if x < d_w {
            return vec3<f32>(104.0, 128.0, 128.0);
        }
        let cx = x - d_w;
        if cx < 7.0 * r_w {
            let idx = u32(min(floor(cx / r_w), 6.0));
            switch idx {
                case 0u: { return vec3<f32>(180.0, 128.0, 128.0); }
                case 1u: { return vec3<f32>(168.0, 44.0, 136.0); }
                case 2u: { return vec3<f32>(145.0, 147.0, 44.0); }
                case 3u: { return vec3<f32>(133.0, 63.0, 52.0); }
                case 4u: { return vec3<f32>(63.0, 193.0, 204.0); }
                case 5u: { return vec3<f32>(51.0, 109.0, 212.0); }
                default: { return vec3<f32>(28.0, 212.0, 120.0); }
            }
        }
        return vec3<f32>(104.0, 128.0, 128.0);
    }

    if y < p3_y0 {
        if x < d_w {
            return vec3<f32>(188.0, 154.0, 16.0);
        }
        if x < d_w + r_w {
            return vec3<f32>(57.0, 156.0, 97.0);
        }
        if x < d_w + 7.0 * r_w {
            return vec3<f32>(180.0, 128.0, 128.0);
        }
        return vec3<f32>(32.0, 240.0, 118.0);
    }

    if y < p4_y0 {
        if x < d_w {
            return vec3<f32>(219.0, 16.0, 138.0);
        }
        if x < d_w + r_w {
            return vec3<f32>(44.0, 171.0, 147.0);
        }
        let ramp_w = 6.0 * r_w;
        if x < d_w + r_w + ramp_w {
            let i = x - (d_w + r_w);
            let luma = floor(i * 255.0 / max(ramp_w, 1.0));
            return vec3<f32>(luma, 128.0, 128.0);
        }
        return vec3<f32>(63.0, 102.0, 240.0);
    }

    if x < d_w {
        return vec3<f32>(49.0, 128.0, 128.0);
    }

    let pluge = max(floor(r_w / 3.0), 1.0);
    var cx = d_w;
    let spans = array<f32, 8>(
        max(floor(r_w * 3.0 / 2.0), 1.0),
        max(floor(r_w * 2.0), 1.0),
        max(floor(r_w * 5.0 / 6.0), 1.0),
        pluge,
        pluge,
        pluge,
        pluge,
        pluge,
    );
    let colors = array<vec3<f32>, 8>(
        vec3<f32>(16.0, 128.0, 128.0),
        vec3<f32>(235.0, 128.0, 128.0),
        vec3<f32>(16.0, 128.0, 128.0),
        vec3<f32>(12.0, 128.0, 128.0),
        vec3<f32>(16.0, 128.0, 128.0),
        vec3<f32>(20.0, 128.0, 128.0),
        vec3<f32>(16.0, 128.0, 128.0),
        vec3<f32>(25.0, 128.0, 128.0),
    );
    for (var i = 0; i < 8; i = i + 1) {
        let end = cx + spans[i];
        if x < end {
            return colors[i];
        }
        cx = end;
    }
    if x < max(l_w, cx) {
        return vec3<f32>(16.0, 128.0, 128.0);
    }
    return vec3<f32>(49.0, 128.0, 128.0);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    var px = floor(in.uv.x * WIDTH);
    var py = floor(in.uv.y * HEIGHT);
    if params.flags > 0.5 {
        let ox = floor(params.scroll * WIDTH);
        let oy = floor(params.scroll_y * HEIGHT);
        px = (px + ox) % WIDTH;
        py = (py + oy) % HEIGHT;
    }
    return ycbcr_to_rgb(hd_yuv(px, py));
}
