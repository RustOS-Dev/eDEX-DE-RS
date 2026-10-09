# wgpu-hal 29.0.4, patched

The crates.io release of `wgpu-hal` 29.0.4 (MIT or Apache-2.0, see the licence files), used
through `[patch.crates-io]` in the workspace manifest, with one change and without its examples:

* `src/gles/adapter.rs`: `PrivateCapabilities::SHADER_BINDING_LAYOUT` (shaders carry
  `layout(binding = N)`) also requires GLSL 4.20 or GLSL ES 3.10. Upstream sets it whenever the
  context supports compute shaders, which a GL 3.3 core context does through
  `GL_ARB_compute_shader` (Mesa's softpipe: GL 3.3, GLSL 3.30). naga then writes GLSL 3.30
  without bindings, wgpu skips the remapping of texture units after linking, and every texture
  in a shader samples unit 0: glyphon's text, which reads its mask atlas from binding 1, comes
  out empty. With the check, wgpu remaps the units by name, as it does on GL ES 3.0.

Drop this copy when wgpu-hal fixes it upstream (still present in 30.0.1).
