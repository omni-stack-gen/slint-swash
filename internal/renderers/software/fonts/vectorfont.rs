// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

use core::num::NonZeroU16;

use alloc::rc::Rc;
use skrifa::MetadataProvider;

use crate::PhysicalLength;
use crate::fixed::Fixed;
use i_slint_common::sharedfontique::fontique;
use i_slint_core::lengths::PhysicalPx;
use i_slint_core::textlayout::{Glyph, TextShaper};

use super::RenderableVectorGlyph;
use super::swash_engine::{SwashEngine, GlyphMetrics};

// A length in font design space.
struct FontUnit;
type FontLength = euclid::Length<i32, FontUnit>;
type FontScaleFactor = euclid::Scale<f32, FontUnit, PhysicalPx>;

type GlyphCacheKey = (u64, u32, PhysicalLength, core::num::NonZeroU16);

struct RenderableGlyphWeightScale;

impl clru::WeightScale<GlyphCacheKey, RenderableVectorGlyph> for RenderableGlyphWeightScale {
    fn weight(&self, _: &GlyphCacheKey, value: &RenderableVectorGlyph) -> usize {
        value.alpha_map.len()
    }
}

type GlyphCache = clru::CLruCache<
    GlyphCacheKey,
    RenderableVectorGlyph,
    std::collections::hash_map::RandomState,
    RenderableGlyphWeightScale,
>;

i_slint_core::thread_local!(static GLYPH_CACHE: core::cell::RefCell<GlyphCache>  =
    core::cell::RefCell::new(
        clru::CLruCache::with_config(
            clru::CLruCacheConfig::new(core::num::NonZeroUsize::new(1024 * 1024).unwrap())
                .with_scale(RenderableGlyphWeightScale)
        )
    )
);

/// Swash 引擎实例，每个线程一个
i_slint_core::thread_local! {
    static SWASH_ENGINE: core::cell::RefCell<SwashEngine> =
        core::cell::RefCell::new(SwashEngine::new(1024));
}

/// 辅助函数：运行 SwashEngine 操作
fn with_swash_engine<F, R>(f: F) -> R
where
    F: FnOnce(&mut SwashEngine) -> R,
{
    SWASH_ENGINE.with(|engine| f(&mut *engine.borrow_mut()))
}

pub struct VectorFont {
    font_index: u32,
    font_blob: fontique::Blob<u8>,
    ascender: PhysicalLength,
    descender: PhysicalLength,
    height: PhysicalLength,
    pixel_size: PhysicalLength,
    x_height: PhysicalLength,
    cap_height: PhysicalLength,
}

impl VectorFont {
    pub fn new(
        font_blob: fontique::Blob<u8>,
        font_index: u32,
        pixel_size: PhysicalLength,
    ) -> Self {
        Self::new_from_blob_and_index(font_blob, font_index, pixel_size)
    }

    pub fn new_from_blob_and_index(
        font_blob: fontique::Blob<u8>,
        font_index: u32,
        pixel_size: PhysicalLength,
    ) -> Self {
        let face = skrifa::FontRef::from_index(font_blob.data(), font_index).unwrap();

        let metrics = face
            .metrics(skrifa::instance::Size::unscaled(), skrifa::instance::LocationRef::new(&[]));

        let ascender = FontLength::new(metrics.ascent as _);
        let descender = FontLength::new(metrics.descent as _);
        let height = FontLength::new((metrics.ascent - metrics.descent) as _);
        let x_height = FontLength::new(metrics.x_height.unwrap_or_default() as _);
        let cap_height = FontLength::new(metrics.cap_height.unwrap_or_default() as _);
        let units_per_em = metrics.units_per_em;
        let scale = FontScaleFactor::new(pixel_size.get() as f32 / units_per_em as f32);
        Self {
            font_index,
            font_blob,
            ascender: (ascender.cast() * scale).cast(),
            descender: (descender.cast() * scale).cast(),
            height: (height.cast() * scale).cast(),
            pixel_size,
            x_height: (x_height.cast() * scale).cast(),
            cap_height: (cap_height.cast() * scale).cast(),
        }
    }

    /// 使用 Swash 渲染字形
    pub fn render_vector_glyph(
        &self,
        glyph_id: core::num::NonZeroU16,
    ) -> Option<RenderableVectorGlyph> {
        GLYPH_CACHE.with(|cache| {
            let mut cache = cache.borrow_mut();

            let cache_key = (self.font_blob.id(), self.font_index, self.pixel_size, glyph_id);

            if let Some(entry) = cache.get(&cache_key) {
                Some(entry.clone())
            } else {
                // 使用 Swash 引擎光栅化
                let mask = with_swash_engine(|engine| {
                    engine.get_or_rasterize(
                        self.font_blob.data(),
                        self.font_blob.id(),
                        glyph_id.get(),
                        self.pixel_size.get() as f32,
                    )
                })?;

                let alpha_map: Rc<[u8]> = mask.data.into();

                // 坐标转换：Swash 的 top 是相对于 baseline 向上为正（像素坐标）
                // 渲染代码使用: baseline_y - gl_y - glyph.height 得到视觉 top 位置
                // 需要满足: baseline_y - gl_y - height = baseline_y - top
                // 因此: gl_y = top - height
                let y_offset = mask.top - mask.height as i32;

                let glyph = super::RenderableVectorGlyph {
                    x: Fixed::from_integer(mask.left),
                    y: Fixed::from_integer(y_offset),
                    width: PhysicalLength::new(mask.width.try_into().unwrap()),
                    height: PhysicalLength::new(mask.height.try_into().unwrap()),
                    alpha_map,
                    pixel_stride: mask.width.try_into().unwrap(),
                    subpixel: true, // Swash now uses BGRA subpixel rendering
                };

                cache.put_with_weight(cache_key, glyph.clone()).ok();
                Some(glyph)
            }
        })
    }

    /// 获取字形度量（使用 Swash）
    fn get_glyph_metrics(&self, glyph_id: u16) -> Option<GlyphMetrics> {
        with_swash_engine(|engine| {
            engine.glyph_metrics(
                self.font_blob.data(),
                glyph_id,
                self.pixel_size.get() as f32,
            )
        })
    }
}

impl TextShaper for VectorFont {
    type LengthPrimitive = i16;
    type Length = PhysicalLength;
    fn shape_text<GlyphStorage: core::iter::Extend<Glyph<PhysicalLength>>>(
        &self,
        text: &str,
        glyphs: &mut GlyphStorage,
    ) {
        // 使用 skrifa 获取字形 ID（替代 fontdue）
        let face = skrifa::FontRef::from_index(self.font_blob.data(), self.font_index).unwrap();
        let char_map = face.charmap();

        glyphs.extend(text.char_indices().map(|(byte_offset, char)| {
            let glyph_id = char_map.map(char).map(|id: skrifa::GlyphId| id.to_u32() as u16).unwrap_or(0);
            let glyph_id = if glyph_id != 0 {
                NonZeroU16::new(glyph_id)
            } else {
                None
            };

            let x_advance = glyph_id.map_or_else(
                || self.pixel_size.get(),
                |id| {
                    self.get_glyph_metrics(id.get())
                        .map(|m| m.advance as i16)
                        .unwrap_or_else(|| self.pixel_size.get())
                },
            );

            Glyph {
                glyph_id,
                advance: PhysicalLength::new(x_advance),
                text_byte_offset: byte_offset,
                ..Default::default()
            }
        }));
    }

    fn glyph_for_char(&self, ch: char) -> Option<Glyph<PhysicalLength>> {
        // 使用 skrifa 获取字形 ID
        let face = skrifa::FontRef::from_index(self.font_blob.data(), self.font_index).unwrap();
        let char_map = face.charmap();

        let glyph_id = char_map.map(ch).map(|id: skrifa::GlyphId| id.to_u32() as u16).unwrap_or(0);
        if glyph_id == 0 {
            return None;
        }
        let glyph_id = NonZeroU16::new(glyph_id)?;

        let advance = self.get_glyph_metrics(glyph_id.get())
            .map(|m| m.advance as i16)
            .unwrap_or_else(|| self.pixel_size.get());

        Some(Glyph {
            glyph_id: Some(glyph_id),
            advance: PhysicalLength::new(advance),
            ..Default::default()
        })
    }

    fn max_lines(&self, max_height: PhysicalLength) -> usize {
        (max_height / self.height).get() as _
    }
}

impl i_slint_core::textlayout::FontMetrics<PhysicalLength> for VectorFont {
    fn ascent(&self) -> PhysicalLength {
        self.ascender
    }

    fn height(&self) -> PhysicalLength {
        self.height
    }

    fn descent(&self) -> PhysicalLength {
        self.descender
    }

    fn x_height(&self) -> PhysicalLength {
        self.x_height
    }

    fn cap_height(&self) -> PhysicalLength {
        self.cap_height
    }
}

impl super::GlyphRenderer for VectorFont {
    fn render_glyph(&self, glyph_id: core::num::NonZeroU16) -> Option<super::RenderableGlyph> {
        self.render_vector_glyph(glyph_id).map(|glyph| super::RenderableGlyph {
            x: glyph.x,
            y: glyph.y,
            width: glyph.width,
            height: glyph.height,
            alpha_map: glyph.alpha_map.into(),
            pixel_stride: glyph.pixel_stride,
            sdf: false,
            subpixel: glyph.subpixel,
        })
    }

    fn scale_delta(&self) -> super::Fixed<u16, 8> {
        super::Fixed::from_integer(1)
    }
}
