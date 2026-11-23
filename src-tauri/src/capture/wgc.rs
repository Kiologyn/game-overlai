//! One frame of a window (or monitor) with Windows Graphics Capture.
//! Ported from spikes/capture-probe: ~10–18 ms to start a session, ~5–7 ms to the first
//! frame, ~6 ms readback for 2560×1440. No yellow border; must run on an MTA thread.

use std::time::{Duration, Instant};

use windows::Foundation::TypedEventHandler;
use windows::Graphics::Capture::{Direct3D11CaptureFramePool, GraphicsCaptureItem};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Win32::Foundation::{HMODULE, HWND, RECT};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE,
    D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING, D3D11CreateDevice, ID3D11Device,
    ID3D11DeviceContext, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow};
use windows::Win32::System::WinRT::Direct3D11::{CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::UI::WindowsAndMessaging::{GetWindowRect, IsIconic, IsWindow};
use windows::core::{IInspectable, Interface, factory};

use tracing::trace;

use super::{Frame, Source};

/// Created once (~140 ms); recreated after a GPU reset.
pub struct D3D {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    winrt_device: IDirect3DDevice,
}

impl D3D {
    pub fn new() -> windows::core::Result<Self> {
        unsafe {
            let mut device = None;
            let mut context = None;
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )?;
            let device: ID3D11Device = device.ok_or_else(windows::core::Error::empty)?;
            let context = context.ok_or_else(windows::core::Error::empty)?;
            let dxgi: IDXGIDevice = device.cast()?;
            let winrt_device: IDirect3DDevice = CreateDirect3D11DeviceFromDXGIDevice(&dxgi)?.cast()?;
            Ok(Self { device, context, winrt_device })
        }
    }
}

/// Why there is no frame. `device` = the D3D device is probably broken: make a new one.
#[derive(Debug)]
pub struct Failure {
    pub message: String,
    pub device: bool,
}

impl Failure {
    fn user(message: &str) -> Self {
        Self { message: message.into(), device: false }
    }
}

impl From<windows::core::Error> for Failure {
    fn from(e: windows::core::Error) -> Self {
        Self { message: format!("ошибка захвата: {}", e.message()), device: true }
    }
}

/// Capture item of the last captured window, reused: creating one costs 50–120 ms, while a
/// whole capture with an existing one takes ~30 ms (this is on the hotkey's path).
#[derive(Default)]
pub struct ItemCache(Option<(isize, GraphicsCaptureItem)>);

/// The window's own content; if that fails or comes out black and the game covers its
/// monitor (fullscreen), the monitor instead — the overlay panel is excluded from capture.
pub fn grab(d3d: &D3D, items: &mut ItemCache, hwnd: HWND) -> Result<Frame, Failure> {
    unsafe {
        if hwnd.is_invalid() || !IsWindow(Some(hwnd)).as_bool() {
            return Err(Failure::user("окно игры закрыто"));
        }
        if IsIconic(hwnd).as_bool() {
            return Err(Failure::user("окно игры свёрнуто"));
        }
    }
    let interop = factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
    let key = hwnd.0 as isize;
    let mut window_capture = |fresh: bool| -> Result<Frame, Failure> {
        if fresh || !matches!(&items.0, Some((k, _)) if *k == key) {
            items.0 = None;
            let t0 = Instant::now();
            let item = unsafe { interop.CreateForWindow::<GraphicsCaptureItem>(hwnd)? };
            trace!("wgc: capture item for the window in {} ms", t0.elapsed().as_millis());
            items.0 = Some((key, item));
        }
        let (_, item) = items.0.as_ref().expect("just set");
        capture(d3d, item, Source::Window)
    };
    // A cached item can go stale (window recreated, display mode change): retry once with a new one.
    let window = match window_capture(false) {
        Ok(frame) => Ok(frame),
        Err(_) => window_capture(true),
    };
    let failure = match window {
        Ok(frame) if !frame.is_black() => return Ok(frame),
        Ok(_) => Failure::user("кадр получился чёрным: игра не даёт себя захватить"),
        Err(e) => e,
    };
    if !covers_monitor(hwnd) {
        return Err(failure);
    }
    let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
    let item: GraphicsCaptureItem = unsafe { interop.CreateForMonitor(monitor)? };
    match capture(d3d, &item, Source::Monitor)? {
        frame if frame.is_black() => Err(Failure::user("кадр получился чёрным: игра не даёт себя захватить")),
        frame => Ok(frame),
    }
}

fn covers_monitor(hwnd: HWND) -> bool {
    unsafe {
        let mut r = RECT::default();
        let _ = GetWindowRect(hwnd, &mut r);
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if !GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST), &mut mi).as_bool() {
            return false;
        }
        let m = mi.rcMonitor;
        r.left <= m.left && r.top <= m.top && r.right >= m.right && r.bottom >= m.bottom
    }
}

fn capture(d3d: &D3D, item: &GraphicsCaptureItem, source: Source) -> Result<Frame, Failure> {
    let t_setup = Instant::now();
    let size = item.Size()?;
    if size.Width <= 0 || size.Height <= 0 {
        return Err(Failure::user("окно игры не имеет размера"));
    }
    let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
        &d3d.winrt_device,
        DirectXPixelFormat::B8G8R8A8UIntNormalized,
        1,
        size,
    )?;
    // Wait for the frame on the pool's event: polling with sleep() would round every wait up to
    // the 15.6 ms system timer tick.
    let (arrived, wait) = std::sync::mpsc::channel::<()>();
    let token = pool.FrameArrived(&TypedEventHandler::<Direct3D11CaptureFramePool, IInspectable>::new(
        move |_, _| {
            let _ = arrived.send(());
            Ok(())
        },
    ))?;
    let session = pool.CreateCaptureSession(item)?;
    // The cursor shows what "this" means in "what is this?". No border around the window.
    let _ = session.SetIsCursorCaptureEnabled(true);
    let _ = session.SetIsBorderRequired(false);
    trace!("wgc setup {} ms", t_setup.elapsed().as_millis());
    let t0 = Instant::now();
    session.StartCapture()?;
    let t_start = t0.elapsed();

    let deadline = t0 + Duration::from_millis(1000);
    let frame = loop {
        if let Ok(f) = pool.TryGetNextFrame() {
            break Some(f);
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() || wait.recv_timeout(left).is_err() {
            break pool.TryGetNextFrame().ok();
        }
    };
    let t_frame = t0.elapsed();
    let result = match &frame {
        Some(f) => read_back(d3d, f, source),
        None => Err(Failure::user("игра не отдала кадр за секунду")),
    };
    trace!(
        "wgc {source:?}: start {} ms, first frame {} ms, readback {} ms",
        t_start.as_millis(),
        (t_frame - t_start).as_millis(),
        (t0.elapsed() - t_frame).as_millis()
    );
    if let Some(f) = frame {
        let _ = f.Close();
    }
    let _ = pool.RemoveFrameArrived(token);
    let _ = session.Close();
    let _ = pool.Close();
    result
}

fn read_back(
    d3d: &D3D,
    frame: &windows::Graphics::Capture::Direct3D11CaptureFrame,
    source: Source,
) -> Result<Frame, Failure> {
    let content = frame.ContentSize()?;
    let texture: ID3D11Texture2D = unsafe { frame.Surface()?.cast::<IDirect3DDxgiInterfaceAccess>()?.GetInterface()? };
    let mut desc = D3D11_TEXTURE2D_DESC::default();
    unsafe { texture.GetDesc(&mut desc) };
    let width = (content.Width.max(1) as u32).min(desc.Width);
    let height = (content.Height.max(1) as u32).min(desc.Height);
    desc.Usage = D3D11_USAGE_STAGING;
    desc.BindFlags = 0;
    desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
    desc.MiscFlags = 0;

    let row = width as usize * 4;
    let mut bgra = vec![0u8; row * height as usize];
    unsafe {
        let mut staging = None;
        d3d.device.CreateTexture2D(&desc, None, Some(&mut staging))?;
        let staging = staging.ok_or_else(windows::core::Error::empty)?;
        d3d.context.CopyResource(&staging, &texture);
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        d3d.context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
        let src = mapped.pData as *const u8;
        for (y, dst) in bgra.chunks_exact_mut(row).enumerate() {
            let line = std::slice::from_raw_parts(src.add(y * mapped.RowPitch as usize), row);
            dst.copy_from_slice(line);
        }
        d3d.context.Unmap(&staging, 0);
    }
    Ok(Frame { width, height, bgra, source, taken: Instant::now() })
}
