//! Cached glyphon text shaping: identical text specs reuse their shaped buffer across frames.

use std::{
    collections::hash_map::DefaultHasher,
    collections::HashMap,
    hash::{Hash, Hasher},
};

use glyphon::{
    cosmic_text::{Align as CtAlign, Weight, Wrap},
    Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, Style, TextArea, TextBounds,
};
use ui::scene::{Align, TextSpec};

struct Cached {
    buffer: Buffer,
    last_used: u64,
}

pub struct TextCache {
    entries: HashMap<u64, Cached>,
    generation: u64,
    family: Option<String>,
    /// Leaked once per family change so `Attrs<'static>` can borrow it.
    family_static: Option<&'static str>,
}

fn key_of(spec: &TextSpec) -> u64 {
    let mut h = DefaultHasher::new();
    for s in &spec.spans {
        s.text.hash(&mut h);
        for c in s.color {
            c.to_bits().hash(&mut h);
        }
        s.bold.hash(&mut h);
        s.italic.hash(&mut h);
    }
    spec.size.to_bits().hash(&mut h);
    spec.line_height.to_bits().hash(&mut h);
    (spec.bounds.w.round() as i64).hash(&mut h);
    (spec.bounds.h.round() as i64).hash(&mut h);
    (spec.align as u8).hash(&mut h);
    spec.wrap.hash(&mut h);
    spec.mono.hash(&mut h);
    h.finish()
}

fn to_color(c: [f32; 4]) -> Color {
    Color::rgba(
        (c[0].clamp(0.0, 1.0) * 255.0).round() as u8,
        (c[1].clamp(0.0, 1.0) * 255.0).round() as u8,
        (c[2].clamp(0.0, 1.0) * 255.0).round() as u8,
        (c[3].clamp(0.0, 1.0) * 255.0).round() as u8,
    )
}

impl TextCache {
    pub fn new(family: Option<String>) -> Self {
        let family_static = family
            .as_deref()
            .map(|f| &*Box::leak(f.to_string().into_boxed_str()));
        Self {
            entries: HashMap::new(),
            generation: 0,
            family,
            family_static,
        }
    }

    pub fn set_family(&mut self, family: Option<String>) {
        if self.family != family {
            self.family_static = family
                .as_deref()
                .map(|f| &*Box::leak(f.to_string().into_boxed_str()));
            self.family = family;
            self.entries.clear();
        }
    }

    fn family(&self) -> Family<'static> {
        match self.family_static {
            Some(name) => Family::Name(name),
            None => Family::Monospace,
        }
    }

    /// Start a new frame: bump the generation used for eviction.
    pub fn begin_frame(&mut self) {
        self.generation += 1;
    }

    /// Drop buffers unused for more than `max_age` frames.
    pub fn end_frame(&mut self, max_age: u64) {
        let gen = self.generation;
        self.entries.retain(|_, c| gen - c.last_used <= max_age);
    }

    /// Shape (or fetch) the buffer for a spec and return the cache key.
    pub fn prepare(&mut self, font_system: &mut FontSystem, spec: &TextSpec) -> u64 {
        let key = key_of(spec);
        let gen = self.generation;
        let family: Family<'static> = self.family();
        let entry = self.entries.entry(key).or_insert_with(|| {
            let mut buffer = Buffer::new(font_system, Metrics::new(spec.size, spec.line_height));
            let width = if spec.wrap {
                Some(spec.bounds.w.max(1.0))
            } else {
                None
            };
            buffer.set_size(
                font_system,
                width,
                Some(spec.bounds.h.max(spec.line_height)),
            );
            buffer.set_wrap(
                font_system,
                if spec.wrap {
                    Wrap::WordOrGlyph
                } else {
                    Wrap::None
                },
            );
            let base = Attrs::new().family(family);
            let spans = spec.spans.iter().map(|s| {
                let mut a = Attrs::new().family(family).color(to_color(s.color));
                if s.bold {
                    a = a.weight(Weight::BOLD);
                }
                if s.italic {
                    a = a.style(Style::Italic);
                }
                (s.text.as_str(), a)
            });
            let align = match spec.align {
                Align::Left => None,
                Align::Center => Some(CtAlign::Center),
                Align::Right => Some(CtAlign::Right),
            };
            buffer.set_rich_text(font_system, spans, &base, Shaping::Advanced, align);
            buffer.shape_until_scroll(font_system, false);
            Cached {
                buffer,
                last_used: gen,
            }
        });
        entry.last_used = gen;
        key
    }

    /// Build the text area for a prepared spec.
    pub fn area<'a>(
        &'a self,
        key: u64,
        spec: &TextSpec,
        scale: f32,
        physical: (u32, u32),
    ) -> Option<TextArea<'a>> {
        let cached = self.entries.get(&key)?;
        let b = &spec.bounds;
        // For non-wrapping right/center aligned text we shaped without a width; position it
        // by measuring the run width so alignment still works.
        let mut left = b.x;
        if !spec.wrap && spec.align != Align::Left {
            let run_w = cached
                .buffer
                .layout_runs()
                .map(|r| r.line_w)
                .fold(0.0f32, f32::max);
            left = match spec.align {
                Align::Right => b.x + b.w - run_w,
                Align::Center => b.x + (b.w - run_w) / 2.0,
                Align::Left => b.x,
            };
        }
        let bounds = TextBounds {
            left: (b.x * scale).floor().max(0.0) as i32,
            top: (b.y * scale).floor().max(0.0) as i32,
            right: ((b.x + b.w) * scale).ceil().min(physical.0 as f32) as i32,
            bottom: ((b.y + b.h) * scale).ceil().min(physical.1 as f32) as i32,
        };
        let default_color = spec
            .spans
            .first()
            .map(|s| to_color(s.color))
            .unwrap_or(Color::rgb(255, 255, 255));
        Some(TextArea {
            buffer: &cached.buffer,
            left: left * scale,
            top: b.y * scale,
            scale,
            bounds,
            default_color,
            custom_glyphs: &[],
        })
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

/// Measure the advance width and line height of a monospace cell at `size`.
pub fn measure_mono(
    font_system: &mut FontSystem,
    family: Option<&str>,
    size: f32,
    line_height: f32,
) -> (f32, f32) {
    let mut buffer = Buffer::new(font_system, Metrics::new(size, line_height));
    buffer.set_size(font_system, None, None);
    let fam = match family {
        Some(n) => Family::Name(n),
        None => Family::Monospace,
    };
    buffer.set_text(
        font_system,
        "MMMMMMMMMM",
        &Attrs::new().family(fam),
        Shaping::Advanced,
        None,
    );
    buffer.shape_until_scroll(font_system, false);
    let w = buffer
        .layout_runs()
        .map(|r| r.line_w)
        .fold(0.0f32, f32::max)
        / 10.0;
    (if w > 0.0 { w } else { size * 0.6 }, line_height)
}
