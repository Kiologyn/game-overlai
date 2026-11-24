//! Game frames for the agent: it takes them itself through the MCP tool (`screenshot_game`)
//! when a question needs the screen. Encoded images are kept with their chat in the
//! history (`shots\<chat>\<shot>.jpg`) and served to the UI at `/api/shots/{id}`.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine as _;

use crate::capture::{self, Frame};
use crate::protocol::Shot;
use crate::state::AppState;

/// The frame grabbed when the hotkey opened the overlay is given out as the first one only
/// this soon after; later the screen has likely moved on.
const HOTKEY_FRAME_FOR: Duration = Duration::from_secs(120);

pub struct AgentFrame {
    pub frame: Arc<Frame>,
    pub when: Taken,
}

pub enum Taken {
    Now,
    /// The frame grabbed when the hotkey opened the overlay: the moment the user asks about.
    AtHotkey { age: Duration },
    /// The game can't be captured now; the hotkey's frame instead: why, and its age.
    Fallback { reason: String, age: Duration },
}

/// Frame for the agent's tool. The first one after the hotkey opened the overlay is the frame
/// grabbed at that moment, before the panel took focus (what the user asks about; some games
/// also pause or hide tooltips once unfocused); later ones are fresh. If the game can't be
/// captured now (e.g. a fullscreen game minimised itself), the hotkey's frame again.
pub async fn for_agent(state: &AppState) -> Result<AgentFrame, String> {
    let (hwnd, first, prefetched) = {
        let mut ov = state.overlay.lock().unwrap();
        let first = ov.prefetch.as_mut().filter(|p| !p.used && p.frame.taken.elapsed() < HOTKEY_FRAME_FOR).map(|p| {
            p.used = true;
            p.frame.clone()
        });
        (ov.game_hwnd, first, ov.prefetch.as_ref().map(|p| p.frame.clone()))
    };
    if let Some(frame) = first {
        let age = frame.taken.elapsed();
        return Ok(AgentFrame { frame, when: Taken::AtHotkey { age } });
    }
    match grab_game(state, hwnd).await {
        Ok(frame) => Ok(AgentFrame { frame: Arc::new(frame), when: Taken::Now }),
        Err(reason) => match prefetched {
            Some(frame) => {
                let age = frame.taken.elapsed();
                Ok(AgentFrame { frame, when: Taken::Fallback { reason, age } })
            }
            None => Err(reason),
        },
    }
}

/// A fresh frame of the game (no hotkey frame): `zoom_screen` with `fresh`, `watch_screen`.
pub async fn grab_now(state: &AppState) -> Result<Frame, String> {
    let hwnd = state.overlay.lock().unwrap().game_hwnd;
    grab_game(state, hwnd).await
}

/// The last frame the agent was shown whole (full resolution) and the size it saw it at:
/// `zoom_screen` regions are in those coordinates.
#[derive(Clone)]
pub struct Seen {
    pub frame: Arc<Frame>,
    pub width: u32,
    pub height: u32,
}

static SEEN: Mutex<Option<Seen>> = Mutex::new(None);

pub fn seen() -> Option<Seen> {
    SEEN.lock().unwrap().clone()
}

pub fn set_seen(frame: Arc<Frame>, width: u32, height: u32) {
    *SEEN.lock().unwrap() = Some(Seen { frame, width, height });
}

async fn grab_game(state: &AppState, hwnd: isize) -> Result<Frame, String> {
    if hwnd == 0 {
        return Err(tr!("err_no_game"));
    }
    state.capture.grab(hwnd).await
}

pub struct Stored {
    /// The saved frame (full resolution).
    pub shot: Shot,
    /// What the agent gets: possibly scaled down (`[capture] agent_max_height`).
    pub mime: &'static str,
    pub base64: String,
    pub sent_width: u32,
    pub sent_height: u32,
}

/// Saves the frame with the chat `chat_id` at full resolution and encodes the copy for the
/// agent, scaled down to `[capture] agent_max_height`. Both run in parallel off the async threads.
pub async fn store(state: &AppState, chat_id: &str, frame: Arc<Frame>) -> Result<Stored, String> {
    let (width, height) = capture::agent_size(frame.width, frame.height, state.config.read().unwrap().capture.agent_max_height);
    store_as(state, chat_id, frame, width, height).await
}

/// `store`, with the agent's copy at `sent_width`×`sent_height` (scaled either way).
pub async fn store_as(
    state: &AppState,
    chat_id: &str,
    frame: Arc<Frame>,
    sent_width: u32,
    sent_height: u32,
) -> Result<Stored, String> {
    let started = std::time::Instant::now();
    let shot = Shot { id: uuid::Uuid::new_v4().simple().to_string(), width: frame.width, height: frame.height };

    let for_agent = tauri::async_runtime::spawn_blocking({
        let frame = frame.clone();
        move || {
            let scaled = if (sent_width, sent_height) == (frame.width, frame.height) {
                None
            } else {
                Some(capture::downscale(&frame, sent_width, sent_height)?)
            };
            let image = capture::encode(scaled.as_ref().unwrap_or(&frame))?;
            Ok::<_, String>((image.mime, base64::engine::general_purpose::STANDARD.encode(&image.bytes), image.bytes.len()))
        }
    });
    // The agent gets the frame even if saving it fails (it then has no thumbnail).
    let path = state.history.shot_path(chat_id, &shot.id, "image/jpeg");
    let saved = tauri::async_runtime::spawn_blocking({
        let path = path.clone();
        move || {
            let image = capture::encode(&frame)?;
            std::fs::write(&path, &image.bytes).map_err(|e| e.to_string())?;
            Ok::<_, String>((image.mime, image.bytes.len()))
        }
    });
    let (for_agent, saved) = tokio::join!(for_agent, saved);
    let (mime, base64, sent_bytes) = for_agent.map_err(|_| tr!("err_encode_interrupted"))??;
    match saved {
        Ok(Ok((saved_mime, size))) => {
            if !state.history.add_shot(chat_id, &shot, saved_mime, size) {
                let _ = std::fs::remove_file(&path);
            }
        }
        Ok(Err(e)) => tracing::warn!("frame not saved to {}: {e}", path.display()),
        Err(_) => tracing::warn!("frame not saved to {}", path.display()),
    }
    tracing::debug!(
        "frame {}x{} stored, agent gets {sent_width}x{sent_height} ({:.2} MB) in {} ms",
        shot.width,
        shot.height,
        sent_bytes as f64 / 1e6,
        started.elapsed().as_millis()
    );
    Ok(Stored { shot, mime, base64, sent_width, sent_height })
}

/// The saved image of a frame: bytes and MIME type.
pub async fn load(state: &AppState, id: &str) -> Option<(Vec<u8>, String)> {
    let (path, mime) = state.history.shot_file(id)?;
    let bytes = tauri::async_runtime::spawn_blocking(move || std::fs::read(path)).await.ok()?.ok()?;
    Some((bytes, mime))
}
