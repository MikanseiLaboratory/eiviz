// Custom transition contract.
// The host prepends vertex + MixParams bindings and wraps this fragment as:
//   t<=0 → Program, t>=1 → Preview, otherwise user_transition(uv, mix).
// Bindings: pgm_tex, pvw_tex, src_samp (linear), prev_tex (last mixed frame),
//           src_samp_n (nearest), params.
// params: mix, kind, direction, softness, dip, param, time (seconds @ 60fps), resolution.

fn user_transition(uv: vec2<f32>, t: f32) -> vec4<f32> {
    let pgm = textureSample(pgm_tex, src_samp, uv);
    let pvw = textureSample(pvw_tex, src_samp, uv);
    let prev = textureSample(prev_tex, src_samp, uv);
    return mix(mix(pgm, prev, 0.15), pvw, t);
}
