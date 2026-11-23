// Game frames in the chat: thumbnails and a full-size viewer (inside the overlay panel too,
// so looking at a frame doesn't throw the user out of the game into a browser).

import { createContext, useContext, useEffect, useState, type ReactNode } from "react";
import { ExternalLink, X } from "lucide-react";
import type { Shot } from "../api/generated/Shot";
import { OpenLinkContext } from "./Markdown";

export const shotUrl = (shot: Shot) => `/api/shots/${shot.id}`;

const ViewShotContext = createContext<(shot: Shot) => void>(() => {});

export function ShotViewerProvider({ children }: { children: ReactNode }) {
  const [shot, setShot] = useState<Shot | null>(null);
  return (
    <ViewShotContext.Provider value={setShot}>
      {children}
      {shot && <ShotViewer shot={shot} onClose={() => setShot(null)} />}
    </ViewShotContext.Provider>
  );
}

export function ShotThumb({ shot, width, label }: { shot: Shot; width: number; label: string }) {
  const view = useContext(ViewShotContext);
  return (
    <button
      type="button"
      className="shot-thumb"
      style={{ width, aspectRatio: `${shot.width} / ${shot.height}` }}
      onClick={() => view(shot)}
      title={`${label} · ${shot.width}×${shot.height} — открыть`}
    >
      <img src={shotUrl(shot)} alt={label} loading="lazy" draggable={false} />
    </button>
  );
}

function ShotViewer({ shot, onClose }: { shot: Shot; onClose(): void }) {
  const openLink = useContext(OpenLinkContext);
  const url = shotUrl(shot);

  // Esc closes the viewer first (capture phase: the panel's own Esc would hide the overlay).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [onClose]);

  const openOutside = () => {
    const absolute = new URL(url, location.href).href;
    if (openLink) openLink(absolute);
    else window.open(absolute, "_blank", "noopener");
  };

  return (
    <div className="shot-viewer" role="dialog" aria-label="Кадр игры" onClick={onClose}>
      <img src={url} alt="Кадр игры" onClick={(e) => e.stopPropagation()} draggable={false} />
      <div className="shot-viewer-bar" onClick={(e) => e.stopPropagation()}>
        <span className="shot-viewer-size">
          {shot.width}×{shot.height}
        </span>
        <button type="button" className="icon-btn" onClick={openOutside} title="Открыть в браузере">
          <ExternalLink size={16} />
        </button>
        <button type="button" className="icon-btn" onClick={onClose} title="Закрыть (Esc)">
          <X size={16} />
        </button>
      </div>
    </div>
  );
}
