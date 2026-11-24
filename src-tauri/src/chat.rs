//! The current chat and its link to the agent: one agent session per chat, streamed into an
//! assistant message part by part. Chats are kept in the history (`history.rs`): a chat is
//! saved with its first message, and any saved chat can be made current again; its agent
//! session is then resumed, or (if the agent lost it) a new one gets the conversation so far.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use crate::agent::{AgentEvent, PromptBlock, StopReason, Update};
use crate::history::{StoredChat, now_ms};
use crate::mcp::IMAGE_TOOLS;
use crate::protocol::{
    AppendKind, ChatMessage, GameInfo, GameProfile, MessageStatus, Part, PermissionPrompt, Role, ServerMsg, Shot, ToolInfo,
    ToolStatus,
};
use crate::games;
use crate::state::{AppState, Target};

/// How often a streaming answer is written to the history (so a crash loses little).
const FLUSH_EVERY: Duration = Duration::from_secs(2);

#[derive(Debug)]
pub struct ChatState {
    /// Id in the history. An empty new chat has one too; it is saved with its first message.
    pub id: String,
    pub messages: Vec<ChatMessage>,
    pub game: Option<GameInfo>,
    pub title: Option<String>,
    /// Already in the history.
    saved: bool,
    /// Agent session of this chat.
    session_id: Option<String>,
    /// Agent process generation the session is live in; `None` or another one means
    /// "restore it first".
    session_generation: Option<u64>,
    /// Version of the game's profile the session was told (`profile_key`). A resumed session
    /// keeps its first system prompt, so a newer profile goes into the next question instead.
    session_profile: Option<u64>,
    /// The assistant message being streamed.
    active: Option<String>,
    /// That question was asked in the overlay: tell the user when the answer is ready (or the
    /// agent asks for a permission) if they hid the panel meanwhile.
    notify: bool,
    pub permissions: Vec<PermissionPrompt>,
    /// Frame the agent's screenshot tool took before its tool call reached us.
    pending_tool_shot: Option<Shot>,
    /// Changes whenever another chat becomes current, so late results of the old one are dropped.
    epoch: u64,
    /// When the streaming answer was last written to the history.
    flushed: Instant,
}

impl ChatState {
    pub fn empty(game: Option<GameInfo>) -> Self {
        Self {
            id: new_id(),
            messages: vec![],
            game,
            title: None,
            saved: false,
            session_id: None,
            session_generation: None,
            session_profile: None,
            active: None,
            notify: false,
            permissions: vec![],
            pending_tool_shot: None,
            epoch: 0,
            flushed: Instant::now(),
        }
    }

    /// A chat from the history. Its session is resumed on the next question if it belongs to
    /// `agent` (another agent can't continue it: it gets the conversation as text instead).
    pub fn from_stored(stored: StoredChat, agent: &str) -> Self {
        let mut messages = stored.messages;
        for m in &mut messages {
            if m.status == MessageStatus::Streaming {
                m.status = MessageStatus::Cancelled; // the app closed while it was answering
            }
        }
        Self {
            id: stored.id,
            messages,
            game: stored.game,
            title: stored.title,
            saved: true,
            session_id: stored.session_id.filter(|_| stored.agent == agent),
            session_profile: stored.profile_key.filter(|_| stored.agent == agent).map(|k| k as u64),
            ..Self::empty(None)
        }
    }

    fn message_mut(&mut self, id: &str) -> Option<&mut ChatMessage> {
        self.messages.iter_mut().find(|m| m.id == id)
    }

    /// Writes one message (by id) to the history.
    fn save(&self, state: &AppState, id: &str) {
        if !self.saved {
            return;
        }
        if let Some(seq) = self.messages.iter().position(|m| m.id == id) {
            state.history.save_message(&self.id, seq, &self.messages[seq]);
        }
    }
}

pub fn snapshot(state: &AppState) -> ServerMsg {
    let chat = state.chat.lock().unwrap();
    ServerMsg::ChatSnapshot {
        chat_id: chat.id.clone(),
        messages: chat.messages.clone(),
        game: chat.game.clone(),
        title: chat.title.clone(),
    }
}

pub fn permissions(state: &AppState) -> ServerMsg {
    ServerMsg::Permissions { prompts: state.chat.lock().unwrap().permissions.clone() }
}

pub fn chat_list(state: &AppState) -> ServerMsg {
    ServerMsg::ChatsSnapshot { chats: state.history.list() }
}

fn new_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

/// Makes `next` the current chat. The old one is left: an answer still streaming is stopped
/// and saved as it is, and its agent session is closed; it is resumed if the user comes back.
fn switch_to(state: &AppState, chat: &mut ChatState, mut next: ChatState) {
    if let Some(active) = chat.active.take() {
        if let Some(session_id) = chat.session_id.clone() {
            state.agent.cancel(session_id);
        }
        if let Some(m) = chat.message_mut(&active)
            && m.status == MessageStatus::Streaming
        {
            m.status = MessageStatus::Cancelled;
        }
        chat.save(state, &active);
    }
    if let Some(old) = chat.session_id.take() {
        state.agent.close_session(old);
    }
    next.epoch = chat.epoch + 1;
    *chat = next;
}

fn broadcast_chat(state: &AppState) {
    state.hub.send(Target::All, snapshot(state));
    state.hub.send(Target::All, permissions(state));
}

/// The list entry of a saved chat changed (new chat, new question, title).
fn broadcast_summary(state: &AppState, chat_id: &str) {
    if let Some(chat) = state.history.summary(chat_id) {
        state.hub.send(Target::All, ServerMsg::ChatsUpsert { chat });
    }
}

pub fn new_chat(state: &Arc<AppState>) {
    let game = state.overlay.lock().unwrap().game.clone();
    switch_to(state, &mut state.chat.lock().unwrap(), ChatState::empty(game));
    broadcast_chat(state);
}

/// Makes a chat from the history the current one.
pub fn open(state: &Arc<AppState>, id: &str) {
    if state.chat.lock().unwrap().id == id {
        return;
    }
    let Some(stored) = state.history.load(id) else {
        warn!("chat.open: {id} is not in the history");
        return;
    };
    debug!("open chat {id} ({} messages)", stored.messages.len());
    switch_to(state, &mut state.chat.lock().unwrap(), ChatState::from_stored(stored, state.agent.spec.id));
    broadcast_chat(state);
}

/// Deletes a chat with its frames. The current chat is replaced by an empty one (same game).
pub fn delete(state: &Arc<AppState>, id: &str) {
    let was_current = {
        let mut chat = state.chat.lock().unwrap();
        let current = chat.id == id;
        if current {
            let game = chat.game.clone();
            switch_to(state, &mut chat, ChatState::empty(game));
        }
        current
    };
    if state.history.delete(id) {
        info!("chat {id} deleted");
        state.hub.send(Target::All, ServerMsg::ChatsRemove { id: id.to_string() });
    }
    if was_current {
        broadcast_chat(state);
    }
}

/// Alt+W over a game: if the current chat is about another game (or none) and this game has
/// chats, its latest chat opens. Otherwise the current chat stays (also over non-game windows,
/// which have no chats). Not while an answer is streaming.
pub fn follow_game(state: &Arc<AppState>, game: &GameInfo) {
    if game.exe.is_empty() {
        return;
    }
    let same_game = {
        let chat = state.chat.lock().unwrap();
        if chat.active.is_some() {
            return;
        }
        chat.game.as_ref().is_some_and(|g| g.exe.eq_ignore_ascii_case(&game.exe))
    };
    if same_game {
        return;
    }
    if let Some(id) = state.history.latest_for_game(&game.exe) {
        debug!("Alt+W over {}: its latest chat {id}", game.exe);
        open(state, &id);
    }
}

pub fn send(state: &Arc<AppState>, text: &str, new_chat: bool, from_overlay: bool) {
    let text = text.trim().to_string();
    if text.is_empty() {
        return;
    }
    let game = state.overlay.lock().unwrap().game.clone();
    let user = ChatMessage {
        id: new_id(),
        role: Role::User,
        parts: vec![Part::Text { text: text.clone() }],
        screenshot: None, // only in chats from before the agent took frames itself
        status: MessageStatus::Done,
        error: None,
    };
    let reply = ChatMessage {
        id: new_id(),
        role: Role::Assistant,
        parts: vec![],
        screenshot: None,
        status: MessageStatus::Streaming,
        error: None,
    };
    let now = now_ms();
    let (chat_id, epoch, started_new) = {
        let mut chat = state.chat.lock().unwrap();
        if chat.active.is_some() {
            warn!("chat.send while an answer is streaming: ignored");
            return;
        }
        // An empty chat is replaced too: it takes the game the user is in now.
        let started_new = new_chat || chat.messages.is_empty();
        if started_new {
            switch_to(state, &mut chat, ChatState::empty(game));
        }
        if chat.saved {
            state.history.touch(&chat.id, now);
        } else {
            state.history.create_chat(&chat.id, chat.game.as_ref(), state.agent.spec.id, &text, now);
            chat.saved = true;
            if let Some(g) = &chat.game {
                games::broadcast(state, &g.exe); // a new game, or its latest window title
            }
        }
        chat.messages.push(user.clone());
        chat.messages.push(reply.clone());
        chat.save(state, &user.id);
        chat.save(state, &reply.id);
        chat.active = Some(reply.id.clone());
        chat.notify = from_overlay;
        chat.flushed = Instant::now();
        (chat.id.clone(), chat.epoch, started_new)
    };
    let turn = Turn { chat_id, epoch, reply_id: reply.id.clone(), text };
    if started_new {
        broadcast_chat(state);
    } else {
        state.hub.send(Target::All, ServerMsg::ChatMessage { message: user });
        state.hub.send(Target::All, ServerMsg::ChatMessage { message: reply });
    }
    broadcast_summary(state, &turn.chat_id);
    debug!("chat.send new_chat={started_new} len={}", turn.text.len());
    tauri::async_runtime::spawn(run_turn(state.clone(), turn));
}

/// The user picked a model / effort: remembered (config.toml, per agent) and applied to the
/// current chat's live session now; other chats get it when their session starts or resumes.
pub fn set_agent_option(state: &Arc<AppState>, id: &str, value: &str) {
    let offered = state
        .agent_options
        .lock()
        .unwrap()
        .iter()
        .any(|o| o.id == id && o.values.iter().any(|v| v.value == value));
    if !offered {
        warn!("agent.setOption {id}={value}: not offered by the agent");
        return;
    }
    let agent_id = state.agent.spec.id;
    let saved = {
        let mut cfg = state.config.write().unwrap();
        cfg.agent_options.entry(agent_id.to_string()).or_default().insert(id.to_string(), value.to_string());
        crate::config::save(&state.config_file, &cfg)
    };
    if let Err(e) = saved {
        warn!("can't save the agent option: {e}");
    }
    let live = {
        let chat = state.chat.lock().unwrap();
        chat.session_id.clone().filter(|_| chat.session_generation == Some(state.agent.generation()))
    };
    info!("agent option {id} = {value} (live session: {})", live.is_some());
    state.agent.set_option(live, id.to_string(), value.to_string());
}

/// Stops the answer being generated (the prompt then ends with "cancelled").
pub fn cancel(state: &Arc<AppState>) {
    let session = {
        let chat = state.chat.lock().unwrap();
        chat.active.as_ref().and(chat.session_id.clone())
    };
    if let Some(session_id) = session {
        state.agent.cancel(session_id);
    }
}

pub fn respond_permission(state: &Arc<AppState>, id: &str, option_id: Option<String>) {
    if !state.agent.respond_permission(id, option_id) {
        debug!("permission {id} already resolved");
    }
}

/// Saves the answer being streamed as it is (the app is quitting).
pub fn flush(state: &AppState) {
    let chat = state.chat.lock().unwrap();
    if let Some(active) = &chat.active {
        chat.save(state, active);
    }
}

fn profile_dir(state: &AppState) -> PathBuf {
    // One shared profile for now; per-game profiles come in phase 7. Sessions are resumed with
    // the same directory, so it must not change for existing chats.
    let dir = state.profiles_dir.join("default");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

struct Turn {
    chat_id: String,
    epoch: u64,
    reply_id: String,
    text: String,
}

/// Runs one question/answer turn: makes sure the chat has a live agent session, sends the
/// prompt and records how it ended. Streamed content arrives separately (`handle_event`).
async fn run_turn(state: Arc<AppState>, turn: Turn) {
    let Turn { chat_id: _, epoch, reply_id: message_id, text } = turn;
    let agent = state.agent.clone();
    let result = async {
        let session = ensure_session(&state, epoch).await?;
        let mut blocks = Vec::new();
        if session.needs_history {
            blocks.extend(conversation_so_far(&state, epoch).map(PromptBlock::Text));
        }
        blocks.extend(session.profile_note.map(PromptBlock::Text));
        blocks.push(PromptBlock::Text(text));
        agent.prompt(session.id, blocks).await
    }
    .await;

    let (status, error) = match result {
        Ok(StopReason::EndTurn) | Ok(StopReason::Other(_)) => (MessageStatus::Done, None),
        Ok(StopReason::Cancelled) => (MessageStatus::Cancelled, None),
        Ok(StopReason::Refusal) => (MessageStatus::Error, Some(tr!("err_refusal"))),
        Ok(StopReason::MaxTokens) => (MessageStatus::Error, Some(tr!("err_max_tokens"))),
        Err(e) => (MessageStatus::Error, Some(humanize_error(&e, crate::i18n::text(agent.spec.login_hint)))),
    };
    let mut chat = state.chat.lock().unwrap();
    if chat.epoch != epoch {
        return; // another chat became current meanwhile (the answer was saved then)
    }
    chat.active = None;
    let mut answer = String::new();
    if let Some(m) = chat.message_mut(&message_id) {
        m.status = status;
        m.error = error.clone();
        answer = m.parts.iter().filter_map(|p| match p {
            Part::Text { text } => Some(text.as_str()),
            _ => None,
        }).collect();
    }
    chat.save(&state, &message_id);
    if std::mem::take(&mut chat.notify) {
        match (status, &error) {
            (MessageStatus::Done, _) => crate::notify::answer_ready(&state, chat.title.as_deref(), &answer),
            (MessageStatus::Error, Some(e)) => crate::notify::answer_failed(&state, e),
            _ => {}
        }
    }
    state.hub.send(Target::All, ServerMsg::ChatStatus { message_id, status, error });
}

struct Session {
    id: String,
    /// A new session for a chat that already has messages: the agent hasn't seen them.
    needs_history: bool,
    /// The game's profile changed since the session was told it: the new one, for this prompt.
    profile_note: Option<String>,
}

/// The chat's agent session: a new one, the live one, or the earlier one restored (after a
/// restart of the agent or the app). If it can't be restored, a new one is started.
async fn ensure_session(state: &Arc<AppState>, epoch: u64) -> Result<Session, String> {
    let agent = &state.agent;
    let (chat_id, session, live_in, known_profile, game, earlier) = {
        let chat = state.chat.lock().unwrap();
        // The last two messages are this turn's question and answer.
        let earlier = chat.messages.len() > 2;
        let (id, generation, known) = (chat.session_id.clone(), chat.session_generation, chat.session_profile);
        (chat.id.clone(), id, generation, known, chat.game.clone(), earlier)
    };
    let cwd = profile_dir(state);
    let profile = game.as_ref().and_then(|g| games::profile(state, &g.exe));
    let memory = state.cfg().memory.progress;
    let key = profile_key(profile.as_ref(), memory);
    let prompt = system_prompt(game.as_ref(), profile.as_ref(), memory);
    // (id, a new session: its system prompt has the current profile)
    let (id, needs_history, new) = match session {
        Some(id) if live_in == Some(agent.generation()) => (id, false, false),
        Some(id) => {
            let started = Instant::now();
            match agent.restore_session(id.clone(), cwd.clone(), prompt.clone()).await {
                Ok(()) => {
                    info!("agent session {id} restored in {} ms", started.elapsed().as_millis());
                    (id, false, false)
                }
                Err(e) => {
                    warn!("agent session {id} can't be restored ({e}); a new one gets the chat so far");
                    (agent.new_session(cwd, prompt).await?, earlier, true)
                }
            }
        }
        None => (agent.new_session(cwd, prompt).await?, earlier, true),
    };
    // A continued session was told an older profile (chats from before profiles: none at all).
    let outdated = match known_profile {
        Some(known) => known != key,
        None => key != profile_key(None, false),
    };
    let profile_note = (!new && outdated).then(|| {
        info!("chat {chat_id}: the game's profile changed; the agent gets the new one with this question");
        format!(
            "(Note from the overlay app, not written by the user: the game's profile was changed since this \
             conversation started. From now on it replaces anything said about the game earlier:\n{})",
            game_section(game.as_ref(), profile.as_ref(), memory).trim_end()
        )
    });
    let mut chat = state.chat.lock().unwrap();
    if chat.epoch != epoch {
        return Err(tr!("err_chat_closed"));
    }
    if chat.session_id.as_deref() != Some(&id) {
        debug!("chat {chat_id}: agent session {id}");
        state.history.set_session(&chat_id, agent.spec.id, &id);
    }
    if chat.session_profile != Some(key) {
        state.history.set_profile_key(&chat_id, key as i64);
    }
    chat.session_id = Some(id.clone());
    chat.session_generation = Some(agent.generation());
    chat.session_profile = Some(key);
    Ok(Session { id, needs_history, profile_note })
}

/// Total size of the earlier conversation given to a new session (newest messages kept).
const HISTORY_CHARS: usize = 12_000;
/// Per message: long answers are cut.
const MESSAGE_CHARS: usize = 1_500;

/// The chat before this turn as text, for a new agent session of an existing chat.
fn conversation_so_far(state: &AppState, epoch: u64) -> Option<String> {
    let chat = state.chat.lock().unwrap();
    if chat.epoch != epoch {
        return None;
    }
    let earlier = &chat.messages[..chat.messages.len().saturating_sub(2)];
    let text = transcript(earlier, HISTORY_CHARS);
    (!text.is_empty()).then(|| {
        format!(
            "(This chat continues an earlier conversation that you no longer have in context. \
             Here it is, for reference; frames of the game that were attached are not included.)\n\n\
             {text}\n\n(End of the earlier conversation. The user's new message follows.)"
        )
    })
}

/// "User: …" / "Assistant: …" lines of the messages' text, the newest that fit in `budget`.
fn transcript(messages: &[ChatMessage], budget: usize) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut used = 0;
    for m in messages.iter().rev() {
        let text: String = m
            .parts
            .iter()
            .filter_map(|p| match p {
                Part::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("")
            .trim()
            .to_string();
        if text.is_empty() {
            continue;
        }
        let text = match text.char_indices().nth(MESSAGE_CHARS) {
            Some((i, _)) => format!("{}…", &text[..i]),
            None => text,
        };
        let who = match m.role {
            Role::User => "User",
            Role::Assistant => "Assistant",
        };
        let line = format!("{who}: {text}");
        used += line.chars().count();
        if used > budget && !lines.is_empty() {
            break;
        }
        lines.push(line);
    }
    lines.reverse();
    lines.join("\n\n")
}

/// One of the agent's image tools (screenshot, zoom, watch) made a frame: show it on that tool
/// call in the answer.
pub fn attach_tool_shot(state: &AppState, shot: Shot) {
    let mut chat = state.chat.lock().unwrap();
    let Some(active) = chat.active.clone() else { return };
    let Some(message) = chat.message_mut(&active) else { return };
    let waiting = message.parts.iter_mut().rev().find_map(|p| match p {
        Part::Tool { tool } if IMAGE_TOOLS.contains(&tool.title.as_str()) && tool.shot.is_none() => Some(tool),
        _ => None,
    });
    match waiting {
        Some(tool) => {
            tool.shot = Some(shot);
            let tool = tool.clone();
            chat.save(state, &active);
            state.hub.send(Target::All, ServerMsg::ChatTool { message_id: active, tool });
        }
        None => chat.pending_tool_shot = Some(shot), // its tool call hasn't arrived yet
    }
}

fn humanize_error(e: &str, login_hint: &str) -> String {
    let lower = e.to_lowercase();
    if ["login", "auth", "credential", "401", "unauthorized", "api key"].iter().any(|k| lower.contains(k)) {
        format!("{e}\n\n{login_hint}")
    } else if lower.contains("rate limit") || lower.contains("usage limit") || lower.contains("429") {
        format!("{}\n\n{e}", tr!("err_usage_limit"))
    } else if ["econnrefused", "etimedout", "enotfound", "econnreset", "fetch failed", "connection error", "network"]
        .iter()
        .any(|k| lower.contains(k))
    {
        format!("{}\n\n{e}", tr!("err_no_connection"))
    } else {
        e.to_string()
    }
}

/// Identifies what a session is told about the game's profile (the name, instructions and
/// sources; not the 📷 default).
fn profile_key(profile: Option<&GameProfile>, memory: bool) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    if let Some(p) = profile {
        p.name.as_deref().unwrap_or("").trim().hash(&mut h);
        p.instructions.trim().hash(&mut h);
        p.sources.trim().hash(&mut h);
    } else {
        ("", "", "").hash(&mut h);
    }
    // Progress memory: on or off, and the notes (keys of chats from before it stay the same while off).
    if memory {
        "memory".hash(&mut h);
        for n in profile.map(|p| p.progress.as_slice()).unwrap_or_default() {
            (&n.id, &n.text).hash(&mut h);
        }
    }
    h.finish()
}

/// The agent changed the game's progress notes from the current chat: its session knows the
/// change, so the next question needn't repeat the game's section.
pub fn progress_changed_by_agent(state: &AppState) {
    let memory = state.cfg().memory.progress;
    let mut chat = state.chat.lock().unwrap();
    let Some(game) = chat.game.clone() else { return };
    if chat.session_profile.is_none() {
        return;
    }
    let key = profile_key(games::profile(state, &game.exe).as_ref(), memory);
    chat.session_profile = Some(key);
    if chat.saved {
        state.history.set_profile_key(&chat.id, key as i64);
    }
}

const BASE_PROMPT: &str = "You are an AI assistant inside an overlay on top of a video game. The user is playing right \
     now and reads your answers between actions.\n\
     - Lead with the answer. Be brief and concrete; use short numbered steps for instructions.\n\
     - Reply in the language the user writes in.\n\
     - For game facts you are not sure about (items, quests, bosses, builds, patch changes), use web search \
       and mention the source briefly. Don't invent details.\n\
     - Format with Markdown; keep answers short enough to read in a few seconds unless asked for detail.\n\
     - You don't see the game unless you call the `screenshot_game` tool, which returns a frame of the game \
       window. Call it whenever the answer may depend on what is on screen: the user says \"this\", \"here\", \
       \"what is that\", asks about their current situation, UI, map, inventory, an enemy or a puzzle, or seeing \
       it would make the answer more precise. Don't call it for general questions about the game. The first \
       frame after the user opens the overlay shows the moment they opened it (the mouse cursor may point at \
       the thing); later calls take a fresh one. If small text in it matters and you can't read it, look closer \
       with `zoom_screen`. If the user asks you to wait for something on screen, use `watch_screen`.\n\
     - Widgets (`widget_create` and the other widget tools) are small cards pinned over the game for the user: \
       quest steps, a build, a checklist, a countdown. Make one when the user asks to pin or track something, \
       keep it short, and read `widget_list` before changing one (the user ticks items).\n";

fn system_prompt(game: Option<&GameInfo>, profile: Option<&GameProfile>, memory: bool) -> String {
    format!("{BASE_PROMPT}{}", game_section(game, profile, memory))
}

/// What the agent is told about the game: which one it is, the user's profile of it and (with
/// progress memory on) what it remembers about the user's progress.
fn game_section(game: Option<&GameInfo>, profile: Option<&GameProfile>, memory: bool) -> String {
    let mut p = String::new();
    let name = profile.and_then(|p| p.name.as_deref()).filter(|n| !n.is_empty());
    match (game, name) {
        (Some(g), Some(name)) => p.push_str(&format!(
            "- The game is {name} (window \"{}\", process {}). Unless the user names another game, questions are \
             about this game.\n",
            g.title, g.exe
        )),
        (Some(g), None) if !g.title.is_empty() || !g.exe.is_empty() => p.push_str(&format!(
            "- The game window is \"{}\" (process {}). Unless the user names another game, questions are about \
             this game. The window title may be generic: infer the game from it and the process name.\n",
            g.title, g.exe
        )),
        _ => p.push_str("- The game is unknown; ask which game if it matters.\n"),
    }
    if memory && game.is_some_and(|g| !g.exe.is_empty()) {
        p.push_str(&progress_section(profile.map(|p| p.progress.as_slice()).unwrap_or_default()));
    }
    let Some(profile) = profile else { return p };
    let sources: Vec<&str> = profile.sources.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    if !sources.is_empty() {
        p.push_str(
            "\nPreferred sources for this game: search these first (e.g. with `site:`) and prefer them when \
             citing; use other sites only when these don't have the answer.\n",
        );
        for s in sources {
            p.push_str(&format!("- {s}\n"));
        }
    }
    let instructions = profile.instructions.trim();
    if !instructions.is_empty() {
        p.push_str(&format!(
            "\nThe user's standing instructions for this game. Follow them in every answer; they take priority \
             over the defaults above:\n<instructions>\n{instructions}\n</instructions>\n"
        ));
    }
    p
}

/// Progress memory: what to save, and what is saved so far.
fn progress_section(notes: &[crate::protocol::ProgressNote]) -> String {
    let mut p = String::from(
        "\nProgress memory: save lasting facts about the user's progress in this game with `progress_add`: the \
         chapter, act or area reached, their character, class or build, bosses beaten and quests done, important \
         choices, goals they mention. One short self-contained fact per note. When a fact changes, fix its note with \
         `progress_update` (or `progress_remove`) instead of adding a new one. Don't save temporary details, general \
         game facts or anything the user asks you not to remember, and don't announce saving. Use what you know \
         below without asking again, but the user's newer words win.\n",
    );
    if notes.is_empty() {
        p.push_str("Nothing is saved for this game yet.\n");
    } else {
        p.push_str("Saved so far (id: fact):\n");
        for n in notes {
            p.push_str(&format!("- {}: {}\n", n.id, n.text.replace('\n', " ")));
        }
    }
    p
}

// ------------------------------------------------------------------ agent events

fn refresh_tray(state: &AppState) {
    if let Some(app) = state.app.get() {
        let handle = app.clone();
        let _ = app.run_on_main_thread(move || crate::tray::refresh_tooltip(&handle));
    }
}

pub fn spawn_event_loop(state: Arc<AppState>, mut events: mpsc::UnboundedReceiver<AgentEvent>) {
    tauri::async_runtime::spawn(async move {
        while let Some(event) = events.recv().await {
            handle_event(&state, event);
        }
    });
}

fn handle_event(state: &Arc<AppState>, event: AgentEvent) {
    match event {
        AgentEvent::Status(status) => {
            debug!("agent status: {:?} {:?}", status.state, status.detail);
            *state.agent_status.lock().unwrap() = status.clone();
            state.hub.send(Target::All, ServerMsg::AgentStatus { status });
            refresh_tray(state);
        }
        AgentEvent::Options(options) => {
            debug!(
                "agent options: {}",
                options.iter().map(|o| format!("{}={}", o.id, o.current)).collect::<Vec<_>>().join(", ")
            );
            *state.agent_options.lock().unwrap() = options.clone();
            state.hub.send(Target::All, ServerMsg::AgentOptions { options });
        }
        AgentEvent::Usage(usage) => {
            let changed = state.usage.lock().unwrap().replace(usage.clone()).as_ref() != Some(&usage);
            if changed {
                state.hub.send(Target::All, ServerMsg::AgentUsage { usage });
                refresh_tray(state);
            }
        }
        AgentEvent::PermissionRequested { session_id, prompt } => {
            let mut chat = state.chat.lock().unwrap();
            if chat.session_id.as_deref() == Some(&session_id) {
                if chat.notify {
                    crate::notify::permission(state, &prompt.title);
                }
                chat.permissions.push(prompt);
                state.hub.send(Target::All, ServerMsg::Permissions { prompts: chat.permissions.clone() });
            } else {
                drop(chat);
                state.agent.respond_permission(&prompt.id, None); // stale session
            }
        }
        AgentEvent::PermissionClosed { id } => {
            let mut chat = state.chat.lock().unwrap();
            let before = chat.permissions.len();
            chat.permissions.retain(|p| p.id != id);
            if chat.permissions.len() != before {
                state.hub.send(Target::All, ServerMsg::Permissions { prompts: chat.permissions.clone() });
            }
        }
        AgentEvent::Update { session_id, update } => {
            let mut chat = state.chat.lock().unwrap();
            if chat.session_id.as_deref() != Some(&session_id) {
                return;
            }
            if let Update::Title(title) = update {
                if chat.title.as_deref() == Some(&title) {
                    return;
                }
                chat.title = Some(title.clone());
                let chat_id = chat.id.clone();
                if chat.saved {
                    state.history.set_title(&chat_id, &title);
                }
                drop(chat);
                state.hub.send(Target::All, ServerMsg::ChatTitle { title });
                broadcast_summary(state, &chat_id);
                return;
            }
            let Some(active) = chat.active.clone() else { return };
            let pending_shot = chat.pending_tool_shot.take();
            let Some(message) = chat.message_mut(&active) else { return };
            let mut out = match update {
                Update::Text(text) => {
                    message.append(AppendKind::Text, &text);
                    ServerMsg::ChatAppend { message_id: active.clone(), kind: AppendKind::Text, text }
                }
                Update::Thought(text) => {
                    message.append(AppendKind::Thought, &text);
                    ServerMsg::ChatAppend { message_id: active.clone(), kind: AppendKind::Thought, text }
                }
                Update::Tool(tool) => {
                    let tool = message.upsert_tool(tool);
                    ServerMsg::ChatTool { message_id: active.clone(), tool }
                }
                Update::ToolPatch { id, title, kind, status } => {
                    let mut tool = message
                        .parts
                        .iter()
                        .find_map(|p| match p {
                            Part::Tool { tool } if tool.id == id => Some(tool.clone()),
                            _ => None,
                        })
                        .unwrap_or(ToolInfo {
                            id,
                            title: String::new(),
                            kind: "other".into(),
                            status: ToolStatus::Pending,
                            shot: None,
                        });
                    if let Some(t) = title {
                        tool.title = t;
                    }
                    if let Some(k) = kind {
                        tool.kind = k;
                    }
                    if let Some(s) = status {
                        tool.status = s;
                    }
                    let tool = message.upsert_tool(tool);
                    ServerMsg::ChatTool { message_id: active.clone(), tool }
                }
                Update::Title(_) => unreachable!(),
            };
            // A frame the screenshot tool took before its tool call showed up.
            if let Some(shot) = pending_shot {
                match &mut out {
                    ServerMsg::ChatTool { tool, .. } if IMAGE_TOOLS.contains(&tool.title.as_str()) && tool.shot.is_none() => {
                        tool.shot = Some(shot);
                        message.upsert_tool(tool.clone());
                    }
                    _ => chat.pending_tool_shot = Some(shot),
                }
            }
            if chat.flushed.elapsed() >= FLUSH_EVERY {
                chat.save(state, &active);
                chat.flushed = Instant::now();
            }
            state.hub.send(Target::All, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(role: Role, parts: Vec<Part>) -> ChatMessage {
        ChatMessage { id: new_id(), role, parts, screenshot: None, status: MessageStatus::Done, error: None }
    }

    fn text(t: &str) -> Part {
        Part::Text { text: t.into() }
    }

    #[test]
    fn transcript_keeps_text_and_the_newest_messages() {
        let messages = vec![
            msg(Role::User, vec![text("first question")]),
            msg(Role::Assistant, vec![Part::Thought { text: "hidden".into() }, text("first "), text("answer")]),
            msg(Role::User, vec![text("second question")]),
            msg(Role::Assistant, vec![]), // cancelled before any text
        ];
        assert_eq!(
            transcript(&messages, 10_000),
            "User: first question\n\nAssistant: first answer\n\nUser: second question"
        );
        // Over the budget: the oldest messages go first, the newest one is always kept.
        assert_eq!(transcript(&messages, 30), "User: second question");
        assert_eq!(transcript(&messages, 1), "User: second question");
        let long = msg(Role::Assistant, vec![text(&"я".repeat(MESSAGE_CHARS + 50))]);
        let t = transcript(&[long], 100_000);
        assert!(t.ends_with('…') && t.chars().count() == "Assistant: ".len() + MESSAGE_CHARS + 1);
    }

    #[test]
    fn system_prompt_carries_the_game_profile() {
        let game = GameInfo { title: "UnityWndClass".into(), exe: "hollow.exe".into() };
        let plain = system_prompt(Some(&game), None, false);
        assert!(plain.contains("\"UnityWndClass\" (process hollow.exe)") && plain.contains("infer the game"));
        assert!(!plain.contains("<instructions>") && !plain.contains("Preferred sources"));

        let mut profile = GameProfile {
            exe: "hollow.exe".into(),
            title: "UnityWndClass".into(),
            name: Some("Hollow Knight".into()),
            instructions: "  без спойлеров  ".into(),
            sources: "hollowknight.wiki\n\n  fextralife.com ".into(),
            progress: vec![],
        };
        let p = system_prompt(Some(&game), Some(&profile), false);
        assert!(p.contains("The game is Hollow Knight (window \"UnityWndClass\""), "{p}");
        assert!(!p.contains("infer the game"));
        assert!(p.contains("- hollowknight.wiki\n- fextralife.com\n"), "{p}");
        assert!(p.contains("<instructions>\nбез спойлеров\n</instructions>"), "{p}");

        assert_ne!(profile_key(Some(&profile), false), profile_key(None, false));
        // An empty profile is the same as none.
        profile = GameProfile { name: None, instructions: String::new(), sources: " \n".into(), ..profile };
        assert_eq!(system_prompt(Some(&game), Some(&profile), false), plain);
        assert_eq!(profile_key(Some(&profile), false), profile_key(None, false));
    }

    #[test]
    fn progress_memory_in_the_prompt() {
        let game = GameInfo { title: "ELDEN RING".into(), exe: "eldenring.exe".into() };
        let note = |id: &str, text: &str| crate::protocol::ProgressNote { id: id.into(), text: text.into(), updated_at: 0 };
        let mut profile = GameProfile {
            exe: "eldenring.exe".into(),
            title: "ELDEN RING".into(),
            name: None,
            instructions: String::new(),
            sources: String::new(),
            progress: vec![note("a1", "Chapter 3"), note("b2", "Plays a mage")],
        };
        let off = system_prompt(Some(&game), Some(&profile), false);
        assert!(!off.contains("progress_add") && !off.contains("Chapter 3"), "memory off: nothing about it");
        let on = system_prompt(Some(&game), Some(&profile), true);
        assert!(on.contains("progress_add") && on.contains("- a1: Chapter 3\n- b2: Plays a mage\n"), "{on}");
        assert!(system_prompt(Some(&game), None, true).contains("Nothing is saved for this game yet."));
        assert!(!system_prompt(None, None, true).contains("progress_add"), "no game: nothing to remember it for");

        let key = profile_key(Some(&profile), true);
        assert_ne!(key, profile_key(Some(&profile), false));
        profile.progress[0].text = "Chapter 4".into();
        assert_ne!(key, profile_key(Some(&profile), true), "a changed note changes the key");
        assert_eq!(profile_key(Some(&profile), false), profile_key(None, false), "notes don't count while off");
    }

    #[test]
    fn stored_chat_keeps_its_session_only_for_the_same_agent() {
        let stored = |agent: &str| StoredChat {
            id: "c".into(),
            game: None,
            title: Some("t".into()),
            agent: agent.into(),
            session_id: Some("s".into()),
            profile_key: Some(7),
            messages: vec![ChatMessage { status: MessageStatus::Streaming, ..msg(Role::Assistant, vec![text("ha")]) }],
        };
        let chat = ChatState::from_stored(stored("claude"), "claude");
        assert_eq!(chat.session_id.as_deref(), Some("s"));
        assert_eq!(chat.session_profile, Some(7));
        assert_eq!(chat.session_generation, None, "restored before the first prompt");
        assert_eq!(chat.messages[0].status, MessageStatus::Cancelled, "interrupted answers don't stay streaming");
        assert!(chat.saved);
        assert_eq!(ChatState::from_stored(stored("codex"), "claude").session_id, None);
    }
}
