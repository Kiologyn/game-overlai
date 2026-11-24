// Game profiles on the UI side: the user's name for a game wins over its window title.

import type { GameInfo } from "./generated/GameInfo";
import type { GameProfile } from "./generated/GameProfile";

export const sameExe = (a: string, b: string) => a.toLowerCase() === b.toLowerCase();

export function findProfile(games: GameProfile[], exe: string | null | undefined): GameProfile | null {
  if (!exe) return null;
  return games.find((g) => sameExe(g.exe, exe)) ?? null;
}

/** The name to show for a game: the profile's name, else the window title, else the exe. */
export function gameLabel(game: GameInfo | null, games: GameProfile[]): string {
  if (!game) return "";
  return findProfile(games, game.exe)?.name || game.title || game.exe;
}

/** A profile for a game the core doesn't know yet (saving it creates the row). */
export function blankProfile(exe: string, title = ""): GameProfile {
  return { exe, title, name: null, instructions: "", sources: "", progress: [] };
}

/** URL of a game's page in the browser chat. */
export function gamePageUrl(exe: string): string {
  return `${location.origin}/?game=${encodeURIComponent(exe)}`;
}
