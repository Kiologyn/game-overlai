//! Frame -> JPEG for the agent, at full resolution (no downscaling).
//!
//! JPEG q90: PNG of a busy 1440p game frame is 5–10 MB, over Claude's 5 MB (base64) image
//! limit, and several times slower to encode. Chroma is subsampled (4:2:0): text shapes live in
//! the full-resolution luma, and 4:4:4 cost ~60% more time for no visible gain to the model.
//! `jpeg-encoder` (SIMD) takes BGRA as it comes from the GPU, so there is no conversion pass.
//! Measured: 2560×1440 with heavy grain ~210 ms (debug build, the encoder itself is optimised).

use jpeg_encoder::{ColorType, Encoder, SamplingFactor};

use super::Frame;

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
    let too_big = || format!("кадр {}×{} слишком большой", frame.width, frame.height);
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
    Err(format!("кадр {}×{} не помещается в лимит 5 МБ", frame.width, frame.height))
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
    fn black_detection() {
        assert!(frame(50, 50, |_, _| [0, 0, 0]).is_black());
        assert!(!frame(50, 50, |_, y| if y >= 25 { [3, 3, 3] } else { [0, 0, 0] }).is_black());
    }
}

