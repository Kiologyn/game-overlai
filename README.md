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

Using it: `Alt+Q` — quick question (starts a new chat), `Alt+W` — chat panel (the game's latest chat),
`Esc` / ✕ — hide. The panel moves by its header (grip in the compact field) and resizes by its
edges, always inside the game window; its place is remembered per game. 📷 in the input field attaches a frame of the game. The tray icon has the rest;
the chat in a browser is at http://127.0.0.1:47823/.

Settings: tray → "Настройки…" (http://127.0.0.1:47823/settings): hotkeys, agent (Claude Code or Codex),
proxy, default panel size, frame size for the agent, phone port, notifications, start with Windows.
Hotkeys and most values apply at once; the agent, proxy and ports after the restart the page offers.

Game profiles: a game's name in the browser's chat list (or the game chip in the overlay) opens its page —
its name for the agent, standing instructions (spoilers, character, language...), sites to search first,
the 📷 default, and its chats. The agent gets the profile in the system prompt of a new session; a resumed
session keeps its first system prompt (Claude Code), so a changed profile is sent with the next question.

Phone: tray → "Доступ с телефона (Wi-Fi)" turns on a second server for the local network (port 47824,
off by default) and opens http://127.0.0.1:47823/phone with the QR code. The QR link carries an access
token once and leaves a cookie (SameSite=Lax: QR scanners open the link from their own site); without
it the LAN server answers 401. It serves the browser chat only
(no MCP, no overlay or access controls); "Сбросить доступ" signs every device out. A question from the
phone takes the window in the foreground on the PC as the game (and 📷 captures it).

## Layout

| Path | What |
|---|---|
| `src-tauri/src/main.rs`, `state.rs` | startup, shared app state |
| `src-tauri/src/overlay/` | the panel window: placement over the game, focus, capture exclusion (Win32) |
| `src-tauri/src/agent/` | agents over ACP: adapter install, supervision, sessions, permissions; `spec.rs` = per-agent settings |
| `src-tauri/src/chat.rs`, `history.rs` | current chat ↔ agent session, system prompt; chat history in SQLite, frames on disk |
| `src-tauri/src/games.rs` | game profiles (per exe): name, instructions, sources, 📷 default |
| `src-tauri/src/capture/`, `shots.rs` | game frames: Windows Graphics Capture, JPEG encoding, storage |
| `src-tauri/src/mcp.rs` | MCP server for the agent (`/mcp`, bearer token): `screenshot_game` |
| `src-tauri/src/server/` | HTTP + WebSocket servers the UI talks to: local (127.0.0.1) and LAN (token) |
| `src-tauri/src/lan.rs` | phone access: on/off, token, addresses, QR codes |
| `src-tauri/src/protocol.rs` | WebSocket protocol; TS types are generated from it |
| `src-tauri/src/proxy.rs`, `config.rs`, `tray.rs`, `hotkeys.rs` | network proxy, settings, tray, global hotkeys |
| `src-tauri/src/settings.rs`, `notify.rs` | backend of the settings page, start with Windows; notifications |
| `web/` | UI (Vite + React + TS): `/panel` = overlay panel, `/` = browser chat, `/phone`, `/settings` |
| `spikes/` | phase 0 prototypes (ACP, capture, overlay), kept for reference |

## Conventions

- After changing `protocol.rs`, run `cargo test` in `src-tauri/`: it regenerates
  `web/src/api/generated/` (don't edit those files).
- Before committing: `cargo test`, `cargo clippy` (no warnings) in `src-tauri/`, `npm run build` in `web/`.
- No DLL injection or hooks into game processes (anti-cheat): the overlay is a separate topmost window,
  frames come from Windows Graphics Capture.
- UI: neutral dark theme, no blue. UI text is in Russian.

## Runtime files

- Settings: `%APPDATA%\game-overlai\config.toml` (created on first run; missing keys = defaults; the settings
  page rewrites it). `agent = "claude"` or `"codex"`; `[notifications] enabled` — a Windows notification when
  an answer to a question from the overlay is ready (or the agent asks for a permission) while the panel is
  hidden. Start with Windows = the `game-overlai` value under `HKCU\...\CurrentVersion\Run`.
  `[network] proxy = "system"` takes the Windows proxy (incl. PAC), else HTTP(S)_PROXY, else direct;
  `require_proxy_for = ["claude"]` refuses to start that agent without a proxy.
  `[capture] agent_max_height = 720` — frames go to the agent scaled to this height (aspect kept,
  0 = full; never over 2000 px a side); saved frames stay full size.
  `[lan] port = 47824` — port for phones (a free one if busy).
- History: `%LOCALAPPDATA%\game-overlai\history.db` (chats, game profiles, panel places) + `shots\` (frames,
  deleted with their chat). `lan.json` there: phone access on/off and its token.
- Logs: `%LOCALAPPDATA%\game-overlai\logs\` (`OVERLAI_LOG=debug` for more).

Debug switches: `OVERLAI_UI_DEV=http://localhost:5173` (UI from Vite), `OVERLAI_DEBUG_SHOW=compact|expanded`
(open the panel on start), `OVERLAI_DEBUG_CAPTURABLE=1` (panel visible in screenshots),
`OVERLAI_LAN_ADDR=127.0.0.1` (phone server on this PC only: tests without the firewall prompt).
