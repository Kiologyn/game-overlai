//! Frame -> JPEG for the agent, at full resolution (no downscaling).
//!
//! JPEG q90: PNG of a busy 1440p game frame is 5–10 MB, over Claude's 5 MB (base64) image
//! limit, and several times slower to encode. Chroma is subsampled (4:2:0): text shapes live in
//! the full-resolution luma, and 4:4:4 cost ~60% more time for no visible gain to the model.
//! `jpeg-encoder` (SIMD) takes BGRA as it comes from the GPU, so there is no conversion pass.
//! Measured: 2560×1440 with heavy grain ~210 ms (debug build, the encoder itself is optimised).

use fast_image_resize::images::{Image as ResizeImage, ImageRef};
use fast_image_resize::{FilterType, PixelType, ResizeAlg, ResizeOptions, Resizer};
use jpeg_encoder::{ColorType, Encoder, SamplingFactor};

use super::Frame;

/// Longest side of a frame sent to the agent: Claude rejects images over 2000 px once a request
/// has more than 20 of them (long chats with frames do), whatever `agent_max_height` says.
pub const AGENT_MAX_SIDE: u32 = 2000;

/// Size of the frame for the agent: at most `max_height` tall (0 = no limit) and
/// `AGENT_MAX_SIDE` on either side, aspect ratio kept, never upscaled.
pub fn agent_size(width: u32, height: u32, max_height: u32) -> (u32, u32) {
    let mut scale = 1.0_f64;
    if max_height > 0 && height > max_height {
        scale = scale.min(max_height as f64 / height as f64);
    }
    let longest = width.max(height);
    if longest > AGENT_MAX_SIDE {
        scale = scale.min(AGENT_MAX_SIDE as f64 / longest as f64);
    }
    if scale >= 1.0 {
        return (width, height);
    }
    let w = ((width as f64 * scale).round() as u32).clamp(1, AGENT_MAX_SIDE);
    let h = ((height as f64 * scale).round() as u32).clamp(1, AGENT_MAX_SIDE);
    (w, h)
}

/// The frame scaled to `width`×`height` (Lanczos: keeps small text readable).
pub fn downscale(frame: &Frame, width: u32, height: u32) -> Result<Frame, String> {
    let src = ImageRef::new(frame.width, frame.height, &frame.bgra, PixelType::U8x4)
        .map_err(|e| tr!("err_frame", error = e))?;
    let mut dst = ResizeImage::new(width, height, PixelType::U8x4);
    let options = ResizeOptions::new()
        .resize_alg(ResizeAlg::Convolution(FilterType::Lanczos3))
        .use_alpha(false); // BGRA from the GPU is opaque
    Resizer::new().resize(&src, &mut dst, &options).map_err(|e| tr!("err_frame_scale", error = e))?;
    Ok(Frame { width, height, bgra: dst.into_vec(), source: frame.source, taken: frame.taken })
}

/// Encoded bytes; base64 of this stays under 5 MB (5,242,880).
pub const MAX_BYTES: usize = 3_800_000;
/// Tried in order until the image fits (only huge or very noisy frames get past the first).
const QUALITIES: [u8; 5] = [90, 85, 78, 70, 60];

pub struct Image {
    pub bytes: Vec<u8>,
    pub mime: &'static str,
    pub width: u32,
    pub height: u32,
}

impl std::fmt::Debug for Image {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Image({}x{} {} {:.2} MB)", self.width, self.height, self.mime, self.bytes.len() as f64 / 1e6)
    }
}

pub fn encode(frame: &Frame) -> Result<Image, String> {
    let size = format!("{}×{}", frame.width, frame.height);
    let too_big = || tr!("err_frame_too_big", size = size);
    let w = u16::try_from(frame.width).map_err(|_| too_big())?;
    let h = u16::try_from(frame.height).map_err(|_| too_big())?;
    for quality in QUALITIES {
        let mut bytes = Vec::with_capacity(2 << 20);
        let mut encoder = Encoder::new(&mut bytes, quality);
        encoder.set_sampling_factor(SamplingFactor::R_4_2_0);
        encoder.encode(&frame.bgra, w, h, ColorType::Bgra).map_err(|e| format!("JPEG: {e}"))?;
        if bytes.len() <= MAX_BYTES {
            return Ok(Image { bytes, mime: "image/jpeg", width: frame.width, height: frame.height });
        }
    }
    Err(tr!("err_frame_over_limit", size = size))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::Source;
    use std::time::Instant;

    fn frame(w: u32, h: u32, mut f: impl FnMut(u32, u32) -> [u8; 3]) -> Frame {
        let mut bgra = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                let [r, g, b] = f(x, y);
                bgra.extend_from_slice(&[b, g, r, 255]);
            }
        }
        Frame { width: w, height: h, bgra, source: Source::Window, taken: Instant::now() }
    }

    #[test]
    fn colours_survive_bgra_input() {
        let img = encode(&frame(64, 32, |x, _| if x < 32 { [200, 0, 0] } else { [0, 0, 200] })).unwrap();
        assert_eq!((img.mime, img.width, img.height), ("image/jpeg", 64, 32));
        let decoded = image::load_from_memory(&img.bytes).unwrap().to_rgb8();
        let [r, _, b] = decoded.get_pixel(4, 16).0;
        assert!(r > 150 && b < 50, "left half should be red, got {:?}", decoded.get_pixel(4, 16));
        let [r, _, b] = decoded.get_pixel(60, 16).0;
        assert!(b > 150 && r < 50, "right half should be blue, got {:?}", decoded.get_pixel(60, 16));
    }

    #[test]
    fn busy_1440p_frame_fits_the_limit() {
        // Gradient + grain, like a busy game scene (~9 MB as PNG).
        let mut seed = 1u32;
        let f = frame(2560, 1440, |x, y| {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let grain = (seed >> 24) as u8 / 8;
            [(x / 11) as u8, (y / 6) as u8, ((x + y) / 16) as u8].map(|c| c.saturating_add(grain))
        });
        let img = encode(&f).unwrap();
        assert!(img.bytes.len() <= MAX_BYTES, "{img:?}");
        assert_eq!((img.width, img.height), (2560, 1440));
    }

    #[test]
    fn agent_sizes() {
        assert_eq!(agent_size(2560, 1440, 720), (1280, 720));
        assert_eq!(agent_size(3440, 1440, 720), (1720, 720)); // 21:9 keeps its shape
        assert_eq!(agent_size(1280, 720, 720), (1280, 720)); // no upscaling or work
        assert_eq!(agent_size(1024, 600, 720), (1024, 600));
        assert_eq!(agent_size(5120, 1440, 720), (2000, 563)); // 32:9: the 2000 px API cap wins
        assert_eq!(agent_size(2560, 1440, 0), (2000, 1125)); // "full" still respects the cap
        assert_eq!(agent_size(1920, 1080, 0), (1920, 1080));
    }

    #[test]
    fn downscale_keeps_colours() {
        let f = frame(2560, 1440, |x, _| if x < 1280 { [200, 0, 0] } else { [0, 0, 200] });
        let (w, h) = agent_size(f.width, f.height, 720);
        let small = downscale(&f, w, h).unwrap();
        assert_eq!((small.width, small.height, small.bgra.len()), (1280, 720, 1280 * 720 * 4));
        let px = |x: usize, y: usize| &small.bgra[(y * 1280 + x) * 4..][..4];
        assert_eq!(px(100, 300), [0, 0, 200, 255]); // BGRA: red
        assert_eq!(px(1200, 300), [200, 0, 0, 255]); // blue
    }

    #[test]
    fn black_detection() {
        assert!(frame(50, 50, |_, _| [0, 0, 0]).is_black());
        assert!(!frame(50, 50, |_, y| if y >= 25 { [3, 3, 3] } else { [0, 0, 0] }).is_black());
    }
}

