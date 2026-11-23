//! Game frames for the agent: attached to the user's message (📷 in the input field) or taken
//! by the agent itself through the MCP tool. Encoded images are kept with their chat in the
//! history (`shots\<chat>\<shot>.jpg`) and served to the UI at `/api/shots/{id}`.

use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;

use crate::capture::{self, Frame};
use crate::protocol::Shot;
use crate::state::AppState;

/// Frame for a message the user sends with 📷: the one grabbed when the hotkey opened the
/// overlay (the moment the user asks about; some games also pause or hide tooltips once they
/// lose focus) if this is the first message since, otherwise a fresh one.
pub async fn for_message(state: &AppState) -> Result<Arc<Frame>, String> {
    let (hwnd, prefetched) = {
        let mut ov = state.overlay.lock().unwrap();
        let prefetched = ov.prefetch.as_mut().filter(|p| !p.used).map(|p| {
            p.used = true;
            p.frame.clone()
        });
        (ov.game_hwnd, prefetched)
    };
    match prefetched {
        Some(frame) => Ok(frame),
        None => grab_game(state, hwnd).await.map(Arc::new),
    }
}

pub struct AgentFrame {
    pub frame: Arc<Frame>,
    /// Set when the game can't be captured now and the frame from when the overlay opened
    /// is used instead: why, and how old that frame is.
    pub fallback: Option<(String, Duration)>,
}

/// Frame for the agent's tool: a fresh one; if the game can't be captured right now (e.g. a
/// fullscreen game minimised itself when the overlay took focus), the one from the hotkey.
pub async fn for_agent(state: &AppState) -> Result<AgentFrame, String> {
    let (hwnd, prefetched) = {
        let ov = state.overlay.lock().unwrap();
        (ov.game_hwnd, ov.prefetch.as_ref().map(|p| p.frame.clone()))
    };
    match grab_game(state, hwnd).await {
        Ok(frame) => Ok(AgentFrame { frame: Arc::new(frame), fallback: None }),
        Err(reason) => match prefetched {
            Some(frame) => {
                let age = frame.taken.elapsed();
                Ok(AgentFrame { frame, fallback: Some((reason, age)) })
            }
            None => Err(reason),
        },
    }
}

async fn grab_game(state: &AppState, hwnd: isize) -> Result<Frame, String> {
    if hwnd == 0 {
        return Err("игра не определена — откройте оверлей горячей клавишей поверх игры".into());
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
    let started = std::time::Instant::now();
    let shot = Shot { id: uuid::Uuid::new_v4().simple().to_string(), width: frame.width, height: frame.height };
    let (sent_width, sent_height) = capture::agent_size(frame.width, frame.height, state.config.read().unwrap().capture.agent_max_height);

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
    let (mime, base64, sent_bytes) = for_agent.map_err(|_| "кодирование кадра прервалось".to_string())??;
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
