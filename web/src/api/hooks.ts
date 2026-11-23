import { useCallback, useState, useSyncExternalStore } from "react";
import type { Connection, UiState } from "./connection";

export function useUiState(conn: Connection): UiState {
  return useSyncExternalStore(conn.subscribe, conn.getState);
}

/** A boolean preference remembered in this browser/webview (falls back to memory). */
export function useStoredFlag(key: string, fallback: boolean): [boolean, (v: boolean) => void] {
  const [value, setValue] = useState<boolean>(() => {
    try {
      const raw = localStorage.getItem(key);
      return raw === null ? fallback : raw === "1";
    } catch {
      return fallback;
    }
  });
  const update = useCallback(
    (v: boolean) => {
      setValue(v);
      try {
        localStorage.setItem(key, v ? "1" : "0");
      } catch {
        /* storage unavailable: keep it in memory */
      }
    },
    [key],
  );
  return [value, update];
}
