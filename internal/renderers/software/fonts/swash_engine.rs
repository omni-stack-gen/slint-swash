// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

//! Swash 字体光栅化引擎
//!
//! 使用 Swash 库实现按需光栅化，配合 LRU 缓存减少内存占用

use alloc::rc::Rc;
use alloc::vec;
use alloc::vec::Vec;
use core::cell::RefCell;
use core::num::NonZeroUsize;

use swash::scale::{Render, ScaleContext, Source, StrikeWith};
use swash::zeno::{Format, Vector};
use swash::{FontRef, GlyphId};

use crate::PhysicalLength;
use crate::fixed::Fixed;

/// 单个字形的透明度蒙版数据
#[derive(Clone)]
pub struct AlphaMask {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub left: i32,
    pub top: i32,
}

/// 缓存键：(字体ID, 字形ID, 像素大小)
type CacheKey = (u64, u16, u32, u32);

/// Swash 光栅化引擎
pub struct SwashEngine {
    /// Swash 缩放上下文，复用避免重复分配
    context: ScaleContext,
    /// LRU 缓存
    cache: lru::LruCache<CacheKey, AlphaMask>,
}

impl SwashEngine {
    /// 创建新的 SwashEngine 实例
    ///
    /// # Arguments
    /// * `cache_capacity` - LRU 缓存容量（字形数量）
    pub fn new(cache_capacity: usize) -> Self {
        Self {
            context: ScaleContext::new(),
            cache: lru::LruCache::new(
                NonZeroUsize::new(cache_capacity).unwrap_or(NonZeroUsize::new(512).unwrap()),
            ),
        }
    }

    /// 获取或光栅化字形
    ///
    /// # Arguments
    /// * `font_data` - 字体原始数据（零拷贝）
    /// * `font_id` - 字体唯一标识（用于缓存键）
    /// * `glyph_id` - 字形ID
    /// * `size_px` - 像素大小
    ///
    /// # Returns
    /// * `Some(&AlphaMask)` - 字形 alpha 蒙版
    /// * `None` - 光栅化失败
    pub fn get_or_rasterize(
        &mut self,
        font_data: &[u8],
        font_id: u64,
        glyph_id: u16,
        size_px: f32,
        weight: i32,
    ) -> Option<AlphaMask> {
        let size_u32 = size_px.round() as u32;
        let key: CacheKey = (font_id, glyph_id, size_u32, weight as u32);

        // 检查缓存
        if let Some(mask) = self.cache.get(&key) {
            return Some(mask.clone());
        }

        // 缓存未命中，执行光栅化
        let mask = self.rasterize_glyph(font_data, glyph_id, size_px, weight)?;

        // 存入缓存
        self.cache.put(key, mask.clone());

        Some(mask)
    }

    /// 光栅化单个字形
    fn rasterize_glyph(
        &mut self,
        font_data: &[u8],
        glyph_id: u16,
        size_px: f32,
        weight: i32,
    ) -> Option<AlphaMask> {
        // Swash 零拷贝解析字体
        let font = FontRef::from_index(font_data, 0)?;

        // 构建缩放器（禁用 hinting 以获得更平滑的字体边缘）
        let mut scaler = self.context.builder(font).size(size_px).hint(false).build();

        // 配置渲染器，使用标准 Alpha 渲染（避免 BGRA 子像素格式不匹配问题）
        // 优先使用 Outline，然后是 ColorOutline
        let mut render = Render::new(&[Source::Outline, Source::ColorOutline(0)]);
        render.format(Format::Alpha);

        // 执行渲染
        let image = render.render(&mut scaler, glyph_id)?;

        // 提取位图数据
        let mut data = image.data.to_vec();
        let width = image.placement.width;
        let height = image.placement.height;

        // Alpha 增粗法：非线性增强半透明边缘，消除虚线感
        // 将所有非零 Alpha 值乘以 1.4，使笔画更实
        if !data.is_empty() {
            for alpha in data.iter_mut() {
                if *alpha > 0 {
                    // 方案 A：乘法增强（1.4 倍），消除笔画断裂
                    let enhanced = (*alpha as u16 * 14) / 10;
                    *alpha = enhanced.min(255) as u8;
                }
            }
        }

        // 根据字重应用合成粗体（膨胀）
        // weight 400 = 正常，0次
        // weight 500-600 = 半粗，1次
        // weight 700-800 = 粗体，1-2次（保守设置，避免过大）
        let bold_iterations = 0;
        // if weight <= 400 {
        //     0
        // } else if weight <= 600 {
        //     if size_px >= 36.0 {
        //         0 // 中大字（如24px）
        //     } else {
        //         0 // 小字不需要膨胀，保持清晰
        //     }
        // } else if weight <= 800 {
        //     // 大字只需要1-2次，小字不膨胀（避免小字糊成一团）
        //     if size_px >= 48.0 {
        //         2 // 特大字（如140px的"26"）
        //     } else if size_px >= 36.0 {
        //         1 // 中大字（如24px）
        //     } else {
        //         0 // 小字不需要膨胀，保持清晰
        //     }
        // } else {
        //     if size_px >= 48.0 { 2 } else { 1 }
        // };

        // 计算膨胀后的尺寸和坐标调整
        let (data, width, height, left_adjust, top_adjust) = if bold_iterations > 0
            && !data.is_empty()
        {
            let padding = bold_iterations as i32;
            // 添加 padding 防止裁剪
            let data_with_padding = add_padding(&data, width, height, padding);
            let new_width = width + 2 * padding as u32;
            let new_height = height + 2 * padding as u32;

            // 执行膨胀
            let dilated = dilate_alpha(&data_with_padding, new_width, new_height, bold_iterations);

            // 计算坐标调整（膨胀后向四周扩展，left/top 需要减去 padding）
            (dilated, new_width, new_height, -padding, -padding)
        } else {
            (data, width, height, 0, 0)
        };

        let mask = AlphaMask {
            data,
            width,
            height,
            left: image.placement.left + left_adjust,
            top: image.placement.top + top_adjust,
        };

        Some(mask)
    }

    /// 获取字形度量信息
    pub fn glyph_metrics(
        &self,
        font_data: &[u8],
        glyph_id: u16,
        size_px: f32,
    ) -> Option<GlyphMetrics> {
        let font = FontRef::from_index(font_data, 0)?;

        // 获取字体单位每 EM（units per em）用于缩放
        let units_per_em = font.metrics(&[]).units_per_em as f32;
        let scale = size_px / units_per_em;

        // 使用 font.glyph_metrics 获取字形度量（空坐标数组表示非变体字体）
        let glyph_metrics = font.glyph_metrics(&[]);
        let advance = glyph_metrics.advance_width(glyph_id) * scale;

        Some(GlyphMetrics { advance: advance as i16 })
    }

    /// 清除缓存
    pub fn clear_cache(&mut self) {
        self.cache.clear();
    }

    /// 获取缓存统计
    pub fn cache_stats(&self) -> (usize, usize) {
        (self.cache.len(), self.cache.cap().get())
    }
}

/// 字形度量信息
#[derive(Clone, Copy, Debug)]
pub struct GlyphMetrics {
    pub advance: i16,
}

/// Alpha 蒙版膨胀（形态学膨胀）- 用于小字体增粗
///
/// 使用全 3x3 核膨胀（8 邻居），笔画增粗更圆润均匀
/// 为 alpha 图像添加 padding（防止膨胀时边缘被裁剪）
fn add_padding(data: &[u8], width: u32, height: u32, padding: i32) -> Vec<u8> {
    if padding <= 0 {
        return data.to_vec();
    }
    let p = padding as usize;
    let w = width as usize;
    let h = height as usize;
    let new_w = w + 2 * p;
    let new_h = h + 2 * p;
    let mut result = vec![0u8; new_w * new_h];
    for y in 0..h {
        for x in 0..w {
            result[(y + p) * new_w + (x + p)] = data[y * w + x];
        }
    }
    result
}
fn dilate_alpha(data: &[u8], width: u32, height: u32, iterations: u32) -> Vec<u8> {
    if width == 0 || height == 0 {
        return data.to_vec();
    }

    let w = width as usize;
    let h = height as usize;
    let mut buffer = data.to_vec();
    let mut temp = vec![0u8; data.len()];

    for _ in 0..iterations {
        // 全 3x3 核膨胀（8 邻居：上下左右 + 四个对角）
        for y in 0..h {
            for x in 0..w {
                let idx = y * w + x;
                let mut max_val = buffer[idx];

                // 检查 8 个方向的邻居
                // 左
                if x > 0 {
                    max_val = max_val.max(buffer[idx - 1]);
                }
                // 右
                if x + 1 < w {
                    max_val = max_val.max(buffer[idx + 1]);
                }
                // 上
                if y > 0 {
                    max_val = max_val.max(buffer[idx - w]);
                }
                // 下
                if y + 1 < h {
                    max_val = max_val.max(buffer[idx + w]);
                }
                // 左上
                if x > 0 && y > 0 {
                    max_val = max_val.max(buffer[idx - w - 1]);
                }
                // 右上
                if x + 1 < w && y > 0 {
                    max_val = max_val.max(buffer[idx - w + 1]);
                }
                // 左下
                if x > 0 && y + 1 < h {
                    max_val = max_val.max(buffer[idx + w - 1]);
                }
                // 右下
                if x + 1 < w && y + 1 < h {
                    max_val = max_val.max(buffer[idx + w + 1]);
                }

                temp[idx] = max_val;
            }
        }
        std::mem::swap(&mut buffer, &mut temp);
    }

    buffer
}

/// 全局 SwashEngine 实例
///
/// 使用线程本地存储，每个线程一个实例
#[cfg(feature = "systemfonts")]
i_slint_core::thread_local! {
    static SWASH_ENGINE: RefCell<SwashEngine> = RefCell::new(SwashEngine::new(1024));
}

/// 获取全局 SwashEngine 实例的便捷函数
#[cfg(feature = "systemfonts")]
pub fn with_swash_engine<F, R>(f: F) -> R
where
    F: FnOnce(&SwashEngine) -> R,
{
    SWASH_ENGINE.with(|engine| f(&*engine.borrow()))
}
