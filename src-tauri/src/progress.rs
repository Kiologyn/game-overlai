//! Progress memory: lasting facts about the user's progress in each game (chapter, build,
//! bosses beaten...). The agent saves and fixes them with MCP tools (`mcp.rs`); the user sees and
//! edits them on the game's page. While `[memory] progress` is on, every chat of the game gets
//! them in its system prompt (`chat::game_section`).

use tracing::info;

use crate::games;
use crate::protocol::ProgressNote;
use crate::state::AppState;

/// Per game: they all go into every prompt of the game's chats.
pub const MAX_NOTES: usize = 40;
pub const MAX_CHARS: usize = 300;

pub fn enabled(state: &AppState) -> bool {
    state.cfg().memory.progress
}

/// Errors are for the agent (English); the page only sends valid edits.
pub fn add(state: &AppState, exe: &str, text: &str) -> Result<ProgressNote, String> {
    let text = clean(text)?;
    let exe = exe.trim();
    if exe.is_empty() {
        return Err("the game is unknown".into());
    }
    let notes = state.history.progress(exe);
    if notes.len() >= MAX_NOTES {
        return Err(format!("this game already has {MAX_NOTES} notes: update or remove outdated ones first"));
    }
    if let Some(same) = notes.iter().find(|n| n.text.eq_ignore_ascii_case(&text)) {
        return Ok(same.clone());
    }
    let note = state
        .history
        .add_progress(exe, &games::known_title(state, exe), &text)
        .ok_or("the note could not be saved")?;
    info!("progress note for {exe}: {} chars", text.chars().count());
    games::broadcast(state, exe);
    Ok(note)
}

pub fn edit(state: &AppState, id: &str, text: &str) -> Result<(), String> {
    let text = clean(text)?;
    let exe = state.history.edit_progress(id.trim(), &text).ok_or_else(|| format!("no note with id {id}"))?;
    games::broadcast(state, &exe);
    Ok(())
}

pub fn delete(state: &AppState, id: &str) -> Result<(), String> {
    let exe = state.history.delete_progress(id.trim()).ok_or_else(|| format!("no note with id {id}"))?;
    games::broadcast(state, &exe);
    Ok(())
}

/// One line, trimmed, at most `MAX_CHARS`.
fn clean(text: &str) -> Result<String, String> {
    let one_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.is_empty() {
        return Err("the note is empty".into());
    }
    Ok(games::cut(&one_line, MAX_CHARS))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_are_one_short_line() {
        assert_eq!(clean("  Chapter 3,\n  mage ").unwrap(), "Chapter 3, mage");
        assert!(clean(" \n ").is_err());
        assert_eq!(clean(&"я".repeat(400)).unwrap().chars().count(), MAX_CHARS);
    }
}
