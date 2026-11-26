// /settings on this PC: language, hotkeys, agent, proxy, panel, frames, phone port,
// notifications, starting with Windows, updates. The core sends the settings to this PC's UI only.

import { useEffect, useRef, useState } from "react";
import { RotateCcw, Settings as SettingsIcon } from "lucide-react";
import type { Connection } from "../api/connection";
import type { ClientMsg } from "../api/generated/ClientMsg";
import type { UpdateStatus } from "../api/generated/UpdateStatus";
import { progressText } from "../components/UpdateButton";
import { HotkeyInput, prettyHotkey } from "../components/HotkeyInput";
import type { SettingsValues } from "../api/generated/SettingsValues";
import { useUiState } from "../api/hooks";
import { LANGUAGES, useLanguage, useT } from "../i18n";

/** Frame heights with their string codes (1440p needs no words). */
const FRAME_HEIGHTS = [
  { value: 480, code: "frame_480" },
  { value: 720, code: "frame_720" },
  { value: 1080, code: "frame_1080" },
  { value: 1440, code: null },
  { value: 0, code: "frame_full" },
];

/** Same steps as the core's (`overlay::ZOOM_STEPS`). */
const ZOOMS = [0.5, 0.67, 0.75, 0.8, 0.9, 1, 1.1, 1.25, 1.5, 1.75, 2];

type ProxyMode = "system" | "none" | "custom";

const proxyMode = (p: string): ProxyMode => (p === "system" || p === "" ? "system" : p === "none" ? "none" : "custom");

const same = (a: SettingsValues, b: SettingsValues) => JSON.stringify(a) === JSON.stringify(b);

export function SettingsPage({ conn }: { conn: Connection }) {
  const ui = useUiState(conn);
  const t = useT();
  const language = useLanguage();
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
        <h1>{t("settings")}</h1>
        <p className="phone-note">{ui.status === "open" ? t("settings_only_on_pc") : t("connecting_app")}</p>
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
        <h1>{t("settings")}</h1>
      </div>

      {settings.restartNeeded && (
        <div className="settings-banner">
          <span>{t("settings_restart_needed")}</span>
          <button type="button" className="btn primary" onClick={() => conn.send({ type: "app.restart" })}>
            <RotateCcw size={13} aria-hidden /> {t("restart")}
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
          {/* Also in English: someone who picked a language they can't read finds the way back. */}
          <h2>{language === "en" ? "Language" : `${t("language")} · Language`}</h2>
          <div className="settings-row">
            {/* Applies at once, apart from the form's Save. Names in the languages themselves. */}
            <select
              value={language}
              lang="mul"
              onChange={(e) => conn.send({ type: "settings.language", language: e.target.value as typeof language })}
            >
              {LANGUAGES.map((l) => (
                <option key={l.code} value={l.code} lang={l.html}>
                  {l.english === l.name ? l.name : `${l.name} — ${l.english}`}
                </option>
              ))}
            </select>
          </div>
          <p className="field-hint">{t("language_hint")}</p>
        </section>

        <section>
          <h2>{t("hotkeys")}</h2>
          <div className="settings-row">
            <span>{t("hotkey_quick")}</span>
            <HotkeyInput send={(m) => conn.send(m)} value={form.hotkeyQuick} onChange={(v) => set("hotkeyQuick", v)} />
          </div>
          <div className="settings-row">
            <span>{t("hotkey_chat")}</span>
            <HotkeyInput send={(m) => conn.send(m)} value={form.hotkeyChat} onChange={(v) => set("hotkeyChat", v)} />
          </div>
          <div className="settings-row">
            <span>{t("hotkey_focus")}</span>
            <HotkeyInput send={(m) => conn.send(m)} value={form.hotkeyFocus} onChange={(v) => set("hotkeyFocus", v)} />
          </div>
          {settings.hotkeysTaken.length > 0 && (
            <p className="settings-warn">{t("hotkeys_taken", { keys: settings.hotkeysTaken.map(prettyHotkey).join(", ") })}</p>
          )}
          <p className="field-hint">{t("hotkeys_hint")}</p>
        </section>

        <section>
          <h2>{t("agent_section")}</h2>
          <div className="settings-choices">
            {settings.agents.map((a) => (
              <label key={a.id} className="settings-choice">
                <input type="radio" name="agent" checked={form.agent === a.id} onChange={() => set("agent", a.id)} />
                {a.name}
              </label>
            ))}
          </div>
          {chosen && <p className="field-hint">{chosen.loginHint}</p>}
          <label className="settings-check">
            <input type="checkbox" checked={form.progressMemory} onChange={(e) => set("progressMemory", e.target.checked)} />
            {t("progress_memory")}
          </label>
          <p className="field-hint">{t("progress_memory_hint")}</p>
        </section>

        <section>
          <h2>{t("network")}</h2>
          <div className="settings-row">
            <span>{t("proxy")}</span>
            <select value={mode} onChange={(e) => setMode(e.target.value as ProxyMode)}>
              <option value="system">{t("proxy_system")}</option>
              <option value="none">{t("proxy_none")}</option>
              <option value="custom">{t("proxy_custom")}</option>
            </select>
          </div>
          {mode === "custom" && (
            <div className="settings-row">
              <span>{t("proxy_address")}</span>
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
              {t("require_proxy", { agent: agentName(a.id) })}
            </label>
          ))}
          <p className="field-hint">{t("require_proxy_hint")}</p>
        </section>

        <section>
          <h2>{t("panel")}</h2>
          <div className="settings-row">
            <span>{t("panel_default_size")}</span>
            <span className="settings-pair">
              <input type="number" min={360} max={2000} value={form.panelWidth} onChange={(e) => set("panelWidth", Number(e.target.value))} />
              ×
              <input type="number" min={220} max={1600} value={form.panelHeight} onChange={(e) => set("panelHeight", Number(e.target.value))} />
            </span>
          </div>
          <div className="settings-row">
            <span>{t("panel_default_zoom")}</span>
            <select value={form.panelZoom} onChange={(e) => set("panelZoom", Number(e.target.value))}>
              {ZOOMS.map((z) => (
                <option key={z} value={z}>
                  {Math.round(z * 100)}%
                </option>
              ))}
            </select>
          </div>
          <p className="field-hint">{t("panel_hint")}</p>
          <label className="settings-check">
            <input
              type="checkbox"
              checked={form.panelCapturable}
              onChange={(e) => set("panelCapturable", e.target.checked)}
            />
            {t("panel_capturable")}
          </label>
          <p className="field-hint">{t("panel_capturable_hint")}</p>
          <button
            type="button"
            className="btn"
            onClick={() => {
              if (confirm(t("reset_panel_places_confirm"))) conn.send({ type: "settings.resetPanelPlaces" });
            }}
          >
            {t("reset_panel_places")}
          </button>
        </section>

        <section>
          <h2>{t("frames")}</h2>
          <div className="settings-row">
            <span>{t("frame_height")}</span>
            <select value={form.agentMaxHeight} onChange={(e) => set("agentMaxHeight", Number(e.target.value))}>
              {FRAME_HEIGHTS.map((h) => (
                <option key={h.value} value={h.value}>
                  {h.code ? t(h.code) : `${h.value}p`}
                </option>
              ))}
            </select>
          </div>
          <p className="field-hint">{t("frames_hint")}</p>
        </section>

        <section>
          <h2>{t("phone")}</h2>
          <div className="settings-row">
            <span>{t("port")}</span>
            <input type="number" min={1024} max={65535} value={form.lanPort} onChange={(e) => set("lanPort", Number(e.target.value))} />
          </div>
          <p className="field-hint">
            {t("phone_settings_hint")} <a href="/phone">{t("phone_page_link")}</a>
          </p>
        </section>

        <section>
          <h2>{t("notifications_startup")}</h2>
          <label className="settings-check">
            <input type="checkbox" checked={form.notifications} onChange={(e) => set("notifications", e.target.checked)} />
            {t("notify_when_hidden")}
          </label>
          <label className="settings-check">
            <input
              type="checkbox"
              checked={settings.autostart}
              onChange={(e) => conn.send({ type: "settings.autostart", enabled: e.target.checked })}
            />
            {t("autostart")}
          </label>
          {settings.autostartOther && (
            <p className="settings-warn">{t("autostart_other", { path: settings.autostartOther })}</p>
          )}
          <label className="settings-check">
            <input type="checkbox" checked={form.updateCheck} onChange={(e) => set("updateCheck", e.target.checked)} />
            {t("update_auto_check")}
          </label>
          <p className="field-hint">{t("update_auto_check_hint")}</p>
        </section>

        <div className="game-form-actions settings-actions">
          <button type="submit" className="btn primary" disabled={!dirty && mode === proxyMode(settings.values.proxy)}>
            {t("save")}
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
              {t("discard")}
            </button>
          )}
          {settings.error && <span className="settings-error">{settings.error}</span>}
        </div>
      </form>

      <UpdateRow update={ui.update} send={(m) => conn.send(m)} />
      <p className="settings-about">
        game-overlai {settings.version} · {t("settings_about", { config: settings.configFile, data: settings.dataDir })}
      </p>
    </div>
  );
}

/** The version line's companion: check now, or the version found with its Update button. */
function UpdateRow({ update, send }: { update: UpdateStatus | null; send(msg: ClientMsg): void }) {
  const t = useT();
  if (!update) return null;
  const s = update.state;
  const busy = s.state === "downloading" || s.state === "installing";
  const found = update.version && s.state !== "upToDate" ? update.version : null;
  return (
    <div className="settings-update">
      {found ? (
        <>
          <span>{t("update_available", { version: found })}</span>
          {update.notesUrl && (
            <a href={update.notesUrl} target="_blank" rel="noreferrer">
              {t("update_whats_new")} ↗
            </a>
          )}
          {busy ? (
            <span>{progressText(update, t)}</span>
          ) : (
            <button type="button" className="btn primary" onClick={() => send({ type: "update.install" })}>
              {update.portable ? t("update_download") : t("update_install")}
            </button>
          )}
        </>
      ) : (
        <>
          <button type="button" className="btn" disabled={s.state === "checking"} onClick={() => send({ type: "update.check" })}>
            {s.state === "checking" ? t("update_checking") : t("update_check")}
          </button>
          {s.state === "upToDate" && <span>{t("update_up_to_date")}</span>}
        </>
      )}
      {s.state === "error" && <span className="settings-error">{s.message}</span>}
      {found && update.portable && !busy && <p className="field-hint">{t("update_portable_hint")}</p>}
    </div>
  );
}
