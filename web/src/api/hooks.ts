import { useSyncExternalStore } from "react";
import type { Connection, UiState } from "./connection";

export function useUiState(conn: Connection): UiState {
  return useSyncExternalStore(conn.subscribe, conn.getState);
}
