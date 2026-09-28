// Instanced signed-distance rectangles: plain (rounded, bordered, glowing), key caps,
// circles and hexagons. Coordinates are logical pixels; `screen.scale` maps to physical.

struct Screen {
    size: vec2<f32>,     // logical size
    scale: f32,
    srgb: f32,           // 1.0 when the target is an sRGB format (convert colours to linear)
};

@group(0) @binding(0)
var<uniform> screen: Screen;

struct Instance {
    @location(0) pos: vec2<f32>,
    @location(1) size: vec2<f32>,
    @location(2) fill: vec4<f32>,
    @location(3) border: vec4<f32>,
    @location(4) params: vec4<f32>, // border_width, radius, glow, kind
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) local: vec2<f32>,   // logical px relative to rect origin
    @location(1) size: vec2<f32>,
    @location(2) fill: vec4<f32>,
    @location(3) border: vec4<f32>,
    @location(4) params: vec4<f32>,
};

const GLOW_PAD: f32 = 12.0;

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, inst: Instance) -> VsOut {
    var corners = array<vec2<f32>, 6>(
        vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(0.0, 1.0),
        vec2(0.0, 1.0), vec2(1.0, 0.0), vec2(1.0, 1.0),
    );
    let pad = select(0.0, GLOW_PAD, inst.params.z > 0.0);
    let corner = corners[vi];
    let local = corner * (inst.size + 2.0 * pad) - vec2(pad, pad);
    let logical = inst.pos + local;
    let ndc = vec2(logical.x / screen.size.x * 2.0 - 1.0, 1.0 - logical.y / screen.size.y * 2.0);
    var out: VsOut;
    out.clip = vec4(ndc, 0.0, 1.0);
    out.local = local;
    out.size = inst.size;
    out.fill = inst.fill;
    out.border = inst.border;
    out.params = inst.params;
    return out;
}

fn to_linear(c: vec4<f32>) -> vec4<f32> {
    if screen.srgb > 0.5 {
        let rgb = pow(c.rgb, vec3(2.2, 2.2, 2.2));
        return vec4(rgb, c.a);
    }
    return c;
}

fn sd_round_box(p: vec2<f32>, half: vec2<f32>, r: f32) -> f32 {
    let q = abs(p) - half + vec2(r, r);
    return length(max(q, vec2(0.0, 0.0))) + min(max(q.x, q.y), 0.0) - r;
}

fn sd_keycap(p: vec2<f32>, half: vec2<f32>) -> f32 {
    let cut = min(half.x, half.y) * 0.45;
    let q = abs(p);
    let d_box = max(q.x - half.x, q.y - half.y);
    let d_diag = (q.x + q.y - (half.x + half.y - cut)) * 0.70710678;
    return max(d_box, d_diag);
}

fn sd_hexagon(p: vec2<f32>, r: f32) -> f32 {
    let k = vec3(-0.866025404, 0.5, 0.577350269);
    var q = abs(p);
    q = q - 2.0 * min(dot(k.xy, q), 0.0) * k.xy;
    q = q - vec2(clamp(q.x, -k.z * r, k.z * r), r);
    return length(q) * sign(q.y);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let half = in.size * 0.5;
    let p = in.local - half;
    let kind = u32(in.params.w + 0.5);
    let border_w = in.params.x;
    let radius = in.params.y;
    let glow = in.params.z;

    var d: f32;
    if kind == 1u {
        d = sd_keycap(p, half);
    } else if kind == 2u {
        d = length(p) - min(half.x, half.y);
    } else if kind == 3u {
        d = sd_hexagon(p, min(half.x, half.y));
    } else {
        d = sd_round_box(p, half, radius);
    }

    // Anti-aliasing width of one physical pixel in logical units.
    let aa = 1.0 / max(screen.scale, 0.5);
    let fill = to_linear(in.fill);
    let border = to_linear(in.border);

    if d > 0.0 {
        if glow > 0.0 && d < GLOW_PAD {
            let t = 1.0 - d / GLOW_PAD;
            let a = border.a * glow * t * t * 0.55;
            return vec4(border.rgb, a);
        }
        // Outer anti-aliased edge of the border (or fill when there is no border).
        let edge_color = select(fill, border, border_w > 0.0);
        let a = edge_color.a * clamp(1.0 - d / aa, 0.0, 1.0);
        return vec4(edge_color.rgb, a);
    }
    if border_w > 0.0 && d > -border_w {
        let inner = clamp((-d) / aa, 0.0, 1.0);
        let mixed = mix(fill, border, inner);
        // Pulse the border brightness slightly with the glow amount.
        let boost = 1.0 + glow * 0.25;
        return vec4(min(mixed.rgb * boost, vec3(1.0, 1.0, 1.0)), mixed.a);
    }
    return fill;
}
