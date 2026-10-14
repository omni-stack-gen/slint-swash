// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

use super::*;
use alloc::vec;

fn render(
    data: &[u8],
    format: TexturePixelFormat,
    extra: SceneTextureExtra,
    use_texture_path: bool,
) -> Vec<[u8; 4]> {
    let geometry = euclid::rect(7, 9, 131, 155);
    let mut dirty = PhysicalRegion::default();
    for (index, rect) in
        [euclid::rect(11, 13, 115, 71), euclid::rect(37, 61, 99, 83), euclid::rect(17, 151, 80, 15)]
            .into_iter()
            .enumerate()
    {
        dirty.rectangles[index] = rect.to_box2d();
        dirty.count += 1;
    }
    let mut pixels =
        vec![PremultipliedRgbaColor { red: 17, green: 37, blue: 61, alpha: 127 }; 173 * 181];
    let mut buffer = TargetPixelSlice { data: &mut pixels, pixel_stride: 173 };
    let mut renderer = RenderToBuffer {
        buffer: &mut buffer,
        dirty_range_cache: Vec::new(),
        dirty_region: dirty,
        scale_factor: ScaleFactor::new(1.),
    };
    let texture = SceneTexture { data, format, pixel_stride: 144, extra };
    if use_texture_path {
        renderer.process_texture_impl(geometry, texture);
    } else {
        renderer.foreach_ranges(&geometry, |line, buffer, left, right| {
            draw_functions::draw_texture_line(
                &geometry,
                PhysicalLength::new(line),
                &texture,
                buffer,
                left,
                right,
            );
        });
    }
    pixels.iter().map(|p| [p.red, p.green, p.blue, p.alpha]).collect()
}

#[test]
fn texture_tiles_match_scanlines_with_clipping_scaling_and_alpha() {
    for format in [
        TexturePixelFormat::Rgb,
        TexturePixelFormat::Rgba,
        TexturePixelFormat::RgbaPremultiplied,
        TexturePixelFormat::AlphaMap,
    ] {
        let mut data = vec![0u8; (82 * 144 + 137) * format.bpp()];
        for (i, pixel) in data.chunks_exact_mut(format.bpp()).enumerate() {
            let alpha = ((i * 113 + i / 144 * 17) % 256) as u8;
            for (channel, value) in pixel.iter_mut().enumerate() {
                *value = if channel == 3 || format == TexturePixelFormat::AlphaMap {
                    alpha
                } else {
                    let value = ((i * (channel + 13) + channel * 53) % 256) as u8;
                    if format == TexturePixelFormat::RgbaPremultiplied {
                        (value as u16 * alpha as u16 / 255) as u8
                    } else {
                        value
                    }
                };
            }
        }
        for rotation in [
            RenderingRotation::NoRotation,
            RenderingRotation::Rotate90,
            RenderingRotation::Rotate180,
            RenderingRotation::Rotate270,
        ] {
            for (dx, dy) in [(256, 256), (173, 239), (371, 481)] {
                for offset in [0, 203] {
                    for alpha in [0, 128, 255] {
                        for colorize in [Color::default(), Color::from_argb_u8(189, 40, 170, 231)] {
                            let extra = SceneTextureExtra {
                                dx: Fixed(dx),
                                dy: Fixed(dy),
                                off_x: Fixed(offset),
                                off_y: Fixed(offset / 3),
                                colorize,
                                alpha,
                                rotation,
                            };
                            let reference = render(&data, format, extra, false);
                            let actual = render(&data, format, extra, true);
                            assert_eq!(
                                actual.iter().zip(&reference).position(|(a, b)| a != b),
                                None,
                                "{format:?} {rotation:?} dx={dx} dy={dy} offset={offset} alpha={alpha}"
                            );
                        }
                    }
                }
            }
        }
    }
}
