use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tauri::plugin::TauriPlugin;
use tauri::{AppHandle, Manager, Wry};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
use tracing::{info, warn};

use crate::config::Hotkeys;
use crate::overlay;
use crate::protocol::PanelMode;
use crate::state::{AppState, HotkeySet};

/// The settings page is recording a new hotkey: ours are off meanwhile.
static CAPTURING: AtomicBool = AtomicBool::new(false);

/// The plugin dispatches every registered shortcut to this handler (runs on the main thread).
pub fn plugin() -> TauriPlugin<Wry> {
    tauri_plugin_global_shortcut::Builder::new()
        .with_handler(|app, shortcut, event| {
            if event.state != ShortcutState::Pressed {
                return;
            }
            let st = app.state::<Arc<AppState>>();
            let (quick, chat, focus) = {
                let keys = st.hotkeys.lock().unwrap();
                let is = |k: Option<tauri_plugin_global_shortcut::Shortcut>| k.is_some_and(|s| s.id() == shortcut.id());
                (is(keys.quick), is(keys.chat), is(keys.focus))
            };
            if quick {
                overlay::on_hotkey(app, PanelMode::Compact);
            } else if chat {
                overlay::on_hotkey(app, PanelMode::Expanded);
            } else if focus {
                overlay::toggle_focus(app);
            }
        })
        .build()
}

/// Registers the configured hotkeys. Returns the ones another program already holds.
pub fn register(app: &AppHandle) -> Vec<String> {
    let st = app.state::<Arc<AppState>>();
    let keys = {
        let set = st.hotkeys.lock().unwrap();
        [("quick", set.quick), ("chat", set.chat), ("focus", set.focus)]
    };
    let mut failed = Vec::new();
    for (name, shortcut) in keys {
        let Some(sc) = shortcut else { continue };
        match app.global_shortcut().register(sc) {
            Ok(()) => info!("hotkey {name}: {sc}"),
            Err(e) => {
                warn!("hotkey {name} ({sc}) could not be registered, probably used by another program: {e}");
                failed.push(sc.to_string());
            }
        }
    }
    failed
}

fn active(st: &AppState) -> bool {
    !st.hotkeys_paused.load(Ordering::SeqCst) && !CAPTURING.load(Ordering::SeqCst)
}

/// Registers the hotkeys again (new ones from the settings, or after a pause).
pub fn reload(app: &AppHandle) {
    let st = app.state::<Arc<AppState>>();
    if let Err(e) = app.global_shortcut().unregister_all() {
        warn!("could not unregister hotkeys: {e}");
    }
    *st.hotkeys.lock().unwrap() = HotkeySet::parse(&st.cfg().hotkeys);
    let failed = if active(&st) { register(app) } else { vec![] };
    *st.failed_hotkeys.lock().unwrap() = failed;
    crate::tray::refresh_hotkeys(app);
}

pub fn set_paused(app: &AppHandle, paused: bool) {
    let st = app.state::<Arc<AppState>>();
    st.hotkeys_paused.store(paused, Ordering::SeqCst);
    reload(app);
    info!("hotkeys {}", if paused { "paused" } else { "resumed" });
}

/// The settings page started/stopped recording a hotkey.
pub fn set_capturing(app: &AppHandle, capturing: bool) {
    if CAPTURING.swap(capturing, Ordering::SeqCst) != capturing {
        reload(app);
    }
}

/// Letters for Ctrl+Alt hotkeys under the left hand, nearest to its resting place first.
const LEFT_HAND: [char; 11] = ['q', 'w', 'a', 's', 'd', 'e', 'r', 'f', 'z', 'x', 'c'];

/// Windows treats Ctrl+Alt as AltGr: in layouts that type a character with Ctrl+Alt+<key>
/// (German Q = @, Polish A = ą, Czech W = |) a global hotkey would take that character away
/// from every program. Our default Ctrl+Alt hotkeys that do it in a keyboard layout installed
/// here (or clash with another hotkey) move to other letters under the left hand. Only the
/// values still at their default change: the user's own choice stays.
pub fn avoid_altgr(keys: &mut Hotkeys) {
    let layouts = installed_layouts();
    let defaults = Hotkeys::default();
    let fields = [
        (&mut keys.quick, defaults.quick.as_str()),
        (&mut keys.chat, defaults.chat.as_str()),
        (&mut keys.focus, defaults.focus.as_str()),
    ];
    for i in 0..fields.len() {
        if *fields[i].0 != fields[i].1 {
            continue;
        }
        let taken: Vec<String> = fields.iter().enumerate().filter(|(j, _)| *j != i).map(|(_, f)| f.0.clone()).collect();
        let free = |letter: char| {
            let combo = format!("ctrl+alt+{letter}");
            !taken.contains(&combo) && !layouts.iter().any(|&l| altgr_types(l, letter))
        };
        let own = fields[i].1.rsplit('+').next().and_then(|k| k.chars().next()).unwrap_or('q');
        if free(own) {
            continue;
        }
        match LEFT_HAND.into_iter().find(|&c| free(c)) {
            Some(c) => {
                let combo = format!("ctrl+alt+{c}");
                info!("hotkey {} would type a character in an installed keyboard layout: {combo} instead", fields[i].1);
                *fields[i].0 = combo;
            }
            None => warn!("no Ctrl+Alt letter is free in the installed keyboard layouts; keeping {}", fields[i].1),
        }
    }
}

fn installed_layouts() -> Vec<windows::Win32::UI::Input::KeyboardAndMouse::HKL> {
    use windows::Win32::UI::Input::KeyboardAndMouse::GetKeyboardLayoutList;
    let n = unsafe { GetKeyboardLayoutList(None) };
    if n <= 0 {
        return vec![];
    }
    let mut list = vec![Default::default(); n as usize];
    let got = unsafe { GetKeyboardLayoutList(Some(&mut list)) };
    list.truncate(got.max(0) as usize);
    list
}

/// Ctrl+Alt+<letter> types a printable character (or starts a dead key) in this layout.
fn altgr_types(layout: windows::Win32::UI::Input::KeyboardAndMouse::HKL, letter: char) -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        MAPVK_VK_TO_VSC, MapVirtualKeyExW, ToUnicodeEx, VK_CONTROL, VK_LCONTROL, VK_LMENU, VK_MENU,
    };
    let vk = letter.to_ascii_uppercase() as u32;
    let mut state = [0u8; 256];
    for k in [VK_CONTROL, VK_LCONTROL, VK_MENU, VK_LMENU] {
        state[k.0 as usize] = 0x80;
    }
    let mut buf = [0u16; 8];
    // Flag 4: don't change the keyboard state (no dead key left pending).
    let n = unsafe {
        let scan = MapVirtualKeyExW(vk, MAPVK_VK_TO_VSC, Some(layout));
        ToUnicodeEx(vk, scan, &state, &mut buf, 4, Some(layout))
    };
    match n {
        0 => false,
        n if n < 0 => true,
        n => buf[..n as usize].iter().any(|&c| c >= 0x20), // control codes are not typed text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_stay_distinct_and_own_choices_stay() {
        let mut keys = Hotkeys::default();
        avoid_altgr(&mut keys);
        let all = [&keys.quick, &keys.chat, &keys.focus];
        assert!(all.iter().all(|k| k.starts_with("ctrl+alt+")));
        assert!(all[0] != all[1] && all[1] != all[2] && all[0] != all[2]);
        // A user's combination equal to a default's letter pushes that default away.
        let mut keys = Hotkeys { quick: "ctrl+alt+a".into(), ..Hotkeys::default() };
        avoid_altgr(&mut keys);
        assert_eq!(keys.quick, "ctrl+alt+a");
        assert_ne!(keys.focus, "ctrl+alt+a");
    }

    /// The layouts installed here and what Ctrl+Alt types in them (cargo test -- --ignored --nocapture).
    #[test]
    #[ignore]
    fn print_altgr_letters() {
        for l in installed_layouts() {
            let typed: String = LEFT_HAND.into_iter().filter(|&c| altgr_types(l, c)).collect();
            println!("{:08x}: {typed:?}", l.0 as usize);
        }
        use windows::Win32::UI::Input::KeyboardAndMouse::{KLF_NOTELLSHELL, LoadKeyboardLayoutW, UnloadKeyboardLayout};
        let installed = installed_layouts();
        for id in ["00000407", "00000415", "00000405"] {
            let name: Vec<u16> = id.encode_utf16().chain([0]).collect();
            let Ok(l) = (unsafe { LoadKeyboardLayoutW(windows::core::PCWSTR(name.as_ptr()), KLF_NOTELLSHELL) }) else { continue };
            let typed: String = LEFT_HAND.into_iter().filter(|&c| altgr_types(l, c)).collect();
            println!("{id} (loaded for the test): {typed:?}");
            if !installed.contains(&l) {
                let _ = unsafe { UnloadKeyboardLayout(l) };
            }
        }
    }
}
