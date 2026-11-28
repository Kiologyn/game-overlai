//! Chat history on disk, in %LOCALAPPDATA%\game-overlai\ (machine-local: frames are big and
//! shouldn't roam with the user profile):
//!
//!   history.db                   SQLite: games, chats, messages, frames
//!   shots\<chat id>\<shot id>.jpg  game frames of a chat (deleted with the chat)
//!
//! A chat is saved with its first message; an empty new chat exists only in memory.
//! Write errors are logged and don't stop the chat: the history is a convenience.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, params};
use tracing::{info, warn};

use crate::protocol::{ChatMessage, ChatSummary, GameHotkeys, GameInfo, GameProfile, ProgressNote, Shot, Widget};

/// Bumped with every schema change; `migrate` brings older files up to date.
const SCHEMA_VERSION: i32 = 7;

const SCHEMA_V1: &str = "
CREATE TABLE games (
    exe        TEXT PRIMARY KEY COLLATE NOCASE,  -- process name, e.g. cs2.exe
    title      TEXT NOT NULL,                    -- latest window title
    last_seen  INTEGER NOT NULL                  -- unix ms
);
CREATE TABLE chats (
    id          TEXT PRIMARY KEY,
    game_exe    TEXT COLLATE NOCASE REFERENCES games(exe),
    game_title  TEXT,             -- window title when the chat started (also when the exe is unknown)
    agent       TEXT NOT NULL,    -- agent id, e.g. claude
    session_id  TEXT,             -- the agent's session, resumed after a restart
    title       TEXT,             -- given by the agent
    preview     TEXT NOT NULL,    -- start of the first question (title until the agent gives one)
    created_at  INTEGER NOT NULL, -- unix ms
    updated_at  INTEGER NOT NULL  -- unix ms of the last question
);
CREATE INDEX chats_by_time ON chats(updated_at DESC);
CREATE INDEX chats_by_game ON chats(game_exe, updated_at DESC);
CREATE TABLE messages (
    id       TEXT PRIMARY KEY,
    chat_id  TEXT NOT NULL REFERENCES chats(id) ON DELETE CASCADE,
    seq      INTEGER NOT NULL,    -- position in the chat
    body     TEXT NOT NULL        -- protocol::ChatMessage as JSON
);
CREATE INDEX messages_by_chat ON messages(chat_id, seq);
CREATE TABLE shots (
    id          TEXT PRIMARY KEY,
    chat_id     TEXT NOT NULL REFERENCES chats(id) ON DELETE CASCADE,
    mime        TEXT NOT NULL,
    width       INTEGER NOT NULL,
    height      INTEGER NOT NULL,
    bytes       INTEGER NOT NULL,
    created_at  INTEGER NOT NULL
);
CREATE INDEX shots_by_chat ON shots(chat_id);
";

const SCHEMA_V2: &str = "
CREATE TABLE panel_layouts (
    game_exe    TEXT PRIMARY KEY COLLATE NOCASE,  -- '' = no game
    width       REAL NOT NULL,     -- logical px
    height      REAL NOT NULL,     -- logical px, expanded panel
    center_x    REAL NOT NULL,     -- 0..1 across the game window
    bottom      REAL NOT NULL,     -- 0..1 down the game window
    updated_at  INTEGER NOT NULL
);
";

/// Game profiles: what the user tells the agent about a game (phase 7).
const SCHEMA_V3: &str = "
ALTER TABLE games ADD COLUMN name TEXT;                          -- the user's name for the game
ALTER TABLE games ADD COLUMN instructions TEXT NOT NULL DEFAULT '';
ALTER TABLE games ADD COLUMN sources TEXT NOT NULL DEFAULT '';   -- sites to search first, one per line
ALTER TABLE games ADD COLUMN screenshot INTEGER;                 -- unused: was the 📷 default (the button is gone)
";

/// Which version of its game's profile a chat's agent session was told (`chat::profile_key`).
const SCHEMA_V4: &str = "
ALTER TABLE chats ADD COLUMN profile_key INTEGER;
";

/// Panel zoom per game (NULL = rows saved before zoom existed: 100%).
const SCHEMA_V5: &str = "
ALTER TABLE panel_layouts ADD COLUMN zoom REAL;
";

/// What the agent remembers about the user's progress per game, and the widgets pinned over
/// each game (a widget is kept whole as JSON: its fields change with the app).
const SCHEMA_V6: &str = "
CREATE TABLE progress_notes (
    id          TEXT PRIMARY KEY,
    game_exe    TEXT NOT NULL COLLATE NOCASE,
    text        TEXT NOT NULL,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);
CREATE INDEX progress_by_game ON progress_notes(game_exe, created_at);
CREATE TABLE widgets (
    id          TEXT PRIMARY KEY,
    game_exe    TEXT NOT NULL COLLATE NOCASE,
    body        TEXT NOT NULL,     -- protocol::Widget as JSON
    created_at  INTEGER NOT NULL
);
CREATE INDEX widgets_by_game ON widgets(game_exe, created_at);
";

/// A game's own hotkeys (protocol::GameHotkeys as JSON; NULL = the ones from the settings).
const SCHEMA_V7: &str = "
ALTER TABLE games ADD COLUMN hotkeys TEXT;
";

/// Where the user put the overlay panel over a game, relative to the game window (so it stays
/// in place when the game changes resolution or moves).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PanelLayout {
    /// Logical px, at `zoom`.
    pub width: f64,
    pub height: f64,
    pub center_x: f64,
    pub bottom: f64,
    /// Scale of the panel's content (1.0 = 100%).
    pub zoom: f64,
}

pub fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// A chat as stored, to make it the current one again.
#[derive(Debug)]
pub struct StoredChat {
    pub id: String,
    pub game: Option<GameInfo>,
    pub title: Option<String>,
    pub agent: String,
    pub session_id: Option<String>,
    /// Version of the game's profile the session was told.
    pub profile_key: Option<i64>,
    pub messages: Vec<ChatMessage>,
}

pub struct History {
    db: Mutex<Connection>,
    shots_dir: PathBuf,
}

impl History {
    /// Opens (or creates) the history in `dir`. A file that can't be opened is moved aside and
    /// a new one started; if even that fails, the history lives in memory for this run.
    pub fn open(dir: &Path) -> Self {
        let shots_dir = dir.join("shots");
        let file = dir.join("history.db");
        let db = match open_file(&file) {
            Ok(db) => db,
            Err(e) => {
                let aside = dir.join(format!("history.broken-{}.db", now_ms()));
                warn!("history {} can't be opened ({e}); moved to {}", file.display(), aside.display());
                let _ = std::fs::rename(&file, &aside);
                open_file(&file).unwrap_or_else(|e| {
                    warn!("history: {e}; keeping it in memory for this run");
                    let db = Connection::open_in_memory().expect("in-memory SQLite");
                    migrate(&db).expect("schema in memory");
                    db
                })
            }
        };
        Self { db: Mutex::new(db), shots_dir }
    }

    #[cfg(test)]
    fn in_memory(shots_dir: PathBuf) -> Self {
        let db = Connection::open_in_memory().unwrap();
        db.pragma_update(None, "foreign_keys", true).unwrap();
        migrate(&db).unwrap();
        Self { db: Mutex::new(db), shots_dir }
    }

    fn write(&self, what: &str, f: impl FnOnce(&Connection) -> rusqlite::Result<()>) {
        if let Err(e) = f(&self.db.lock().unwrap()) {
            warn!("history: {what}: {e}");
        }
    }

    fn read<T: Default>(&self, what: &str, f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> T {
        f(&self.db.lock().unwrap()).unwrap_or_else(|e| {
            warn!("history: {what}: {e}");
            T::default()
        })
    }

    /// Saves a new chat (with its first question as the preview).
    pub fn create_chat(&self, id: &str, game: Option<&GameInfo>, agent: &str, preview: &str, now: i64) {
        self.write("create chat", |db| {
            let exe = game.map(|g| g.exe.as_str()).filter(|e| !e.is_empty());
            if let (Some(exe), Some(g)) = (exe, game) {
                db.execute(
                    "INSERT INTO games (exe, title, last_seen) VALUES (?1, ?2, ?3)
                     ON CONFLICT (exe) DO UPDATE SET
                         title = CASE WHEN excluded.title <> '' THEN excluded.title ELSE games.title END,
                         last_seen = excluded.last_seen",
                    params![exe, g.title, now],
                )?;
            }
            db.execute(
                "INSERT INTO chats (id, game_exe, game_title, agent, preview, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
                params![id, exe, game.map(|g| g.title.as_str()), agent, preview_of(preview), now],
            )?;
            Ok(())
        });
    }

    pub fn save_message(&self, chat_id: &str, seq: usize, message: &ChatMessage) {
        let Ok(body) = serde_json::to_string(message) else { return };
        self.write("save message", |db| {
            db.execute(
                "INSERT INTO messages (id, chat_id, seq, body) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (id) DO UPDATE SET seq = excluded.seq, body = excluded.body",
                params![message.id, chat_id, seq as i64, body],
            )
            .map(drop)
        });
    }

    /// The chat got a new question: it moves to the top of the list.
    pub fn touch(&self, chat_id: &str, now: i64) {
        self.write("touch chat", |db| {
            db.execute("UPDATE chats SET updated_at = ?2 WHERE id = ?1", params![chat_id, now]).map(drop)
        });
    }

    pub fn set_title(&self, chat_id: &str, title: &str) {
        self.write("set title", |db| {
            db.execute("UPDATE chats SET title = ?2 WHERE id = ?1", params![chat_id, title]).map(drop)
        });
    }

    pub fn set_session(&self, chat_id: &str, agent: &str, session_id: &str) {
        self.write("set session", |db| {
            db.execute("UPDATE chats SET agent = ?2, session_id = ?3 WHERE id = ?1", params![chat_id, agent, session_id])
                .map(drop)
        });
    }

    pub fn set_profile_key(&self, chat_id: &str, key: i64) {
        self.write("set profile key", |db| {
            db.execute("UPDATE chats SET profile_key = ?2 WHERE id = ?1", params![chat_id, key]).map(drop)
        });
    }

    /// All chats, newest first.
    pub fn list(&self) -> Vec<ChatSummary> {
        self.read("list chats", |db| {
            let mut q = db.prepare(&format!("{SUMMARY} ORDER BY c.updated_at DESC"))?;
            q.query_map([], summary_row)?.collect()
        })
    }

    pub fn summary(&self, id: &str) -> Option<ChatSummary> {
        self.read("chat summary", |db| {
            db.query_row(&format!("{SUMMARY} WHERE c.id = ?1"), [id], summary_row).optional()
        })
    }

    pub fn load(&self, id: &str) -> Option<StoredChat> {
        self.read("load chat", |db| {
            let Some(mut chat) = db
                .query_row(
                    "SELECT c.id, c.game_exe, COALESCE(g.title, c.game_title), c.agent, c.session_id, c.title,
                            c.profile_key
                     FROM chats c LEFT JOIN games g ON g.exe = c.game_exe WHERE c.id = ?1",
                    [id],
                    |r| {
                        Ok(StoredChat {
                            id: r.get(0)?,
                            game: game_of(r.get(1)?, r.get(2)?),
                            agent: r.get(3)?,
                            session_id: r.get(4)?,
                            title: r.get(5)?,
                            profile_key: r.get(6)?,
                            messages: vec![],
                        })
                    },
                )
                .optional()?
            else {
                return Ok(None);
            };
            let mut q = db.prepare("SELECT body FROM messages WHERE chat_id = ?1 ORDER BY seq")?;
            let bodies = q.query_map([id], |r| r.get::<_, String>(0))?;
            for body in bodies {
                match serde_json::from_str::<ChatMessage>(&body?) {
                    Ok(m) => chat.messages.push(m),
                    Err(e) => warn!("history: chat {id}: unreadable message skipped: {e}"),
                }
            }
            Ok(Some(chat))
        })
    }

    /// The chat the user had last (the one to show after a restart).
    pub fn latest(&self) -> Option<String> {
        self.read("latest chat", |db| {
            db.query_row("SELECT id FROM chats ORDER BY updated_at DESC LIMIT 1", [], |r| r.get(0)).optional()
        })
    }

    pub fn latest_for_game(&self, exe: &str) -> Option<String> {
        self.read("latest chat of a game", |db| {
            db.query_row(
                "SELECT id FROM chats WHERE game_exe = ?1 ORDER BY updated_at DESC LIMIT 1",
                [exe],
                |r| r.get(0),
            )
            .optional()
        })
    }

    /// Deletes the chat, its messages and its frames. Returns whether it existed.
    pub fn delete(&self, id: &str) -> bool {
        let deleted = self.read("delete chat", |db| db.execute("DELETE FROM chats WHERE id = ?1", [id]));
        match std::fs::remove_dir_all(self.chat_shots_dir(id)) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => warn!("history: frames of chat {id} not deleted: {e}"),
        }
        deleted > 0
    }

    fn chat_shots_dir(&self, chat_id: &str) -> PathBuf {
        // Ids are our own uuids; refuse anything that could leave the directory.
        let safe: String = chat_id.chars().filter(char::is_ascii_alphanumeric).collect();
        self.shots_dir.join(safe)
    }

    /// Where a frame's file goes (the directory is created).
    pub fn shot_path(&self, chat_id: &str, shot_id: &str, mime: &str) -> PathBuf {
        let dir = self.chat_shots_dir(chat_id);
        let _ = std::fs::create_dir_all(&dir);
        let ext = if mime == "image/png" { "png" } else { "jpg" };
        let safe: String = shot_id.chars().filter(char::is_ascii_alphanumeric).collect();
        dir.join(format!("{safe}.{ext}"))
    }

    /// Records a frame whose file was written to `shot_path`. False if it can't be recorded
    /// (e.g. the chat was deleted meanwhile): the caller removes the file.
    pub fn add_shot(&self, chat_id: &str, shot: &Shot, mime: &str, bytes: usize) -> bool {
        let rows = self.read("add frame", |db| {
            db.execute(
                "INSERT INTO shots (id, chat_id, mime, width, height, bytes, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![shot.id, chat_id, mime, shot.width, shot.height, bytes as i64, now_ms()],
            )
        });
        rows == 1
    }

    /// File and MIME type of a frame.
    pub fn shot_file(&self, id: &str) -> Option<(PathBuf, String)> {
        let row: Option<(String, String)> = self.read("find frame", |db| {
            db.query_row("SELECT chat_id, mime FROM shots WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?)))
                .optional()
        });
        row.map(|(chat_id, mime)| {
            let ext = if mime == "image/png" { "png" } else { "jpg" };
            let safe: String = id.chars().filter(char::is_ascii_alphanumeric).collect();
            (self.chat_shots_dir(&chat_id).join(format!("{safe}.{ext}")), mime)
        })
    }
}

/// What the user can change in a game's profile.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProfileEdit {
    pub name: Option<String>,
    pub instructions: String,
    pub sources: String,
}

impl History {
    /// All games with chats, a profile, progress notes or widgets, most recently played first.
    pub fn games(&self) -> Vec<GameProfile> {
        let mut games: Vec<GameProfile> = self.read("list games", |db| {
            let mut q = db.prepare(&format!("{GAME} ORDER BY last_seen DESC"))?;
            q.query_map([], game_row)?.collect()
        });
        let mut notes: Vec<(String, ProgressNote)> = self.read("list progress", |db| {
            let mut q = db.prepare("SELECT game_exe, id, text, updated_at FROM progress_notes ORDER BY created_at")?;
            q.query_map([], |r| Ok((r.get::<_, String>(0)?.to_lowercase(), note_row(r, 1)?)))?.collect()
        });
        for game in &mut games {
            let exe = game.exe.to_lowercase();
            game.progress = notes.extract_if(.., |(e, _)| *e == exe).map(|(_, n)| n).collect();
        }
        games
    }

    pub fn game(&self, exe: &str) -> Option<GameProfile> {
        if exe.is_empty() {
            return None;
        }
        let mut game = self.read("game profile", |db| db.query_row(&format!("{GAME} WHERE exe = ?1"), [exe], game_row).optional())?;
        game.progress = self.progress(exe);
        Some(game)
    }

    /// What the agent remembers about the user's progress in a game, oldest first.
    pub fn progress(&self, exe: &str) -> Vec<ProgressNote> {
        self.read("progress", |db| {
            let mut q = db.prepare(
                "SELECT id, text, updated_at FROM progress_notes WHERE game_exe = ?1 ORDER BY created_at",
            )?;
            q.query_map([exe], |r| note_row(r, 0))?.collect()
        })
    }

    /// Adds a progress note (a game not seen yet is added with `title`).
    pub fn add_progress(&self, exe: &str, title: &str, text: &str) -> Option<ProgressNote> {
        if exe.is_empty() {
            return None;
        }
        let note = ProgressNote { id: short_id(), text: text.to_string(), updated_at: now_ms() };
        let mut ok = false;
        self.write("add progress", |db| {
            ensure_game(db, exe, title)?;
            db.execute(
                "INSERT INTO progress_notes (id, game_exe, text, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?4)",
                params![note.id, exe, note.text, note.updated_at],
            )?;
            ok = true;
            Ok(())
        });
        ok.then_some(note)
    }

    /// Changes a note's text; returns its game.
    pub fn edit_progress(&self, id: &str, text: &str) -> Option<String> {
        let exe = self.progress_game(id)?;
        self.write("edit progress", |db| {
            db.execute("UPDATE progress_notes SET text = ?2, updated_at = ?3 WHERE id = ?1", params![id, text, now_ms()])
                .map(drop)
        });
        Some(exe)
    }

    /// Removes a note; returns its game.
    pub fn delete_progress(&self, id: &str) -> Option<String> {
        let exe = self.progress_game(id)?;
        self.write("delete progress", |db| db.execute("DELETE FROM progress_notes WHERE id = ?1", [id]).map(drop));
        Some(exe)
    }

    fn progress_game(&self, id: &str) -> Option<String> {
        self.read("progress note", |db| {
            db.query_row("SELECT game_exe FROM progress_notes WHERE id = ?1", [id], |r| r.get(0)).optional()
        })
    }

    /// Every game's widgets, oldest first.
    pub fn widgets(&self) -> Vec<Widget> {
        let rows: Vec<String> = self.read("list widgets", |db| {
            let mut q = db.prepare("SELECT body FROM widgets ORDER BY created_at")?;
            q.query_map([], |r| r.get(0))?.collect()
        });
        rows.iter()
            .filter_map(|body| {
                serde_json::from_str(body).map_err(|e| warn!("history: a widget doesn't parse: {e}")).ok()
            })
            .collect()
    }

    /// Adds or replaces a widget (a game not seen yet is added with `title`).
    pub fn save_widget(&self, widget: &Widget, title: &str) {
        let Ok(body) = serde_json::to_string(widget) else { return };
        self.write("save widget", |db| {
            ensure_game(db, &widget.exe, title)?;
            db.execute(
                "INSERT INTO widgets (id, game_exe, body, created_at) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (id) DO UPDATE SET body = excluded.body",
                params![widget.id, widget.exe, body, now_ms()],
            )
            .map(drop)
        });
    }

    pub fn delete_widget(&self, id: &str) {
        self.write("delete widget", |db| db.execute("DELETE FROM widgets WHERE id = ?1", [id]).map(drop));
    }

    /// Saves the profile (a game not seen yet is added with `title`). Returns the stored profile.
    pub fn update_game(&self, exe: &str, title: &str, edit: &ProfileEdit) -> Option<GameProfile> {
        if exe.is_empty() {
            return None;
        }
        let name = edit.name.as_deref().map(str::trim).filter(|n| !n.is_empty());
        self.write("update game", |db| {
            db.execute(
                "INSERT INTO games (exe, title, last_seen, name, instructions, sources)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT (exe) DO UPDATE SET name = excluded.name, instructions = excluded.instructions,
                     sources = excluded.sources",
                params![exe, title, now_ms(), name, edit.instructions.trim(), edit.sources.trim()],
            )
            .map(drop)
        });
        self.game(exe)
    }

    /// A game's own hotkeys (all "" for a game without them or not seen yet).
    pub fn game_hotkeys(&self, exe: &str) -> GameHotkeys {
        if exe.is_empty() {
            return GameHotkeys::default();
        }
        let json: Option<Option<String>> = self.read("game hotkeys", |db| {
            db.query_row("SELECT hotkeys FROM games WHERE exe = ?1", [exe], |r| r.get(0)).optional()
        });
        hotkeys_of(json.flatten())
    }

    /// Saves a game's own hotkeys (a game not seen yet is added with `title`).
    pub fn set_game_hotkeys(&self, exe: &str, title: &str, keys: &GameHotkeys) -> Option<GameProfile> {
        if exe.is_empty() {
            return None;
        }
        let json = (*keys != GameHotkeys::default()).then(|| serde_json::to_string(keys).ok()).flatten();
        self.write("game hotkeys", |db| {
            ensure_game(db, exe, title)?;
            db.execute("UPDATE games SET hotkeys = ?2 WHERE exe = ?1", params![exe, json]).map(drop)
        });
        self.game(exe)
    }

    pub fn panel_layout(&self, game_exe: &str) -> Option<PanelLayout> {
        self.read("panel layout", |db| {
            db.query_row(
                "SELECT width, height, center_x, bottom, COALESCE(zoom, 1.0) FROM panel_layouts WHERE game_exe = ?1",
                [game_exe],
                |r| {
                    Ok(PanelLayout {
                        width: r.get(0)?,
                        height: r.get(1)?,
                        center_x: r.get(2)?,
                        bottom: r.get(3)?,
                        zoom: r.get(4)?,
                    })
                },
            )
            .optional()
        })
    }

    pub fn clear_panel_layouts(&self) {
        self.write("clear panel layouts", |db| db.execute("DELETE FROM panel_layouts", []).map(drop));
    }

    pub fn save_panel_layout(&self, game_exe: &str, l: &PanelLayout) {
        self.write("save panel layout", |db| {
            db.execute(
                "INSERT INTO panel_layouts (game_exe, width, height, center_x, bottom, zoom, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT (game_exe) DO UPDATE SET width = excluded.width, height = excluded.height,
                     center_x = excluded.center_x, bottom = excluded.bottom, zoom = excluded.zoom,
                     updated_at = excluded.updated_at",
                params![game_exe, l.width, l.height, l.center_x, l.bottom, l.zoom, now_ms()],
            )
            .map(drop)
        });
    }
}

const GAME: &str = "SELECT exe, title, name, instructions, sources, hotkeys FROM games";

fn game_row(r: &rusqlite::Row) -> rusqlite::Result<GameProfile> {
    Ok(GameProfile {
        exe: r.get(0)?,
        title: r.get(1)?,
        name: r.get(2)?,
        instructions: r.get(3)?,
        sources: r.get(4)?,
        progress: Vec::new(),
        hotkeys: hotkeys_of(r.get(5)?),
    })
}

fn hotkeys_of(json: Option<String>) -> GameHotkeys {
    json.and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default()
}

fn note_row(r: &rusqlite::Row, at: usize) -> rusqlite::Result<ProgressNote> {
    Ok(ProgressNote { id: r.get(at)?, text: r.get(at + 1)?, updated_at: r.get(at + 2)? })
}

/// A game row for a game that has none yet (it got notes or widgets before any chat).
fn ensure_game(db: &Connection, exe: &str, title: &str) -> rusqlite::Result<()> {
    db.execute(
        "INSERT INTO games (exe, title, last_seen) VALUES (?1, ?2, ?3) ON CONFLICT (exe) DO NOTHING",
        params![exe, title, now_ms()],
    )
    .map(drop)
}

/// Short ids: the agent quotes them back (progress notes, widgets, checklist items).
pub fn short_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..8].to_string()
}

const SUMMARY: &str = "SELECT c.id, COALESCE(c.title, c.preview), c.game_exe, COALESCE(g.title, c.game_title), c.updated_at
                       FROM chats c LEFT JOIN games g ON g.exe = c.game_exe";

fn summary_row(r: &rusqlite::Row) -> rusqlite::Result<ChatSummary> {
    Ok(ChatSummary { id: r.get(0)?, title: r.get(1)?, game: game_of(r.get(2)?, r.get(3)?), updated_at: r.get(4)? })
}

fn game_of(exe: Option<String>, title: Option<String>) -> Option<GameInfo> {
    match (exe, title) {
        (None, None) => None,
        (exe, title) => Some(GameInfo { title: title.unwrap_or_default(), exe: exe.unwrap_or_default() }),
    }
}

/// First line of the question, at most ~80 characters.
fn preview_of(text: &str) -> String {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    match line.char_indices().nth(80) {
        Some((i, _)) => format!("{}…", line[..i].trim_end()),
        None => line.to_string(),
    }
}

fn open_file(file: &Path) -> rusqlite::Result<Connection> {
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let db = Connection::open(file)?;
    db.pragma_update(None, "journal_mode", "WAL")?;
    db.pragma_update(None, "synchronous", "NORMAL")?;
    db.pragma_update(None, "foreign_keys", true)?;
    db.busy_timeout(std::time::Duration::from_secs(2))?;
    migrate(&db)?;
    let chats: i64 = db.query_row("SELECT COUNT(*) FROM chats", [], |r| r.get(0))?;
    info!("history: {} ({chats} chats)", file.display());
    Ok(db)
}

fn migrate(db: &Connection) -> rusqlite::Result<()> {
    let version: i32 = db.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if version > SCHEMA_VERSION {
        warn!("history was written by a newer version (schema {version}); using it as is");
        return Ok(());
    }
    if version < 1 {
        db.execute_batch(&format!("BEGIN; {SCHEMA_V1} PRAGMA user_version = 1; COMMIT;"))?;
    }
    if version < 2 {
        db.execute_batch(&format!("BEGIN; {SCHEMA_V2} PRAGMA user_version = 2; COMMIT;"))?;
    }
    if version < 3 {
        db.execute_batch(&format!("BEGIN; {SCHEMA_V3} PRAGMA user_version = 3; COMMIT;"))?;
    }
    if version < 4 {
        db.execute_batch(&format!("BEGIN; {SCHEMA_V4} PRAGMA user_version = 4; COMMIT;"))?;
    }
    if version < 5 {
        db.execute_batch(&format!("BEGIN; {SCHEMA_V5} PRAGMA user_version = 5; COMMIT;"))?;
    }
    if version < 6 {
        db.execute_batch(&format!("BEGIN; {SCHEMA_V6} PRAGMA user_version = 6; COMMIT;"))?;
    }
    if version < 7 {
        db.execute_batch(&format!("BEGIN; {SCHEMA_V7} PRAGMA user_version = 7; COMMIT;"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{MessageStatus, Part, Role};

    fn message(id: &str, text: &str) -> ChatMessage {
        ChatMessage {
            id: id.into(),
            role: Role::User,
            parts: vec![Part::Text { text: text.into() }],
            screenshot: None,
            status: MessageStatus::Done,
            error: None,
        }
    }

    #[test]
    fn chats_roundtrip() {
        let tmp = std::env::temp_dir().join(format!("overlai-history-{}", uuid::Uuid::new_v4().simple()));
        let h = History::in_memory(tmp.join("shots"));
        let cs2 = GameInfo { title: "Counter-Strike 2".into(), exe: "cs2.exe".into() };
        h.create_chat("a", Some(&cs2), "claude", "  \n как раскидать смоки на мираже?\nвторая строка", 1000);
        h.save_message("a", 0, &message("m1", "как раскидать смоки на мираже?"));
        h.save_message("a", 1, &message("m2", "ответ"));
        h.save_message("a", 1, &message("m2", "ответ, исправленный")); // a new version replaces it
        h.create_chat("b", None, "claude", "просто вопрос", 2000);

        let list = h.list();
        assert_eq!(list.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), ["b", "a"]);
        assert_eq!(list[1].title, "как раскидать смоки на мираже?");
        assert_eq!(list[1].game, Some(cs2.clone()));
        assert_eq!(list[0].game, None);

        h.touch("a", 3000);
        h.set_title("a", "Смоки на Mirage");
        h.set_session("a", "claude", "sess-1");
        h.set_profile_key("a", -42);
        assert_eq!(h.latest().as_deref(), Some("a"));
        assert_eq!(h.latest_for_game("CS2.EXE").as_deref(), Some("a")); // exe matches without case
        assert_eq!(h.latest_for_game("dota2.exe"), None);
        assert_eq!(h.summary("a").unwrap().title, "Смоки на Mirage");

        // The latest window title of a game is shown for all its chats.
        let renamed = GameInfo { title: "Counter-Strike 2 (beta)".into(), exe: "cs2.exe".into() };
        h.create_chat("c", Some(&renamed), "claude", "ещё", 4000);
        assert_eq!(h.summary("a").unwrap().game.unwrap().title, "Counter-Strike 2 (beta)");

        let chat = h.load("a").unwrap();
        assert_eq!(chat.session_id.as_deref(), Some("sess-1"));
        assert_eq!(chat.profile_key, Some(-42));
        assert_eq!(chat.messages.len(), 2);
        assert_eq!(chat.messages[1].parts, vec![Part::Text { text: "ответ, исправленный".into() }]);

        let shot = Shot { id: "s1".into(), width: 4, height: 2 };
        let path = h.shot_path("a", &shot.id, "image/jpeg");
        std::fs::write(&path, b"jpeg").unwrap();
        assert!(h.add_shot("a", &shot, "image/jpeg", 4));
        assert!(!h.add_shot("gone", &Shot { id: "s2".into(), ..shot.clone() }, "image/jpeg", 4), "no such chat");
        assert_eq!(h.shot_file("s1"), Some((path.clone(), "image/jpeg".to_string())));

        assert!(h.delete("a"));
        assert!(h.load("a").is_none());
        assert!(h.shot_file("s1").is_none());
        assert!(!path.exists(), "frames are deleted with the chat");
        assert!(!h.delete("a"));
        let _ = std::fs::remove_dir_all(tmp);
    }

    #[test]
    fn panel_layouts() {
        let h = History::in_memory(std::env::temp_dir().join("overlai-unused"));
        assert_eq!(h.panel_layout("cs2.exe"), None);
        let l = PanelLayout { width: 500.0, height: 400.0, center_x: 0.25, bottom: 0.9, zoom: 1.25 };
        h.save_panel_layout("cs2.exe", &l);
        h.save_panel_layout("CS2.EXE", &PanelLayout { width: 600.0, ..l }); // same game
        assert_eq!(h.panel_layout("cs2.exe").map(|l| (l.width, l.zoom)), Some((600.0, 1.25)));
        assert_eq!(h.panel_layout(""), None);
        h.clear_panel_layouts();
        assert_eq!(h.panel_layout("cs2.exe"), None);
    }

    #[test]
    fn schema_version_is_the_last_migration() {
        let h = History::in_memory(std::env::temp_dir().join("overlai-unused"));
        let version: i32 = h.db.lock().unwrap().pragma_query_value(None, "user_version", |r| r.get(0)).unwrap();
        assert_eq!(version, SCHEMA_VERSION, "bump SCHEMA_VERSION with every new migration");
    }

    #[test]
    fn game_profiles() {
        let h = History::in_memory(std::env::temp_dir().join("overlai-unused"));
        let cs2 = GameInfo { title: "Counter-Strike 2".into(), exe: "cs2.exe".into() };
        h.create_chat("a", Some(&cs2), "claude", "вопрос", 1000);
        let p = h.game("CS2.exe").unwrap();
        assert_eq!((p.name, p.instructions.as_str()), (None, ""));

        let edit = ProfileEdit {
            name: Some("  CS2 ".into()),
            instructions: " без спойлеров 
".into(),
            sources: "liquipedia.net".into(),
        };
        let p = h.update_game("cs2.exe", "ignored for a known game", &edit).unwrap();
        assert_eq!(p.name.as_deref(), Some("CS2"));
        assert_eq!(p.instructions, "без спойлеров");
        assert_eq!(p.title, "Counter-Strike 2");
        // A new chat refreshes the window title but keeps the profile.
        h.create_chat("b", Some(&GameInfo { title: "CS2 beta".into(), ..cs2 }), "claude", "ещё", 2000);
        let p = h.game("cs2.exe").unwrap();
        assert_eq!((p.title.as_str(), p.name.as_deref()), ("CS2 beta", Some("CS2")));
        // A blank name clears it; a game without chats can get a profile too.
        h.update_game("cs2.exe", "", &ProfileEdit { name: Some(" ".into()), ..edit.clone() });
        assert_eq!(h.game("cs2.exe").unwrap().name, None);
        h.update_game("eldenring.exe", "ELDEN RING", &edit);
        assert_eq!(h.games().len(), 2);
        assert!(h.update_game("", "", &edit).is_none());
    }

    #[test]
    fn previews() {
        assert_eq!(preview_of("короткий"), "короткий");
        let long = "слово ".repeat(30);
        let p = preview_of(&long);
        assert!(p.ends_with('…') && p.chars().count() <= 81, "{p}");
    }

    #[test]
    fn reopening_a_file_keeps_chats() {
        let dir = std::env::temp_dir().join(format!("overlai-history-{}", uuid::Uuid::new_v4().simple()));
        History::open(&dir).create_chat("a", None, "claude", "вопрос", 1);
        assert_eq!(History::open(&dir).list().len(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn progress_notes_and_widgets() {
        let h = History::in_memory(std::env::temp_dir().join("overlai-unused-shots"));
        // A game with no chats yet gets a row with its first note.
        let a = h.add_progress("eldenring.exe", "ELDEN RING", "Chapter 3, playing a mage").unwrap();
        let b = h.add_progress("EldenRing.exe", "", "Margit defeated").unwrap();
        assert_eq!(h.game("eldenring.exe").unwrap().title, "ELDEN RING");
        let texts = |exe: &str| h.progress(exe).into_iter().map(|n| n.text).collect::<Vec<_>>();
        assert_eq!(texts("ELDENRING.EXE"), ["Chapter 3, playing a mage", "Margit defeated"]);
        assert_eq!(h.edit_progress(&a.id, "Chapter 4").as_deref(), Some("eldenring.exe"));
        assert_eq!(h.delete_progress(&b.id).as_deref(), Some("EldenRing.exe"));
        assert_eq!(h.delete_progress("nope"), None);
        let games = h.games();
        assert_eq!(games.len(), 1);
        assert_eq!(games[0].progress.iter().map(|n| n.text.as_str()).collect::<Vec<_>>(), ["Chapter 4"]);

        let mut w = Widget { id: short_id(), exe: "cs2.exe".into(), title: "Smokes".into(), width: 260.0, ..Default::default() };
        h.save_widget(&w, "Counter-Strike 2");
        w.title = "Smokes on Mirage".into();
        h.save_widget(&w, "");
        assert_eq!(h.widgets(), [w.clone()]);
        assert_eq!(h.game("cs2.exe").unwrap().title, "Counter-Strike 2");
        h.delete_widget(&w.id);
        assert!(h.widgets().is_empty());
    }

    #[test]
    fn game_hotkeys() {
        let h = History::in_memory(std::env::temp_dir().join("overlai-unused-shots"));
        assert_eq!(h.game_hotkeys("pubg.exe"), GameHotkeys::default());
        let keys = GameHotkeys { quick: "ctrl+shift+q".into(), focus: "off".into(), ..Default::default() };
        let game = h.set_game_hotkeys("pubg.exe", "PUBG", &keys).unwrap();
        assert_eq!((game.title.as_str(), &game.hotkeys), ("PUBG", &keys));
        assert_eq!(h.game_hotkeys("PUBG.EXE"), keys);
        assert_eq!(h.games()[0].hotkeys, keys);
        h.set_game_hotkeys("pubg.exe", "", &GameHotkeys::default());
        assert_eq!(h.game_hotkeys("pubg.exe"), GameHotkeys::default());
    }

}
