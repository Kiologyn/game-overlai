//! New versions from GitHub Releases (the Tauri updater plugin).
//!
//! A release has `latest.json` next to its installer; the app reads the one of the latest
//! release, and if it names a newer version, offers it: in the chat header, on the settings
//! page and in the tray menu (never on its own: installing restarts the app). Checks run on the
//! user's click, and with `[updates] auto_check` (default on) shortly after the start and then
//! every few hours, release builds only. A copy without the installer (the portable exe, a dev
//! build) can't update itself: "Update" opens the release page to download it.

use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use tauri::{AppHandle, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};
use tracing::{info, warn};

use crate::protocol::{ServerMsg, UpdateState, UpdateStatus};
use crate::state::{AppState, Target};

pub const RELEASES_URL: &str = "https://github.com/Kiologyn/game-overlai/releases";
/// The first automatic check waits for the start to settle (the agent, the game).
const FIRST_CHECK: Duration = Duration::from_secs(20);
const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);
/// The check (a small JSON). reqwest counts the whole request, body included.
const TIMEOUT: Duration = Duration::from_secs(30);
/// The download of the installer (a few MB, maybe through a slow proxy).
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(10 * 60);

struct Inner {
    status: UpdateStatus,
    /// The newer version found (downloaded on "Update").
    found: Option<Update>,
}

static INNER: LazyLock<Mutex<Inner>> = LazyLock::new(|| {
    Mutex::new(Inner {
        status: UpdateStatus {
            state: UpdateState::Idle,
            current: env!("APP_VERSION").into(),
            version: None,
            notes_url: None,
            portable: is_portable(),
            dismissed: false,
        },
        found: None,
    })
});

/// For a UI that connects.
pub fn snapshot() -> ServerMsg {
    ServerMsg::Update { update: INNER.lock().unwrap().status.clone() }
}

/// Changes the status and tells this PC's UIs and the tray.
fn change(app: &AppHandle, f: impl FnOnce(&mut Inner)) {
    let status = {
        let mut inner = INNER.lock().unwrap();
        let before = inner.status.clone();
        f(&mut inner);
        if inner.status == before {
            return;
        }
        inner.status.clone()
    };
    app.state::<Arc<AppState>>().hub.send(Target::Local, ServerMsg::Update { update: status.clone() });
    crate::tray::refresh_update(app, &status);
}

/// The automatic checks (release builds; each one only while `[updates] auto_check` is on).
pub fn start(app: &AppHandle) {
    if cfg!(debug_assertions) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_CHECK).await;
        loop {
            if app.state::<Arc<AppState>>().cfg().updates.auto_check {
                check(&app, false).await;
            }
            tokio::time::sleep(CHECK_EVERY).await;
        }
    });
}

/// "Check for updates" on the settings page.
pub fn check_now(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move { check(&app, true).await });
}

/// An update is being downloaded or installed: nothing else may start meanwhile.
fn installing(state: &UpdateState) -> bool {
    matches!(state, UpdateState::Downloading { .. } | UpdateState::Installing)
}

async fn check(app: &AppHandle, by_user: bool) {
    // Checked and set under one lock: the automatic check and a click can't both start.
    let mut started = false;
    change(app, |i| {
        if !installing(&i.status.state) && i.status.state != UpdateState::Checking {
            i.status.state = UpdateState::Checking;
            started = true;
        }
    });
    if !started {
        return;
    }
    let found = find(app).await;
    // "Update" clicked meanwhile (on the version found before): the download goes on, this
    // result is dropped.
    if INNER.lock().unwrap().status.state != UpdateState::Checking {
        return;
    }
    match found {
        Ok(Some(update)) => {
            info!("update: {} is out (this is {})", update.version, update.current_version);
            let version = update.version.clone();
            change(app, |i| {
                // "Later" was for another version.
                if i.status.version.as_deref() != Some(version.as_str()) {
                    i.status.dismissed = false;
                }
                i.status.notes_url = Some(format!("{RELEASES_URL}/tag/v{version}"));
                i.status.version = Some(version);
                i.status.state = UpdateState::Available;
                i.found = Some(update);
            });
        }
        Ok(None) => {
            info!("update: {} is the latest version", env!("APP_VERSION"));
            change(app, |i| {
                i.status.state = UpdateState::UpToDate;
                i.status.version = None;
                i.status.notes_url = None;
                i.found = None;
            });
        }
        Err(e) => {
            warn!("update check failed: {e}");
            change(app, |i| {
                i.status.state = if i.found.is_some() && !by_user {
                    UpdateState::Available // a failed background check keeps what was found
                } else {
                    UpdateState::Error { message: tr!("update_check_failed", error = e) }
                };
            });
        }
    }
}

async fn find(app: &AppHandle) -> anyhow::Result<Option<Update>> {
    // The same network as the agent's: the system proxy (v2rayN & co.), none, or the user's.
    let setting = app.state::<Arc<AppState>>().cfg().network.proxy;
    let proxy = tokio::task::spawn_blocking(move || crate::proxy::resolve(&setting)).await?;
    let mut builder = app.updater_builder().timeout(TIMEOUT);
    // Tests: OVERLAI_UPDATE_URL=http://127.0.0.1:8000/latest.json instead of the GitHub release.
    if let Ok(url) = std::env::var("OVERLAI_UPDATE_URL") {
        builder = builder.endpoints(vec![url.parse()?])?;
    }
    builder = match proxy.url {
        Some(url) if url.starts_with("http://") || url.starts_with("https://") => builder.proxy(url.parse()?),
        // The updater's HTTP client has no SOCKS: it takes the Windows proxy settings itself.
        Some(_) => builder,
        None => builder.no_proxy(),
    };
    Ok(builder.build()?.check().await?)
}

/// "Update": downloads the installer, which closes the app, installs and starts it again.
/// A copy without the installer opens the release page instead.
pub fn install(app: &AppHandle) {
    // Checked and set under one lock: a double click downloads once.
    let mut job = None;
    let mut page = None;
    change(app, |i| {
        if installing(&i.status.state) {
            return;
        }
        if i.status.portable {
            page = Some(i.status.notes_url.clone().unwrap_or_else(|| format!("{RELEASES_URL}/latest")));
        } else if let Some(update) = i.found.clone() {
            i.status.state = UpdateState::Downloading { percent: None };
            job = Some(update);
        }
    });
    if let Some(page) = page {
        info!("update: portable copy, opening {page}");
        crate::tray::open_url(app, &page);
        return;
    }
    let Some(mut update) = job else { return };
    update.timeout = Some(DOWNLOAD_TIMEOUT);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut received: u64 = 0;
        let progress = app.clone();
        let bytes = update
            .download(
                move |chunk, total| {
                    received += chunk as u64;
                    let percent = total.filter(|t| *t > 0).map(|t| (received * 100 / t).min(100) as u8);
                    change(&progress, |i| i.status.state = UpdateState::Downloading { percent });
                },
                || {},
            )
            .await;
        let result = bytes.and_then(|bytes| {
            info!("update: {} downloaded, starting the installer", update.version);
            change(&app, |i| i.status.state = UpdateState::Installing);
            // The installer closes the app at once: an answer still streaming is saved first.
            crate::chat::flush(&app.state::<Arc<AppState>>());
            update.install(bytes) // returns only if the installer could not start
        });
        if let Err(e) = result {
            warn!("update failed: {e}");
            change(&app, |i| i.status.state = UpdateState::Error { message: tr!("update_failed", error = e) });
        }
    });
}

/// "Later" in the chat header: the button hides until the next start (or a newer version).
pub fn dismiss(app: &AppHandle) {
    change(app, |i| i.status.dismissed = true);
}

/// Installed copies have the uninstaller next to them; the portable exe and dev builds don't.
fn is_portable() -> bool {
    let exe = std::env::current_exe().ok();
    !exe.as_deref().and_then(|e| e.parent()).is_some_and(|dir| dir.join("uninstall.exe").exists())
}
