import { useState } from "react";
import { History, MessageSquare, Plus, Trash2 } from "lucide-react";
import type { ChatSummary } from "../api/generated/ChatSummary";
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
  onNewChat(): void;
  onOpenChat(id: string): void;
  onDeleteChat(id: string): void;
}

export function ChatHeader({ title, usage, agent, chats, chatId, onNewChat, onOpenChat, onDeleteChat }: HeaderProps) {
  const [historyOpen, setHistoryOpen] = useState(false);
  const groups = new Map<string, ChatSummary[]>();
  for (const chat of chats) {
    const game = chat.game?.title ?? "Без игры";
    groups.set(game, [...(groups.get(game) ?? []), chat]);
  }

  return (
    <div className="chat-header">
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
          <div className="overlay-history-menu" role="region" aria-label="История чатов по играм">
            {chats.length === 0 ? (
              <div className="history-empty">Чаты появятся здесь</div>
            ) : (
              [...groups.entries()].map(([game, gameChats]) => (
                <section className="history-group" key={game}>
                  <h2>{game}</h2>
                  {gameChats.map((chat) => (
                    <div key={chat.id} className={`history-item${chatId === chat.id ? " active" : ""}`}>
                      <button
                        type="button"
                        className="history-open"
                        aria-pressed={chatId === chat.id}
                        onClick={() => { onOpenChat(chat.id); setHistoryOpen(false); }}
                      >
                        <MessageSquare size={15} />
                        <span className="history-item-text"><span className="history-item-title">{chat.title}</span></span>
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
        )}
      </div>
      <button type="button" className="icon-btn" onClick={onNewChat} title="Новый чат">
        <Plus size={16} />
      </button>
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
