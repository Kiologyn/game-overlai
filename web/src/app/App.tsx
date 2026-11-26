// The chat in a regular browser tab (http://127.0.0.1:<port>/). Shares state with the overlay.
// `?game=<exe>` opens that game's page (profile + chats) instead of the chat.

import { useEffect, useMemo, useState } from "react";
import { Menu, MessageSquare, Plus, Search, Settings2, Trash2 } from "lucide-react";
import { isBusy, type Connection } from "../api/connection";
import { blankProfile, findProfile, gameLabel, sameExe } from "../api/games";
import { useUiState } from "../api/hooks";
import { AgentDot, AgentNotice, UsageBadge } from "../components/ChatHeader";
import { Composer } from "../components/Composer";
import { UpdateButton } from "../components/UpdateButton";
import { MessageList } from "../components/MessageList";
import { ShotViewerProvider } from "../components/Shots";
import { groupChats } from "../components/chatGroups";
import { GamePage } from "./GamePage";
import { useT } from "../i18n";

/** String codes of the connection states. */
const STATUS_TEXT = { open: "status_open", connecting: "status_connecting", closed: "status_closed" } as const;

const gameInUrl = () => new URLSearchParams(location.search).get("game");

export function App({ conn }: { conn: Connection }) {
  const ui = useUiState(conn);
  const t = useT();
  const [draft, setDraft] = useState("");
  const [query, setQuery] = useState("");
  /** Exe of the game whose page is open; null = the chat. */
  const [gameExe, setGameExe] = useState<string | null>(gameInUrl);
  /** Narrow screens (phone): the chat list slides in over the chat. */
  const [navOpen, setNavOpen] = useState(false);
  const busy = isBusy(ui);
  const noGame = t("no_game");
  const chatGroups = useMemo(() => groupChats(ui.chats, query, ui.games, noGame), [query, ui.chats, ui.games, noGame]);

  // The URL follows the page (reload keeps it); back/forward switch it.
  const showGame = (exe: string | null) => {
    setGameExe(exe);
    setNavOpen(false);
    const url = exe ? `?game=${encodeURIComponent(exe)}` : location.pathname;
    if (gameInUrl() !== exe) history.pushState(null, "", url);
  };
  useEffect(() => {
    const onPop = () => setGameExe(gameInUrl());
    window.addEventListener("popstate", onPop);
    return () => window.removeEventListener("popstate", onPop);
  }, []);

  const submit = () => {
    const text = draft.trim();
    if (!text || busy) return;
    conn.send({ type: "chat.send", text, newChat: false });
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
          <button
            type="button"
            className="icon-btn nav-toggle"
            onClick={() => setNavOpen((open) => !open)}
            aria-label={t("chat_list")}
            aria-expanded={navOpen}
          >
            <Menu size={18} />
          </button>
          <AgentDot agent={ui.agent} />
          <span className="brand">game-overlai</span>
          <span className="topbar-title">
            {gameProfile ? t("game_profile") : (ui.title ?? (ui.messages.length ? "" : t("new_chat")))}
          </span>
          <span className="topbar-right">
            <UsageBadge usage={ui.usage} />
            <span className={`status ${ui.status}`}>{t(STATUS_TEXT[ui.status])}</span>
            <UpdateButton update={ui.update} send={(msg) => conn.send(msg)} />
            <button
              type="button"
              className="icon-btn"
              onClick={() => {
                conn.send({ type: "chat.new" });
                showGame(null);
              }}
              title={t("new_chat")}
            >
              <Plus size={16} />
            </button>
          </span>
        </header>
        <div className="browser-workspace">
          {navOpen && <div className="nav-backdrop" onClick={() => setNavOpen(false)} />}
          <aside className={`history-sidebar${navOpen ? " open" : ""}`} aria-label={t("chat_history")}>
            <div className="history-heading">
              <span>{t("history")}</span>
            </div>
            <label className="history-search">
              <Search size={15} />
              <input value={query} onChange={(e) => setQuery(e.target.value)} placeholder={t("search_chats")} />
            </label>
            <div className="history-list">
              {chatGroups.length === 0 ? (
                <div className="history-empty">{ui.chats.length ? t("nothing_found") : t("chats_appear_here")}</div>
              ) : (
                chatGroups.map((group) => (
                  <section className="history-group" key={group.key}>
                    {group.exe ? (
                      <button
                        type="button"
                        className={`history-group-title${gameExe && sameExe(gameExe, group.exe) ? " active" : ""}`}
                        onClick={() => showGame(group.exe)}
                        title={t("game_profile_hint")}
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
                          title={t("delete_chat")}
                          aria-label={t("delete_chat_named", { title: chat.title })}
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
                widgets={ui.widgets.filter((w) => sameExe(w.exe, gameProfile.exe))}
                memoryOn={ui.settings ? ui.settings.values.progressMemory : null}
                globalHotkeys={
                  ui.settings
                    ? { quick: ui.settings.values.hotkeyQuick, chat: ui.settings.values.hotkeyChat, focus: ui.settings.values.hotkeyFocus }
                    : null
                }
                send={(msg) => conn.send(msg)}
                onSave={(p) =>
                  conn.send({
                    type: "game.update",
                    exe: p.exe,
                    name: p.name,
                    instructions: p.instructions,
                    sources: p.sources,
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
                emptyHint={t("browser_empty_hint")}
              />
              <AgentNotice agent={ui.agent} />
              <div className="page-composer">
                <Composer
                  value={draft}
                  onChange={setDraft}
                  onSubmit={submit}
                  busy={busy}
                  onStop={() => conn.send({ type: "chat.cancel" })}
                  game={ui.chatGame}
                  gameName={gameLabel(ui.chatGame, ui.games)}
                  onOpenGame={ui.chatGame?.exe ? () => showGame(ui.chatGame?.exe ?? null) : undefined}
                  agentName={ui.agent?.agent ?? "Claude"}
                  agentOptions={ui.agentOptions}
                  onSetAgentOption={(id, value) => conn.send({ type: "agent.setOption", id, value })}
                  placeholder={t("message_placeholder")}
                />
              </div>
            </main>
          )}
        </div>
      </div>
    </ShotViewerProvider>
  );
}
