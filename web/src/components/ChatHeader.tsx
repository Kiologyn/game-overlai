import { useEffect, useMemo, useRef, useState } from "react";
import { History, MessageSquare, Plus, Search, Trash2 } from "lucide-react";
import { groupChats } from "./chatGroups";
import type { ChatSummary } from "../api/generated/ChatSummary";
import type { GameProfile } from "../api/generated/GameProfile";
import type { AgentStatus } from "../api/generated/AgentStatus";
import type { Usage } from "../api/generated/Usage";

function formatReset(unixSeconds: number | null): string {
  if (!unixSeconds) return "";
  const d = new Date(unixSeconds * 1000);
  return ` · сброс в ${d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}`;
}

/** Subscription usage (5-hour window), with the weekly one in the tooltip. */
export function UsageBadge({ usage }: { usage: Usage | null }) {
  if (!usage || usage.fiveHour === null) return null;
  const five = Math.round(usage.fiveHour * 100);
  const week = usage.sevenDay === null ? null : Math.round(usage.sevenDay * 100);
  const level = five >= 95 ? "bad" : five >= 80 ? "warn" : "";
  return (
    <span
      className={`usage ${level}`}
      title={`Лимит подписки: 5 часов — ${five}%${formatReset(usage.fiveHourResetsAt)}${week === null ? "" : `; неделя — ${week}%`}`}
    >
      <span className="usage-bar">
        <span style={{ width: `${Math.min(100, five)}%` }} />
      </span>
      {five}%
    </span>
  );
}

const AGENT_TEXT: Record<AgentStatus["state"], string> = {
  ready: "готов",
  starting: "запускается…",
  installing: "устанавливается…",
  error: "недоступен",
};

export function AgentDot({ agent }: { agent: AgentStatus | null }) {
  if (!agent) return null;
  return (
    <span
      className={`agent-dot ${agent.state}`}
      title={`${agent.agent}: ${AGENT_TEXT[agent.state]}${agent.detail ? ` — ${agent.detail}` : ""}`}
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
}

export function ChatHeader(props: HeaderProps) {
  const { title, usage, agent, chats, chatId, games, onNewChat, onOpenChat, onDeleteChat, onDragStart } = props;
  const [historyOpen, setHistoryOpen] = useState(false);
  return (
    <div
      className={`chat-header${onDragStart ? " draggable" : ""}`}
      onPointerDown={(e) => {
        if (!onDragStart || e.button !== 0) return;
        if ((e.target as HTMLElement).closest("button, input, a, .overlay-history-menu")) return;
        e.preventDefault();
        onDragStart();
      }}
    >
      <AgentDot agent={agent} />
      <span className="chat-title">{title ?? "Новый чат"}</span>
      <UsageBadge usage={usage} />
      <div className="overlay-history">
        <button
          type="button"
          className={`icon-btn${historyOpen ? " selected" : ""}`}
          onClick={() => setHistoryOpen((open) => !open)}
          title="История чатов"
          aria-label="История чатов"
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
      <button type="button" className="icon-btn" onClick={onNewChat} title="Новый чат">
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
  const [query, setQuery] = useState("");
  const ref = useRef<HTMLDivElement>(null);
  const groups = useMemo(() => groupChats(chats, query, games), [chats, query, games]);

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
    <div className="overlay-history-menu" ref={ref} role="region" aria-label="История чатов по играм">
      <label className="history-search overlay">
        <Search size={14} aria-hidden />
        <input autoFocus value={query} onChange={(e) => setQuery(e.target.value)} placeholder="Поиск по чатам" />
      </label>
      {groups.length === 0 ? (
        <div className="history-empty">{chats.length ? "Ничего не найдено" : "Чаты появятся здесь"}</div>
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
                  title="Удалить чат"
                  aria-label={`Удалить чат ${chat.title}`}
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
  if (!agent || agent.state === "ready") return null;
  if (agent.state === "error") {
    return <div className="agent-notice bad">{agent.detail ?? `${agent.agent} недоступен`}</div>;
  }
  return (
    <div className="agent-notice">
      {agent.detail ?? `${agent.agent} ${AGENT_TEXT[agent.state]}`}
    </div>
  );
}
