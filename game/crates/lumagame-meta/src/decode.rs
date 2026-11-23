use crate::{Error, Kind, Result};
use image::{GenericImageView, ImageReader, RgbaImage, imageops::{self, FilterType}};
use std::io::Cursor;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Density { Compact, Standard, Cinematic }
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageSpec { pub width: u32, pub height: u32, pub contain: bool, pub trim_alpha: bool }
impl ImageSpec {
    /// Reference-layout pixels from DESIGN.md. Art is never tinted, rounded or given a theme scrim here.
    pub fn for_kind(kind: Kind, density: Density) -> Self {
        let width = match density { Density::Compact => 224, Density::Standard => 280, Density::Cinematic => 336 };
        let (width, height, contain, trim_alpha) = match kind {
            Kind::Cover => (width, width * 3 / 2, false, false),
            Kind::Hero => (1920, 1080, false, false),
            Kind::Logo => (900, match density { Density::Compact => 72, Density::Standard => 120, Density::Cinematic => 176 }, true, true),
            Kind::Header => (448, 252, false, false),
            Kind::Icon => (128, 128, true, false),
        };
        Self { width, height, contain, trim_alpha }
    }
}
/// Premultiplied RGBA, tightly packed top-down. Transparent logo padding remains transparent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image { pub width: u32, pub height: u32, pub rgba: Vec<u8> }

pub(crate) fn read(bytes: &[u8]) -> Result<image::DynamicImage> {
    if bytes.len() > super::providers::IMAGE_LIMIT { return Err(Error::Invalid("image byte limit exceeded")); }
    let mut reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    // An animated GIF or WebP decodes to its first frame: moving pictures play on a video layer, never as artwork.
    if !matches!(reader.format(), Some(image::ImageFormat::Jpeg | image::ImageFormat::Png | image::ImageFormat::WebP | image::ImageFormat::Gif)) {
        return Err(Error::Invalid("artwork is not JPEG, PNG, WebP or GIF"));
    }
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(96 << 20);
    reader.limits(limits);
    reader.decode().map_err(|_| Error::Invalid("artwork is damaged or exceeds decode limits"))
}

pub(crate) fn normalize(mut source: image::DynamicImage, spec: ImageSpec) -> Result<Image> {
    if spec.width == 0 || spec.height == 0 || u64::from(spec.width) * u64::from(spec.height) > 1920 * 1080 {
        return Err(Error::Invalid("invalid normalized artwork size"));
    }
    if spec.trim_alpha {
        let rgba = source.to_rgba8();
        let (mut left, mut top, mut right, mut bottom) = (rgba.width(), rgba.height(), 0, 0);
        for (x, y, pixel) in rgba.enumerate_pixels() {
            if pixel[3] != 0 { left = left.min(x); top = top.min(y); right = right.max(x + 1); bottom = bottom.max(y + 1); }
        }
        if right > left && bottom > top { source = source.crop_imm(left, top, right - left, bottom - top); }
    }
    let (sw, sh) = source.dimensions();
    if sw == 0 || sh == 0 { return Err(Error::Invalid("empty artwork")); }
    let sx = spec.width as f64 / sw as f64;
    let sy = spec.height as f64 / sh as f64;
    let scale = if spec.contain { sx.min(sy) } else { sx.max(sy) };
    // Crop before resizing so a pathological aspect ratio cannot allocate a huge intermediate.
    let (cw, ch) = if spec.contain { (sw, sh) } else {
        (((spec.width as f64 / scale).round() as u32).clamp(1, sw), ((spec.height as f64 / scale).round() as u32).clamp(1, sh))
    };
    let mut pixels = source.crop_imm((sw - cw) / 2, (sh - ch) / 2, cw, ch).into_rgba8();
    // Resize premultiplied pixels to avoid dark fringes around transparent logos.
    for pixel in pixels.pixels_mut() {
        let alpha = u16::from(pixel[3]);
        for channel in &mut pixel.0[..3] { *channel = ((u16::from(*channel) * alpha + 127) / 255) as u8; }
    }
    let (dw, dh) = if spec.contain {
        (((sw as f64 * scale).round() as u32).clamp(1, spec.width), ((sh as f64 * scale).round() as u32).clamp(1, spec.height))
    } else { (spec.width, spec.height) };
    let resized = imageops::resize(&pixels, dw, dh, FilterType::Triangle);
    let rgba = if spec.contain && (dw != spec.width || dh != spec.height) {
        let mut canvas = RgbaImage::new(spec.width, spec.height);
        imageops::replace(&mut canvas, &resized, i64::from((spec.width - dw) / 2), i64::from((spec.height - dh) / 2));
        canvas.into_raw()
    } else { resized.into_raw() };
    Ok(Image { width: spec.width, height: spec.height, rgba })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn design_sizes_are_density_specific() {
        for (density, width, height) in [(Density::Compact, 224, 72), (Density::Standard, 280, 120), (Density::Cinematic, 336, 176)] {
            let cover = ImageSpec::for_kind(Kind::Cover, density);
            assert_eq!((cover.width, cover.height), (width, width * 3 / 2));
            assert_eq!(ImageSpec::for_kind(Kind::Logo, density).height, height);
        }
    }
    #[test]
    fn cover_crops_the_middle_without_stretching_or_tinting() {
        let mut rgba = RgbaImage::from_pixel(6, 2, image::Rgba([255, 0, 0, 255]));
        for y in 0..2 { for x in 2..4 { rgba.put_pixel(x, y, image::Rgba([20, 180, 90, 255])); } }
        let image = normalize(rgba.into(), ImageSpec { width: 2, height: 2, contain: false, trim_alpha: false }).unwrap();
        assert!(image.rgba.chunks_exact(4).all(|p| p == [20, 180, 90, 255]));
    }
    #[test]
    fn logo_trims_alpha_contains_and_premultiplies() {
        let mut rgba = RgbaImage::new(8, 8);
        rgba.put_pixel(3, 3, image::Rgba([200, 100, 50, 128]));
        rgba.put_pixel(4, 3, image::Rgba([200, 100, 50, 128]));
        let image = normalize(rgba.into(), ImageSpec { width: 4, height: 4, contain: true, trim_alpha: true }).unwrap();
        assert_eq!(&image.rgba[..16], &[0; 16]);
        assert_eq!(&image.rgba[16..20], &[100, 50, 25, 128]);
        assert_eq!(&image.rgba[48..], &[0; 16]);
    }
    #[test]
    fn supported_encodings_decode_but_garbage_and_oversized_output_do_not() {
        let source = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(2, 2, image::Rgb([60, 70, 80])));
        for format in [image::ImageFormat::Png, image::ImageFormat::Jpeg, image::ImageFormat::WebP, image::ImageFormat::Gif] {
            let mut encoded = Cursor::new(Vec::new());
            source.write_to(&mut encoded, format).unwrap();
            assert_eq!(read(encoded.get_ref()).unwrap().dimensions(), (2, 2));
        }
        assert!(read(b"not an image").is_err());
        assert!(normalize(source, ImageSpec { width: 9000, height: 9000, contain: false, trim_alpha: false }).is_err());
    }
    #[test]
    fn an_animated_gif_reads_as_its_first_frame() {
        let frame = |rgb: [u8; 3]| image::Frame::new(RgbaImage::from_pixel(2, 2, image::Rgba([rgb[0], rgb[1], rgb[2], 255])));
        let mut encoded = Vec::new();
        image::codecs::gif::GifEncoder::new(&mut encoded).encode_frames([frame([255, 0, 0]), frame([0, 0, 255])]).unwrap();
        assert_eq!(read(&encoded).unwrap().to_rgba8().get_pixel(0, 0).0, [255, 0, 0, 255]);
    }
}
