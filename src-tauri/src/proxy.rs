//! Which proxy the agent (and npm) should use.
//!
//! `[network] proxy` in config.toml:
//!   "system" (default) — the Windows system proxy (Settings > Network > Proxy, what v2rayN
//!                        & co. set), including PAC scripts; if Windows has none, HTTP(S)_PROXY
//!                        from our environment;
//!   "none"             — direct connection;
//!   "http://host:port" — this proxy.
//!
//! The result is passed to child processes as HTTPS_PROXY / HTTP_PROXY / NO_PROXY, which
//! Claude Code and npm understand. Local addresses always bypass the proxy (our own MCP server
//! will be on 127.0.0.1).

use tracing::warn;
use windows::Win32::Foundation::{GlobalFree, HGLOBAL};
use windows::Win32::Networking::WinHttp::{
    WINHTTP_ACCESS_TYPE_NAMED_PROXY, WINHTTP_ACCESS_TYPE_NO_PROXY, WINHTTP_AUTO_DETECT_TYPE_DHCP,
    WINHTTP_AUTO_DETECT_TYPE_DNS_A, WINHTTP_AUTOPROXY_AUTO_DETECT, WINHTTP_AUTOPROXY_CONFIG_URL,
    WINHTTP_AUTOPROXY_OPTIONS, WINHTTP_CURRENT_USER_IE_PROXY_CONFIG, WINHTTP_PROXY_INFO, WinHttpCloseHandle,
    WinHttpGetIEProxyConfigForCurrentUser, WinHttpGetProxyForUrl, WinHttpOpen,
};
use windows::core::{BOOL, HSTRING, PCWSTR, PWSTR};

/// Host the agent talks to; used to evaluate PAC scripts.
const PROBE_URL: &str = "https://api.anthropic.com/";
const ALWAYS_DIRECT: [&str; 3] = ["localhost", "127.0.0.1", "::1"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxyEnv {
    /// Proxy URL, `None` = direct.
    pub url: Option<String>,
    pub no_proxy: String,
    /// Where it came from, for the log.
    pub source: &'static str,
}

impl ProxyEnv {
    /// Environment variables for child processes. With no proxy they are set to empty values,
    /// so a proxy from our own environment is not inherited when the user chose "none".
    pub fn vars(&self) -> Vec<(&'static str, String)> {
        let url = self.url.clone().unwrap_or_default();
        vec![("HTTPS_PROXY", url.clone()), ("HTTP_PROXY", url), ("NO_PROXY", self.no_proxy.clone())]
    }

    /// For logs: no credentials.
    pub fn describe(&self) -> String {
        match &self.url {
            Some(url) => format!("{} ({})", redact(url), self.source),
            None => format!("direct ({})", self.source),
        }
    }
}

fn redact(url: &str) -> String {
    match (url.find("://"), url.rfind('@')) {
        (Some(scheme), Some(at)) if at > scheme => format!("{}***@{}", &url[..scheme + 3], &url[at + 1..]),
        _ => url.to_string(),
    }
}

pub fn resolve(setting: &str) -> ProxyEnv {
    let setting = setting.trim();
    match setting.to_ascii_lowercase().as_str() {
        "none" | "direct" | "off" => direct("config: none"),
        // Windows first: a stale HTTPS_PROXY in the environment must not hide the proxy the user
        // turned on in v2rayN & co.
        "" | "system" | "auto" => from_windows().or_else(from_environment).unwrap_or_else(|| direct("system: none")),
        _ => ProxyEnv { url: Some(normalize(setting)), no_proxy: no_proxy(&[]), source: "config" },
    }
}

fn direct(source: &'static str) -> ProxyEnv {
    ProxyEnv { url: None, no_proxy: no_proxy(&[]), source }
}

fn from_environment() -> Option<ProxyEnv> {
    let get = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
    let url = get("HTTPS_PROXY").or_else(|| get("HTTP_PROXY")).or_else(|| get("ALL_PROXY"))?;
    let extra: Vec<String> =
        get("NO_PROXY").map(|v| v.split(',').map(|s| s.trim().to_string()).collect()).unwrap_or_default();
    Some(ProxyEnv { url: Some(normalize(&url)), no_proxy: no_proxy(&extra), source: "environment" })
}

/// Windows "Internet options" proxy of the current user: manual proxy or PAC / auto-detect.
fn from_windows() -> Option<ProxyEnv> {
    let mut ie = WINHTTP_CURRENT_USER_IE_PROXY_CONFIG::default();
    if let Err(e) = unsafe { WinHttpGetIEProxyConfigForCurrentUser(&mut ie) } {
        warn!("cannot read the system proxy settings: {e}");
        return None;
    }
    let auto_config = take_string(ie.lpszAutoConfigUrl);
    let manual = take_string(ie.lpszProxy);
    let bypass = take_string(ie.lpszProxyBypass);
    let bypass_list: Vec<String> = bypass.as_deref().map(split_bypass).unwrap_or_default();

    if auto_config.is_some() || ie.fAutoDetect.as_bool() {
        match evaluate_pac(auto_config.as_deref()) {
            Some(Some(proxy)) => {
                return Some(ProxyEnv {
                    url: Some(proxy),
                    no_proxy: no_proxy(&bypass_list),
                    source: "system PAC",
                });
            }
            Some(None) => return Some(direct("system PAC: direct")),
            None => {} // PAC failed: fall back to the manual proxy below
        }
    }
    let url = pick_from_list(manual.as_deref()?)?;
    Some(ProxyEnv { url: Some(url), no_proxy: no_proxy(&bypass_list), source: "system" })
}

/// Runs the PAC script / WPAD for PROBE_URL. `Some(None)` = PAC says direct, `None` = failed.
fn evaluate_pac(auto_config_url: Option<&str>) -> Option<Option<String>> {
    unsafe {
        let session = WinHttpOpen(&HSTRING::from("game-overlai"), WINHTTP_ACCESS_TYPE_NO_PROXY, PCWSTR::null(), PCWSTR::null(), 0);
        if session.is_null() {
            return None;
        }
        let config_url = auto_config_url.map(HSTRING::from);
        let mut options = WINHTTP_AUTOPROXY_OPTIONS {
            fAutoLogonIfChallenged: BOOL(1),
            ..Default::default()
        };
        match &config_url {
            Some(u) => {
                options.dwFlags = WINHTTP_AUTOPROXY_CONFIG_URL;
                options.lpszAutoConfigUrl = PCWSTR(u.as_ptr());
            }
            None => {
                options.dwFlags = WINHTTP_AUTOPROXY_AUTO_DETECT;
                options.dwAutoDetectFlags = WINHTTP_AUTO_DETECT_TYPE_DHCP | WINHTTP_AUTO_DETECT_TYPE_DNS_A;
            }
        }
        let mut info = WINHTTP_PROXY_INFO::default();
        let result = WinHttpGetProxyForUrl(session, &HSTRING::from(PROBE_URL), &mut options, &mut info);
        let _ = WinHttpCloseHandle(session);
        if let Err(e) = result {
            warn!("system proxy script could not be evaluated: {e}");
            return None;
        }
        let proxy = take_string(info.lpszProxy);
        let _ = take_string(info.lpszProxyBypass);
        if info.dwAccessType == WINHTTP_ACCESS_TYPE_NAMED_PROXY {
            Some(proxy.as_deref().and_then(pick_from_list))
        } else {
            Some(None)
        }
    }
}

/// Copies a WinHTTP-allocated string and frees it.
fn take_string(p: PWSTR) -> Option<String> {
    if p.is_null() {
        return None;
    }
    let s = unsafe { p.to_string() }.ok();
    unsafe {
        let _ = GlobalFree(Some(HGLOBAL(p.0 as *mut _)));
    }
    s.filter(|s| !s.trim().is_empty())
}

/// "host:port" | "http=h:p;https=h:p;socks=h:p" | "PROXY h:p; DIRECT" -> URL for HTTPS traffic.
fn pick_from_list(list: &str) -> Option<String> {
    let entries: Vec<&str> = list.split([';', ' ']).map(str::trim).filter(|s| !s.is_empty()).collect();
    let by_scheme = |scheme: &str| {
        entries.iter().find_map(|e| e.split_once('=').filter(|(k, _)| k.eq_ignore_ascii_case(scheme)).map(|(_, v)| v))
    };
    if let Some(v) = by_scheme("https").or_else(|| by_scheme("http")) {
        return Some(normalize(v));
    }
    if let Some(v) = entries.iter().find(|e| !e.contains('=') && !e.eq_ignore_ascii_case("PROXY") && !e.eq_ignore_ascii_case("DIRECT")) {
        return Some(normalize(v));
    }
    if let Some(v) = by_scheme("socks") {
        warn!("only a SOCKS system proxy is set; Claude Code may not support SOCKS");
        return Some(format!("socks5://{v}"));
    }
    None
}

fn normalize(proxy: &str) -> String {
    let p = proxy.trim();
    if p.contains("://") { p.to_string() } else { format!("http://{p}") }
}

/// Windows bypass list ("<local>;localhost;127.*;*.corp") -> NO_PROXY entries.
fn split_bypass(list: &str) -> Vec<String> {
    list.split(';')
        .map(str::trim)
        .filter(|e| !e.is_empty() && *e != "<local>" && !e.ends_with(".*")) // IP wildcards: not expressible
        .map(|e| e.strip_prefix('*').unwrap_or(e).to_string()) // "*.corp" -> ".corp"
        .collect()
}

fn no_proxy(extra: &[String]) -> String {
    let mut all: Vec<String> = ALWAYS_DIRECT.iter().map(|s| s.to_string()).collect();
    for e in extra {
        if !e.is_empty() && !all.contains(e) {
            all.push(e.clone());
        }
    }
    all.join(",")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_lists() {
        assert_eq!(pick_from_list("127.0.0.1:10808").as_deref(), Some("http://127.0.0.1:10808"));
        assert_eq!(
            pick_from_list("http=127.0.0.1:1;https=127.0.0.1:2;socks=127.0.0.1:3").as_deref(),
            Some("http://127.0.0.1:2")
        );
        assert_eq!(pick_from_list("socks=127.0.0.1:3").as_deref(), Some("socks5://127.0.0.1:3"));
        assert_eq!(pick_from_list("PROXY 10.0.0.1:8080; DIRECT").as_deref(), Some("http://10.0.0.1:8080"));
        assert_eq!(normalize("http://u:p@h:1"), "http://u:p@h:1");
    }

    #[test]
    fn bypass_and_no_proxy() {
        let b = split_bypass("<local>;localhost;127.*;10.*;*.corp.local;intranet");
        assert_eq!(b, vec!["localhost", ".corp.local", "intranet"]);
        assert_eq!(no_proxy(&b), "localhost,127.0.0.1,::1,.corp.local,intranet");
    }

    #[test]
    fn settings() {
        assert_eq!(resolve("none").url, None);
        assert_eq!(resolve("127.0.0.1:9000").url.as_deref(), Some("http://127.0.0.1:9000"));
        assert_eq!(redact("http://user:pass@h:1"), "http://***@h:1");
    }
}
