// The overlay panel (loaded by the core's transparent window at /panel).
// The core sizes the window: compact = just the input row, expanded = header + history + input.

import { useCallback, useEffect, useRef, useState } from "react";
import { isBusy, type Connection } from "../api/connection";
import type { PanelMode } from "../api/generated/PanelMode";
import type { ResizeEdge } from "../api/generated/ResizeEdge";
import { useStoredFlag, useUiState } from "../api/hooks";
import { AgentNotice, ChatHeader } from "../components/ChatHeader";
import { Composer } from "../components/Composer";
import { OpenLinkContext } from "../components/Markdown";
import { MessageList } from "../components/MessageList";
import { ShotViewerProvider } from "../components/Shots";

export function Panel({ conn }: { conn: Connection }) {
  const ui = useUiState(conn);
  const [mode, setMode] = useState<PanelMode>("compact");
  const [draft, setDraft] = useState("");
  const [screenshot, setScreenshot] = useStoredFlag("overlai.screenshot", true);
  const inputRef = useRef<HTMLInputElement>(null);
  const busy = isBusy(ui);

  // Every time the core shows the panel: take its mode and focus the input.
  useEffect(() => {
    if (ui.overlay.showSeq === 0) return;
    setMode(ui.overlay.mode);
    const focus = () => inputRef.current?.focus();
    focus();
    requestAnimationFrame(focus);
  }, [ui.overlay.showSeq, ui.overlay.mode]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        conn.send({ type: "ui.hide" });
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [conn]);

  const expand = () => {
    conn.send({ type: "ui.setMode", mode: "expanded" }); // the core grows the window upward
    setMode("expanded");
  };

  const submit = () => {
    const text = draft.trim();
    if (!text || busy) return;
    const fromQuickField = mode === "compact";
    if (fromQuickField) expand();
    conn.send({ type: "chat.send", text, screenshot, newChat: fromQuickField });
    setDraft("");
  };

  const openLink = useCallback((url: string) => conn.send({ type: "ui.openUrl", url }), [conn]);
  // The core runs the native move/resize loop (kept inside the game window, remembered per game).
  const startDrag = useCallback(() => conn.send({ type: "ui.dragStart" }), [conn]);
  const startResize = useCallback((edge: ResizeEdge) => conn.send({ type: "ui.resizeStart", edge }), [conn]);

  const game = mode === "expanded" ? (ui.chatGame ?? ui.overlay.game) : ui.overlay.game;
  const agentDown = ui.agent?.state === "error";

  return (
    <OpenLinkContext.Provider value={openLink}>
      <ShotViewerProvider>
        <div className={`panel ${mode}`}>
          {mode === "expanded" && (
            <>
              <ChatHeader
                title={ui.title}
                usage={ui.usage}
                agent={ui.agent}
                onDragStart={startDrag}
                chats={ui.chats}
                chatId={ui.chatId}
                onNewChat={() => {
                  conn.send({ type: "chat.new" });
                  inputRef.current?.focus();
                }}
                onOpenChat={(id) => conn.send({ type: "chat.open", id })}
                onDeleteChat={(id) => conn.send({ type: "chat.delete", id })}
              />
              <MessageList
                messages={ui.messages}
                permissions={ui.permissions}
                onPermission={(id, optionId) => conn.send({ type: "permission.respond", id, optionId })}
                emptyHint="Новый чат. Спросите что-нибудь про игру."
              />
              <AgentNotice agent={ui.agent} />
            </>
          )}
          <Composer
            value={draft}
            onChange={setDraft}
            onSubmit={submit}
            busy={busy}
            onStop={() => conn.send({ type: "chat.cancel" })}
            onClose={() => conn.send({ type: "ui.hide" })}
            screenshot={screenshot}
            onScreenshotChange={setScreenshot}
            game={game}
            agentName={ui.agent?.agent ?? "Claude"}
            placeholder={
              mode === "compact"
                ? agentDown
                  ? `${ui.agent?.agent ?? "Агент"} недоступен — подробности в чате`
                  : "Быстрый вопрос — новый чат…"
                : "Сообщение…"
            }
            inputRef={inputRef}
            onDragStart={mode === "compact" ? startDrag : undefined}
          />
          <ResizeHandles edges={mode === "compact" ? COMPACT_EDGES : ALL_EDGES} onStart={startResize} />
          {ui.status !== "open" && <div className="offline" title="Нет связи с приложением" />}
        </div>
      </ShotViewerProvider>
    </OpenLinkContext.Provider>
  );
}

const ALL_EDGES: ResizeEdge[] = ["n", "s", "e", "w", "ne", "nw", "se", "sw"];
/** The compact panel only gets wider or narrower. */
const COMPACT_EDGES: ResizeEdge[] = ["e", "w"];

/** Invisible strips along the panel's edges and corners to resize it by. */
function ResizeHandles({ edges, onStart }: { edges: ResizeEdge[]; onStart(edge: ResizeEdge): void }) {
  return (
    <>
      {edges.map((edge) => (
        <div
          key={edge}
          className={`resize-handle ${edge}`}
          onPointerDown={(e) => {
            if (e.button !== 0) return;
            e.preventDefault();
            onStart(edge);
          }}
        />
      ))}
    </>
  );
}
