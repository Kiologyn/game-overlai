// The chat in a regular browser tab (http://127.0.0.1:<port>/). Shares state with the overlay.

import { useMemo, useState } from "react";
import { MessageSquare, Plus, Search, Trash2 } from "lucide-react";
import { isBusy, type Connection } from "../api/connection";
import { useStoredFlag, useUiState } from "../api/hooks";
import { AgentDot, AgentNotice, UsageBadge } from "../components/ChatHeader";
import { Composer } from "../components/Composer";
import { MessageList } from "../components/MessageList";
import { ShotViewerProvider } from "../components/Shots";
import { groupChats } from "../components/chatGroups";

const STATUS_TEXT = { open: "подключено", connecting: "подключение…", closed: "нет связи" } as const;

export function App({ conn }: { conn: Connection }) {
  const ui = useUiState(conn);
  const [draft, setDraft] = useState("");
  const [query, setQuery] = useState("");
  const [screenshot, setScreenshot] = useStoredFlag("overlai.screenshot", true);
  const busy = isBusy(ui);
  const chatGroups = useMemo(() => groupChats(ui.chats, query), [query, ui.chats]);

  const submit = () => {
    const text = draft.trim();
    if (!text || busy) return;
    conn.send({ type: "chat.send", text, screenshot, newChat: false });
    setDraft("");
  };

  return (
    <ShotViewerProvider>
      <div className="page">
        <header className="topbar">
          <AgentDot agent={ui.agent} />
          <span className="brand">game-overlai</span>
          <span className="topbar-title">{ui.title ?? (ui.messages.length ? "" : "Новый чат")}</span>
          <span className="topbar-right">
            <UsageBadge usage={ui.usage} />
            <span className={`status ${ui.status}`}>{STATUS_TEXT[ui.status]}</span>
            <button type="button" className="icon-btn" onClick={() => conn.send({ type: "chat.new" })} title="Новый чат">
              <Plus size={16} />
            </button>
          </span>
        </header>
        <div className="browser-workspace">
          <aside className="history-sidebar" aria-label="История чатов">
            <div className="history-heading">
              <span>История</span>
              <button type="button" className="icon-btn" onClick={() => conn.send({ type: "chat.new" })} title="Новый чат">
                <Plus size={16} />
              </button>
            </div>
            <label className="history-search">
              <Search size={15} />
              <input value={query} onChange={(e) => setQuery(e.target.value)} placeholder="Поиск по чатам" />
            </label>
            <div className="history-list">
              {chatGroups.length === 0 ? (
                <div className="history-empty">{ui.chats.length ? "Ничего не найдено" : "Чаты появятся здесь"}</div>
              ) : chatGroups.map(([game, chats]) => (
                <section className="history-group" key={game}>
                  <h2>{game}</h2>
                  {chats.map((chat) => (
                    <div key={chat.id} className={`history-item${ui.chatId === chat.id ? " active" : ""}`}>
                      <button type="button" className="history-open" aria-pressed={ui.chatId === chat.id} onClick={() => conn.send({ type: "chat.open", id: chat.id })}>
                        <MessageSquare size={15} />
                        <span className="history-item-text"><span className="history-item-title">{chat.title}</span></span>
                      </button>
                      <button type="button" className="history-delete" title="Удалить чат" aria-label={`Удалить чат ${chat.title}`} onClick={() => conn.send({ type: "chat.delete", id: chat.id })}>
                        <Trash2 size={14} />
                      </button>
                    </div>
                  ))}
                </section>
              ))}
            </div>
          </aside>
        <main className="page-chat">
          <MessageList
            messages={ui.messages}
            permissions={ui.permissions}
            onPermission={(id, optionId) => conn.send({ type: "permission.respond", id, optionId })}
            emptyHint="Здесь появится текущий чат из оверлея."
          />
          <AgentNotice agent={ui.agent} />
          <div className="page-composer">
            <Composer
              value={draft}
              onChange={setDraft}
              onSubmit={submit}
              busy={busy}
              onStop={() => conn.send({ type: "chat.cancel" })}
              screenshot={screenshot}
              onScreenshotChange={setScreenshot}
              game={ui.chatGame}
              agentName={ui.agent?.agent ?? "Claude"}
              placeholder="Сообщение…"
            />
          </div>
        </main>
        </div>
      </div>
    </ShotViewerProvider>
  );
}
