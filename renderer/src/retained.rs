//! Partial redraws for software rasterizers.
//!
//! On a CPU rasterizer (Mesa's softpipe on RustOS) every pixel of a frame costs, and the shell
//! redraws for small changes: the clock, a terminal line, a sysmon bar. There the scene is drawn
//! into a texture that keeps its contents between frames, only inside the tiles whose drawing
//! changed, and that texture is then copied onto the surface with one full-screen draw (wgpu
//! clears surface textures every frame, and on GL they cannot be a copy destination).
//!
//! A tile's fingerprint hashes, in drawing order, every item (rectangle with its glow, text with
//! its bounds) that touches it; a tile is redrawn when its fingerprint changes.

use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
};

use ui::scene::{RectInstance, Scene, TextSpec};

/// Tile edge in physical pixels.
pub const TILE: u32 = 32;
/// More damage rectangles than this are drawn as their bounding box.
const MAX_RECTS: usize = 12;
/// How far a glowing rectangle draws outside its rectangle (`GLOW_PAD` in rect.wgsl), plus
/// anti-aliasing.
const GLOW_REACH: f32 = 13.0;

/// A rectangle in physical pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Damage {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// Tile fingerprints of one frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tiles {
    cols: u32,
    rows: u32,
    width: u32,
    height: u32,
    hashes: Vec<u64>,
}

fn hash_debug<T: std::fmt::Debug>(value: &T) -> u64 {
    // Scene items hold floats, which are not `Hash`; their Debug form is exact and cheap enough
    // for the few hundred items of a frame.
    let mut h = DefaultHasher::new();
    format!("{value:?}").hash(&mut h);
    h.finish()
}

impl Tiles {
    /// Fingerprint `scene` drawn at `scale` into `width`×`height` physical pixels.
    pub fn of(scene: &Scene, scale: f32, width: u32, height: u32) -> Self {
        let cols = width.div_ceil(TILE).max(1);
        let rows = height.div_ceil(TILE).max(1);
        let base = hash_debug(&(scene.clear, scale.to_bits()));
        let mut t = Self {
            cols,
            rows,
            width,
            height,
            hashes: vec![base; (cols * rows) as usize],
        };
        for r in &scene.rects {
            t.add_rect(r, scale);
        }
        for s in &scene.texts {
            t.add_text(s, scale);
        }
        t
    }

    fn add(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, item: u64) {
        if x1 <= x0 || y1 <= y0 || x1 <= 0.0 || y1 <= 0.0 {
            return;
        }
        let tile = TILE as f32;
        let c0 = (x0.max(0.0) / tile).floor() as u32;
        let r0 = (y0.max(0.0) / tile).floor() as u32;
        let c1 = ((x1 / tile).ceil() as u32).min(self.cols);
        let r1 = ((y1 / tile).ceil() as u32).min(self.rows);
        for row in r0..r1 {
            for col in c0..c1 {
                let h = &mut self.hashes[(row * self.cols + col) as usize];
                *h = (h.rotate_left(7) ^ item).wrapping_mul(0x100_0000_01b3);
            }
        }
    }

    fn add_rect(&mut self, r: &RectInstance, scale: f32) {
        if r.rect.w <= 0.0 || r.rect.h <= 0.0 {
            return;
        }
        let pad = if r.glow > 0.0 { GLOW_REACH } else { 1.0 };
        self.add(
            (r.rect.x - pad) * scale,
            (r.rect.y - pad) * scale,
            (r.rect.x + r.rect.w + pad) * scale,
            (r.rect.y + r.rect.h + pad) * scale,
            hash_debug(r),
        );
    }

    fn add_text(&mut self, s: &TextSpec, scale: f32) {
        // Text is clipped to its bounds (see `TextCache::area`).
        let b = &s.bounds;
        self.add(
            (b.x * scale).floor(),
            (b.y * scale).floor(),
            ((b.x + b.w) * scale).ceil(),
            ((b.y + b.h) * scale).ceil(),
            hash_debug(s),
        );
    }

    /// Where this frame differs from `prev`: everything when the sizes differ.
    pub fn damage(&self, prev: Option<&Tiles>) -> Vec<Damage> {
        let full = Damage {
            x: 0,
            y: 0,
            w: self.width,
            h: self.height,
        };
        let Some(prev) = prev.filter(|p| p.cols == self.cols && p.rows == self.rows) else {
            return vec![full];
        };
        if prev.width != self.width || prev.height != self.height {
            return vec![full];
        }
        // Runs of changed tiles per row, then runs with the same columns merged downwards.
        let mut rects: Vec<(u32, u32, u32, u32)> = Vec::new(); // col0, col1, row0, row1
        for row in 0..self.rows {
            let mut col = 0;
            while col < self.cols {
                let i = (row * self.cols + col) as usize;
                if self.hashes[i] == prev.hashes[i] {
                    col += 1;
                    continue;
                }
                let start = col;
                while col < self.cols {
                    let j = (row * self.cols + col) as usize;
                    if self.hashes[j] == prev.hashes[j] {
                        break;
                    }
                    col += 1;
                }
                match rects
                    .iter_mut()
                    .find(|r| r.0 == start && r.1 == col && r.3 == row)
                {
                    Some(r) => r.3 = row + 1,
                    None => rects.push((start, col, row, row + 1)),
                }
            }
        }
        let to_damage = |(c0, c1, r0, r1): (u32, u32, u32, u32)| {
            let x = c0 * TILE;
            let y = r0 * TILE;
            Damage {
                x,
                y,
                w: (c1 * TILE).min(self.width) - x,
                h: (r1 * TILE).min(self.height) - y,
            }
        };
        if rects.len() > MAX_RECTS {
            let bbox = rects.iter().fold((u32::MAX, 0, u32::MAX, 0), |a, r| {
                (a.0.min(r.0), a.1.max(r.1), a.2.min(r.2), a.3.max(r.3))
            });
            return vec![to_damage(bbox)];
        }
        rects.into_iter().map(to_damage).collect()
    }
}

/// Copies a texture onto the render target, pixel for pixel.
pub struct BlitPipeline {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
}

impl BlitPipeline {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::include_wgsl!("shaders/blit.wgsl"));
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("edex blit bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("edex blit pl"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("edex blit pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        Self { pipeline, layout }
    }

    pub fn bind(&self, device: &wgpu::Device, view: &wgpu::TextureView) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("edex blit bg"),
            layout: &self.layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(view),
            }],
        })
    }

    pub fn draw<'p>(&'p self, pass: &mut wgpu::RenderPass<'p>, bind_group: &'p wgpu::BindGroup) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// The retained canvas of one surface.
pub struct Canvas {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub bind_group: wgpu::BindGroup,
    /// Fingerprints of what the canvas holds.
    pub tiles: Option<Tiles>,
}

impl Canvas {
    pub fn new(
        device: &wgpu::Device,
        blit: &BlitPipeline,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("edex retained canvas"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = blit.bind(device, &view);
        Self {
            texture,
            view,
            bind_group,
            tiles: None,
        }
    }

    pub fn size(&self) -> (u32, u32) {
        (self.texture.width(), self.texture.height())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ui::geometry::Rect;

    fn scene_with(text: &str) -> Scene {
        let mut s = Scene::new(640.0, 400.0, [0.0, 0.0, 0.0, 1.0]);
        s.panel(
            Rect::new(10.0, 10.0, 300.0, 200.0),
            [0.1, 0.1, 0.1, 1.0],
            [0.0, 0.8, 1.0, 1.0],
            1.0,
            0.5,
        );
        s.text(Rect::new(400.0, 300.0, 100.0, 20.0), 14.0, [1.0; 4], text);
        s
    }

    #[test]
    fn unchanged_scene_has_no_damage() {
        let a = Tiles::of(&scene_with("12:00"), 1.0, 640, 400);
        let b = Tiles::of(&scene_with("12:00"), 1.0, 640, 400);
        assert!(b.damage(Some(&a)).is_empty());
    }

    #[test]
    fn changed_text_damages_its_tiles_only() {
        let a = Tiles::of(&scene_with("12:00"), 1.0, 640, 400);
        let b = Tiles::of(&scene_with("12:01"), 1.0, 640, 400);
        let d = b.damage(Some(&a));
        assert_eq!(d.len(), 1);
        let r = d[0];
        // The text spans x 400..500, y 300..320: tile columns 12..16, row 9.
        assert_eq!((r.x, r.y, r.w, r.h), (384, 288, 128, 32));
    }

    #[test]
    fn first_frame_and_resize_are_full() {
        let a = Tiles::of(&scene_with("x"), 1.0, 640, 400);
        assert_eq!(
            a.damage(None),
            vec![Damage {
                x: 0,
                y: 0,
                w: 640,
                h: 400
            }]
        );
        let b = Tiles::of(&scene_with("x"), 1.0, 600, 400);
        assert_eq!(
            b.damage(Some(&a)),
            vec![Damage {
                x: 0,
                y: 0,
                w: 600,
                h: 400
            }]
        );
    }
}
