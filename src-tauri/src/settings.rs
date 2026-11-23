//! The /settings page (browser on this PC): edits config.toml. Hotkeys, the panel's default
//! size, the frame size for the agent and notifications apply at once; the agent, the proxy and
//! the ports after a restart (the page offers it). Also: starting with Windows.

use std::sync::{Arc, Mutex};

use tracing::{info, warn};

use crate::agent::spec;
use crate::config::{self, Config};
use crate::protocol::{AgentChoice, ServerMsg, Settings, SettingsValues};
use crate::state::{AppState, Target};

/// Why the last save was refused (shown on the page until the next save).
static LAST_ERROR: Mutex<Option<String>> = Mutex::new(None);

pub fn snapshot(state: &AppState) -> ServerMsg {
    let cfg = state.cfg();
    let (autostart, autostart_other) = autostart::status();
    ServerMsg::Settings {
        settings: Settings {
            values: values_of(&cfg),
            agents: spec::ALL
                .iter()
                .map(|s| AgentChoice { id: s.id.into(), name: s.name.into(), login_hint: s.login_hint.into() })
                .collect(),
            restart_needed: needs_restart(&state.started_with, &cfg),
            hotkeys_taken: state.failed_hotkeys.lock().unwrap().clone(),
            autostart,
            autostart_other,
            version: env!("CARGO_PKG_VERSION").into(),
            config_file: state.config_file.display().to_string(),
            data_dir: state.profiles_dir.parent().unwrap_or(&state.profiles_dir).display().to_string(),
            error: LAST_ERROR.lock().unwrap().clone(),
        },
    }
}

pub fn broadcast(state: &AppState) {
    state.hub.send(Target::Local, snapshot(state));
}

fn values_of(cfg: &Config) -> SettingsValues {
    SettingsValues {
        hotkey_quick: cfg.hotkeys.quick.clone(),
        hotkey_chat: cfg.hotkeys.chat.clone(),
        agent: spec::by_id(&cfg.agent).id.into(),
        proxy: cfg.network.proxy.clone(),
        require_proxy_for: cfg.network.require_proxy_for.clone(),
        panel_width: cfg.panel.width,
        panel_height: cfg.panel.expanded_height,
        panel_zoom: crate::overlay::snap_zoom(cfg.panel.zoom),
        agent_max_height: cfg.capture.agent_max_height,
        lan_port: cfg.lan.port,
        notifications: cfg.notifications.enabled,
    }
}

/// Checks the values and puts them into `cfg`. Errors are for the user (Russian).
fn apply(v: &SettingsValues, cfg: &mut Config) -> Result<(), String> {
    let quick = normalize_hotkey(&v.hotkey_quick, "быстрого вопроса")?;
    let chat = normalize_hotkey(&v.hotkey_chat, "чата")?;
    if quick == chat {
        return Err("У быстрого вопроса и чата должны быть разные сочетания клавиш.".into());
    }
    if !spec::ALL.iter().any(|s| s.id == v.agent) {
        return Err(format!("Неизвестный агент: {}", v.agent));
    }
    let proxy = v.proxy.trim();
    let proxy = match proxy.to_ascii_lowercase().as_str() {
        "" | "system" => "system".to_string(),
        "none" => "none".to_string(),
        _ if valid_proxy_url(proxy) => proxy.to_string(),
        _ => return Err("Прокси: укажите адрес вида http://127.0.0.1:10808.".into()),
    };
    if !(360.0..=2000.0).contains(&v.panel_width) || !(220.0..=1600.0).contains(&v.panel_height) {
        return Err("Размер панели: ширина от 360 до 2000, высота от 220 до 1600.".into());
    }
    if !(0.5..=2.0).contains(&v.panel_zoom) {
        return Err("Масштаб панели: от 50 до 200 %.".into());
    }
    if v.agent_max_height != 0 && !(360..=2160).contains(&v.agent_max_height) {
        return Err("Высота кадра для агента: от 360 до 2160 (или «полный размер»).".into());
    }
    if v.lan_port < 1024 || v.lan_port == cfg.port {
        return Err(format!("Порт для телефона: от 1024 до 65535, не {}.", cfg.port));
    }
    cfg.hotkeys.quick = quick;
    cfg.hotkeys.chat = chat;
    cfg.agent = v.agent.clone();
    cfg.network.proxy = proxy;
    cfg.network.require_proxy_for = spec::ALL
        .iter()
        .filter(|s| v.require_proxy_for.iter().any(|r| r.trim().eq_ignore_ascii_case(s.id)))
        .map(|s| s.id.to_string())
        .collect();
    cfg.panel.width = v.panel_width.round();
    cfg.panel.expanded_height = v.panel_height.round();
    cfg.panel.zoom = crate::overlay::snap_zoom(v.panel_zoom);
    cfg.capture.agent_max_height = v.agent_max_height;
    cfg.lan.port = v.lan_port;
    cfg.notifications.enabled = v.notifications;
    Ok(())
}

fn normalize_hotkey(s: &str, what: &str) -> Result<String, String> {
    let s = s.trim().to_ascii_lowercase().replace(' ', "");
    match s.parse::<tauri_plugin_global_shortcut::Shortcut>() {
        Ok(_) if !s.is_empty() => Ok(s),
        _ => Err(format!("Сочетание клавиш для {what} не распознано: «{s}».")),
    }
}

fn valid_proxy_url(s: &str) -> bool {
    // Agents take it from HTTPS_PROXY: http(s) proxies only.
    let rest = ["http://", "https://"]
        .iter()
        .find_map(|p| s.strip_prefix(p))
        .unwrap_or(s);
    if rest.contains("://") {
        return false; // another scheme
    }
    let host_port = rest.rsplit('@').next().unwrap_or(rest).trim_end_matches('/');
    match host_port.rsplit_once(':') {
        Some((host, port)) => !host.is_empty() && port.parse::<u16>().is_ok_and(|p| p > 0),
        None => false,
    }
}

/// What only a restart applies: the agent and its network, the ports.
fn needs_restart(started: &Config, now: &Config) -> bool {
    let mut a = started.network.require_proxy_for.clone();
    let mut b = now.network.require_proxy_for.clone();
    a.sort();
    b.sort();
    spec::by_id(&started.agent).id != spec::by_id(&now.agent).id
        || started.network.proxy.trim() != now.network.proxy.trim()
        || a != b
        || started.lan.port != now.lan.port
        || started.port != now.port
}

pub fn save(state: &Arc<AppState>, values: SettingsValues) {
    let before = state.cfg();
    let mut next = before.clone();
    let result = apply(&values, &mut next).and_then(|()| {
        config::save(&state.config_file, &next).map_err(|e| format!("Не удалось записать {}: {e}", state.config_file.display()))
    });
    match result {
        Ok(()) => {
            info!("settings saved");
            *LAST_ERROR.lock().unwrap() = None;
            *state.config.write().unwrap() = next.clone();
            if before.hotkeys != next.hotkeys
                && let Some(app) = state.app.get()
            {
                // Registered on the main thread; the page learns which ones are taken after.
                let st = state.clone();
                crate::overlay::run_on_main(app, move |app| {
                    crate::hotkeys::reload(app);
                    broadcast(&st);
                });
            }
        }
        Err(e) => {
            warn!("settings not saved: {e}");
            *LAST_ERROR.lock().unwrap() = Some(e);
        }
    }
    broadcast(state);
}

pub fn set_autostart(state: &AppState, on: bool) {
    match autostart::set(on) {
        Ok(()) => info!("start with Windows: {on}"),
        Err(e) => {
            warn!("start with Windows: {e}");
            *LAST_ERROR.lock().unwrap() = Some(format!("Автозапуск: {e}"));
        }
    }
    broadcast(state);
}

/// Restarts the app (agent, proxy, ports). The open browser pages reconnect by themselves.
pub fn restart(state: &AppState) {
    info!("restart requested from the settings");
    if let Some(app) = state.app.get() {
        app.request_restart();
    }
}

/// Starting with Windows: the per-user Run key (no admin rights, no scheduled task).
pub mod autostart {
    use windows::Win32::Foundation::ERROR_FILE_NOT_FOUND;
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ, RRF_RT_REG_SZ, RegCloseKey,
        RegCreateKeyExW, RegDeleteKeyValueW, RegGetValueW, RegSetValueExW,
    };
    use windows::core::{HSTRING, PCWSTR};

    pub const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const NAME: &str = "game-overlai";

    fn command() -> Result<String, String> {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        Ok(format!("\"{}\" --autostart", exe.display()))
    }

    /// (on for this exe, the command of another copy if the entry points elsewhere)
    pub fn status() -> (bool, Option<String>) {
        match get_string(RUN, NAME) {
            Some(cmd) if command().is_ok_and(|mine| mine.eq_ignore_ascii_case(&cmd)) => (true, None),
            Some(cmd) => (false, Some(cmd)),
            None => (false, None),
        }
    }

    pub fn set(on: bool) -> Result<(), String> {
        if on { set_string(RUN, NAME, &command()?) } else { delete_value(RUN, NAME) }
    }

    pub fn get_string(subkey: &str, name: &str) -> Option<String> {
        let (subkey, name) = (HSTRING::from(subkey), HSTRING::from(name));
        let mut buf = vec![0u16; 2048];
        let mut size = (buf.len() * 2) as u32;
        let rc = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                &subkey,
                &name,
                RRF_RT_REG_SZ,
                None,
                Some(buf.as_mut_ptr().cast()),
                Some(&mut size),
            )
        };
        if rc.is_err() {
            return None;
        }
        let len = (size as usize / 2).saturating_sub(1); // without the terminating NUL
        Some(String::from_utf16_lossy(&buf[..len.min(buf.len())]))
    }

    pub fn set_string(subkey: &str, name: &str, value: &str) -> Result<(), String> {
        let subkey = HSTRING::from(subkey);
        let mut key = HKEY::default();
        let rc = unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                &subkey,
                None,
                PCWSTR::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_SET_VALUE,
                None,
                &mut key,
                None,
            )
        };
        rc.ok().map_err(|e| e.to_string())?;
        let data: Vec<u16> = value.encode_utf16().chain(std::iter::once(0)).collect();
        let bytes = unsafe { std::slice::from_raw_parts(data.as_ptr().cast::<u8>(), data.len() * 2) };
        let rc = unsafe { RegSetValueExW(key, &HSTRING::from(name), None, REG_SZ, Some(bytes)) };
        unsafe {
            let _ = RegCloseKey(key);
        }
        rc.ok().map_err(|e| e.to_string())
    }

    pub fn delete_value(subkey: &str, name: &str) -> Result<(), String> {
        let rc = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, &HSTRING::from(subkey), &HSTRING::from(name)) };
        if rc.is_ok() || rc == ERROR_FILE_NOT_FOUND { Ok(()) } else { Err(rc.to_hresult().message()) }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn registry_roundtrip() {
            // A scratch key of our own, not the Run key.
            let key = r"Software\game-overlai-test";
            set_string(key, "v", "\"C:\\x y\\a.exe\" --autostart").unwrap();
            assert_eq!(get_string(key, "v").as_deref(), Some("\"C:\\x y\\a.exe\" --autostart"));
            delete_value(key, "v").unwrap();
            assert_eq!(get_string(key, "v"), None);
            delete_value(key, "v").unwrap(); // deleting twice is fine
            unsafe {
                let _ = windows::Win32::System::Registry::RegDeleteKeyW(HKEY_CURRENT_USER, &HSTRING::from(key));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values() -> SettingsValues {
        values_of(&Config::default())
    }

    #[test]
    fn valid_values_apply() {
        let mut cfg = Config::default();
        let v = SettingsValues {
            hotkey_quick: " Ctrl+Shift+Space ".into(),
            agent: "codex".into(),
            proxy: "127.0.0.1:10808".into(),
            require_proxy_for: vec!["Claude".into(), "gemini".into()],
            panel_width: 700.4,
            panel_zoom: 1.24,
            agent_max_height: 0,
            ..values()
        };
        apply(&v, &mut cfg).unwrap();
        assert_eq!(cfg.hotkeys.quick, "ctrl+shift+space");
        assert_eq!(cfg.agent, "codex");
        assert_eq!(cfg.network.proxy, "127.0.0.1:10808");
        assert_eq!(cfg.network.require_proxy_for, vec!["claude".to_string()], "known agents only");
        assert_eq!(cfg.panel.width, 700.0);
        assert_eq!(cfg.panel.zoom, 1.25, "snapped to a step");
        assert!(needs_restart(&Config::default(), &cfg));
        let mut only_keys = Config::default();
        apply(&SettingsValues { hotkey_chat: "alt+e".into(), ..values() }, &mut only_keys).unwrap();
        assert!(!needs_restart(&Config::default(), &only_keys), "hotkeys apply at once");
    }

    #[test]
    fn bad_values_are_refused() {
        let mut cfg = Config::default();
        let bad = [
            SettingsValues { hotkey_quick: "alt+w".into(), ..values() },
            SettingsValues { hotkey_quick: "alt+nonsense".into(), ..values() },
            SettingsValues { agent: "gpt".into(), ..values() },
            SettingsValues { proxy: "my proxy".into(), ..values() },
            SettingsValues { panel_width: 100.0, ..values() },
            SettingsValues { panel_zoom: 3.0, ..values() },
            SettingsValues { agent_max_height: 100, ..values() },
            SettingsValues { lan_port: 47823, ..values() },
        ];
        for v in bad {
            assert!(apply(&v, &mut cfg).is_err(), "{v:?}");
        }
        assert_eq!(cfg.hotkeys.quick, "alt+q", "nothing changes on error");
        assert!(valid_proxy_url("http://user:pw@proxy.lan:3128/") && valid_proxy_url("127.0.0.1:10808"));
        assert!(!valid_proxy_url("http://proxy") && !valid_proxy_url(":80") && !valid_proxy_url("socks5://127.0.0.1:1080"));
    }
}
