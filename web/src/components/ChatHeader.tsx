import { useEffect, useMemo, useRef, useState } from "react";
import { History, MessageSquare, Plus, Search, Trash2 } from "lucide-react";
import { groupChats } from "./chatGroups";
import type { ChatSummary } from "../api/generated/ChatSummary";
import type { GameProfile } from "../api/generated/GameProfile";
import type { AgentStatus } from "../api/generated/AgentStatus";
import type { Usage } from "../api/generated/Usage";
import type { ClientMsg } from "../api/generated/ClientMsg";
import type { UpdateStatus } from "../api/generated/UpdateStatus";
import { UpdateButton } from "./UpdateButton";
import { htmlLang, useT, type T } from "../i18n";

function formatReset(unixSeconds: number | null, t: T): string {
  if (!unixSeconds) return "";
  const time = new Date(unixSeconds * 1000).toLocaleTimeString(htmlLang(), { hour: "2-digit", minute: "2-digit" });
  return ` · ${t("usage_resets_at", { time })}`;
}

/** Subscription usage (5-hour window), with the weekly one in the tooltip. */
export function UsageBadge({ usage }: { usage: Usage | null }) {
  const t = useT();
  if (!usage || usage.fiveHour === null) return null;
  const five = Math.round(usage.fiveHour * 100);
  const week = usage.sevenDay === null ? null : Math.round(usage.sevenDay * 100);
  const level = five >= 95 ? "bad" : five >= 80 ? "warn" : "";
  return (
    <span
      className={`usage ${level}`}
      title={`${t("usage_five_hours", { percent: five })}${formatReset(usage.fiveHourResetsAt, t)}${week === null ? "" : `; ${t("usage_week", { percent: week })}`}`}
    >
      <span className="usage-bar">
        <span style={{ width: `${Math.min(100, five)}%` }} />
      </span>
      {five}%
    </span>
  );
}

/** String codes of the agent states. */
const AGENT_TEXT: Record<AgentStatus["state"], string> = {
  ready: "agent_ready",
  starting: "agent_starting",
  installing: "agent_installing",
  error: "agent_error",
};

export function AgentDot({ agent }: { agent: AgentStatus | null }) {
  const t = useT();
  if (!agent) return null;
  return (
    <span
      className={`agent-dot ${agent.state}`}
      title={`${t("agent_state_line", { agent: agent.agent, state: t(AGENT_TEXT[agent.state]) })}${agent.detail ? ` — ${agent.detail}` : ""}`}
    />
  );
}

interface HeaderProps {
  title: string | null;
  usage: Usage | null;
  agent: AgentStatus | null;
  chats: ChatSummary[];
  chatId: string | null;
  games: GameProfile[];
  onNewChat(): void;
  onOpenChat(id: string): void;
  onDeleteChat(id: string): void;
  /** The mouse went down on the header (not on a control): move the panel. */
  onDragStart?: () => void;
  /** New versions (the button shows only while one is found). */
  update: UpdateStatus | null;
  send(msg: ClientMsg): void;
}

export function ChatHeader(props: HeaderProps) {
  const { title, usage, agent, chats, chatId, games, onNewChat, onOpenChat, onDeleteChat, onDragStart, update, send } = props;
  const t = useT();
  const [historyOpen, setHistoryOpen] = useState(false);
  return (
    <div
      className={`chat-header${onDragStart ? " draggable" : ""}`}
      onPointerDown={(e) => {
        if (!onDragStart || e.button !== 0) return;
        if ((e.target as HTMLElement).closest("button, input, a, .overlay-history-menu, .update-card")) return;
        e.preventDefault();
        onDragStart();
      }}
    >
      <AgentDot agent={agent} />
      <span className="chat-title">{title ?? t("new_chat")}</span>
      <UsageBadge usage={usage} />
      <UpdateButton update={update} send={send} />
      <div className="overlay-history">
        <button
          type="button"
          className={`icon-btn${historyOpen ? " selected" : ""}`}
          onClick={() => setHistoryOpen((open) => !open)}
          title={t("chat_history")}
          aria-label={t("chat_history")}
          aria-expanded={historyOpen}
        >
          <History size={16} />
        </button>
        {historyOpen && (
          <HistoryMenu
            chats={chats}
            chatId={chatId}
            games={games}
            onOpenChat={(id) => {
              onOpenChat(id);
              setHistoryOpen(false);
            }}
            onDeleteChat={onDeleteChat}
            onClose={() => setHistoryOpen(false)}
          />
        )}
      </div>
      <button type="button" className="icon-btn" onClick={onNewChat} title={t("new_chat")}>
        <Plus size={16} />
      </button>
    </div>
  );
}

interface MenuProps {
  chats: ChatSummary[];
  games: GameProfile[];
  chatId: string | null;
  onOpenChat(id: string): void;
  onDeleteChat(id: string): void;
  onClose(): void;
}

/** History dropdown of the overlay panel: search, chats grouped by game. */
function HistoryMenu({ chats, games, chatId, onOpenChat, onDeleteChat, onClose }: MenuProps) {
  const t = useT();
  const [query, setQuery] = useState("");
  const ref = useRef<HTMLDivElement>(null);
  const noGame = t("no_game");
  const groups = useMemo(() => groupChats(chats, query, games, noGame), [chats, query, games, noGame]);

  useEffect(() => {
    // Esc closes the menu, not the panel (capture phase: the panel hides on Esc itself).
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        onClose();
      }
    };
    // A click anywhere outside the menu (its toggle button included) closes it.
    const onDown = (e: PointerEvent) => {
      const menu = ref.current;
      if (menu && !menu.parentElement?.contains(e.target as Node)) onClose();
    };
    window.addEventListener("keydown", onKey, true);
    window.addEventListener("pointerdown", onDown, true);
    return () => {
      window.removeEventListener("keydown", onKey, true);
      window.removeEventListener("pointerdown", onDown, true);
    };
  }, [onClose]);

  return (
    <div className="overlay-history-menu" ref={ref} role="region" aria-label={t("chat_history_by_game")}>
      <label className="history-search overlay">
        <Search size={14} aria-hidden />
        <input autoFocus value={query} onChange={(e) => setQuery(e.target.value)} placeholder={t("search_chats")} />
      </label>
      {groups.length === 0 ? (
        <div className="history-empty">{chats.length ? t("nothing_found") : t("chats_appear_here")}</div>
      ) : (
        groups.map((group) => (
          <section className="history-group" key={group.key}>
            <h2>{group.label}</h2>
            {group.chats.map((chat) => (
              <div key={chat.id} className={`history-item${chatId === chat.id ? " active" : ""}`}>
                <button
                  type="button"
                  className="history-open"
                  aria-pressed={chatId === chat.id}
                  onClick={() => onOpenChat(chat.id)}
                >
                  <MessageSquare size={15} />
                  <span className="history-item-text">
                    <span className="history-item-title">{chat.title}</span>
                  </span>
                </button>
                <button
                  type="button"
                  className="history-delete"
                  title={t("delete_chat")}
                  aria-label={t("delete_chat_named", { title: chat.title })}
                  onClick={() => onDeleteChat(chat.id)}
                >
                  <Trash2 size={14} />
                </button>
              </div>
            ))}
          </section>
        ))
      )}
    </div>
  );
}

/** Shown above the input when the agent can't answer right now. */
export function AgentNotice({ agent }: { agent: AgentStatus | null }) {
  const t = useT();
  if (!agent || agent.state === "ready") return null;
  if (agent.state === "error") {
    return <div className="agent-notice bad">{agent.detail ?? t("agent_unavailable", { agent: agent.agent })}</div>;
  }
  const doing = agent.state === "installing" ? "agent_is_installing" : "agent_is_starting";
  return <div className="agent-notice">{agent.detail ?? t(doing, { agent: agent.agent })}</div>;
}
