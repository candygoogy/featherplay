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

/// featherplay patch (change 23, VENDORED.md): what the item renderer keeps of an image: the
/// Skia image, and for one made over the Slint image's own pixels instead of a copy of them
/// (`in_place`), that pixel buffer.
///
/// Why it's sound: the Skia image made in place reads bytes it doesn't own (`Data::new_bytes`).
/// They're the buffer held in `pixels`, a `SharedImageBuffer`: a reference-counted vector
/// that's never written while shared (Slint copies a shared one before any write), so its bytes
/// stay where they are, unchanged, while any clone of it lives. The Skia image is private to
/// this type: whoever has the image has the buffer, a clone of this has its own reference to
/// both, and dropping one drops `image` before `pixels` (fields drop in declaration order).
/// Outside it, the image is only lent, for a draw on the canvas the renderer draws on at that
/// moment (`image()`), never kept: `into_owned` won't give one made in place, and the item
/// renderer makes one in place only where it draws it as it is (no tiling, no colourize, which
/// would keep it in an image filter or a subset). A raster canvas reads the pixels during
/// the draw call and keeps nothing after it. The one other canvas is the picture recorder of
/// change 5, whose picture is dropped in the same block, before the caches holding these are
/// cleared; and in a pass an item's entry is only replaced before it's drawn, so the recording
/// never holds an image whose buffer was let go.
#[derive(Clone)]
pub struct CachedSkImage {
    image: skia_safe::Image,
    pixels: Option<SharedImageBuffer>,
}

impl CachedSkImage {
    /// An image Skia made of its own pixels.
    pub fn copied(image: skia_safe::Image) -> Self {
        Self { image, pixels: None }
    }

    /// The image, lent for a draw now.
    pub fn image(&self) -> &skia_safe::Image {
        &self.image
    }

    /// The image to keep: only one Skia made of its own pixels.
    pub fn into_owned(self) -> Option<skia_safe::Image> {
        if self.pixels.is_none() { Some(self.image) } else { None }
    }

    /// The Skia image over `image`'s own pixels, without a copy: for an image the app made
    /// from a pixel buffer (no cache key, so the image cache never replaces it with a Skia
    /// copy of its own) in four bytes a pixel. `None` for any other: those take the copy.
    pub fn in_place(image: &Image) -> Option<Self> {
        let ImageInner::EmbeddedImage { buffer, cache_key: ImageCacheKey::Invalid } =
            <&ImageInner>::from(image)
        else {
            return None;
        };
        let (bytes, width, height, alpha_type) = match buffer {
            SharedImageBuffer::RGBA8(p) => {
                (p.as_bytes(), p.width(), p.height(), skia_safe::AlphaType::Unpremul)
            }
            SharedImageBuffer::RGBA8Premultiplied(p) => {
                (p.as_bytes(), p.width(), p.height(), skia_safe::AlphaType::Premul)
            }
            SharedImageBuffer::RGB8(_) => return None,
        };
        if width == 0 || height == 0 {
            return None;
        }
        // featherplay patch (change 24): the buffer `in_bgra` names holds BGRA, and is
        // looked over for transparency once, not by every element that draws it.
        let named = IN_BGRA.with(|b| {
            let named = b.borrow();
            named
                .as_ref()
                .filter(|(b, _)| std::ptr::eq(b.as_bytes().as_ptr(), bytes.as_ptr()))
                .map(|(_, alpha)| alpha.get())
        });
        let bgra = named.is_some();
        let alpha_type = named.flatten().unwrap_or_else(|| {
            let alpha = opaque_or(bytes, alpha_type);
            if bgra {
                IN_BGRA.with(|b| {
                    if let Some((_, known)) = b.borrow().as_ref() {
                        known.set(Some(alpha));
                    }
                });
            }
            alpha
        });
        let color_type =
            if bgra { skia_safe::ColorType::BGRA8888 } else { skia_safe::ColorType::RGBA8888 };
        let image_info = crate::image_info(
            skia_safe::ISize::new(width as i32, height as i32),
            color_type,
            alpha_type,
        );
        let pixels = buffer.clone();
        // SAFETY: the bytes are `pixels`' (a clone of the buffer they're in, the same
        // allocation), which this value keeps for as long as the image lives; see the type's
        // note.
        let data = unsafe { skia_safe::Data::new_bytes(bytes) };
        let image = skia_safe::images::raster_from_data(&image_info, data, width as usize * 4)?;
        Some(Self { image, pixels: Some(pixels) })
    }
}

thread_local! {
    /// featherplay patch (change 24, VENDORED.md): `in_bgra`'s buffer, and its alpha type
    /// once an element has drawn it.
    static IN_BGRA: std::cell::RefCell<
        Option<(SharedPixelBuffer<Rgba8Pixel>, std::cell::Cell<Option<skia_safe::AlphaType>>)>,
    > = const { std::cell::RefCell::new(None) };
}

/// featherplay patch (change 24, VENDORED.md): the one pixel buffer of the app's whose bytes
/// are in BGRA, the order the window surfaces draw in, instead of RGBA (`None`: no buffer).
/// Drawn as it is from an `Image::from_rgba8` or `from_rgba8_premultiplied` of it (change 23),
/// Skia copies it instead of swapping each pixel's red and blue. Name it before the image is
/// set, and let it go (`None`) with the image: a clone of the buffer is kept until then, so
/// its bytes can't be another buffer's. Per thread, the renderer's.
pub fn in_bgra(buffer: Option<SharedPixelBuffer<Rgba8Pixel>>) {
    IN_BGRA.with(|b| *b.borrow_mut() = buffer.map(|b| (b, std::cell::Cell::new(None))));
}

/// Report fully opaque images as such to Skia. This allows Skia to skip
/// reading destination pixels back when blending, which is particularly
/// important for the software renderer when it renders directly into
/// write-combined (uncached) frame buffer memory, where CPU reads are an
/// order of magnitude slower than writes. The scan happens once per image;
/// the resulting skia_safe::Image is cached.
fn opaque_or(rgba_bytes: &[u8], alpha_type: skia_safe::AlphaType) -> skia_safe::AlphaType {
    if rgba_bytes.as_chunks::<4>().0.iter().all(|px| px[3] == 255) {
        skia_safe::AlphaType::Opaque
    } else {
        alpha_type
    }
}

pub(crate) fn as_skia_image(
    image: Image,
    target_size_fn: &dyn Fn() -> LogicalSize,
    image_fit: ImageFit,
    scale_factor: ScaleFactor,
    canvas: &skia_safe::Canvas,
    surface: Option<&dyn crate::Surface>,
) -> Option<skia_safe::Image> {
    // featherplay patch (change 18): only the SVG arm read it.
    let _ = image_fit;
    let image_inner: &ImageInner = (&image).into();
    match image_inner {
        ImageInner::None => None,
        ImageInner::EmbeddedImage { buffer, cache_key } => {
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
            let target_size = i_slint_core::graphics::scalable_render_size(
                svg.size(),
                image_fit,
                target_size_fn() * scale_factor,
                scale_factor,
                Default::default(),
            )?;
            let pixels = match svg.render(Some(target_size)).ok()? {
                SharedImageBuffer::RGB8(_) => unreachable!(),
                SharedImageBuffer::RGBA8(_) => unreachable!(),
                SharedImageBuffer::RGBA8Premultiplied(pixels) => pixels,
            };

            let image_info = crate::image_info(
                skia_safe::ISize::new(pixels.width() as i32, pixels.height() as i32),
                skia_safe::ColorType::RGBA8888,
                skia_safe::AlphaType::Premul,
            );

            skia_safe::images::raster_from_data(
                &image_info,
                skia_safe::Data::new_copy(pixels.as_bytes()),
                pixels.width() as usize * 4,
            )
        }
        ImageInner::StaticTextures(_) => todo!(),
        ImageInner::BackendStorage(x) => {
            vtable::VRc::borrow(x).downcast::<SkiaCachedImage>().map(|x| x.image.clone())
        }
        ImageInner::BorrowedOpenGLTexture(texture) => {
            surface.and_then(|surface| surface.import_opengl_texture(canvas, texture))
        }
        ImageInner::NineSlice(n) => as_skia_image(
            n.image(),
            target_size_fn,
            ImageFit::Preserve,
            scale_factor,
            canvas,
            surface,
        ),
        #[cfg(any(feature = "unstable-wgpu-29", feature = "unstable-wgpu-30"))]
        ImageInner::WGPUTexture(any_wgpu_texture) => {
            surface.and_then(|surface| surface.import_wgpu_texture(canvas, any_wgpu_texture))
        }
        #[allow(unreachable_patterns)]
        _ => None,
    }
}

fn image_buffer_to_skia_image(buffer: &SharedImageBuffer) -> Option<skia_safe::Image> {
    let (data, bpl, size, color_type, alpha_type) = match buffer {
        SharedImageBuffer::RGB8(pixels) => {
            // RGB888 with one byte per component is not supported by Skia right now. Convert once to RGBA8 :-(
            let rgba = pixels
                .as_bytes()
                .chunks(3)
                .flat_map(|rgb| IntoIterator::into_iter([rgb[0], rgb[1], rgb[2], 255]))
                .collect::<Vec<u8>>();
            (
                skia_safe::Data::new_copy(&rgba),
                pixels.width() as usize * 4,
                pixels.size(),
                skia_safe::ColorType::RGBA8888,
                skia_safe::AlphaType::Opaque,
            )
        }
        SharedImageBuffer::RGBA8(pixels) => (
            skia_safe::Data::new_copy(pixels.as_bytes()),
            pixels.width() as usize * 4,
            pixels.size(),
            skia_safe::ColorType::RGBA8888,
            opaque_or(pixels.as_bytes(), skia_safe::AlphaType::Unpremul),
        ),
        SharedImageBuffer::RGBA8Premultiplied(pixels) => (
            skia_safe::Data::new_copy(pixels.as_bytes()),
            pixels.width() as usize * 4,
            pixels.size(),
            skia_safe::ColorType::RGBA8888,
            opaque_or(pixels.as_bytes(), skia_safe::AlphaType::Premul),
        ),
    };
    let image_info = crate::image_info(
        skia_safe::ISize::new(size.width as i32, size.height as i32),
        color_type,
        alpha_type,
    );
    skia_safe::images::raster_from_data(&image_info, data, bpl)
}
