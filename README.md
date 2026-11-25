# game-overlai

AI assistant overlay for any game on Windows. A hotkey opens a question field over the game; it
expands into a chat panel that can see the game (a frame is attached to the question, and the agent
can take one itself). The same chats open in a browser. The AI runs through a CLI agent the user is
already signed in to (Claude Code; Codex support exists in code) over ACP — no API keys.

## Getting started

Requirements: Windows 10/11, Rust (stable, MSVC), Node.js LTS + npm, WebView2 (built into Windows 11),
Claude Code signed in (`claude`, then `/login`). `scripts\windows\setup.bat` checks and installs the
tools via winget.

```powershell
scripts\windows\dev.bat      # Vite with hot reload + the app, in one console
scripts\windows\build.bat    # installer (per user, no admin) + portable exe -> release\<version>\
```

By hand: `cd web; npm install; npm run build`, then `cd src-tauri; cargo run`. On first start the app
installs the pinned ACP adapter of the agent into `%LOCALAPPDATA%\game-overlai\agents\`.

Using it: `Ctrl+Alt+Q` — quick question (starts a new chat), `Ctrl+Alt+W` — chat panel (the game's latest chat),
`Ctrl+Alt+A` — keyboard focus between the game and the open panel (the panel stays), `Esc` / ✕ — hide. The panel belongs to the game window it was opened over: while another window is in front (the Start menu, another app) it stays drawn over the game behind that window (not over other apps; hidden while the game is minimized), and is on top again with the game. The panel moves by its header (grip in the compact field) and resizes by its
edges, always inside the game window; its place is remembered per game. Ctrl + wheel / Ctrl +/− over the
panel zoom it (content and window together, 50–200%, per game), Ctrl+0 — back to the default zoom. The agent looks at the game itself (`screenshot_game`) when a question
needs the screen; its first frame after the hotkey is the one grabbed as the panel opened. The tray icon has the rest;
the chat in a browser is at http://127.0.0.1:47823/.

Settings: tray → "Настройки…" (http://127.0.0.1:47823/settings): hotkeys (Ctrl+Alt by default: games
almost never bind it; letters that type a character with AltGr in an installed keyboard layout are skipped;
a game's page can give it its own combinations or turn some off while the game is in the foreground), agent (Claude Code or Codex),
proxy, default panel size, frame size for the agent, phone port, notifications, start with Windows.
Hotkeys and most values apply at once; the agent, proxy and ports after the restart the page offers.

Game profiles: a game's name in the browser's chat list (or the game chip in the overlay) opens its page —
its name for the agent, standing instructions (spoilers, character, language...), sites to search first,
and its chats. The agent gets the profile in the system prompt of a new session; a resumed
session keeps its first system prompt (Claude Code), so a changed profile is sent with the next question.

Progress memory (settings → Agent, on by default): the agent saves lasting facts about the user's
progress per game (chapter, build, bosses beaten) with MCP tools, and every chat of that game gets
them in its prompt. The game's page lists them ("Прогресс") to fix, add or delete.

Agent tools (our MCP server): `screenshot_game`; `zoom_screen` — a region of the last screenshot at
full resolution (small text); `watch_screen` — waits (≤ 50 s) until the screen changes or settles and
returns a fresh frame; `progress_add/_update/_remove` (only while memory is on); `widget_list/_create/
_update/_delete`.

Widgets: cards the agent pins over a game's window (title, short Markdown, a checklist the user ticks,
a countdown, and hidden JSON `data` the agent keeps progress in), kept per game. They show whenever that
game is in the foreground, in one transparent topmost window over it that never takes the focus and
lets clicks through outside the cards. A card moves by its title bar (stays inside the game window,
snaps to its edges, place kept as a fraction of the window), its width changes by the bottom-right
corner, the corner button makes it 50% see-through, the chevron folds it. The game's page lists and
deletes them. `/widgets?exe=<game.exe>` previews a game's widgets in a browser tab (the layer itself is
out of screenshots, like the panel, unless captures are allowed in the settings).

Phone: tray → "Доступ с телефона (Wi-Fi)" turns on a second server for the local network (port 47824,
off by default) and opens http://127.0.0.1:47823/phone with the QR code. The QR link carries an access
token once and leaves a cookie (SameSite=Lax: QR scanners open the link from their own site); without
it the LAN server answers 401. It serves the browser chat only
(no MCP, no overlay or access controls); "Сбросить доступ" signs every device out. A question from the
phone takes the window in the foreground on the PC as the game (its frames come from it). The server listens on
every interface; "Сетевой интерфейс" on /phone picks the address in the QR code (automatic = a private
address of a real adapter, the default route first; virtual/VPN ones are listed separately), kept in `lan.json`.

## Layout

| Path | What |
|---|---|
| `src-tauri/src/main.rs`, `state.rs` | startup, shared app state |
| `src-tauri/src/overlay/` | the panel window: placement over the game, focus, capture exclusion (Win32) |
| `src-tauri/src/agent/` | agents over ACP: adapter install, supervision, sessions, permissions; `spec.rs` = per-agent settings |
| `src-tauri/src/chat.rs`, `history.rs` | current chat ↔ agent session, system prompt; chat history in SQLite, frames on disk |
| `src-tauri/src/games.rs`, `progress.rs` | game profiles (per exe): name, instructions, sources; progress memory |
| `src-tauri/src/widgets.rs` | widgets per game and the transparent layer window over the game (click-through outside cards) |
| `src-tauri/src/capture/`, `shots.rs` | game frames: Windows Graphics Capture, JPEG encoding, storage; `look.rs` = zoom crops, change detection |
| `src-tauri/src/mcp.rs` | MCP server for the agent (`/mcp`, bearer token): frames, progress memory, widgets |
| `src-tauri/src/server/` | HTTP + WebSocket servers the UI talks to: local (127.0.0.1) and LAN (token) |
| `src-tauri/src/lan.rs` | phone access: on/off, token, addresses, QR codes |
| `src-tauri/src/protocol.rs` | WebSocket protocol; TS types are generated from it |
| `src-tauri/src/proxy.rs`, `config.rs`, `tray.rs`, `hotkeys.rs` | network proxy, settings, tray, global hotkeys |
| `src-tauri/src/settings.rs`, `notify.rs` | backend of the settings page, start with Windows; notifications |
| `src-tauri/src/i18n.rs`, `web/src/i18n.ts`, `web/src/locales/` | interface languages: string codes → `<language>.json` (en, ru, es, zh, ja), shared by Rust and the UI |
| `web/` | UI (Vite + React + TS): `/panel` = overlay panel, `/` = browser chat, `/phone`, `/settings` |
| `spikes/` | phase 0 prototypes (ACP, capture, overlay), kept for reference |

## Conventions

- After changing `protocol.rs`, run `cargo test` in `src-tauri/`: it regenerates
  `web/src/api/generated/` (don't edit those files).
- Before committing: `cargo test`, `cargo clippy` (no warnings) in `src-tauri/`, `npm run build` in `web/`.
- No DLL injection or hooks into game processes (anti-cheat): the overlay is a separate topmost window,
  frames come from Windows Graphics Capture.
- UI: neutral dark theme, no blue.
- UI text: never inline. Use a string code — `t("code")` (`useT()`) in the UI, `tr!("code", name = value)` in
  Rust — and add it to every `web/src/locales/*.json` (English is the base; `{name}` placeholders). `cargo test`
  checks that every used code exists and every language has every string with the same placeholders.
  Logs and the agent's system prompt stay in English.

## Runtime files

- Settings: `%APPDATA%\game-overlai\config.toml` (created on first run; missing keys = defaults; the settings
  page rewrites it). `agent = "claude"` or `"codex"`; `[notifications] enabled` — a Windows notification when
  an answer to a question from the overlay is ready (or the agent asks for a permission) while the panel is
  hidden. `language = "ru"` — interface language (en, ru, es, zh, ja; default: the Windows one if the app
  has it, else English); the settings page switches it at once, for every UI, the tray and notifications. Start with Windows = the `game-overlai` value under `HKCU\...\CurrentVersion\Run`.
  `[network] proxy = "system"` takes the Windows proxy (incl. PAC), else HTTP(S)_PROXY, else direct;
  `require_proxy_for = ["claude"]` refuses to start that agent without a proxy.
  `[agent_options.<agent>]` — the model / effort picked in the input row (button left of ↑), applied to
  every session of that agent; the choices are whatever the agent offers (ACP session config options).
  `[capture] agent_max_height = 720` — frames go to the agent scaled to this height (aspect kept,
  0 = full; never over 2000 px a side); saved frames stay full size.
  `[lan] port = 47824` — port for phones (a free one if busy).
  `[memory] progress = true` — progress memory (the agent's tools for it and the notes in its prompt).
- History: `%LOCALAPPDATA%\game-overlai\history.db` (chats, game profiles, progress notes, widgets, panel places) + `shots\` (frames,
  deleted with their chat). `lan.json` there: phone access on/off and its token.
- Logs: `%LOCALAPPDATA%\game-overlai\logs\` (`OVERLAI_LOG=debug` for more).

Debug switches: `OVERLAI_UI_DEV=http://localhost:5173` (UI from Vite), `OVERLAI_DEBUG_SHOW=compact|expanded`
(open the panel on start), `OVERLAI_DEBUG_CAPTURABLE=1` (panel visible in screenshots, like the setting),
`OVERLAI_LAN_ADDR=127.0.0.1` (phone server on this PC only: tests without the firewall prompt).
