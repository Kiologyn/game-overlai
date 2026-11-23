//! Chat from a phone (or any device) on the same local network.
//!
//! Off by default. When the user turns it on (tray or the /phone page on this PC), a second
//! server listens on all interfaces on its own port. It serves the browser chat only (no MCP,
//! no overlay controls) and lets nothing through without the access token: the link in the QR
//! code carries it once (`/?t=<token>`), the server answers with a cookie and a clean URL.
//! Resetting the token logs every device out. The token and the on/off choice survive
//! restarts (`lan.json` next to the history).
//!
//! Plain HTTP: fine for a home network, not for a public one (the /phone page says so).

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tokio::sync::{oneshot, watch};
use tracing::{info, warn};

use crate::protocol::{LanAddress, LanStatus, ServerMsg};
use crate::state::{AppState, Target};

/// Cookie a signed-in device keeps (a year).
pub const COOKIE: &str = "overlai_lan";

#[derive(Debug, Default, Serialize, Deserialize)]
struct Saved {
    enabled: bool,
    token: String,
}

pub struct Lan {
    file: PathBuf,
    /// Configured port (`[lan] port`).
    port: u16,
    inner: Mutex<Inner>,
    clients: AtomicUsize,
    /// Bumped when access ends (turned off, token reset): connected devices are dropped.
    generation: watch::Sender<u64>,
}

struct Inner {
    enabled: bool,
    token: String,
    running: Option<Running>,
    error: Option<String>,
}

struct Running {
    port: u16,
    stop: oneshot::Sender<()>,
}

impl Lan {
    pub fn load(dir: &Path, port: u16) -> Self {
        let file = dir.join("lan.json");
        let saved: Saved = std::fs::read_to_string(&file)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let token = if saved.token.len() >= 32 { saved.token } else { new_token() };
        let lan = Self {
            file,
            port,
            inner: Mutex::new(Inner { enabled: saved.enabled, token, running: None, error: None }),
            clients: AtomicUsize::new(0),
            generation: watch::channel(0).0,
        };
        lan.save(&lan.inner.lock().unwrap());
        lan
    }

    fn save(&self, inner: &Inner) {
        let saved = Saved { enabled: inner.enabled, token: inner.token.clone() };
        let result = serde_json::to_string_pretty(&saved)
            .map_err(anyhow::Error::from)
            .and_then(|s| std::fs::write(&self.file, s).map_err(anyhow::Error::from));
        if let Err(e) = result {
            warn!("LAN: can't save {}: {e}", self.file.display());
        }
    }

    /// Whether `token` is the current access token (and access is on).
    pub fn token_ok(&self, token: &str) -> bool {
        let inner = self.inner.lock().unwrap();
        inner.running.is_some() && constant_time_eq(token.as_bytes(), inner.token.as_bytes())
    }

    /// Changes when connected devices must be dropped.
    pub fn generation(&self) -> watch::Receiver<u64> {
        self.generation.subscribe()
    }

    fn drop_devices(&self) {
        self.generation.send_modify(|g| *g += 1);
    }
}

/// Counts a connected device while the guard lives (shown on the /phone page).
pub fn client(state: &Arc<AppState>) -> ClientGuard {
    state.lan.clients.fetch_add(1, Ordering::SeqCst);
    broadcast(state);
    ClientGuard(state.clone())
}

pub struct ClientGuard(Arc<AppState>);

impl Drop for ClientGuard {
    fn drop(&mut self) {
        self.0.lan.clients.fetch_sub(1, Ordering::SeqCst);
        broadcast(&self.0);
    }
}

/// Status for this PC's UI (it contains the sign-in links).
pub fn status(state: &AppState) -> ServerMsg {
    let lan = &state.lan;
    let inner = lan.inner.lock().unwrap();
    let port = inner.running.as_ref().map(|r| r.port);
    let addresses = match port {
        Some(port) => addresses()
            .into_iter()
            .map(|(adapter, ip)| {
                let url = format!("http://{ip}:{port}/?t={}", inner.token);
                LanAddress { adapter, qr_svg: qr_svg(&url), url }
            })
            .collect(),
        None => vec![],
    };
    ServerMsg::LanStatus {
        status: LanStatus {
            enabled: inner.enabled,
            port,
            addresses,
            clients: lan.clients.load(Ordering::SeqCst) as u32,
            error: inner.error.clone(),
        },
    }
}

/// Tells this PC's UIs and the tray that something changed.
pub fn broadcast(state: &AppState) {
    state.hub.send(Target::Local, status(state));
    if let Some(app) = state.app.get() {
        crate::tray::refresh_lan(app);
    }
}

pub fn enabled(state: &AppState) -> bool {
    state.lan.inner.lock().unwrap().enabled
}

/// At startup: the user left access on last time.
pub fn start_if_enabled(state: &Arc<AppState>) {
    if enabled(state) {
        set_enabled(state, true);
    }
}

pub fn set_enabled(state: &Arc<AppState>, on: bool) {
    {
        let lan = &state.lan;
        let mut inner = lan.inner.lock().unwrap();
        if let Some(running) = inner.running.take() {
            let _ = running.stop.send(());
            lan.drop_devices();
            info!("LAN access off (port {})", running.port);
        }
        inner.enabled = on;
        inner.error = None;
        lan.save(&inner);
        if on {
            match start(state, lan.port) {
                Ok(running) => {
                    info!("LAN access on: port {}, {} address(es)", running.port, addresses().len());
                    inner.running = Some(running);
                }
                Err(e) => {
                    warn!("LAN access: can't start: {e}");
                    inner.error = Some(format!("Не удалось открыть порт: {e}"));
                }
            }
        }
    }
    broadcast(state);
}

/// A new token: every signed-in device has to scan the QR code again.
pub fn reset_token(state: &Arc<AppState>) {
    {
        let lan = &state.lan;
        let mut inner = lan.inner.lock().unwrap();
        inner.token = new_token();
        lan.save(&inner);
        lan.drop_devices();
    }
    info!("LAN access token reset; devices are signed out");
    broadcast(state);
}

fn start(state: &Arc<AppState>, preferred: u16) -> anyhow::Result<Running> {
    // OVERLAI_LAN_ADDR=127.0.0.1 keeps the "LAN" server on this PC (tests, no firewall prompt).
    let ip: IpAddr = std::env::var("OVERLAI_LAN_ADDR")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED));
    let listener = std::net::TcpListener::bind(SocketAddr::new(ip, preferred)).or_else(|e| {
        warn!("LAN port {preferred} is busy ({e}); using a free one");
        std::net::TcpListener::bind(SocketAddr::new(ip, 0))
    })?;
    listener.set_nonblocking(true)?;
    let port = listener.local_addr()?.port();
    let app = state.app.get().cloned().ok_or_else(|| anyhow::anyhow!("приложение ещё запускается"))?;
    let (stop, stopped) = oneshot::channel();
    tauri::async_runtime::spawn(crate::server::run_lan(listener, app, state.clone(), stopped));
    Ok(Running { port, stop })
}

/// This PC's IPv4 addresses a phone on the same network can reach, best first: the adapter of
/// the default route, then other private addresses of real adapters (not virtual ones: WSL,
/// Hyper-V, VirtualBox, VPN/proxy tunnels...).
pub fn addresses() -> Vec<(String, Ipv4Addr)> {
    let default_route = match local_ip_address::local_ip() {
        Ok(IpAddr::V4(ip)) => Some(ip),
        _ => None,
    };
    let all = local_ip_address::list_afinet_netifas().unwrap_or_else(|e| {
        warn!("LAN: can't list network adapters: {e}");
        vec![]
    });
    let mut found: Vec<(String, Ipv4Addr)> = all
        .into_iter()
        .filter_map(|(name, ip)| match ip {
            IpAddr::V4(v4) if v4.is_private() && !is_virtual(&name, v4) => Some((name, v4)),
            _ => None,
        })
        .collect();
    found.sort_by_key(|(name, ip)| (Some(*ip) != default_route, rank(ip), !name.to_lowercase().contains("wi-fi")));
    found.dedup_by_key(|(_, ip)| *ip);
    found
}

fn rank(ip: &Ipv4Addr) -> u8 {
    match ip.octets() {
        [192, 168, ..] => 0,
        [10, ..] => 1,
        _ => 2, // 172.16/12
    }
}

fn is_virtual(adapter: &str, ip: Ipv4Addr) -> bool {
    let a = adapter.to_lowercase();
    // VirtualBox host-only network (its adapter is often just "Ethernet 2").
    if ip.octets()[..3] == [192, 168, 56] {
        return true;
    }
    a.starts_with("wg")
        || [
        "vethernet", "virtualbox", "vmware", "hyper-v", "wsl", "loopback", "tap", "tun", "wintun", "wireguard",
        "v2ray", "clash", "sing-box", "singbox", "zerotier", "tailscale", "hamachi", "radmin", "docker", "bluetooth",
        "vpn", "openvpn",
    ]
    .iter()
    .any(|k| a.contains(k))
}

fn qr_svg(url: &str) -> String {
    use qrcode::render::svg;
    match qrcode::QrCode::new(url.as_bytes()) {
        // Dark on white with the quiet zone: every phone camera reads that.
        Ok(code) => code
            .render::<svg::Color>()
            .min_dimensions(240, 240)
            .dark_color(svg::Color("#000000"))
            .light_color(svg::Color("#ffffff"))
            .quiet_zone(true)
            .build(),
        Err(e) => {
            warn!("LAN: no QR code: {e}");
            String::new()
        }
    }
}

fn new_token() -> String {
    format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple())
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn virtual_adapters_are_skipped() {
        let home = Ipv4Addr::new(192, 168, 1, 24);
        assert!(is_virtual("vEthernet (WSL (Hyper-V firewall))", home));
        assert!(is_virtual("singbox_tun", home));
        assert!(is_virtual("wg0", Ipv4Addr::new(10, 200, 200, 3)));
        assert!(is_virtual("Ethernet 2", Ipv4Addr::new(192, 168, 56, 1)), "VirtualBox host-only");
        assert!(!is_virtual("Wi-Fi", home));
        assert!(!is_virtual("Ethernet 2", home));
        assert!(rank(&Ipv4Addr::new(192, 168, 1, 5)) < rank(&Ipv4Addr::new(10, 0, 0, 2)));
    }

    #[test]
    fn tokens() {
        let dir = std::env::temp_dir().join(format!("overlai-lan-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let lan = Lan::load(&dir, 1);
        let token = lan.inner.lock().unwrap().token.clone();
        assert_eq!(token.len(), 64);
        assert!(!lan.token_ok(&token), "no access while the server is off");
        // The token survives a restart.
        assert_eq!(Lan::load(&dir, 1).inner.lock().unwrap().token, token);
        assert!(constant_time_eq(b"abc", b"abc") && !constant_time_eq(b"abc", b"abd") && !constant_time_eq(b"a", b"ab"));
        assert!(qr_svg("http://192.168.1.5:47824/?t=x").contains("<svg"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
