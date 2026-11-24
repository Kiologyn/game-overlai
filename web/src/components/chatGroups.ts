import type { ChatSummary } from "../api/generated/ChatSummary";
import type { GameProfile } from "../api/generated/GameProfile";
import { gameLabel } from "../api/games";

export interface ChatGroup {
  key: string;
  /** Shown name of the game (the profile's name if the user gave one). */
  label: string;
  /** null = chats without a game. */
  exe: string | null;
  chats: ChatSummary[];
}

/** Chats matching `query` (title or game), grouped by game in list order (newest first).
 * `noGame`: the label of chats without a game. */
export function groupChats(chats: ChatSummary[], query: string, games: GameProfile[], noGame: string): ChatGroup[] {
  const needle = query.trim().toLocaleLowerCase();
  const groups = new Map<string, ChatGroup>();
  for (const chat of chats) {
    const exe = chat.game?.exe || null;
    const label = gameLabel(chat.game, games) || noGame;
    const key = exe ? exe.toLowerCase() : `title:${label}`;
    if (needle && !`${chat.title} ${label} ${chat.game?.title ?? ""}`.toLocaleLowerCase().includes(needle)) continue;
    const group = groups.get(key);
    if (group) group.chats.push(chat);
    else groups.set(key, { key, label, exe, chats: [chat] });
  }
  return [...groups.values()];
}
