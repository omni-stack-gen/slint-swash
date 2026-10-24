// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

use super::*;

use i_slint_core::graphics::IntSize;
pub use i_slint_core::graphics::TexturePixelFormat;

/// The pixel data of a for the source of a [`Texture`].
#[derive(Clone)]
#[non_exhaustive]
pub struct TextureData<'a> {
    /// A reference to the pixel bytes of the texture. These bytes are in the format specified by `pixel_format`.
    pub data: &'a [u8],
    /// The pixel format of the texture.
    pub pixel_format: TexturePixelFormat,
    /// The number of bytes between two lines in the data
    pub byte_stride: usize,
    /// The width of the texture in pixels.
    pub width: u32,
    /// The height of the texture in pixels.
    pub height: u32,
}

impl<'a> TextureData<'a> {
    pub fn new(
        data: &'a [u8],
        pixel_format: TexturePixelFormat,
        byte_stride: usize,
        size: IntSize,
    ) -> Self {
        let (width, height) = (size.width, size.height);
        Self { data, pixel_format, byte_stride, width, height }
    }
}

pub(super) enum TextureDataContainer {
    Static(TextureData<'static>),
    Shared { buffer: super::scene::SharedBufferData, source_rect: PhysicalRect },
}

#[derive(Debug, Clone)]
pub struct TilingInfo {
    /// Offset, in destination pixel of the left border of the tile.
    pub offset_x: i32,
    /// Offset, in destination pixel, of the top border of the tile.
    pub offset_y: i32,

    /// Scale factor in the x direction, this is the same as source's width / destination's width of the tile
    pub scale_x: f32,
    /// Scale factor in the y direction, this is the same as source's height / destination's height of the tile
    pub scale_y: f32,

    /// Gap in destination pixel between two tiles on the horizontal source axis.
    pub gap_x: u32,
    /// Gap in destination pixel between two tiles on the vertical source axis.
    pub gap_y: u32,
}

/// This structure describes the properties of a texture for blending with [`TargetPixelBuffer::draw_texture`].
#[non_exhaustive]
pub struct DrawTextureArgs {
    pub(super) data: TextureDataContainer,

    /// When set, the source is to be considered as an alpha map (so for ARGB texture, the RGB component will be ignored).
    /// And the given color is to be blended using the alpha value of the texture.
    pub colorize: Option<Color>,

    /// A value between 0 and 255 that specifies the alpha value of the texture.
    /// If colorize is set, this value can be ignored as the alpha would be part of the `colorize` value.
    /// A value of 0 would mean that the texture is fully transparent (so nothing is drawn),
    /// and a value of 255 would mean fully opaque.
    pub alpha: u8,

    /// The x position in the destination buffer to draw the texture at
    pub dst_x: isize,
    /// The y position in the destination buffer to draw the texture at
    pub dst_y: isize,
    /// The width of the image in the destination. The image should be scaled to fit.
    pub dst_width: usize,
    /// The height of the image in the destination. The Image should be scaled to fit
    pub dst_height: usize,

    /// the rotation to apply to the texture
    pub rotation: RenderingRotation,

    /// If the texture is to be tiled, this contains the information about the tiling
    pub tiling: Option<TilingInfo>,
}

/// A retained identity for an immutable source image view.
///
/// Comparing keys does not compare pixel contents.
/// Keeping a key alive retains its image storage,
/// so modifying a shared pixel buffer detaches the modified pixels through copy-on-write.
#[cfg(feature = "experimental")]
#[derive(Clone, PartialEq)]
pub struct TextureCacheKey {
    image: SharedImageBuffer,
    image_size: IntSize,
    source_rect: PhysicalRect,
}

impl DrawTextureArgs {
    /// Returns a retained cache key for this source view, when its storage supports one.
    ///
    /// Equal keys guarantee equal source pixels and pixel format.
    /// Different keys may contain equal pixels.
    /// Drawing attributes such as opacity and destination position are not part of the key.
    #[cfg(feature = "experimental")]
    pub fn source_cache_key(&self) -> Option<TextureCacheKey> {
        match &self.data {
            TextureDataContainer::Shared {
                buffer: SharedBufferData::SharedImage(image),
                source_rect,
            } => Some(TextureCacheKey {
                image: image.clone(),
                image_size: image.size(),
                source_rect: *source_rect,
            }),
            _ => None,
        }
    }

    /// Returns the source image data for this texture
    pub fn source(&self) -> TextureData<'_> {
        match &self.data {
            TextureDataContainer::Static(data) => data.clone(),
            TextureDataContainer::Shared { buffer, source_rect } => {
                let stride = buffer.width();
                let core::ops::Range { start, end } = compute_range_in_buffer(source_rect, stride);
                let size = source_rect.size.to_untyped().cast();

                match &buffer {
                    SharedBufferData::SharedImage(SharedImageBuffer::RGB8(b)) => TextureData::new(
                        &b.as_bytes()[start * 3..end * 3],
                        TexturePixelFormat::Rgb,
                        stride * 3,
                        size,
                    ),
                    SharedBufferData::SharedImage(SharedImageBuffer::RGBA8(b)) => TextureData::new(
                        &b.as_bytes()[start * 4..end * 4],
                        TexturePixelFormat::Rgba,
                        stride * 4,
                        size,
                    ),
                    SharedBufferData::SharedImage(SharedImageBuffer::RGBA8Premultiplied(b)) => {
                        TextureData::new(
                            &b.as_bytes()[start * 4..end * 4],
                            TexturePixelFormat::RgbaPremultiplied,
                            stride * 4,
                            size,
                        )
                    }
                    SharedBufferData::AlphaMap { data, .. } => TextureData::new(
                        &data[start..end],
                        TexturePixelFormat::AlphaMap,
                        stride,
                        size,
                    ),
                }
            }
        }
    }

    pub(super) fn source_size(&self) -> PhysicalSize {
        match &self.data {
            TextureDataContainer::Static(data) => {
                PhysicalSize::new(data.width as _, data.height as _)
            }
            TextureDataContainer::Shared { source_rect, .. } => source_rect.size,
        }
    }
}

/// This structure describes the properties of a rectangle for blending with [`TargetPixelBuffer::draw_rectangle`].
///
/// All the coordinate are in physical pixels
#[non_exhaustive]
#[derive(Default, Debug)]
pub struct DrawRectangleArgs {
    /// The x position in the destination buffer
    pub x: f32,
    /// The y position in the destination buffer
    pub y: f32,
    /// The width of the image in the destination.
    pub width: f32,
    /// The height of the image in the destination.
    pub height: f32,

    /// The top-left radius.
    pub top_left_radius: f32,
    /// The top-right radius.
    pub top_right_radius: f32,
    /// The bottom-right radius.
    pub bottom_right_radius: f32,
    /// The bottom-left radius.
    pub bottom_left_radius: f32,

    /// The width of the border.
    pub border_width: f32,

    /// The background of the rectangle
    pub background: Brush,
    /// The border of the rectangle
    pub border: Brush,

    /// A value between 0 and 255 that specifies the opacity.
    /// A value of 0 would mean that the rectangle is fully transparent (so nothing is drawn),
    /// and a value of 255 would mean fully opaque.
    /// Note that the brush also might have an alpha value and the two values should be combined.
    pub alpha: u8,
    /// An extra rotation that should be applied to the gradient (and only to the gradient, it doesn't impact the border radius)
    pub rotation: RenderingRotation,
}

impl DrawRectangleArgs {
    pub(super) fn from_rect(geometry: euclid::Rect<f32, PhysicalPx>, background: Brush) -> Self {
        Self {
            x: geometry.origin.x,
            y: geometry.origin.y,
            width: geometry.size.width,
            height: geometry.size.height,
            background,
            alpha: 255,
            ..Default::default()
        }
    }

    pub(super) fn geometry(&self) -> euclid::Rect<f32, PhysicalPx> {
        euclid::rect(self.x, self.y, self.width, self.height)
    }
}

/// This trait represents access to a buffer of pixels the software renderer can render into, as well
/// as certain operations that the renderer will try to delegate to this trait. Implement these functions
/// to delegate rendering further to hardware-provided 2D acceleration units, such as DMA2D or PXP.
pub trait TargetPixelBuffer {
    /// The pixel type the buffer represents.
    type TargetPixel: TargetPixel;

    /// Returns a slice of pixels for the given line.
    fn line_slice(&mut self, line_number: usize) -> &mut [Self::TargetPixel];

    /// Returns the number of lines the buffer has. This is typically the height in pixels.
    fn num_lines(&self) -> usize;

    /// Fill the background of the buffer with the given brush.
    fn fill_background(&mut self, _brush: &Brush, _region: &PhysicalRegion) -> bool {
        false
    }

    /// Draw a rectangle specified by the DrawRectangleArgs. That rectangle must be clipped to the given region
    fn draw_rectangle(&mut self, _: &DrawRectangleArgs, _clip: &PhysicalRegion) -> bool {
        false
    }

    /// Draw a texture into the buffer.
    /// The texture must be clipped to the given region.
    /// Returns true if the operation was successful; false if it could not be
    /// implemented and instead the software renderer needs to draw the texture
    fn draw_texture(&mut self, _: &DrawTextureArgs, _clip: &PhysicalRegion) -> bool {
        false
    }
}

#[cfg(all(test, feature = "experimental"))]
mod texture_cache_key_tests {
    use super::*;

    fn texture(image: SharedImageBuffer, source_rect: PhysicalRect) -> DrawTextureArgs {
        DrawTextureArgs {
            data: TextureDataContainer::Shared {
                buffer: SharedBufferData::SharedImage(image),
                source_rect,
            },
            colorize: None,
            alpha: 255,
            dst_x: 0,
            dst_y: 0,
            dst_width: source_rect.size.width as usize,
            dst_height: source_rect.size.height as usize,
            rotation: RenderingRotation::NoRotation,
            tiling: None,
        }
    }

    #[test]
    fn texture_cache_key_retains_pixels_across_copy_on_write() {
        let mut pixels = SharedPixelBuffer::<Rgba8Pixel>::new(2, 1);
        pixels.make_mut_bytes().fill(255);
        let rect = euclid::rect(0, 0, 2, 1);
        let key =
            texture(SharedImageBuffer::RGBA8(pixels.clone()), rect).source_cache_key().unwrap();
        let same =
            texture(SharedImageBuffer::RGBA8(pixels.clone()), rect).source_cache_key().unwrap();
        assert!(key == same);
        pixels.make_mut_bytes()[0] = 17;
        let changed = texture(SharedImageBuffer::RGBA8(pixels), rect).source_cache_key().unwrap();
        assert!(key != changed);
        assert!(key == same);
    }

    #[test]
    fn texture_cache_key_distinguishes_views_formats_and_allocations() {
        let pixels = SharedPixelBuffer::<Rgba8Pixel>::new(2, 1);
        let first = texture(SharedImageBuffer::RGBA8(pixels.clone()), euclid::rect(0, 0, 1, 1))
            .source_cache_key()
            .unwrap();
        let second = texture(SharedImageBuffer::RGBA8(pixels.clone()), euclid::rect(1, 0, 1, 1))
            .source_cache_key()
            .unwrap();
        let other_format =
            texture(SharedImageBuffer::RGBA8Premultiplied(pixels), euclid::rect(0, 0, 1, 1))
                .source_cache_key()
                .unwrap();
        let other_allocation = texture(
            SharedImageBuffer::RGBA8(SharedPixelBuffer::<Rgba8Pixel>::new(2, 1)),
            euclid::rect(0, 0, 1, 1),
        )
        .source_cache_key()
        .unwrap();
        assert!(first != second);
        assert!(first != other_format);
        assert!(first != other_allocation);
    }

    #[test]
    fn texture_cache_key_is_optional_for_static_data() {
        let mut args = texture(
            SharedImageBuffer::RGBA8(SharedPixelBuffer::<Rgba8Pixel>::new(1, 1)),
            euclid::rect(0, 0, 1, 1),
        );
        args.data = TextureDataContainer::Static(TextureData::new(
            &[0; 4],
            TexturePixelFormat::Rgba,
            4,
            IntSize::new(1, 1),
        ));
        assert!(args.source_cache_key().is_none());
    }
}
