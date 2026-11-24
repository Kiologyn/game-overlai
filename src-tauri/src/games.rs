//! Game profiles: per executable, what the user tells the agent about the game (its name,
//! standing instructions, sites to search first) and what the agent remembers about the user's
//! progress (`progress.rs`). A game gets a profile row with its first chat (or note, or widget);
//! the browser page of a game edits it. Every chat of the game gets the profile in its system
//! prompt (`chat::system_prompt`), from its next question.

use tracing::info;

use crate::history::ProfileEdit;
use crate::protocol::{GameProfile, ServerMsg};
use crate::state::{AppState, Target};

/// Longer texts are cut (they go into every prompt of the game's chats).
const MAX_INSTRUCTIONS: usize = 4_000;
const MAX_SOURCES: usize = 1_000;
const MAX_NAME: usize = 100;

pub fn list(state: &AppState) -> ServerMsg {
    ServerMsg::GamesSnapshot { games: state.history.games() }
}

/// The game's row changed (new game, new window title, profile): tell every UI.
pub fn broadcast(state: &AppState, exe: &str) {
    if let Some(game) = state.history.game(exe) {
        state.hub.send(Target::All, ServerMsg::GamesUpsert { game });
    }
}

pub fn profile(state: &AppState, exe: &str) -> Option<GameProfile> {
    state.history.game(exe)
}

pub fn update(state: &AppState, exe: &str, mut edit: ProfileEdit) {
    let exe = exe.trim();
    if exe.is_empty() {
        return;
    }
    edit.name = edit.name.map(|n| cut(&n, MAX_NAME));
    edit.instructions = cut(&edit.instructions, MAX_INSTRUCTIONS);
    edit.sources = cut(&edit.sources, MAX_SOURCES);
    let title = known_title(state, exe);
    if let Some(game) = state.history.update_game(exe, &title, &edit) {
        info!(
            "game profile {exe}: name={:?}, {} chars of instructions, {} sources",
            game.name,
            game.instructions.chars().count(),
            game.sources.lines().filter(|l| !l.trim().is_empty()).count(),
        );
        state.hub.send(Target::All, ServerMsg::GamesUpsert { game });
    }
}

/// The window title of a game without chats yet (opened from the overlay), if we know it: its
/// row is created with it.
pub fn known_title(state: &AppState, exe: &str) -> String {
    let over = state.overlay.lock().unwrap().game.clone();
    let in_chat = state.chat.lock().unwrap().game.clone();
    [over, in_chat]
        .into_iter()
        .flatten()
        .find(|g| g.exe.eq_ignore_ascii_case(exe))
        .map(|g| g.title)
        .unwrap_or_default()
}

pub fn cut(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) => s[..i].to_string(),
        None => s.to_string(),
    }
}
