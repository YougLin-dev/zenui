@group(0) @binding(0) var under: texture_2d<f32>;
@group(0) @binding(1) var overlay: texture_2d<f32>;

@vertex
fn vs(@builtin(vertex_index) index: u32) -> @builtin(position) vec4f {
    let points = array<vec2f, 3>(vec2f(-1., -1.), vec2f(3., -1.), vec2f(-1., 3.));
    return vec4f(points[index], 0., 1.);
}

@fragment
fn fs(@builtin(position) position: vec4f) -> @location(0) vec4f {
    // v0.12 fine.wgsl un-premultiplies its final RGB values: direct sampling
    // receives a color in that convention, so multiply on blend, not on readback.
    let bottom = textureLoad(under, vec2i(position.xy), 0);
    let top = textureLoad(overlay, vec2i(position.xy), 0);
    // Source-over for two straight-alpha colors, written with no hardware blend.
    let alpha = top.a + bottom.a * (1. - top.a);
    let rgb = (top.rgb * top.a + bottom.rgb * bottom.a * (1. - top.a)) / max(alpha, 1e-6);
    return vec4(rgb, alpha);
}
