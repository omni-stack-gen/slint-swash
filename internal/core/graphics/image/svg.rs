// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

use super::{ImageCacheKey, SharedImageBuffer, SharedPixelBuffer};
#[cfg(all(feature = "image-decoders", not(target_arch = "wasm32")))]
use crate::SharedString;
use crate::lengths::PhysicalPx;
use resvg::{tiny_skia, usvg};

pub struct ParsedSVG {
    svg_tree: usvg::Tree,
    cache_key: ImageCacheKey,
    weight_in_bytes: usize,
}

impl super::OpaqueImage for ParsedSVG {
    fn size(&self) -> crate::graphics::IntSize {
        self.size()
    }
    fn cache_key(&self) -> ImageCacheKey {
        self.cache_key.clone()
    }
}

impl core::fmt::Debug for ParsedSVG {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("ParsedSVG").finish()
    }
}

impl ParsedSVG {
    fn new(svg_tree: usvg::Tree, cache_key: ImageCacheKey, source_size: usize) -> Self {
        // The parsed tree typically costs a few times the source size, and outlining
        // a short `<text>` element expands it far more; charge a generous multiple of
        // the source size, plus a constant for the fixed cost of even a tiny tree,
        // so that the cache stays bounded without measuring the tree.
        let weight_in_bytes = source_size.saturating_mul(16).saturating_add(8192);
        Self { svg_tree, cache_key, weight_in_bytes }
    }

    pub fn size(&self) -> crate::graphics::IntSize {
        let size = self.svg_tree.size().to_int_size();
        [size.width(), size.height()].into()
    }

    pub fn cache_key(&self) -> ImageCacheKey {
        self.cache_key.clone()
    }

    /// Approximate number of bytes the parsed tree keeps alive, for cache accounting.
    pub fn weight_in_bytes(&self) -> usize {
        self.weight_in_bytes
    }

    /// Renders the SVG with the specified size, if no size is specified, get the size from the image
    #[allow(clippy::unnecessary_cast)] // Coord
    pub fn render(
        &self,
        size: Option<euclid::Size2D<u32, PhysicalPx>>,
    ) -> Result<SharedImageBuffer, usvg::Error> {
        let tree = &self.svg_tree;

        let (target_size, transform) = match size {
            Some(size) => {
                let target_size = tiny_skia::IntSize::from_wh(size.width, size.height)
                    .ok_or(usvg::Error::InvalidSize)?;
                let target_size = tree.size().to_int_size().scale_to(target_size);
                let target_size_f = target_size.to_size();

                let transform = tiny_skia::Transform::from_scale(
                    target_size_f.width() as f32 / tree.size().width() as f32,
                    target_size_f.height() as f32 / tree.size().height() as f32,
                );
                (target_size, transform)
            }
            None => (tree.size().to_int_size(), tiny_skia::Transform::default()),
        };

        let mut buffer = SharedPixelBuffer::new(target_size.width(), target_size.height());
        let mut skia_buffer = tiny_skia::PixmapMut::from_bytes(
            buffer.make_mut_bytes(),
            target_size.width(),
            target_size.height(),
        )
        .ok_or(usvg::Error::InvalidSize)?;

        resvg::render(tree, transform, &mut skia_buffer);
        Ok(SharedImageBuffer::RGBA8Premultiplied(buffer))
    }
}

#[cfg(feature = "std")]
struct RasterEntry {
    source: vtable::VWeak<super::OpaqueImageVTable, ParsedSVG>,
    size: Option<euclid::Size2D<u32, PhysicalPx>>,
    buffer: SharedImageBuffer,
    bytes: usize,
}

#[cfg(feature = "std")]
const RASTER_CACHE_BYTES: usize = 2 * 1024 * 1024;
#[cfg(feature = "std")]
const RASTER_CACHE_ENTRIES: usize = 32;

#[cfg(feature = "std")]
crate::thread_local!(static RASTER_CACHE: core::cell::RefCell<alloc::collections::VecDeque<RasterEntry>> =
    const { core::cell::RefCell::new(alloc::collections::VecDeque::new()) });

pub(super) fn render_cached(
    source: &vtable::VRc<super::OpaqueImageVTable, ParsedSVG>,
    size: Option<euclid::Size2D<u32, PhysicalPx>>,
) -> Result<SharedImageBuffer, usvg::Error> {
    #[cfg(feature = "std")]
    let weak = vtable::VRc::downgrade(source);
    #[cfg(feature = "std")]
    if let Some(buffer) = RASTER_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        let index = cache
            .iter()
            .position(|entry| vtable::VWeak::ptr_eq(&entry.source, &weak) && entry.size == size)?;
        let entry = cache.remove(index).unwrap();
        let buffer = entry.buffer.clone();
        cache.push_back(entry);
        Some(buffer)
    }) {
        return Ok(buffer);
    }

    // Rasterize outside the cache borrow. Weak keys do not retain parsed trees.
    let buffer = source.render(size)?;
    #[cfg(feature = "std")]
    RASTER_CACHE.with(|cache| {
        let bytes = match &buffer {
            SharedImageBuffer::RGB8(pixels) => pixels.as_bytes().len(),
            SharedImageBuffer::RGBA8(pixels) | SharedImageBuffer::RGBA8Premultiplied(pixels) => {
                pixels.as_bytes().len()
            }
        };
        let mut cache = cache.borrow_mut();
        cache.retain(|entry| {
            !vtable::VWeak::ptr_eq(&entry.source, &weak) && entry.source.upgrade().is_some()
        });
        if bytes > RASTER_CACHE_BYTES {
            return;
        }
        // Retain one size per source, with both a byte budget and an entry bound.
        while cache.len() >= RASTER_CACHE_ENTRIES
            || cache.iter().map(|entry| entry.bytes).sum::<usize>() + bytes > RASTER_CACHE_BYTES
        {
            cache.pop_front();
        }
        cache.push_back(RasterEntry { source: weak, size, buffer: buffer.clone(), bytes });
    });
    Ok(buffer)
}

/// Resolves SVG `<text>` fonts through the shared [`sharedfontique::svg`] bridge
/// against the running `SlintContext`'s collection, so SVG text uses the same fonts
/// as the rest of the UI.
/// Without a context the text stays unresolved, which never happens for a displayable
/// image since the platform is up by then.
///
/// Gated on `shared-parley`: without Slint's text engine there is no text layout, so
/// SVG text resolution would be moot.
#[cfg(all(feature = "shared-parley", not(target_arch = "wasm32")))]
fn svg_options() -> usvg::Options<'static> {
    use i_slint_common::sharedfontique::{self, fontique};

    fn find_font(
        families: &[fontique::QueryFamily],
        attributes: fontique::Attributes,
        require_char: Option<char>,
    ) -> Option<fontique::QueryFont> {
        crate::context::GLOBAL_CONTEXT.with(|p| {
            let ctx = p.get()?;
            let mut font_context = ctx.font_context().try_borrow_mut().ok()?;
            let parley::FontContext { collection, source_cache } = &mut font_context.inner;
            sharedfontique::svg::query_font(
                collection,
                source_cache,
                families,
                attributes,
                require_char,
            )
        })
    }

    sharedfontique::svg::options(find_font)
}

#[cfg(all(not(feature = "shared-parley"), not(target_arch = "wasm32")))]
fn svg_options() -> usvg::Options<'static> {
    usvg::Options::default()
}

#[cfg(all(feature = "image-decoders", not(target_arch = "wasm32")))]
pub fn load_from_path(
    path: &SharedString,
    cache_key: ImageCacheKey,
) -> Result<ParsedSVG, std::io::Error> {
    let svg_data = std::fs::read(std::path::Path::new(&path.as_str()))?;

    usvg::Tree::from_data(&svg_data, &svg_options())
        .map(|svg| ParsedSVG::new(svg, cache_key, svg_data.len()))
        .map_err(std::io::Error::other)
}

#[cfg(not(target_arch = "wasm32"))]
pub fn load_from_data(slice: &[u8], cache_key: ImageCacheKey) -> Result<ParsedSVG, usvg::Error> {
    usvg::Tree::from_data(slice, &svg_options())
        .map(|svg| ParsedSVG::new(svg, cache_key, slice.len()))
}

#[cfg(all(test, feature = "std", not(target_arch = "wasm32")))]
mod raster_tests {
    use super::super::{ImageInner, OpaqueImageVTable};
    use super::*;
    fn image() -> ImageInner {
        ImageInner::Svg(vtable::VRc::<OpaqueImageVTable, _>::new(load_from_data(
            br##"<svg xmlns="http://www.w3.org/2000/svg" width="32" height="16"><rect x="1" y="1" width="30" height="14" fill="#4080c0" fill-opacity="0.5"/></svg>"##,
            ImageCacheKey::Invalid,
        ).unwrap()))
    }
    fn pixels(buffer: SharedImageBuffer) -> SharedPixelBuffer<super::super::Rgba8Pixel> {
        match buffer {
            SharedImageBuffer::RGBA8Premultiplied(pixels) => pixels,
            _ => panic!("expected SVG premultiplied pixels"),
        }
    }
    #[test]
    fn svg_same_size_reuses_raster() {
        let image = image();
        let a = pixels(image.render_to_buffer(Some(euclid::size2(32, 16))).unwrap());
        let b = pixels(image.render_to_buffer(Some(euclid::size2(32, 16))).unwrap());
        assert_eq!(
            a.as_bytes().as_ptr(),
            b.as_bytes().as_ptr(),
            "same immutable SVG and size should reuse its raster"
        );
    }
    #[test]
    fn raster_resize_and_copy_on_write_preserve_pixels() {
        RASTER_CACHE.with(|cache| cache.borrow_mut().clear());
        let image = image();
        let ImageInner::Svg(source) = &image else { unreachable!() };
        for size in [
            Some(euclid::size2(32, 16)),
            Some(euclid::size2(41, 29)),
            None,
            Some(euclid::size2(32, 16)),
        ] {
            let expected = pixels(source.render(size).unwrap());
            let mut actual = pixels(image.render_to_buffer(size).unwrap());
            assert_eq!(actual.as_bytes(), expected.as_bytes());
            actual.make_mut_bytes().fill(0xff);
            let repeated = pixels(image.render_to_buffer(size).unwrap());
            assert_eq!(repeated.as_bytes(), expected.as_bytes());
            RASTER_CACHE.with(|cache| assert_eq!(cache.borrow().len(), 1));
        }
        assert!(image.render_to_buffer(Some(euclid::size2(0, 16))).is_none());
    }

    #[test]
    fn raster_keys_distinguish_sources_without_retaining_them() {
        RASTER_CACHE.with(|cache| cache.borrow_mut().clear());
        let first = image();
        let second = image();
        let a = pixels(first.render_to_buffer(None).unwrap());
        let b = pixels(second.render_to_buffer(None).unwrap());
        assert_ne!(a.as_bytes().as_ptr(), b.as_bytes().as_ptr());
        let ImageInner::Svg(source) = &first else { unreachable!() };
        let weak = vtable::VRc::downgrade(source);
        drop(first);
        assert!(weak.upgrade().is_none());
        let third = image();
        let _ = third.render_to_buffer(None).unwrap();
        RASTER_CACHE.with(|cache| {
            assert!(cache.borrow().iter().all(|entry| entry.source.upgrade().is_some()))
        });
    }

    #[test]
    fn raster_cache_bounds_entries_bytes_and_oversized_results() {
        RASTER_CACHE.with(|cache| cache.borrow_mut().clear());
        let sources: alloc::vec::Vec<_> = (0..40).map(|_| image()).collect();
        let first = pixels(sources[0].render_to_buffer(None).unwrap());
        for source in &sources[1..] {
            let _ = source.render_to_buffer(None).unwrap();
        }
        RASTER_CACHE.with(|cache| assert_eq!(cache.borrow().len(), RASTER_CACHE_ENTRIES));
        let again = pixels(sources[0].render_to_buffer(None).unwrap());
        assert_ne!(first.as_bytes().as_ptr(), again.as_bytes().as_ptr());
        for source in &sources[0..3] {
            let _ = source.render_to_buffer(Some(euclid::size2(700, 350))).unwrap();
        }
        RASTER_CACHE.with(|cache| {
            let cache = cache.borrow();
            assert!(cache.iter().map(|entry| entry.bytes).sum::<usize>() <= RASTER_CACHE_BYTES);
            assert!(cache.len() <= 2);
        });
        let size = Some(euclid::size2(2048, 1024));
        let a = pixels(sources[0].render_to_buffer(size).unwrap());
        let b = pixels(sources[0].render_to_buffer(size).unwrap());
        assert_ne!(a.as_bytes().as_ptr(), b.as_bytes().as_ptr());
        RASTER_CACHE.with(|cache| assert!(cache.borrow().iter().all(|entry| entry.size != size)));
    }
}
