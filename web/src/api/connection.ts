// WebSocket connection to the core + the UI state derived from server messages.
// Same code for the overlay panel (?client=overlay) and browser tabs (?client=browser).

import type { AgentStatus } from "./generated/AgentStatus";
import type { AppendKind } from "./generated/AppendKind";
import type { ChatMessage } from "./generated/ChatMessage";
import type { ChatSummary } from "./generated/ChatSummary";
import type { ClientKind } from "./generated/ClientKind";
import type { ClientMsg } from "./generated/ClientMsg";
import type { GameInfo } from "./generated/GameInfo";
import type { GameProfile } from "./generated/GameProfile";
import type { LanStatus } from "./generated/LanStatus";
import { setLanguage } from "../i18n";
import type { AgentOption } from "./generated/AgentOption";
import type { Settings } from "./generated/Settings";
import { sameExe } from "./games";
import type { PanelMode } from "./generated/PanelMode";
import type { PermissionPrompt } from "./generated/PermissionPrompt";
import type { ServerMsg } from "./generated/ServerMsg";
import type { ToolInfo } from "./generated/ToolInfo";
import type { Usage } from "./generated/Usage";

export type Status = "connecting" | "open" | "closed";

export interface UiState {
  status: Status;
  messages: ChatMessage[];
  chatId: string | null;
  chats: ChatSummary[];
  /** Game profiles, most recently played first. */
  games: GameProfile[];
  /** Game the current chat belongs to. */
  chatGame: GameInfo | null;
  /** Chat title given by the agent. */
  title: string | null;
  permissions: PermissionPrompt[];
  agent: AgentStatus | null;
  usage: Usage | null;
  /** The agent's model / effort choices (empty: it offers none). */
  agentOptions: AgentOption[];
  /** Phone access over the local network; only this PC's UI gets it. */
  lan: LanStatus | null;
  /** Settings page state; only this PC's UI gets it. */
  settings: Settings | null;
  overlay: {
    visible: boolean;
    mode: PanelMode;
    game: GameInfo | null;
    /** Increments on every overlay.show, so the panel can react even if nothing else changed. */
    showSeq: number;
    /** The panel's zoom (1 = 100%), and a counter of overlay.zoom messages (shows the badge). */
    zoom: number;
    zoomSeq: number;
  };
}

export interface Connection {
  getState(): UiState;
  subscribe(listener: () => void): () => void;
  send(msg: ClientMsg): void;
}

const initial: UiState = {
  status: "connecting",
  messages: [],
  chatId: null,
  chats: [],
  games: [],
  chatGame: null,
  title: null,
  permissions: [],
  agent: null,
  usage: null,
  agentOptions: [],
  lan: null,
  settings: null,
  overlay: { visible: false, mode: "compact", game: null, showSeq: 0, zoom: 1, zoomSeq: 0 },
};

export const isBusy = (s: UiState) => s.messages.some((m) => m.status === "streaming");

// The two rules below mirror ChatMessage::append / upsert_tool in src-tauri/src/protocol.rs
// (the core sends tools already merged, so here a tool simply replaces the one with its id).
function append(m: ChatMessage, kind: AppendKind, text: string): ChatMessage {
  const parts = [...m.parts];
  const last = parts[parts.length - 1];
  if (last && (last.type === "text" || last.type === "thought") && last.type === kind) {
    parts[parts.length - 1] = { ...last, text: last.text + text };
  } else {
    parts.push(kind === "text" ? { type: "text", text } : { type: "thought", text });
  }
  return { ...m, parts };
}

function upsertTool(m: ChatMessage, tool: ToolInfo): ChatMessage {
  const i = m.parts.findIndex((p) => p.type === "tool" && p.tool.id === tool.id);
  const parts = [...m.parts];
  if (i >= 0) parts[i] = { type: "tool", tool };
  else parts.push({ type: "tool", tool });
  return { ...m, parts };
}

function updateMessage(s: UiState, id: string, f: (m: ChatMessage) => ChatMessage): UiState {
  return { ...s, messages: s.messages.map((m) => (m.id === id ? f(m) : m)) };
}

function reduce(s: UiState, msg: ServerMsg): UiState {
  switch (msg.type) {
    case "hello":
      return s;
    case "language":
      setLanguage(msg.language); // components follow it through useT
      return s;
    case "chat.snapshot":
      return { ...s, chatId: msg.chatId, messages: msg.messages, chatGame: msg.game, title: msg.title };
    case "chat.message": {
      // A new message, or a new version of one we have (e.g. its game frame is ready).
      const { message } = msg;
      const known = s.messages.some((m) => m.id === message.id);
      const messages = known ? s.messages.map((m) => (m.id === message.id ? message : m)) : [...s.messages, message];
      return { ...s, messages };
    }
    case "chat.append":
      return updateMessage(s, msg.messageId, (m) => append(m, msg.kind, msg.text));
    case "chat.tool":
      return updateMessage(s, msg.messageId, (m) => upsertTool(m, msg.tool));
    case "chat.status":
      return updateMessage(s, msg.messageId, (m) => ({ ...m, status: msg.status, error: msg.error }));
    case "chat.title":
      return { ...s, title: msg.title };
    case "chats.snapshot":
      return { ...s, chats: msg.chats };
    case "chats.upsert": {
      const chats = [msg.chat, ...s.chats.filter((chat) => chat.id !== msg.chat.id)];
      chats.sort((a, b) => b.updatedAt - a.updatedAt);
      return { ...s, chats };
    }
    case "chats.remove":
      return { ...s, chats: s.chats.filter((chat) => chat.id !== msg.id) };
    case "games.snapshot":
      return { ...s, games: msg.games };
    case "games.upsert": {
      const known = s.games.some((g) => sameExe(g.exe, msg.game.exe));
      const games = known ? s.games.map((g) => (sameExe(g.exe, msg.game.exe) ? msg.game : g)) : [msg.game, ...s.games];
      return { ...s, games };
    }
    case "permissions":
      return { ...s, permissions: msg.prompts };
    case "agent.status":
      return { ...s, agent: msg.status };
    case "agent.usage":
      return { ...s, usage: msg.usage };
    case "agent.options":
      return { ...s, agentOptions: msg.options };
    case "overlay.show":
      return {
        ...s,
        overlay: { ...s.overlay, visible: true, mode: msg.mode, game: msg.game, showSeq: s.overlay.showSeq + 1 },
      };
    case "lan.status":
      return { ...s, lan: msg.status };
    case "settings":
      return { ...s, settings: msg.settings };
    case "overlay.zoom":
      return { ...s, overlay: { ...s.overlay, zoom: msg.zoom, zoomSeq: s.overlay.zoomSeq + 1 } };
    case "overlay.hidden":
      return { ...s, overlay: { ...s.overlay, visible: false } };
  }
}

export function connect(client: ClientKind): Connection {
  let state = initial;
  const listeners = new Set<() => void>();
  const outbox: ClientMsg[] = [];
  let ws: WebSocket | null = null;
  let retryMs = 300;

  const set = (next: UiState) => {
    state = next;
    listeners.forEach((l) => l());
  };

  const open = () => {
    const proto = location.protocol === "https:" ? "wss" : "ws";
    const socket = new WebSocket(`${proto}://${location.host}/ws?client=${client}`);
    ws = socket;
    set({ ...state, status: "connecting" });

    socket.onopen = () => {
      retryMs = 300;
      set({ ...state, status: "open" });
      while (outbox.length) socket.send(JSON.stringify(outbox.shift()));
    };
    socket.onmessage = (e) => {
      try {
        set(reduce(state, JSON.parse(e.data) as ServerMsg));
      } catch (err) {
        console.warn("bad server message", err);
      }
    };
    socket.onclose = () => {
      ws = null;
      set({ ...state, status: "closed" });
      setTimeout(open, retryMs);
      retryMs = Math.min(retryMs * 2, 5000);
    };
  };
  open();

  return {
    getState: () => state,
    subscribe(listener) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    send(msg) {
      if (ws?.readyState === WebSocket.OPEN) ws.send(JSON.stringify(msg));
      else if (outbox.length < 50) outbox.push(msg); // delivered after reconnect
    },
  };
}
