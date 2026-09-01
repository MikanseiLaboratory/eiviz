// Custom transition contract.
// t<=0 → Program, t>=1 → Preview, otherwise user_transition(uv, mix).
// Optional compute: fn user_compute(id: vec3<u32>, dim: vec2<u32>)
//   write with user_store(vec2<i32>(id.xy), color). Use textureSampleLevel in compute.
// Bindings (fragment): pgm_tex, pvw_tex, prev_tex, flow_tex, bloom_tex, aux_tex, aux2_tex,
//   src_samp, src_samp_n, params.

fn user_transition(uv: vec2<f32>, t: f32) -> vec4<f32> {
    let pgm = textureSample(pgm_tex, src_samp, uv);
    let pvw = textureSample(pvw_tex, src_samp, uv);
    let prev = textureSample(prev_tex, src_samp, uv);
    let flow = textureSample(flow_tex, src_samp, uv).xy * 2.0 - 1.0;
    let bloom = textureSample(bloom_tex, src_samp, uv);
    let aux = textureSample(aux_tex, src_samp, uv);
    let warped = textureSample(pgm_tex, src_samp, uv + flow * 0.02 * (1.0 - t));
    return mix(mix(mix(pgm, prev, 0.12), warped, 0.35), pvw, t) + bloom * 0.25 * t + aux * 0.0;
}
