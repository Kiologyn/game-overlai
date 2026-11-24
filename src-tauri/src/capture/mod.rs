//! Game frames. A worker thread owns the D3D device (made once at startup) and grabs frames
//! with Windows Graphics Capture on request; frames stay raw (BGRA) in memory until they are
//! sent to the agent, then `encode` turns them into JPEG at full resolution.

mod encode;
pub mod look;
mod wgc;

use std::sync::mpsc;
use std::time::{Duration, Instant};

use tracing::{debug, info, warn};

pub use encode::{agent_size, downscale, encode};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// The game window's own content.
    Window,
    /// Its whole monitor (fallback for fullscreen games that block window capture).
    Monitor,
}

pub struct Frame {
    pub width: u32,
    pub height: u32,
    /// Tightly packed BGRA rows.
    pub bgra: Vec<u8>,
    pub source: Source,
    pub taken: Instant,
}

impl Frame {
    /// Blocked captures come back as pure (0,0,0). Dark scenes are not pure black, so this
    /// doesn't fire on night levels or loading screens (unlike a "mostly dark" heuristic).
    fn is_black(&self) -> bool {
        self.bgra.as_chunks::<4>().0.iter().step_by(97).all(|[b, g, r, _]| *b == 0 && *g == 0 && *r == 0)
    }
}

impl std::fmt::Debug for Frame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Frame({}x{} {:?})", self.width, self.height, self.source)
    }
}

struct Job {
    hwnd: isize,
    reply: mpsc::Sender<Result<Frame, String>>,
}

pub struct Capturer {
    jobs: mpsc::Sender<Job>,
}

impl Capturer {
    pub fn start() -> Self {
        let (jobs, rx) = mpsc::channel::<Job>();
        let spawned = std::thread::Builder::new().name("capture".into()).spawn(move || worker(rx));
        if let Err(e) = spawned {
            warn!("capture thread did not start: {e}");
        }
        Self { jobs }
    }

    /// For the main thread (hotkey): waits at most `timeout`.
    pub fn grab_blocking(&self, hwnd: isize, timeout: Duration) -> Result<Frame, String> {
        let (reply, rx) = mpsc::channel();
        self.jobs.send(Job { hwnd, reply }).map_err(|_| tr!("err_capture_unavailable"))?;
        rx.recv_timeout(timeout).map_err(|_| tr!("err_capture_timeout"))?
    }

    pub async fn grab(&self, hwnd: isize) -> Result<Frame, String> {
        let (reply, rx) = mpsc::channel();
        self.jobs.send(Job { hwnd, reply }).map_err(|_| tr!("err_capture_unavailable"))?;
        tauri::async_runtime::spawn_blocking(move || rx.recv_timeout(Duration::from_secs(5)))
            .await
            .map_err(|_| tr!("err_capture_interrupted"))?
            .map_err(|_| tr!("err_capture_timeout"))?
    }
}

fn worker(jobs: mpsc::Receiver<Job>) {
    use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
    if let Err(e) = unsafe { RoInitialize(RO_INIT_MULTITHREADED) } {
        warn!("capture: RoInitialize: {e}");
    }
    let started = Instant::now();
    let mut d3d = match wgc::D3D::new() {
        Ok(d) => {
            info!("capture ready (D3D device in {} ms)", started.elapsed().as_millis());
            Some(d)
        }
        Err(e) => {
            warn!("capture: no D3D device yet: {e}");
            None
        }
    };
    let mut items = wgc::ItemCache::default();
    for job in jobs {
        if d3d.is_none() {
            d3d = wgc::D3D::new().map_err(|e| warn!("capture: D3D device: {e}")).ok();
        }
        let started = Instant::now();
        let result = match &d3d {
            Some(device) => wgc::grab(device, &mut items, crate::overlay::win32::hwnd(job.hwnd)).map_err(|f| {
                if f.device {
                    d3d = None; // e.g. GPU driver reset: make a new device next time
                }
                f.message
            }),
            None => Err(tr!("err_no_gpu")),
        };
        match &result {
            Ok(f) => debug!("captured {f:?} in {} ms", started.elapsed().as_millis()),
            Err(e) => debug!("capture failed in {} ms: {e}", started.elapsed().as_millis()),
        }
        let _ = job.reply.send(result);
    }
}
