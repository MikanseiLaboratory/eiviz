// Custom transition contract.
// The host prepends vertex + MixParams bindings (pgm_tex, pvw_tex, src_samp, params)
// and wraps this fragment as:
//   @fragment fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
//       return user_transition(in.uv, params.mix);
//   }
// Available uniforms: params.mix, params.direction, params.dip.

fn user_transition(uv: vec2<f32>, t: f32) -> vec4<f32> {
    let pgm = textureSample(pgm_tex, src_samp, uv);
    let pvw = textureSample(pvw_tex, src_samp, uv);
    return mix(pgm, pvw, t);
}
