// Full-screen CRT scanline and vignette overlay.

struct Params {
    size: vec2<f32>,   // physical size
    scale: f32,
    intensity: f32,
    color: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> params: Params;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    var positions = array<vec2<f32>, 3>(vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
    var out: VsOut;
    let p = positions[vi];
    out.clip = vec4(p, 0.0, 1.0);
    out.uv = vec2((p.x + 1.0) * 0.5, (1.0 - p.y) * 0.5);
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let y = in.uv.y * params.size.y;
    let period = max(2.0 * params.scale, 2.0);
    let scan = step(0.5, fract(y / period));
    let vignette = 1.0 - distance(in.uv, vec2(0.5, 0.5)) * 0.45;
    let alpha = (0.35 + scan * 0.65) * params.intensity * clamp(vignette, 0.55, 1.0);
    // Darken between lines, tint faintly on lines.
    let tint = params.color.rgb * 0.15;
    return vec4(tint, alpha * 0.5);
}
