//! Looking closer and waiting: a region of a frame at full resolution (`zoom_screen`) and a
//! cheap fingerprint of a frame to notice when the screen changes or settles (`watch_screen`).

use super::Frame;

/// A pixel rectangle of a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Region {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Smallest region side worth sending (px of the full frame).
const MIN_SIDE: u32 = 16;

impl Region {
    /// A region given in the coordinates of a `seen_w`×`seen_h` copy of a `full_w`×`full_h` frame,
    /// in full-frame pixels, clipped to the frame. None: nothing of it is inside.
    pub fn from_seen(x: f64, y: f64, w: f64, h: f64, seen: (u32, u32), full: (u32, u32)) -> Option<Region> {
        let (sx, sy) = (full.0 as f64 / seen.0.max(1) as f64, full.1 as f64 / seen.1.max(1) as f64);
        let left = (x * sx).floor().clamp(0.0, full.0 as f64);
        let top = (y * sy).floor().clamp(0.0, full.1 as f64);
        let right = ((x + w) * sx).ceil().clamp(0.0, full.0 as f64);
        let bottom = ((y + h) * sy).ceil().clamp(0.0, full.1 as f64);
        if !(right > left && bottom > top) || [x, y, w, h].iter().any(|v| !v.is_finite()) {
            return None;
        }
        let mut r = Region { x: left as u32, y: top as u32, width: (right - left) as u32, height: (bottom - top) as u32 };
        // Too small to see anything: grow it around its centre, inside the frame.
        for (pos, len, max) in [(&mut r.x, &mut r.width, full.0), (&mut r.y, &mut r.height, full.1)] {
            if *len < MIN_SIDE && max >= MIN_SIDE {
                let centre = *pos + *len / 2;
                *pos = centre.saturating_sub(MIN_SIDE / 2).min(max - MIN_SIDE);
                *len = MIN_SIDE;
            }
        }
        Some(r)
    }
}

/// The region's pixels as a frame of its own.
pub fn crop(frame: &Frame, r: Region) -> Frame {
    let row = (frame.width * 4) as usize;
    let mut bgra = Vec::with_capacity((r.width * r.height * 4) as usize);
    for y in r.y..r.y + r.height {
        let start = y as usize * row + r.x as usize * 4;
        bgra.extend_from_slice(&frame.bgra[start..start + r.width as usize * 4]);
    }
    Frame { width: r.width, height: r.height, bgra, source: frame.source, taken: frame.taken }
}

/// Size to show a zoomed region at: small regions are enlarged (up to 4×, to ~800 px on the
/// longest side) so the model sees small text clearly; big ones are capped at 1600 px.
pub fn zoom_size(width: u32, height: u32) -> (u32, u32) {
    let longest = width.max(height) as f64;
    let scale = if longest < 800.0 { (800.0 / longest).min(4.0) } else { (1600.0 / longest).min(1.0) };
    (((width as f64 * scale).round() as u32).max(1), ((height as f64 * scale).round() as u32).max(1))
}

/// Brightness of a coarse grid over the region (each cell an average of a few pixels).
#[derive(Debug, Clone, PartialEq)]
pub struct Fingerprint(Vec<f32>);

const GRID_W: u32 = 48;
const GRID_H: u32 = 27;
const SAMPLES: u32 = 3;

pub fn fingerprint(frame: &Frame, r: Region) -> Fingerprint {
    let (gw, gh) = (GRID_W.min(r.width), GRID_H.min(r.height));
    let mut cells = Vec::with_capacity((gw * gh) as usize);
    for gy in 0..gh {
        for gx in 0..gw {
            let mut sum = 0.0f32;
            for sy in 0..SAMPLES {
                for sx in 0..SAMPLES {
                    let px = r.x + ((gx * SAMPLES + sx) * r.width) / (gw * SAMPLES);
                    let py = r.y + ((gy * SAMPLES + sy) * r.height) / (gh * SAMPLES);
                    let i = ((py * frame.width + px) * 4) as usize;
                    let [b, g, red] = [frame.bgra[i], frame.bgra[i + 1], frame.bgra[i + 2]].map(f32::from);
                    sum += 0.114 * b + 0.587 * g + 0.299 * red;
                }
            }
            cells.push(sum / (SAMPLES * SAMPLES) as f32);
        }
    }
    Fingerprint(cells)
}

impl Fingerprint {
    /// How different two fingerprints are: 0 = the same, 1 = black vs white everywhere.
    pub fn difference(&self, other: &Fingerprint) -> f32 {
        if self.0.len() != other.0.len() || self.0.is_empty() {
            return 1.0;
        }
        self.0.iter().zip(&other.0).map(|(a, b)| (a - b).abs()).sum::<f32>() / (255.0 * self.0.len() as f32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::Source;
    use std::time::Instant;

    fn frame(w: u32, h: u32, f: impl Fn(u32, u32) -> u8) -> Frame {
        let mut bgra = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let v = f(x, y);
                bgra.extend_from_slice(&[v, v, v, 255]);
            }
        }
        Frame { width: w, height: h, bgra, source: Source::Window, taken: Instant::now() }
    }

    #[test]
    fn regions_map_from_what_the_agent_saw() {
        // The agent saw 1280×720 of a 2560×1440 frame.
        let r = Region::from_seen(100.0, 50.0, 200.0, 100.0, (1280, 720), (2560, 1440)).unwrap();
        assert_eq!(r, Region { x: 200, y: 100, width: 400, height: 200 });
        // Past the edge: clipped.
        let r = Region::from_seen(1200.0, 700.0, 500.0, 500.0, (1280, 720), (2560, 1440)).unwrap();
        assert_eq!(r, Region { x: 2400, y: 1400, width: 160, height: 40 });
        assert_eq!(Region::from_seen(2000.0, 0.0, 10.0, 10.0, (1280, 720), (2560, 1440)), None);
        // A dot grows to the minimum around it.
        let r = Region::from_seen(0.0, 0.0, 1.0, 1.0, (100, 100), (100, 100)).unwrap();
        assert_eq!((r.x, r.y, r.width, r.height), (0, 0, MIN_SIDE, MIN_SIDE));
    }

    #[test]
    fn crop_takes_the_right_pixels() {
        let f = frame(10, 10, |x, y| (y * 10 + x) as u8);
        let c = crop(&f, Region { x: 2, y: 3, width: 3, height: 2 });
        assert_eq!((c.width, c.height), (3, 2));
        let firsts: Vec<u8> = c.bgra.chunks(4).map(|p| p[0]).collect();
        assert_eq!(firsts, [32, 33, 34, 42, 43, 44]);
    }

    #[test]
    fn zoom_sizes() {
        assert_eq!(zoom_size(200, 100), (800, 400));
        assert_eq!(zoom_size(100, 50), (400, 200), "at most 4×");
        assert_eq!(zoom_size(1000, 500), (1000, 500));
        assert_eq!(zoom_size(3200, 1000), (1600, 500));
    }

    #[test]
    fn fingerprints_notice_changes_not_noise() {
        let all = Region { x: 0, y: 0, width: 320, height: 180 };
        let dark = frame(320, 180, |_, _| 20);
        let noisy = frame(320, 180, |x, y| 20 + ((x * 7 + y * 13) % 5) as u8);
        let bright = frame(320, 180, |x, _| if x < 160 { 20 } else { 230 });
        let a = fingerprint(&dark, all);
        assert!(a.difference(&fingerprint(&noisy, all)) < 0.02);
        assert!(a.difference(&fingerprint(&bright, all)) > 0.3);
        // Only the watched region counts.
        let left = Region { x: 0, y: 0, width: 150, height: 180 };
        assert!(fingerprint(&dark, left).difference(&fingerprint(&bright, left)) < 0.001);
    }
}
