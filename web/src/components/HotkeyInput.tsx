// A field that records a key combination (settings page, a game's page). Our global hotkeys are
// off while it has focus, so pressing the current combination records it instead of firing.

import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import type { ClientMsg } from "../api/generated/ClientMsg";
import { useT } from "../i18n";

/** "ctrl+alt+q" -> "Ctrl+Alt+Q" (as in the tray menu). */
export function prettyHotkey(s: string): string {
  const names: Record<string, string> = { ctrl: "Ctrl", control: "Ctrl", alt: "Alt", shift: "Shift", super: "Win" };
  return s
    .split("+")
    .map((p) => names[p.trim().toLowerCase()] ?? p.trim().charAt(0).toUpperCase() + p.trim().slice(1))
    .join("+");
}

/** The key of a keydown as the core's hotkey syntax names it. */
function keyName(e: KeyboardEvent): string | null {
  const code = e.code;
  if (/^(Control|Alt|Shift|Meta|OS)(Left|Right)?$/.test(code) || code === "") return null;
  if (code.startsWith("Key")) return code.slice(3).toLowerCase();
  if (code.startsWith("Digit")) return code.slice(5);
  if (/^F\d+$/.test(code)) return code.toLowerCase();
  if (code === "Space") return "space";
  return code;
}

interface Props {
  send(msg: ClientMsg): void;
  value: string;
  onChange(v: string): void;
  /** Shown dimmed while `value` is empty (a game's page: the combination from the settings). */
  placeholder?: string;
}

export function HotkeyInput({ send, value, onChange, placeholder }: Props) {
  const t = useT();
  const [recording, setRecording] = useState(false);
  const sendRef = useRef(send);
  useEffect(() => {
    sendRef.current = send;
  });
  const setCapturing = (active: boolean) => {
    setRecording(active);
    send({ type: "settings.capturing", active });
  };
  // Leaving the page while recording turns our hotkeys back on.
  useEffect(() => () => sendRef.current({ type: "settings.capturing", active: false }), []);

  const empty = !recording && value === "";
  return (
    <button
      type="button"
      className={`hotkey-input${recording ? " recording" : ""}${empty ? " placeholder" : ""}`}
      onFocus={() => setCapturing(true)}
      onBlur={() => setCapturing(false)}
      onKeyDown={(e) => {
        if (e.key === "Tab") return;
        e.preventDefault();
        if (e.key === "Escape") {
          (e.target as HTMLElement).blur();
          return;
        }
        const key = keyName(e);
        if (!key) return; // only a modifier so far
        const mods = [e.ctrlKey && "ctrl", e.altKey && "alt", e.shiftKey && "shift", e.metaKey && "super"].filter(Boolean);
        if (mods.length === 0 && !/^f\d+$/.test(key)) return; // a plain letter would fire while typing
        onChange([...mods, key].join("+"));
        (e.target as HTMLElement).blur();
      }}
    >
      {recording ? t("press_combination") : empty ? (placeholder ?? "") : prettyHotkey(value)}
    </button>
  );
}
