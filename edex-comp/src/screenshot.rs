//! Screenshots: render an output offscreen and save it as PNG.

use anyhow::{anyhow, Context, Result};
use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            damage::OutputDamageTracker, ExportMem, ImportAll, ImportMem, Offscreen, Renderer,
        },
    },
    desktop::Space,
    output::Output,
    utils::{Logical, Rectangle, Transform},
};

use crate::{
    render::{output_elements, Screen},
    shell::WindowElement,
};

/// Render `output` into an offscreen buffer of type `T` and read it back as RGBA. `screen` is
/// what the output shows: while the session is locked a screenshot shows the lock screen, never
/// the session behind it.
pub fn capture<R, T>(
    renderer: &mut R,
    output: &Output,
    space: &Space<WindowElement>,
    screen: Screen<'_>,
) -> Result<image::RgbaImage>
where
    R: Renderer + ImportAll + ImportMem + Offscreen<T> + ExportMem,
    R::TextureId: Clone + 'static,
    R::Error: std::error::Error + Send + Sync + 'static,
{
    let mode = output.current_mode().context("output has no mode")?;
    let size = output.current_transform().transform_size(mode.size);
    let scale = output.current_scale().fractional_scale();
    let buffer_size = size.to_logical(1).to_buffer(1, Transform::Normal);
    let mut target: T = renderer
        .create_buffer(Fourcc::Abgr8888, buffer_size)
        .context("creating the offscreen buffer")?;
    let (elements, clear) = output_elements(output, space, Vec::new(), renderer, false, screen);
    let mut framebuffer = renderer.bind(&mut target).context("binding the buffer")?;
    let mut damage = OutputDamageTracker::new(size, scale, Transform::Normal);
    damage
        .render_output(renderer, &mut framebuffer, 0, &elements, clear)
        .map_err(|e| anyhow!("rendering the screenshot: {e:?}"))?;
    let mapping = renderer
        .copy_framebuffer(
            &framebuffer,
            Rectangle::from_size(buffer_size),
            Fourcc::Abgr8888,
        )
        .context("reading the buffer back")?;
    let bytes = renderer
        .map_texture(&mapping)
        .context("mapping the read-back")?;
    image::RgbaImage::from_raw(size.w as u32, size.h as u32, bytes.to_vec())
        .context("unexpected read-back size")
}

/// Crop a capture of an output at `output_geo` (global logical) to `region`.
pub fn crop(
    img: image::RgbaImage,
    output_geo: Rectangle<i32, Logical>,
    scale: f64,
    region: Rectangle<i32, Logical>,
) -> Result<image::RgbaImage> {
    let Some(inter) = output_geo.intersection(region) else {
        anyhow::bail!("the region is not on this output");
    };
    let x = ((inter.loc.x - output_geo.loc.x) as f64 * scale).round() as u32;
    let y = ((inter.loc.y - output_geo.loc.y) as f64 * scale).round() as u32;
    let w = ((inter.size.w as f64 * scale).round() as u32).min(img.width().saturating_sub(x));
    let h = ((inter.size.h as f64 * scale).round() as u32).min(img.height().saturating_sub(y));
    Ok(image::imageops::crop_imm(&img, x, y, w, h).to_image())
}

pub fn save(img: &image::RgbaImage, path: &str) -> Result<()> {
    if let Some(dir) = std::path::Path::new(path).parent() {
        std::fs::create_dir_all(dir).ok();
    }
    img.save_with_format(path, image::ImageFormat::Png)
        .with_context(|| format!("writing {path}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crops_in_output_pixels() {
        let img =
            image::RgbaImage::from_fn(200, 100, |x, y| image::Rgba([x as u8, y as u8, 0, 255]));
        let out = Rectangle::new((1000, 0).into(), (100, 50).into());
        let c = crop(
            img,
            out,
            2.0,
            Rectangle::new((1010, 5).into(), (20, 10).into()),
        )
        .unwrap();
        assert_eq!((c.width(), c.height()), (40, 20));
        assert_eq!(c.get_pixel(0, 0).0, [20, 10, 0, 255]);
        let none = image::RgbaImage::new(10, 10);
        assert!(crop(none, out, 1.0, Rectangle::new((0, 0).into(), (5, 5).into())).is_err());
    }
}
