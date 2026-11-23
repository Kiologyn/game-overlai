// /settings on this PC: hotkeys, agent, proxy, panel, frames, phone port, notifications,
// starting with Windows. The core sends the settings to this PC's UI only.

import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { RotateCcw, Settings as SettingsIcon } from "lucide-react";
import type { Connection } from "../api/connection";
import type { SettingsValues } from "../api/generated/SettingsValues";
import { useUiState } from "../api/hooks";

const FRAME_HEIGHTS = [
  { value: 480, label: "480p — меньше лимитов, мелкий текст хуже" },
  { value: 720, label: "720p (по умолчанию)" },
  { value: 1080, label: "1080p — мелкий текст и интерфейс" },
  { value: 1440, label: "1440p" },
  { value: 0, label: "полный размер окна" },
];

type ProxyMode = "system" | "none" | "custom";

const proxyMode = (p: string): ProxyMode => (p === "system" || p === "" ? "system" : p === "none" ? "none" : "custom");

const same = (a: SettingsValues, b: SettingsValues) => JSON.stringify(a) === JSON.stringify(b);

export function SettingsPage({ conn }: { conn: Connection }) {
  const ui = useUiState(conn);
  const settings = ui.settings;
  const [form, setForm] = useState<SettingsValues | null>(null);
  const [mode, setMode] = useState<ProxyMode>("system");
  const previous = useRef<SettingsValues | null>(null);

  // A new state from the core: show it unless the user has unsaved edits.
  const savedKey = settings ? JSON.stringify(settings.values) : "";
  useEffect(() => {
    if (!settings) return;
    const before = previous.current;
    previous.current = settings.values;
    setForm((current) => {
      if (!current || !before || same(current, before) || same(current, settings.values)) {
        setMode(proxyMode(settings.values.proxy));
        return settings.values;
      }
      return current;
    });
  }, [savedKey]); // eslint-disable-line react-hooks/exhaustive-deps

  if (!settings || !form) {
    return (
      <div className="settings-page">
        <h1>Настройки</h1>
        <p className="phone-note">
          {ui.status === "open" ? "Настройки открываются только на ПК, где запущен game-overlai." : "Подключение к приложению…"}
        </p>
      </div>
    );
  }

  const set = <K extends keyof SettingsValues>(key: K, value: SettingsValues[K]) => setForm((f) => (f ? { ...f, [key]: value } : f));
  const dirty = !same(form, settings.values);
  const agentName = (id: string) => settings.agents.find((a) => a.id === id)?.name ?? id;
  const chosen = settings.agents.find((a) => a.id === form.agent);

  const save = () => {
    const proxy = mode === "custom" ? form.proxy.trim() : mode;
    conn.send({ type: "settings.save", values: { ...form, proxy } });
  };

  return (
    <div className="settings-page">
      <div className="phone-head">
        <SettingsIcon size={20} aria-hidden />
        <h1>Настройки</h1>
      </div>

      {settings.restartNeeded && (
        <div className="settings-banner">
          <span>Агент, прокси и порты применятся после перезапуска приложения.</span>
          <button type="button" className="btn primary" onClick={() => conn.send({ type: "app.restart" })}>
            <RotateCcw size={13} aria-hidden /> Перезапустить
          </button>
        </div>
      )}

      <form
        className="settings-form"
        onSubmit={(e) => {
          e.preventDefault();
          if (dirty || mode !== proxyMode(settings.values.proxy)) save();
        }}
      >
        <section>
          <h2>Горячие клавиши</h2>
          <div className="settings-row">
            <span>Быстрый вопрос</span>
            <HotkeyInput conn={conn} value={form.hotkeyQuick} onChange={(v) => set("hotkeyQuick", v)} />
          </div>
          <div className="settings-row">
            <span>Чат</span>
            <HotkeyInput conn={conn} value={form.hotkeyChat} onChange={(v) => set("hotkeyChat", v)} />
          </div>
          {settings.hotkeysTaken.length > 0 && (
            <p className="settings-warn">
              Заняты другой программой: {settings.hotkeysTaken.map(prettyHotkey).join(", ")}. Выберите другое сочетание.
            </p>
          )}
          <p className="field-hint">Нажмите на поле и затем сочетание. Применяется сразу после сохранения.</p>
        </section>

        <section>
          <h2>Агент</h2>
          <div className="settings-choices">
            {settings.agents.map((a) => (
              <label key={a.id} className="settings-choice">
                <input type="radio" name="agent" checked={form.agent === a.id} onChange={() => set("agent", a.id)} />
                {a.name}
              </label>
            ))}
          </div>
          {chosen && <p className="field-hint">{chosen.loginHint}</p>}
        </section>

        <section>
          <h2>Сеть</h2>
          <div className="settings-row">
            <span>Прокси</span>
            <select value={mode} onChange={(e) => setMode(e.target.value as ProxyMode)}>
              <option value="system">системный прокси Windows</option>
              <option value="none">без прокси</option>
              <option value="custom">свой адрес</option>
            </select>
          </div>
          {mode === "custom" && (
            <div className="settings-row">
              <span>Адрес</span>
              <input
                value={form.proxy === "system" || form.proxy === "none" ? "" : form.proxy}
                onChange={(e) => set("proxy", e.target.value)}
                placeholder="http://127.0.0.1:10808"
              />
            </div>
          )}
          {settings.agents.map((a) => (
            <label key={a.id} className="settings-check">
              <input
                type="checkbox"
                checked={form.requireProxyFor.includes(a.id)}
                onChange={(e) =>
                  set(
                    "requireProxyFor",
                    e.target.checked ? [...form.requireProxyFor, a.id] : form.requireProxyFor.filter((x) => x !== a.id),
                  )
                }
              />
              Не подключаться к {agentName(a.id)} без прокси
            </label>
          ))}
          <p className="field-hint">
            Если прокси не найден, такой агент не запустится, а приложение будет ждать, пока прокси появится.
          </p>
        </section>

        <section>
          <h2>Панель</h2>
          <div className="settings-row">
            <span>Размер по умолчанию</span>
            <span className="settings-pair">
              <input type="number" min={360} max={2000} value={form.panelWidth} onChange={(e) => set("panelWidth", Number(e.target.value))} />
              ×
              <input type="number" min={220} max={1600} value={form.panelHeight} onChange={(e) => set("panelHeight", Number(e.target.value))} />
            </span>
          </div>
          <p className="field-hint">
            Для игр, над которыми панель ещё не двигали. Положение и размер над каждой игрой запоминаются сами.
          </p>
          <button
            type="button"
            className="btn"
            onClick={() => {
              if (confirm("Забыть, где стояла панель над каждой игрой?")) conn.send({ type: "settings.resetPanelPlaces" });
            }}
          >
            Сбросить положения панели
          </button>
        </section>

        <section>
          <h2>Кадры игры для агента</h2>
          <div className="settings-row">
            <span>Высота кадра</span>
            <select value={form.agentMaxHeight} onChange={(e) => set("agentMaxHeight", Number(e.target.value))}>
              {FRAME_HEIGHTS.map((h) => (
                <option key={h.value} value={h.value}>
                  {h.label}
                </option>
              ))}
            </select>
          </div>
          <p className="field-hint">Сохранённые кадры в истории всегда в полном размере.</p>
        </section>

        <section>
          <h2>Телефон</h2>
          <div className="settings-row">
            <span>Порт</span>
            <input type="number" min={1024} max={65535} value={form.lanPort} onChange={(e) => set("lanPort", Number(e.target.value))} />
          </div>
          <p className="field-hint">
            QR-код и включение доступа — на странице <a href="/phone">Телефон</a>.
          </p>
        </section>

        <section>
          <h2>Уведомления и запуск</h2>
          <label className="settings-check">
            <input type="checkbox" checked={form.notifications} onChange={(e) => set("notifications", e.target.checked)} />
            Уведомлять, когда ответ готов, а панель скрыта
          </label>
          <label className="settings-check">
            <input
              type="checkbox"
              checked={settings.autostart}
              onChange={(e) => conn.send({ type: "settings.autostart", enabled: e.target.checked })}
            />
            Запускать вместе с Windows
          </label>
          {settings.autostartOther && (
            <p className="settings-warn">Сейчас автозапуск настроен для другой копии: {settings.autostartOther}</p>
          )}
        </section>

        <div className="game-form-actions settings-actions">
          <button type="submit" className="btn primary" disabled={!dirty && mode === proxyMode(settings.values.proxy)}>
            Сохранить
          </button>
          {dirty && (
            <button
              type="button"
              className="btn"
              onClick={() => {
                setForm(settings.values);
                setMode(proxyMode(settings.values.proxy));
              }}
            >
              Отменить
            </button>
          )}
          {settings.error && <span className="settings-error">{settings.error}</span>}
        </div>
      </form>

      <p className="settings-about">
        game-overlai {settings.version} · настройки: {settings.configFile} · данные: {settings.dataDir}
      </p>
    </div>
  );
}

/** "alt+q" -> "Alt+Q" (as in the tray menu). */
function prettyHotkey(s: string): string {
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

/** Records a key combination; our global hotkeys are off while it has focus. */
function HotkeyInput({ conn, value, onChange }: { conn: Connection; value: string; onChange(v: string): void }) {
  const [recording, setRecording] = useState(false);
  const setCapturing = (active: boolean) => {
    setRecording(active);
    conn.send({ type: "settings.capturing", active });
  };
  useEffect(() => () => conn.send({ type: "settings.capturing", active: false }), [conn]);

  return (
    <button
      type="button"
      className={`hotkey-input${recording ? " recording" : ""}`}
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
      {recording ? "Нажмите сочетание…" : prettyHotkey(value)}
    </button>
  );
}
