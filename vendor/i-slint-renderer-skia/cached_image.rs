// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

use i_slint_core::graphics::{
    Image, ImageCacheKey, ImageInner, IntSize, OpaqueImage, OpaqueImageVTable, Rgba8Pixel,
    SharedImageBuffer, SharedPixelBuffer, cache as core_cache,
};
use i_slint_core::items::ImageFit;
use i_slint_core::lengths::{LogicalSize, ScaleFactor};

struct SkiaCachedImage {
    image: skia_safe::Image,
    cache_key: ImageCacheKey,
}

struct PixelBytes<Pixel>(SharedPixelBuffer<Pixel>);

impl<Pixel: Clone + bytemuck::Pod> AsRef<[u8]> for PixelBytes<Pixel> {
    fn as_ref(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

/// Wraps the pixels in Skia data that shares the buffer instead of copying it.
fn shared_pixel_data<Pixel: Clone + bytemuck::Pod + Send + Sync>(
    pixels: &SharedPixelBuffer<Pixel>,
) -> skia_safe::Data {
    skia_safe::Data::new_with_owner(PixelBytes(pixels.clone()))
}

i_slint_core::OpaqueImageVTable_static! {
    static SKIA_CACHED_IMAGE_VT for SkiaCachedImage
}

impl OpaqueImage for SkiaCachedImage {
    fn size(&self) -> IntSize {
        IntSize::new(self.image.width() as u32, self.image.height() as u32)
    }

    fn cache_key(&self) -> ImageCacheKey {
        self.cache_key.clone()
    }
}

thread_local! {
    /// featherplay patch (change 24, VENDORED.md): `field_rows`' bank, and an image of each row.
    static FIELD: std::cell::RefCell<Option<Field>> = const { std::cell::RefCell::new(None) };
}

/// featherplay patch (change 24): the bank of rows a page is drawn from, how, and a Skia image
/// over each row's bytes (each holds a clone of the bank: no copy).
struct Field {
    rows: Vec<skia_safe::Image>,
    bank: SharedPixelBuffer<Rgba8Pixel>,
    block: u32,
    pad: u32,
    /// The page's size it was made for, physical pixels.
    width: u32,
    height: u32,
}

/// featherplay patch (change 24): one row of the bank, as Skia data that shares the bank.
struct FieldRow {
    bank: SharedPixelBuffer<Rgba8Pixel>,
    from: usize,
    to: usize,
}

impl AsRef<[u8]> for FieldRow {
    fn as_ref(&self) -> &[u8] {
        &self.bank.as_bytes()[self.from..self.to]
    }
}

/// featherplay patch (change 24, VENDORED.md): the app's one pixel buffer that's a bank of rows
/// (`None`: none), opaque, in BGRA (the order the window surfaces draw in): one row for every
/// `block` rows of a page `width` × `height` and `pad` pixels wider than it. An image of it
/// (`Image::from_rgba8`) is drawn at its element's size a row at a time: the page's row `y` is
/// the bank's row `y / block` from column `y * 7 % pad` on, each a 1:1 copy, so it costs about
/// what a copy of a whole picture does in a fraction of the memory. At another size (a window
/// being sized, until the app makes the next) the rows are stretched to it. Name it before the
/// image is set, and let it go (`None`) with the image: a clone of the buffer is kept until
/// then, so its address can't be another buffer's. Per thread, the renderer's.
pub fn field_rows(bank: Option<(SharedPixelBuffer<Rgba8Pixel>, u32, u32, u32, u32)>) {
    let field = bank.and_then(|(bank, block, pad, width, height)| {
        let stride = bank.width() as usize * 4;
        if block == 0 || pad == 0 || stride == 0 || width == 0 || height == 0 {
            return None;
        }
        let info = crate::image_info(
            skia_safe::ISize::new(bank.width() as i32, 1),
            skia_safe::ColorType::BGRA8888,
            skia_safe::AlphaType::Opaque,
        );
        let rows = (0..bank.height() as usize)
            .filter_map(|y| {
                let data = skia_safe::Data::new_with_owner(FieldRow {
                    bank: bank.clone(),
                    from: y * stride,
                    to: (y + 1) * stride,
                });
                skia_safe::images::raster_from_data(&info, data, stride)
            })
            .collect();
        Some(Field { rows, bank, block, pad, width, height })
    });
    FIELD.with(|f| *f.borrow_mut() = field);
}

/// featherplay patch (change 24): whether `image` is `field_rows`' bank.
pub(crate) fn is_field(image: &Image) -> bool {
    let ImageInner::EmbeddedImage { buffer: SharedImageBuffer::RGBA8(p), .. } =
        <&ImageInner>::from(image)
    else {
        return false;
    };
    FIELD.with(|f| {
        f.borrow()
            .as_ref()
            .is_some_and(|f| std::ptr::eq(f.bank.as_bytes().as_ptr(), p.as_bytes().as_ptr()))
    })
}

/// featherplay patch (change 24): the page `width` × `height` (physical pixels, at the canvas'
/// origin) drawn from `field_rows`' bank, only the rows the clip shows; stretched if the bank
/// was made for another size. featherplay patch (change 25): with a `tone` that isn't clear,
/// the tone's colour at its alpha over it, in the same pass: the rows' part filled with the
/// opaque colour, then each row blended onto it at the rest of the alpha (Skia's sprite
/// blitter, at about a copy's cost).
pub(crate) fn draw_field(
    canvas: &skia_safe::Canvas,
    width: f32,
    height: f32,
    tone: i_slint_core::Color,
) {
    FIELD.with(|f| {
        let f = f.borrow();
        let Some(f) = f.as_ref() else { return };
        let page = skia_safe::Rect::from_wh(width, height);
        let (fw, fh) = (f.width as f32, f.height as f32);
        let across = (width - fw).abs() >= 1.0;
        let down = if (height - fh).abs() >= 1.0 { fh / height } else { 1.0 };
        let mut paint = skia_safe::Paint::default();
        canvas.save();
        canvas.clip_rect(page, None, None);
        if let Some(shown) = canvas.local_clip_bounds() {
            if tone.alpha() > 0 {
                let mut fill = skia_safe::Paint::default();
                fill.set_color(skia_safe::Color::from_rgb(tone.red(), tone.green(), tone.blue()));
                canvas.draw_rect(shown, &fill);
                paint.set_alpha(255 - tone.alpha());
            }
            let last = f.rows.len().saturating_sub(1);
            for y in shown.top.max(0.0) as u32..shown.bottom.min(height).ceil() as u32 {
                let at = ((y as f32 + 0.5) * down) as u32;
                let Some(row) = f.rows.get((at / f.block) as usize).or(f.rows.get(last)) else {
                    continue;
                };
                let from = (at * 7 % f.pad) as f32;
                if across {
                    canvas.draw_image_rect_with_sampling_options(
                        row,
                        Some((
                            &skia_safe::Rect::from_xywh(from, 0.0, fw, 1.0),
                            skia_safe::canvas::SrcRectConstraint::Fast,
                        )),
                        skia_safe::Rect::from_xywh(0.0, y as f32, width, 1.0),
                        skia_safe::FilterMode::Nearest,
                        &paint,
                    );
                } else {
                    canvas.draw_image(row, (-from, y as f32), Some(&paint));
                }
            }
        }
        canvas.restore();
    });
}

/// `transform_scale` is how much the canvas transform magnifies the image,
/// see [`i_slint_core::graphics::transformed_scalable_render_size`].
pub(crate) fn as_skia_image(
    image: Image,
    target_size_fn: &dyn Fn() -> LogicalSize,
    image_fit: ImageFit,
    scale_factor: ScaleFactor,
    transform_scale: f32,
    canvas: &skia_safe::Canvas,
    surface: Option<&dyn crate::Surface>,
) -> Option<skia_safe::Image> {
    let image_inner: &ImageInner = (&image).into();
    match image_inner {
        ImageInner::None => None,
        ImageInner::EmbeddedImage { buffer, cache_key } => {
            if let Some(ImageInner::BackendStorage(cached)) =
                core_cache::lookup_cached_image(cache_key)
                && let Some(img) = skia_cached_image(&cached)
            {
                return Some(img);
            }
            let result = image_buffer_to_skia_image(buffer);
            if let Some(img) = result.as_ref() {
                core_cache::replace_cached_image(
                    cache_key.clone(),
                    ImageInner::BackendStorage(vtable::VRc::into_dyn(vtable::VRc::new(
                        SkiaCachedImage { image: img.clone(), cache_key: cache_key.clone() },
                    ))),
                )
            }
            result
        }
        // featherplay patch (change 18): i-slint-core is built without `svg`, so there's no
        // `ImageInner::Svg` to draw.
        #[cfg(any())]
        ImageInner::Svg(svg) => {
            // Query target_width/height here again to ensure that changes will invalidate the item rendering cache.
            let target_size = i_slint_core::graphics::transformed_scalable_render_size(
                svg.size(),
                image_fit,
                target_size_fn(),
                scale_factor,
                Default::default(),
                transform_scale,
            )?;
            let SharedImageBuffer::RGBA8Premultiplied(pixels) =
                svg.render(Some(target_size)).ok()?
            else {
                unreachable!()
            };

            let image_info = crate::image_info(
                skia_safe::ISize::new(pixels.width() as i32, pixels.height() as i32),
                skia_safe::ColorType::RGBA8888,
                skia_safe::AlphaType::Premul,
            );

            skia_safe::images::raster_from_data(
                &image_info,
                shared_pixel_data(&pixels),
                pixels.width() as usize * 4,
            )
        }
        ImageInner::BackendStorage(x) => skia_cached_image(x),
        ImageInner::BorrowedOpenGLTexture(texture) => {
            surface.and_then(|surface| surface.import_opengl_texture(canvas, texture))
        }
        ImageInner::NineSlice(n) => as_skia_image(
            n.image(),
            target_size_fn,
            ImageFit::Preserve,
            scale_factor,
            transform_scale,
            canvas,
            surface,
        ),
        ImageInner::AnimatedImage(animated) => as_skia_image(
            animated.first_frame(),
            target_size_fn,
            image_fit,
            scale_factor,
            transform_scale,
            canvas,
            surface,
        ),
        #[cfg(any(feature = "unstable-wgpu-29", feature = "unstable-wgpu-30"))]
        ImageInner::WGPUTexture(any_wgpu_texture) => {
            surface.and_then(|surface| surface.import_wgpu_texture(canvas, any_wgpu_texture))
        }
        _ => image_inner
            .render_to_buffer(None)
            .and_then(|buffer| image_buffer_to_skia_image(&buffer)),
    }
}

fn skia_cached_image(storage: &vtable::VRc<OpaqueImageVTable>) -> Option<skia_safe::Image> {
    vtable::VRc::borrow(storage).downcast::<SkiaCachedImage>().map(|x| x.image.clone())
}

fn image_buffer_to_skia_image(buffer: &SharedImageBuffer) -> Option<skia_safe::Image> {
    // Report fully opaque images as such to Skia. This allows Skia to skip
    // reading destination pixels back when blending, which is particularly
    // important for the software renderer when it renders directly into
    // write-combined (uncached) frame buffer memory, where CPU reads are an
    // order of magnitude slower than writes. The scan happens once per image;
    // the resulting skia_safe::Image is cached.
    fn opaque_or(rgba_bytes: &[u8], alpha_type: skia_safe::AlphaType) -> skia_safe::AlphaType {
        if rgba_bytes.as_chunks::<4>().0.iter().all(|px| px[3] == 255) {
            skia_safe::AlphaType::Opaque
        } else {
            alpha_type
        }
    }

    let (data, bpl, size, color_type, alpha_type) = match buffer {
        SharedImageBuffer::RGB8(pixels) => {
            // RGB888 with one byte per component is not supported by Skia right now. Convert once to RGBA8 :-(
            let mut rgba = Vec::with_capacity(pixels.as_bytes().len() / 3 * 4);
            rgba.extend(
                pixels.as_bytes().as_chunks::<3>().0.iter().flat_map(|&[r, g, b]| [r, g, b, 255]),
            );
            (
                skia_safe::Data::new_with_owner(rgba),
                pixels.width() as usize * 4,
                pixels.size(),
                skia_safe::ColorType::RGBA8888,
                skia_safe::AlphaType::Opaque,
            )
        }
        SharedImageBuffer::RGBA8(pixels) => (
            shared_pixel_data(pixels),
            pixels.width() as usize * 4,
            pixels.size(),
            skia_safe::ColorType::RGBA8888,
            opaque_or(pixels.as_bytes(), skia_safe::AlphaType::Unpremul),
        ),
        SharedImageBuffer::RGBA8Premultiplied(pixels) => (
            shared_pixel_data(pixels),
            pixels.width() as usize * 4,
            pixels.size(),
            skia_safe::ColorType::RGBA8888,
            opaque_or(pixels.as_bytes(), skia_safe::AlphaType::Premul),
        ),
        #[cfg(feature = "image-pixel-format-rgb565")]
        SharedImageBuffer::RGB565(pixels) => (
            shared_pixel_data(pixels),
            pixels.width() as usize * 2,
            pixels.size(),
            skia_safe::ColorType::RGB565,
            skia_safe::AlphaType::Opaque,
        ),
        #[cfg(feature = "image-pixel-format-gray8")]
        SharedImageBuffer::Gray8(pixels) => (
            shared_pixel_data(pixels),
            pixels.width() as usize,
            pixels.size(),
            skia_safe::ColorType::Gray8,
            skia_safe::AlphaType::Opaque,
        ),
        _ => return None,
    };
    let image_info = crate::image_info(
        skia_safe::ISize::new(size.width as i32, size.height as i32),
        color_type,
        alpha_type,
    );
    skia_safe::images::raster_from_data(&image_info, data, bpl)
}

#[cfg(test)]
mod tests {
    use super::*;
    use i_slint_core::graphics::{Rgb8Pixel, load_image_from_embedded_data};
    use i_slint_core::slice::Slice;

    static RED_PNG: &[u8] = include_bytes!("../../../tests/screenshots/cases/image/tiny-1x1.png");

    fn to_skia(image: Image) -> skia_safe::Image {
        let mut surface = skia_safe::surfaces::raster_n32_premul((1, 1)).unwrap();
        as_skia_image(
            image,
            &|| LogicalSize::new(1., 1.),
            ImageFit::Fill,
            ScaleFactor::new(1.),
            1.,
            surface.canvas(),
            None,
        )
        .unwrap()
    }

    #[test]
    fn keyed_images_share_skia_image() {
        let load =
            || load_image_from_embedded_data(Slice::from_slice(RED_PNG), Slice::from_slice(b"png"));
        let (first, second) = (load(), load());
        let first = to_skia(first);
        let second = to_skia(second);
        assert_eq!(first.unique_id(), second.unique_id());
        let pixels = first.peek_pixels().unwrap();
        assert_eq!(pixels.color_type(), skia_safe::ColorType::RGBA8888);
        assert_eq!(pixels.bytes().unwrap(), &[255, 0, 0, 255]);
    }

    #[test]
    fn images_without_cache_key_keep_their_pixels() {
        let rgb = |pixel: &[u8]| {
            Image::from_rgb8(SharedPixelBuffer::<Rgb8Pixel>::clone_from_slice(pixel, 1, 1))
        };
        let red = to_skia(rgb(&[255, 0, 0]));
        let blue = to_skia(rgb(&[0, 0, 255]));
        assert_eq!(red.peek_pixels().unwrap().bytes().unwrap(), &[255, 0, 0, 255]);
        assert_eq!(blue.peek_pixels().unwrap().bytes().unwrap(), &[0, 0, 255, 255]);
    }
}
