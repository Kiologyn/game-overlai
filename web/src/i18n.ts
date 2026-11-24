import { useCallback, useSyncExternalStore } from "react";
import type { Language } from "./api/generated/Language";
import en from "./locales/en.json";
import es from "./locales/es.json";
import ja from "./locales/ja.json";
import ru from "./locales/ru.json";
import zh from "./locales/zh.json";

/**
 * Interface translation, like Android string resources: the code uses string codes
 * (t("new_chat")), and every language has a file src/locales/<language>.json mapping codes to
 * its strings. English (en.json) is the base: a string missing in a language is shown in
 * English. The files are shared with Rust (src-tauri/src/i18n.rs), whose tests check that every
 * code used here exists and that every language has every string. {name} in a string is a
 * placeholder. Add every new code to all language files right away.
 *
 * The language is the app's (config.toml, chosen on the settings page): the core sends it to
 * every UI, phones included. Until it arrives, the last one this browser saw, else the browser's.
 *
 * A new language: a row in LANGUAGES, its dictionary in DICTIONARIES here, a `Language` variant
 * in protocol.rs and its dictionary in i18n.rs.
 */
export const LANGUAGES: { code: Language; name: string; english: string; html: string }[] = [
  { code: "en", name: "English", english: "English", html: "en" },
  { code: "ru", name: "Русский", english: "Russian", html: "ru" },
  { code: "es", name: "Español", english: "Spanish", html: "es" },
  { code: "zh", name: "中文", english: "Chinese (Simplified)", html: "zh-CN" },
  { code: "ja", name: "日本語", english: "Japanese", html: "ja" },
];

const DICTIONARIES: Record<Language, Record<string, string>> = { en, ru, es, zh, ja };

const STORED = "overlai.language";

function initial(): Language {
  try {
    const stored = localStorage.getItem(STORED);
    if (stored && stored in DICTIONARIES) return stored as Language;
  } catch {
    // storage blocked: the browser's language
  }
  const browser = navigator.language.toLowerCase();
  return LANGUAGES.find((l) => l.code !== "en" && browser.startsWith(l.code))?.code ?? "en";
}

let current: Language = initial();
const listeners = new Set<() => void>();

/** The language's tag for <html lang> and Intl formats. */
export const htmlLang = (lang: Language = current) => LANGUAGES.find((l) => l.code === lang)?.html ?? lang;

document.documentElement.lang = htmlLang();

/** The core told the language (on connect and when the user changes it). */
export function setLanguage(lang: Language) {
  if (!(lang in DICTIONARIES) || lang === current) return;
  current = lang;
  document.documentElement.lang = htmlLang();
  try {
    localStorage.setItem(STORED, lang);
  } catch {
    // only the first paint of the next visit uses it
  }
  listeners.forEach((f) => f());
}

function subscribe(f: () => void) {
  listeners.add(f);
  return () => listeners.delete(f);
}

function fill(text: string, vars?: Record<string, string | number>): string {
  if (!vars) return text;
  return text.replace(/\{(\w+)\}/g, (all, key: string) => (key in vars ? String(vars[key]) : all));
}

const missing = new Set<string>();

export function translate(lang: Language, code: string, vars?: Record<string, string | number>): string {
  const found = DICTIONARIES[lang][code];
  if (found === undefined && import.meta.env.DEV && !missing.has(`${lang}:${code}`)) {
    missing.add(`${lang}:${code}`);
    console.warn(`missing string (${lang}): ${code}`);
  }
  // missing in the language: English; missing in English too: the code itself, so it's noticed
  return fill(found ?? DICTIONARIES.en[code] ?? code, vars);
}

export type T = (code: string, vars?: Record<string, string | number>) => string;

/** Translation in a component: subscribes the component to language changes. */
export function useT(): T {
  const lang = useLanguage();
  return useCallback<T>((code, vars) => translate(lang, code, vars), [lang]);
}

/** The current language; the component re-renders when it changes. */
export function useLanguage(): Language {
  return useSyncExternalStore(subscribe, () => current);
}
