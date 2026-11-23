//! Phase 0 / spike 3: grab ONE frame of a game window with Windows Graphics Capture.
//!
//!   capture-probe list                      list capturable top-level windows
//!   capture-probe fg [--delay N]            capture the foreground window after N s (default 5)
//!   capture-probe title <substring>         capture first window whose title contains <substring>
//!   capture-probe exe <name.exe>            capture first window of that process
//!   ... add --monitor to capture the window's monitor instead of the window
//!
//! Writes PNG (and JPEG q90 if the PNG is over 5 MB) to ./out and prints timings,
//! sizes and a black-frame check. Build with --release for meaningful timings.

use std::ffi::c_void;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context as _, bail};
use windows::Graphics::Capture::{Direct3D11CaptureFramePool, GraphicsCaptureItem};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Win32::Foundation::{CloseHandle, HMODULE, HWND, LPARAM, RECT};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ,
    D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::Graphics::Gdi::{MONITOR_DEFAULTTONEAREST, MonitorFromWindow};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::System::WinRT::Direct3D11::{
    CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess,
};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GWL_EXSTYLE, GetForegroundWindow, GetWindowLongW, GetWindowRect,
    GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible, WS_EX_TOOLWINDOW,
};
use windows::core::{BOOL, Interface, PWSTR, factory};

// ---------------------------------------------------------------- window enumeration

#[derive(Debug, Clone)]
struct WinInfo {
    hwnd: HWND,
    title: String,
    exe: String,
    w: i32,
    h: i32,
}

fn window_info(hwnd: HWND) -> WinInfo {
    unsafe {
        let mut buf = [0u16; 512];
        let n = GetWindowTextW(hwnd, &mut buf);
        let title = String::from_utf16_lossy(&buf[..n.max(0) as usize]);
        let mut r = RECT::default();
        let _ = GetWindowRect(hwnd, &mut r);
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        let exe = process_exe(pid).unwrap_or_else(|| format!("pid:{pid}"));
        WinInfo { hwnd, title, exe, w: r.right - r.left, h: r.bottom - r.top }
    }
}

fn process_exe(pid: u32) -> Option<String> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len);
        let _ = CloseHandle(h);
        ok.ok()?;
        let full = String::from_utf16_lossy(&buf[..len as usize]);
        Some(full.rsplit('\\').next().unwrap_or(&full).to_string())
    }
}

fn is_candidate(hwnd: HWND) -> bool {
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() {
            return false;
        }
        if (GetWindowLongW(hwnd, GWL_EXSTYLE) as u32) & WS_EX_TOOLWINDOW.0 != 0 {
            return false;
        }
        let mut cloaked = 0u32;
        let _ = DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            &mut cloaked as *mut u32 as *mut c_void,
            std::mem::size_of::<u32>() as u32,
        );
        if cloaked != 0 {
            return false;
        }
    }
    let i = window_info(hwnd);
    !i.title.is_empty() && i.w >= 100 && i.h >= 100
}

fn list_windows() -> anyhow::Result<Vec<WinInfo>> {
    unsafe extern "system" fn cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let v = unsafe { &mut *(lparam.0 as *mut Vec<HWND>) };
        v.push(hwnd);
        BOOL(1)
    }
    let mut hwnds: Vec<HWND> = Vec::new();
    unsafe { EnumWindows(Some(cb), LPARAM(&mut hwnds as *mut _ as isize))? };
    Ok(hwnds.into_iter().filter(|h| is_candidate(*h)).map(window_info).collect())
}

// ---------------------------------------------------------------- capture

struct D3D {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    winrt_device: IDirect3DDevice,
}

fn create_d3d() -> anyhow::Result<D3D> {
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
        let device: ID3D11Device = device.context("no d3d device")?;
        let context = context.context("no d3d context")?;
        let dxgi: IDXGIDevice = device.cast()?;
        let winrt_device: IDirect3DDevice = CreateDirect3D11DeviceFromDXGIDevice(&dxgi)?.cast()?;
        Ok(D3D { device, context, winrt_device })
    }
}

struct Frame {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

struct Timings {
    item: Duration,
    first_frame: Duration,
    readback: Duration,
}

fn capture_item(item: &GraphicsCaptureItem, d3d: &D3D) -> anyhow::Result<(Frame, Timings)> {
    let t0 = Instant::now();
    let size = item.Size()?;
    let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
        &d3d.winrt_device,
        DirectXPixelFormat::B8G8R8A8UIntNormalized,
        1,
        size,
    )?;
    let session = pool.CreateCaptureSession(item)?;
    if let Err(e) = session.SetIsCursorCaptureEnabled(false) {
        eprintln!("  (cursor capture toggle unsupported: {e})");
    }
    if let Err(e) = session.SetIsBorderRequired(false) {
        eprintln!("  (borderless capture unsupported: {e})");
    }
    session.StartCapture()?;
    let started = Instant::now();

    let frame = loop {
        match pool.TryGetNextFrame() {
            Ok(f) => break f,
            Err(_) if started.elapsed() < Duration::from_secs(2) => std::thread::sleep(Duration::from_millis(2)),
            Err(e) => bail!("no frame within 2 s: {e}"),
        }
    };
    let first_frame = started.elapsed();

    let t_read = Instant::now();
    let content = frame.ContentSize()?;
    let texture: ID3D11Texture2D = unsafe { frame.Surface()?.cast::<IDirect3DDxgiInterfaceAccess>()?.GetInterface()? };
    let mut desc = D3D11_TEXTURE2D_DESC::default();
    unsafe { texture.GetDesc(&mut desc) };
    let (w, h) = (
        (content.Width as u32).min(desc.Width),
        (content.Height as u32).min(desc.Height),
    );
    desc.Usage = D3D11_USAGE_STAGING;
    desc.BindFlags = 0;
    desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
    desc.MiscFlags = 0;

    let rgba = unsafe {
        let mut staging = None;
        d3d.device.CreateTexture2D(&desc, None, Some(&mut staging))?;
        let staging = staging.context("no staging texture")?;
        d3d.context.CopyResource(&staging, &texture);
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        d3d.context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
        let mut out = vec![0u8; (w * h * 4) as usize];
        let src = mapped.pData as *const u8;
        for y in 0..h as usize {
            let row = std::slice::from_raw_parts(src.add(y * mapped.RowPitch as usize), w as usize * 4);
            let dst = &mut out[y * w as usize * 4..(y + 1) * w as usize * 4];
            for (d, s) in dst.chunks_exact_mut(4).zip(row.chunks_exact(4)) {
                d[0] = s[2]; // BGRA -> RGBA
                d[1] = s[1];
                d[2] = s[0];
                d[3] = 255;
            }
        }
        d3d.context.Unmap(&staging, 0);
        out
    };
    let readback = t_read.elapsed();

    let _ = session.Close();
    let _ = pool.Close();
    let _ = frame.Close();

    Ok((
        Frame { width: w, height: h, rgba },
        Timings { item: started.duration_since(t0), first_frame, readback },
    ))
}

/// Fraction of (sampled) pixels that are near-black. ~1.0 means the capture is blocked/black.
fn black_fraction(f: &Frame) -> f64 {
    let px = f.rgba.chunks_exact(4).step_by(37);
    let (mut total, mut black) = (0u64, 0u64);
    for p in px {
        total += 1;
        if p[0].max(p[1]).max(p[2]) < 8 {
            black += 1;
        }
    }
    black as f64 / total.max(1) as f64
}

// ---------------------------------------------------------------- main

fn main() -> anyhow::Result<()> {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        RoInitialize(RO_INIT_MULTITHREADED)?;
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    let monitor = args.iter().any(|a| a == "--monitor");
    let delay = args
        .iter()
        .position(|a| a == "--delay")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(5);

    let target: WinInfo = match args.first().map(String::as_str) {
        Some("list") | None => {
            for w in list_windows()? {
                println!("{:>10?}  {:>5}x{:<5} {:<28} {}", w.hwnd.0, w.w, w.h, w.exe, w.title);
            }
            return Ok(());
        }
        Some("fg") => {
            for s in (1..=delay).rev() {
                eprint!("\rswitch to the game window... {s} ");
                std::thread::sleep(Duration::from_secs(1));
            }
            eprintln!();
            window_info(unsafe { GetForegroundWindow() })
        }
        Some("title") => {
            let q = args.get(1).context("title <substring>")?.to_lowercase();
            list_windows()?.into_iter().find(|w| w.title.to_lowercase().contains(&q)).context("no such window")?
        }
        Some("exe") => {
            let q = args.get(1).context("exe <name.exe>")?.to_lowercase();
            list_windows()?.into_iter().find(|w| w.exe.to_lowercase() == q).context("no such window")?
        }
        Some(other) => bail!("unknown command {other}"),
    };
    println!("target: {} | {} | {}x{} (window rect) | monitor mode: {monitor}", target.exe, target.title, target.w, target.h);

    let t = Instant::now();
    let d3d = create_d3d()?;
    let t_dev = t.elapsed();

    let interop = factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
    let item: GraphicsCaptureItem = unsafe {
        if monitor {
            interop.CreateForMonitor(MonitorFromWindow(target.hwnd, MONITOR_DEFAULTTONEAREST))?
        } else {
            interop.CreateForWindow(target.hwnd)?
        }
    };
    let (frame, tm) = capture_item(&item, &d3d)?;
    let black = black_fraction(&frame);

    let out_dir = PathBuf::from("out");
    std::fs::create_dir_all(&out_dir)?;
    let stem = format!(
        "{}-{}{}",
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs(),
        target.exe.trim_end_matches(".exe"),
        if monitor { "-monitor" } else { "" }
    );
    let img = image::RgbaImage::from_raw(frame.width, frame.height, frame.rgba).context("bad buffer")?;

    let t_png = Instant::now();
    let png_path = out_dir.join(format!("{stem}.png"));
    img.save(&png_path)?;
    let png_time = t_png.elapsed();
    let png_size = std::fs::metadata(&png_path)?.len();

    println!("frame:        {}x{}", frame.width, frame.height);
    println!("d3d device:   {:?} (one-time in the real app)", t_dev);
    println!("session init: {:?}", tm.item);
    println!("first frame:  {:?}", tm.first_frame);
    println!("readback:     {:?}", tm.readback);
    println!("png encode:   {:?} -> {:.2} MB  {}", png_time, png_size as f64 / 1e6, png_path.display());
    if png_size > 5_000_000 {
        let t_jpg = Instant::now();
        let jpg_path = out_dir.join(format!("{stem}.jpg"));
        let rgb = image::DynamicImage::ImageRgba8(img).to_rgb8();
        let mut f = std::io::BufWriter::new(std::fs::File::create(&jpg_path)?);
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut f, 90).encode_image(&rgb)?;
        drop(f);
        println!(
            "jpeg q90:     {:?} -> {:.2} MB  {} (PNG is over Claude's 5 MB image limit)",
            t_jpg.elapsed(),
            std::fs::metadata(&jpg_path)?.len() as f64 / 1e6,
            jpg_path.display()
        );
    }
    println!(
        "black pixels: {:.1}%{}",
        black * 100.0,
        if black > 0.98 { "  <-- looks BLACK: capture blocked? try --monitor" } else { "" }
    );
    Ok(())
}
