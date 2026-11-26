// The chat header's update button (the overlay panel and the browser tab on this PC): shown only
// while a newer version is found and not put off with "Later"; it opens a small card with the
// version, "What's new" and "Update and restart".

import { useEffect, useRef, useState } from "react";
import { CircleArrowDown } from "lucide-react";
import type { ClientMsg } from "../api/generated/ClientMsg";
import type { UpdateStatus } from "../api/generated/UpdateStatus";
import { useT, type T } from "../i18n";

interface Props {
  update: UpdateStatus | null;
  send(msg: ClientMsg): void;
}

export function UpdateButton({ update, send }: Props) {
  const t = useT();
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const busy = update?.state.state === "downloading" || update?.state.state === "installing";

  useEffect(() => {
    if (!open) return;
    // Esc closes the card, not the panel (capture phase: the panel hides on Esc itself).
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        setOpen(false);
      }
    };
    const onDown = (e: PointerEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    window.addEventListener("keydown", onKey, true);
    window.addEventListener("pointerdown", onDown, true);
    return () => {
      window.removeEventListener("keydown", onKey, true);
      window.removeEventListener("pointerdown", onDown, true);
    };
  }, [open]);

  if (!update?.version || update.state.state === "upToDate" || (update.dismissed && !busy)) return null;
  const version = update.version;
  const percent = update.state.state === "downloading" ? update.state.percent : null;

  return (
    <div className="update-btn" ref={ref}>
      <button
        type="button"
        className={`icon-btn${open ? " selected" : ""}`}
        onClick={() => setOpen((o) => !o)}
        title={t("update_button", { version })}
        aria-label={t("update_button", { version })}
        aria-expanded={open}
      >
        {busy ? <ProgressRing percent={percent} /> : <CircleArrowDown size={16} />}
        {!busy && <span className="update-dot" aria-hidden />}
      </button>
      {open && (
        <div className="update-card" role="dialog" aria-label={t("update_button", { version })}>
          <div className="update-card-title">{t("update_available", { version })}</div>
          <div className="update-card-sub">
            {t("update_yours", { current: update.current })}
            {update.notesUrl && (
              <>
                {" · "}
                <button type="button" className="link-btn" onClick={() => send({ type: "ui.openUrl", url: update.notesUrl! })}>
                  {t("update_whats_new")} ↗
                </button>
              </>
            )}
          </div>
          {update.portable && !busy && <p className="update-card-note">{t("update_portable_hint")}</p>}
          {update.state.state === "error" && <p className="update-card-error">{update.state.message}</p>}
          {busy ? (
            <p className="update-card-note">{progressText(update, t)}</p>
          ) : (
            <div className="update-card-actions">
              <button type="button" className="btn primary" onClick={() => send({ type: "update.install" })}>
                {update.portable ? t("update_download") : t("update_install")}
              </button>
              <button
                type="button"
                className="btn"
                onClick={() => {
                  send({ type: "update.dismiss" });
                  setOpen(false);
                }}
              >
                {t("update_later")}
              </button>
            </div>
          )}
        </div>
      )}
    </div>
  );
}

/** "Downloading… 40%" / "Installing…" (also on the settings page). */
export function progressText(update: UpdateStatus, t: T): string {
  const s = update.state;
  if (s.state === "installing") return t("update_installing");
  if (s.state === "downloading") {
    return s.percent === null ? t("update_downloading") : t("update_downloading_percent", { percent: s.percent });
  }
  return "";
}

/** A ring that fills while the update downloads (spins while the size is unknown). */
function ProgressRing({ percent }: { percent: number | null }) {
  const r = 6.5;
  const length = 2 * Math.PI * r;
  const filled = percent === null ? 0.25 : percent / 100;
  return (
    <svg width="16" height="16" viewBox="0 0 16 16" className={percent === null ? "update-ring spin" : "update-ring"} aria-hidden>
      <circle cx="8" cy="8" r={r} fill="none" stroke="var(--border-strong)" strokeWidth="2" />
      <circle
        cx="8"
        cy="8"
        r={r}
        fill="none"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
        strokeDasharray={`${filled * length} ${length}`}
        transform="rotate(-90 8 8)"
      />
    </svg>
  );
}
