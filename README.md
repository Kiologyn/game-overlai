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
scripts\windows\build.bat    # portable release exe -> release\<version>\
```

By hand: `cd web; npm install; npm run build`, then `cd src-tauri; cargo run`. On first start the app
installs the pinned ACP adapter of the agent into `%LOCALAPPDATA%\game-overlai\agents\`.

Using it: `Alt+Q` — quick question (starts a new chat), `Alt+W` — chat panel (the game's latest chat),
`Esc` / ✕ — hide. 📷 in the input field attaches a frame of the game. The tray icon has the rest;
the chat in a browser is at http://127.0.0.1:47823/.

## Layout

| Path | What |
|---|---|
| `src-tauri/src/main.rs`, `state.rs` | startup, shared app state |
| `src-tauri/src/overlay/` | the panel window: placement over the game, focus, capture exclusion (Win32) |
| `src-tauri/src/agent/` | agents over ACP: adapter install, supervision, sessions, permissions; `spec.rs` = per-agent settings |
| `src-tauri/src/chat.rs`, `history.rs` | current chat ↔ agent session; chat history in SQLite, frames on disk |
| `src-tauri/src/capture/`, `shots.rs` | game frames: Windows Graphics Capture, JPEG encoding, storage |
| `src-tauri/src/mcp.rs` | MCP server for the agent (`/mcp`, bearer token): `screenshot_game` |
| `src-tauri/src/server/` | local HTTP + WebSocket server the UI talks to |
| `src-tauri/src/protocol.rs` | WebSocket protocol; TS types are generated from it |
| `src-tauri/src/proxy.rs`, `config.rs`, `tray.rs`, `hotkeys.rs` | network proxy, settings, tray, global hotkeys |
| `web/` | UI (Vite + React + TS): `/panel` = overlay panel, `/` = browser chat |
| `spikes/` | phase 0 prototypes (ACP, capture, overlay), kept for reference |

## Conventions

- After changing `protocol.rs`, run `cargo test` in `src-tauri/`: it regenerates
  `web/src/api/generated/` (don't edit those files).
- Before committing: `cargo test`, `cargo clippy` (no warnings) in `src-tauri/`, `npm run build` in `web/`.
- No DLL injection or hooks into game processes (anti-cheat): the overlay is a separate topmost window,
  frames come from Windows Graphics Capture.
- UI: neutral dark theme, no blue. UI text is in Russian.

## Runtime files

- Settings: `%APPDATA%\game-overlai\config.toml` (created on first run; missing keys = defaults).
  `[network] proxy = "system"` takes the Windows proxy (incl. PAC), else HTTP(S)_PROXY, else direct;
  `require_proxy_for = ["claude"]` refuses to start that agent without a proxy.
- History: `%LOCALAPPDATA%\game-overlai\history.db` + `shots\` (frames, deleted with their chat).
- Logs: `%LOCALAPPDATA%\game-overlai\logs\` (`OVERLAI_LOG=debug` for more).

Debug switches: `OVERLAI_UI_DEV=http://localhost:5173` (UI from Vite), `OVERLAI_DEBUG_SHOW=compact|expanded`
(open the panel on start), `OVERLAI_DEBUG_CAPTURABLE=1` (panel visible in screenshots).
