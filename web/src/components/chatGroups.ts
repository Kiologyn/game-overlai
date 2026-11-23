import type { ChatSummary } from "../api/generated/ChatSummary";

/** Chats matching `query` (title or game), grouped by game in list order (newest first). */
export function groupChats(chats: ChatSummary[], query: string): [string, ChatSummary[]][] {
  const needle = query.trim().toLocaleLowerCase();
  const groups = new Map<string, ChatSummary[]>();
  for (const chat of chats) {
    const game = chat.game?.title || chat.game?.exe || "Без игры";
    if (needle && !`${chat.title} ${game}`.toLocaleLowerCase().includes(needle)) continue;
    groups.set(game, [...(groups.get(game) ?? []), chat]);
  }
  return [...groups.entries()];
}
