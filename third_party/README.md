# Vendored crates

## wgpu-hal 29.0.4

`wgpu-hal` from crates.io (MIT OR Apache-2.0), used through `[patch.crates-io]`
in the workspace `Cargo.toml`. Examples and dev-dependencies are left out.

One change, in `src/gles/adapter.rs`: the GL backend assumed that a context
with compute shaders can put `layout(binding = N)` in its shaders and skipped
binding resources by name. naga writes those qualifiers only for GLSL 4.20+
and GLSL ES 3.10+, so on a GL 3.3 core context that offers
`GL_ARB_compute_shader` (Mesa's softpipe) every texture read unit 0 and glyphs
drew as blocks or not at all. The capability now also needs a shading language
version naga writes bindings for.

Drop this directory and the patch entry once wgpu ships the fix.
