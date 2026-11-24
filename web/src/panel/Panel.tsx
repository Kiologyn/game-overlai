// The overlay panel (loaded by the core's transparent window at /panel).
// The core sizes the window: compact = just the input row, expanded = header + history + input.

import { useCallback, useEffect, useRef, useState } from "react";
import { isBusy, type Connection } from "../api/connection";
import type { PanelMode } from "../api/generated/PanelMode";
import type { ResizeEdge } from "../api/generated/ResizeEdge";
import type { ZoomStep } from "../api/generated/ZoomStep";
import { gameLabel, gamePageUrl } from "../api/games";
import { useUiState } from "../api/hooks";
import { AgentNotice, ChatHeader } from "../components/ChatHeader";
import { Composer } from "../components/Composer";
import { OpenLinkContext } from "../components/Markdown";
import { MessageList } from "../components/MessageList";
import { ShotViewerProvider } from "../components/Shots";
import { PageTooltip } from "./PageTooltip";
import { useT } from "../i18n";

export function Panel({ conn }: { conn: Connection }) {
  const ui = useUiState(conn);
  const t = useT();
  const [mode, setMode] = useState<PanelMode>("compact");
  const [draft, setDraft] = useState("");
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
    const zoom = (step: ZoomStep) => conn.send({ type: "ui.zoom", step });
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        conn.send({ type: "ui.hide" });
        return;
      }
      // Ctrl +/-/0 scale the panel (content and window), as in a browser.
      if (!e.ctrlKey || e.altKey || e.metaKey) return;
      const step = zoomKey(e);
      if (step) {
        e.preventDefault();
        if (!e.repeat || step !== "reset") zoom(step);
      }
    };
    // Ctrl + wheel (and touchpad pinch, which comes as one): one step per notch; small deltas
    // add up, and steps are spaced so a fast spin doesn't fly through the whole range.
    let wheel = 0;
    let lastStep = 0;
    const onWheel = (e: WheelEvent) => {
      if (!e.ctrlKey) return;
      e.preventDefault();
      wheel += e.deltaMode === 0 ? e.deltaY : e.deltaY * 40;
      if (Math.abs(wheel) < 50 || performance.now() - lastStep < 90) return;
      lastStep = performance.now();
      zoom(wheel < 0 ? "in" : "out");
      wheel = 0;
    };
    // No WebView2 context menu (Back, Reload, Print, Save as...) on the overlay.
    const noMenu = (e: MouseEvent) => e.preventDefault();
    window.addEventListener("keydown", onKey);
    window.addEventListener("wheel", onWheel, { passive: false });
    window.addEventListener("contextmenu", noMenu);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("wheel", onWheel);
      window.removeEventListener("contextmenu", noMenu);
    };
  }, [conn]);

  // A zoom change shows the new value for a moment.
  const [zoomShown, setZoomShown] = useState(false);
  useEffect(() => {
    if (ui.overlay.zoomSeq === 0) return;
    setZoomShown(true);
    const t = setTimeout(() => setZoomShown(false), 1200);
    return () => clearTimeout(t);
  }, [ui.overlay.zoomSeq]);

  const expand = () => {
    conn.send({ type: "ui.setMode", mode: "expanded" }); // the core grows the window upward
    setMode("expanded");
  };

  const submit = () => {
    const text = draft.trim();
    if (!text || busy) return;
    const fromQuickField = mode === "compact";
    if (fromQuickField) expand();
    conn.send({ type: "chat.send", text, newChat: fromQuickField });
    setDraft("");
  };

  const openLink = useCallback((url: string) => conn.send({ type: "ui.openUrl", url }), [conn]);
  // The core runs the native move/resize loop (kept inside the game window, remembered per game).
  const startDrag = useCallback(() => conn.send({ type: "ui.dragStart" }), [conn]);
  const startResize = useCallback((edge: ResizeEdge) => conn.send({ type: "ui.resizeStart", edge }), [conn]);

  const game = mode === "expanded" ? (ui.chatGame ?? ui.overlay.game) : ui.overlay.game;
  const openGame = game?.exe ? () => conn.send({ type: "ui.openUrl", url: gamePageUrl(game.exe) }) : undefined;
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
                games={ui.games}
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
                emptyHint={t("panel_empty_hint")}
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
            game={game}
            gameName={gameLabel(game, ui.games)}
            onOpenGame={openGame}
            agentName={ui.agent?.agent ?? "Claude"}
            agentOptions={ui.agentOptions}
            onSetAgentOption={(id, value) => conn.send({ type: "agent.setOption", id, value })}
            onBeforeModelMenu={mode === "compact" ? expand : undefined}
            placeholder={
              mode === "compact"
                ? agentDown
                  ? t("agent_unavailable_see_chat", { agent: ui.agent?.agent ?? t("agent") })
                  : t("quick_placeholder")
                : t("message_placeholder")
            }
            inputRef={inputRef}
            onDragStart={mode === "compact" ? startDrag : undefined}
          />
          {ui.status !== "open" && <div className="offline" title={t("no_app_connection")} />}
          {zoomShown && (
            <div className="zoom-badge" role="status">
              {Math.round(ui.overlay.zoom * 100)}%<span> · {t("zoom_reset")}</span>
            </div>
          )}
          <PageTooltip />
        </div>
        {/* Outside .panel: its rounded overflow clip would cut the corner handles. */}
        <ResizeHandles edges={mode === "compact" ? COMPACT_EDGES : ALL_EDGES} onStart={startResize} />
      </ShotViewerProvider>
    </OpenLinkContext.Provider>
  );
}

/** Ctrl + this key zooms the panel ("=" is "+" without Shift; the numpad has its own). */
function zoomKey(e: KeyboardEvent): ZoomStep | null {
  if (e.key === "+" || e.key === "=" || e.code === "NumpadAdd") return "in";
  if (e.key === "-" || e.key === "_" || e.code === "NumpadSubtract") return "out";
  if (e.key === "0" || e.code === "Numpad0") return "reset";
  return null;
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
