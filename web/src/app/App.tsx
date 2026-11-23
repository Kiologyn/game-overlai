// The chat in a regular browser tab (http://127.0.0.1:<port>/). Shares state with the overlay.
// `?game=<exe>` opens that game's page (profile + chats) instead of the chat.

import { useEffect, useMemo, useState } from "react";
import { MessageSquare, Plus, Search, Settings2, Trash2 } from "lucide-react";
import { isBusy, type Connection } from "../api/connection";
import { blankProfile, findProfile, gameLabel, sameExe } from "../api/games";
import { useStoredFlag, useUiState } from "../api/hooks";
import { AgentDot, AgentNotice, UsageBadge } from "../components/ChatHeader";
import { Composer } from "../components/Composer";
import { MessageList } from "../components/MessageList";
import { ShotViewerProvider } from "../components/Shots";
import { groupChats } from "../components/chatGroups";
import { GamePage } from "./GamePage";

const STATUS_TEXT = { open: "подключено", connecting: "подключение…", closed: "нет связи" } as const;

const gameInUrl = () => new URLSearchParams(location.search).get("game");

export function App({ conn }: { conn: Connection }) {
  const ui = useUiState(conn);
  const [draft, setDraft] = useState("");
  const [query, setQuery] = useState("");
  const [storedShot, setStoredShot] = useStoredFlag("overlai.screenshot", true);
  const [shotOverride, setShotOverride] = useState<boolean | null>(null);
  /** Exe of the game whose page is open; null = the chat. */
  const [gameExe, setGameExe] = useState<string | null>(gameInUrl);
  const busy = isBusy(ui);
  const chatGroups = useMemo(() => groupChats(ui.chats, query, ui.games), [query, ui.chats, ui.games]);

  // The URL follows the page (reload keeps it); back/forward switch it.
  const showGame = (exe: string | null) => {
    setGameExe(exe);
    const url = exe ? `?game=${encodeURIComponent(exe)}` : location.pathname;
    if (gameInUrl() !== exe) history.pushState(null, "", url);
  };
  useEffect(() => {
    const onPop = () => setGameExe(gameInUrl());
    window.addEventListener("popstate", onPop);
    return () => window.removeEventListener("popstate", onPop);
  }, []);

  // 📷 as in the panel: the game's profile may set it, a toggle overrides it for this chat.
  useEffect(() => setShotOverride(null), [ui.chatId]);
  const profileShot = findProfile(ui.games, ui.chatGame?.exe)?.screenshot ?? null;
  const screenshot = shotOverride ?? profileShot ?? storedShot;
  const setScreenshot = (on: boolean) => (profileShot === null ? setStoredShot(on) : setShotOverride(on));

  const submit = () => {
    const text = draft.trim();
    if (!text || busy) return;
    conn.send({ type: "chat.send", text, screenshot, newChat: false });
    setDraft("");
  };

  const openChat = (id: string) => {
    conn.send({ type: "chat.open", id });
    showGame(null);
  };

  const gameProfile = gameExe ? (findProfile(ui.games, gameExe) ?? blankProfile(gameExe)) : null;
  const gameChats = gameExe ? ui.chats.filter((c) => c.game?.exe && sameExe(c.game.exe, gameExe)) : [];

  return (
    <ShotViewerProvider>
      <div className="page">
        <header className="topbar">
          <AgentDot agent={ui.agent} />
          <span className="brand">game-overlai</span>
          <span className="topbar-title">
            {gameProfile ? "Профиль игры" : (ui.title ?? (ui.messages.length ? "" : "Новый чат"))}
          </span>
          <span className="topbar-right">
            <UsageBadge usage={ui.usage} />
            <span className={`status ${ui.status}`}>{STATUS_TEXT[ui.status]}</span>
            <button
              type="button"
              className="icon-btn"
              onClick={() => {
                conn.send({ type: "chat.new" });
                showGame(null);
              }}
              title="Новый чат"
            >
              <Plus size={16} />
            </button>
          </span>
        </header>
        <div className="browser-workspace">
          <aside className="history-sidebar" aria-label="История чатов">
            <div className="history-heading">
              <span>История</span>
            </div>
            <label className="history-search">
              <Search size={15} />
              <input value={query} onChange={(e) => setQuery(e.target.value)} placeholder="Поиск по чатам" />
            </label>
            <div className="history-list">
              {chatGroups.length === 0 ? (
                <div className="history-empty">{ui.chats.length ? "Ничего не найдено" : "Чаты появятся здесь"}</div>
              ) : (
                chatGroups.map((group) => (
                  <section className="history-group" key={group.key}>
                    {group.exe ? (
                      <button
                        type="button"
                        className={`history-group-title${gameExe && sameExe(gameExe, group.exe) ? " active" : ""}`}
                        onClick={() => showGame(group.exe)}
                        title="Профиль игры: название, указания агенту, где искать"
                      >
                        <span>{group.label}</span>
                        <Settings2 size={13} aria-hidden />
                      </button>
                    ) : (
                      <h2>{group.label}</h2>
                    )}
                    {group.chats.map((chat) => (
                      <div key={chat.id} className={`history-item${!gameExe && ui.chatId === chat.id ? " active" : ""}`}>
                        <button
                          type="button"
                          className="history-open"
                          aria-pressed={ui.chatId === chat.id}
                          onClick={() => openChat(chat.id)}
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
                          onClick={() => conn.send({ type: "chat.delete", id: chat.id })}
                        >
                          <Trash2 size={14} />
                        </button>
                      </div>
                    ))}
                  </section>
                ))
              )}
            </div>
          </aside>
          {gameProfile ? (
            <main className="page-game">
              <GamePage
                key={gameProfile.exe.toLowerCase()}
                profile={gameProfile}
                chats={gameChats}
                chatId={ui.chatId}
                onSave={(p) =>
                  conn.send({
                    type: "game.update",
                    exe: p.exe,
                    name: p.name,
                    instructions: p.instructions,
                    sources: p.sources,
                    screenshot: p.screenshot,
                  })
                }
                onOpenChat={openChat}
                onBack={() => showGame(null)}
              />
            </main>
          ) : (
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
                  gameName={gameLabel(ui.chatGame, ui.games)}
                  onOpenGame={ui.chatGame?.exe ? () => showGame(ui.chatGame?.exe ?? null) : undefined}
                  agentName={ui.agent?.agent ?? "Claude"}
                  placeholder="Сообщение…"
                />
              </div>
            </main>
          )}
        </div>
      </div>
    </ShotViewerProvider>
  );
}
