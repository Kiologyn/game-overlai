// /widgets: the transparent layer the core lays over the game window (src-tauri/src/widgets.rs),
// with the widgets of that game. It never takes the focus from the game, and the mouse goes
// through it except over a widget: the page reports where its widgets are (widgets.rects).
//
// Moving: drag a widget by its title bar. It stays inside the game window and snaps to its
// edges; the place is kept as a fraction of the window, so it survives resolution changes.
// Width: drag the bottom-right corner. The corner button makes it 50% see-through.
// ?exe=<game.exe> previews a game's widgets in a browser tab.

import { useCallback, useEffect, useLayoutEffect, useRef, useState, type PointerEvent as ReactPointerEvent } from "react";
import { Check, ChevronDown, ChevronUp, Contrast, Timer } from "lucide-react";
import type { Connection } from "../api/connection";
import type { Widget } from "../api/generated/Widget";
import { sameExe } from "../api/games";
import { useUiState } from "../api/hooks";
import { Markdown, OpenLinkContext } from "../components/Markdown";
import { useT } from "../i18n";

/** Gap kept from the game window's edges; a widget dragged near an edge snaps to it. */
const MARGIN = 8;
const SNAP = 12;
const MIN_WIDTH = 160;
const MAX_WIDTH = 640;

const clamp = (v: number, min: number, max: number) => Math.min(Math.max(v, min), Math.max(min, max));

/** Keeps a coordinate in [MARGIN, max - MARGIN], snapping to those ends within SNAP px. */
const place1 = (v: number, max: number) => {
  const lo = MARGIN;
  const hi = Math.max(lo, max - MARGIN);
  const c = clamp(v, lo, hi);
  return c - lo < SNAP ? lo : hi - c < SNAP ? hi : c;
};

function useViewport() {
  const [size, setSize] = useState({ w: window.innerWidth, h: window.innerHeight });
  useEffect(() => {
    const onResize = () => setSize({ w: window.innerWidth, h: window.innerHeight });
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);
  return size;
}

/** Re-renders every second while some widget counts down. */
function useTick(active: boolean) {
  const [, setNow] = useState(0);
  useEffect(() => {
    if (!active) return;
    const id = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(id);
  }, [active]);
}

export function WidgetLayer({ conn }: { conn: Connection }) {
  const ui = useUiState(conn);
  const view = useViewport();
  // ?exe=<game.exe> shows that game's widgets in a normal browser tab (to look at them; the layer
  // itself is kept out of screenshots).
  const exe = new URLSearchParams(location.search).get("exe") ?? ui.widgetsShown;
  const list = exe ? ui.widgets.filter((w) => sameExe(w.exe, exe)) : [];
  useTick(list.some((w) => w.timerEndsAt !== null));

  // Where the widgets are, for the core's click-through (sent when it changes, and on reconnect).
  const cards = useRef(new Map<string, HTMLDivElement>());
  const sent = useRef("");
  const report = useCallback(() => {
    const rects = [...cards.current.values()].map((el) => {
      const r = el.getBoundingClientRect();
      return { x: r.left, y: r.top, width: r.width, height: r.height };
    });
    const key = JSON.stringify(rects);
    if (key !== sent.current) {
      sent.current = key;
      conn.send({ type: "widgets.rects", rects });
    }
  }, [conn]);
  useLayoutEffect(report);
  useEffect(() => {
    if (ui.status === "open") sent.current = "";
  }, [ui.status]);
  useEffect(() => {
    const observer = new ResizeObserver(report);
    cards.current.forEach((el) => observer.observe(el));
    return () => observer.disconnect();
  });

  const register = useCallback((id: string, el: HTMLDivElement | null) => {
    if (el) cards.current.set(id, el);
    else cards.current.delete(id);
  }, []);
  const openLink = useCallback((url: string) => conn.send({ type: "ui.openUrl", url }), [conn]);

  return (
    <OpenLinkContext.Provider value={openLink}>
      <div className="widget-layer">
        {list.map((w) => (
          <WidgetCard key={w.id} widget={w} view={view} conn={conn} register={register} onMoved={report} />
        ))}
      </div>
    </OpenLinkContext.Provider>
  );
}

interface CardProps {
  widget: Widget;
  view: { w: number; h: number };
  conn: Connection;
  register(id: string, el: HTMLDivElement | null): void;
  onMoved(): void;
}

/** A place while the user drags (px); null: the stored one. */
interface Live {
  left: number;
  top: number;
  width: number;
}

function WidgetCard({ widget: w, view, conn, register, onMoved }: CardProps) {
  const t = useT();
  const ref = useRef<HTMLDivElement | null>(null);
  const [live, setLive] = useState<Live | null>(null);
  const gesture = useRef<{ kind: "move" | "resize"; x: number; y: number; from: Live } | null>(null);

  // The stored place, kept inside the window (the window may have shrunk since).
  const height = ref.current?.offsetHeight ?? 40;
  const width = clamp(w.width, MIN_WIDTH, Math.min(MAX_WIDTH, view.w - 2 * MARGIN));
  const stored: Live = {
    width,
    left: clamp(w.x * view.w, MARGIN, view.w - width - MARGIN),
    top: clamp(w.y * view.h, MARGIN, view.h - height - MARGIN),
  };
  const place = live ?? stored;

  // The core's copy came back: drop the local one.
  useEffect(() => setLive(null), [w.x, w.y, w.width]);
  useLayoutEffect(onMoved, [place.left, place.top, place.width, onMoved]);

  const start = (kind: "move" | "resize") => (e: ReactPointerEvent<HTMLElement>) => {
    if (e.button !== 0) return;
    if (kind === "move" && (e.target as HTMLElement).closest("button")) return;
    e.preventDefault();
    e.currentTarget.setPointerCapture(e.pointerId);
    gesture.current = { kind, x: e.clientX, y: e.clientY, from: place };
  };
  const move = (e: ReactPointerEvent<HTMLElement>) => {
    const g = gesture.current;
    if (!g) return;
    const dx = e.clientX - g.x;
    const dy = e.clientY - g.y;
    const h = ref.current?.offsetHeight ?? height;
    if (g.kind === "move") {
      const left = place1(g.from.left + dx, view.w - g.from.width);
      const top = place1(g.from.top + dy, view.h - h);
      setLive({ ...g.from, left, top });
    } else {
      setLive({ ...g.from, width: clamp(g.from.width + dx, MIN_WIDTH, Math.min(MAX_WIDTH, view.w - g.from.left - MARGIN)) });
    }
  };
  const end = () => {
    const g = gesture.current;
    gesture.current = null;
    if (!g || !live) return;
    if (g.kind === "move") {
      conn.send({ type: "widget.set", id: w.id, x: live.left / Math.max(1, view.w), y: live.top / Math.max(1, view.h) });
    } else {
      conn.send({ type: "widget.set", id: w.id, width: Math.round(live.width) });
    }
  };

  const done = w.items.filter((i) => i.done).length;
  return (
    <div
      ref={(el) => {
        ref.current = el;
        register(w.id, el);
      }}
      className={`widget${w.translucent ? " translucent" : ""}${w.collapsed ? " collapsed" : ""}${gesture.current ? " moving" : ""}`}
      style={{ left: place.left, top: place.top, width: place.width }}
    >
      <div className="widget-head" onPointerDown={start("move")} onPointerMove={move} onPointerUp={end} onPointerCancel={end}>
        <span className="widget-title" title={w.title}>
          {w.title}
        </span>
        {w.collapsed && w.items.length > 0 && (
          <span className="widget-count">
            {done}/{w.items.length}
          </span>
        )}
        <button
          type="button"
          className="widget-btn"
          title={w.collapsed ? t("widget_expand") : t("widget_collapse")}
          onClick={() => conn.send({ type: "widget.set", id: w.id, collapsed: !w.collapsed })}
        >
          {w.collapsed ? <ChevronDown size={13} /> : <ChevronUp size={13} />}
        </button>
        <button
          type="button"
          className={`widget-btn${w.translucent ? " on" : ""}`}
          title={t("widget_translucent")}
          aria-pressed={w.translucent}
          onClick={() => conn.send({ type: "widget.set", id: w.id, translucent: !w.translucent })}
        >
          <Contrast size={13} />
        </button>
      </div>
      {!w.collapsed && (
        <div className="widget-content">
          {w.timerEndsAt !== null && <Countdown endsAt={w.timerEndsAt} />}
          {w.body && <Markdown text={w.body} />}
          {w.items.length > 0 && (
            <ul className="widget-items">
              {w.items.map((item) => (
                <li key={item.id}>
                  <button
                    type="button"
                    className={`widget-item${item.done ? " done" : ""}`}
                    aria-pressed={item.done}
                    onClick={() => conn.send({ type: "widget.toggleItem", id: w.id, itemId: item.id })}
                  >
                    <span className="widget-check">{item.done && <Check size={11} strokeWidth={3} />}</span>
                    <span>{item.text}</span>
                  </button>
                </li>
              ))}
            </ul>
          )}
        </div>
      )}
      <span
        className="widget-resize"
        title={t("widget_resize")}
        onPointerDown={start("resize")}
        onPointerMove={move}
        onPointerUp={end}
        onPointerCancel={end}
      />
    </div>
  );
}

function Countdown({ endsAt }: { endsAt: number }) {
  const t = useT();
  const left = Math.max(0, Math.round((endsAt - Date.now()) / 1000));
  const h = Math.floor(left / 3600);
  const m = Math.floor((left % 3600) / 60);
  const s = String(left % 60).padStart(2, "0");
  const text = h > 0 ? `${h}:${String(m).padStart(2, "0")}:${s}` : `${m}:${s}`;
  return (
    <div className={`widget-timer${left === 0 ? " done" : ""}`}>
      <Timer size={13} aria-hidden />
      {left === 0 ? t("widget_timer_done") : text}
    </div>
  );
}
