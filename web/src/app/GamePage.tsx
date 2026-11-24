// A game's page in the browser chat: its profile (what the agent is told about the game) and
// its chats. The profile applies to the game's chats from their next question.

import { useEffect, useMemo, useRef, useState } from "react";
import { ArrowLeft, MessageSquare, Plus, Trash2, X } from "lucide-react";
import type { ChatSummary } from "../api/generated/ChatSummary";
import type { ClientMsg } from "../api/generated/ClientMsg";
import type { GameProfile } from "../api/generated/GameProfile";
import type { ProgressNote } from "../api/generated/ProgressNote";
import type { Widget } from "../api/generated/Widget";
import { htmlLang, useLanguage, useT } from "../i18n";

interface Props {
  profile: GameProfile;
  chats: ChatSummary[];
  chatId: string | null;
  onSave(profile: GameProfile): void;
  onOpenChat(id: string): void;
  onBack(): void;
  /** This game's widgets. */
  widgets: Widget[];
  /** Progress memory on (settings); null: unknown (a phone doesn't get the settings). */
  memoryOn: boolean | null;
  send(msg: ClientMsg): void;
}

interface Form {
  name: string;
  instructions: string;
  sources: string;
}

const toForm = (p: GameProfile): Form => ({
  name: p.name ?? "",
  instructions: p.instructions,
  sources: p.sources,
});

const sameForm = (a: Form, b: Form) =>
  a.name.trim() === b.name.trim() &&
  a.instructions.trim() === b.instructions.trim() &&
  a.sources.trim() === b.sources.trim();

export function GamePage({ profile, chats, chatId, onSave, onOpenChat, onBack, widgets, memoryOn, send }: Props) {
  const t = useT();
  const language = useLanguage();
  const date = useMemo(
    () => new Intl.DateTimeFormat(htmlLang(language), { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" }),
    [language],
  );
  const saved = toForm(profile);
  const [form, setForm] = useState<Form>(saved);
  const dirty = !sameForm(form, saved);

  // The stored profile changed (our save came back, or another tab saved): show it, unless the
  // user has unsaved edits here.
  const previous = useRef(saved);
  const savedKey = JSON.stringify(saved);
  useEffect(() => {
    const before = previous.current;
    previous.current = toForm(profile);
    setForm((current) => (sameForm(current, before) || sameForm(current, previous.current) ? previous.current : current));
  }, [savedKey]); // eslint-disable-line react-hooks/exhaustive-deps

  const set = <K extends keyof Form>(key: K, value: Form[K]) => setForm((f) => ({ ...f, [key]: value }));

  const save = () => {
    onSave({
      ...profile,
      name: form.name.trim() || null,
      instructions: form.instructions.trim(),
      sources: form.sources.trim(),
    });
  };

  const title = profile.name || profile.title || profile.exe;

  return (
    <div className="game-page">
      <div className="game-page-head">
        <button type="button" className="icon-btn" onClick={onBack} title={t("back_to_chat")}>
          <ArrowLeft size={16} />
        </button>
        <div className="game-page-title">
          <h1>{title}</h1>
          <span>
            {profile.exe}
            {profile.title && profile.title !== title ? ` · ${t("game_window_title", { title: profile.title })}` : ""}
          </span>
        </div>
      </div>

      <form
        className="game-form"
        onSubmit={(e) => {
          e.preventDefault();
          if (dirty) save();
        }}
      >
        <label className="field">
          <span className="field-label">{t("game_name")}</span>
          <input
            value={form.name}
            onChange={(e) => set("name", e.target.value)}
            placeholder={profile.title || profile.exe}
            maxLength={100}
          />
          <span className="field-hint">{t("game_name_hint")}</span>
        </label>

        <label className="field">
          <span className="field-label">{t("game_instructions")}</span>
          <textarea
            value={form.instructions}
            onChange={(e) => set("instructions", e.target.value)}
            rows={6}
            maxLength={4000}
            placeholder={t("game_instructions_placeholder")}
          />
          <span className="field-hint">{t("game_instructions_hint")}</span>
        </label>

        <label className="field">
          <span className="field-label">{t("game_sources")}</span>
          <textarea
            value={form.sources}
            onChange={(e) => set("sources", e.target.value)}
            rows={3}
            maxLength={1000}
            placeholder={t("game_sources_placeholder")}
          />
          <span className="field-hint">{t("game_sources_hint")}</span>
        </label>

        <div className="game-form-actions">
          <button type="submit" className="btn primary" disabled={!dirty}>
            {t("save")}
          </button>
          {dirty && (
            <button type="button" className="btn" onClick={() => setForm(saved)}>
              {t("discard")}
            </button>
          )}
          <span className="game-form-status">
            {dirty ? t("unsaved_changes") : t("game_applies_next")}
          </span>
        </div>
      </form>

      <ProgressSection exe={profile.exe} notes={profile.progress} memoryOn={memoryOn} send={send} />
      <WidgetsSection widgets={widgets} send={send} />

      <section className="game-chats">
        <h2>{t("game_chats")}{chats.length ? ` · ${chats.length}` : ""}</h2>
        {chats.length === 0 ? (
          <div className="history-empty">{t("game_no_chats")}</div>
        ) : (
          chats.map((chat) => (
            <div key={chat.id} className={`history-item${chatId === chat.id ? " active" : ""}`}>
              <button type="button" className="history-open" onClick={() => onOpenChat(chat.id)}>
                <MessageSquare size={15} />
                <span className="history-item-text">
                  <span className="history-item-title">{chat.title}</span>
                </span>
                <span className="game-chat-date">{date.format(chat.updatedAt)}</span>
              </button>
            </div>
          ))
        )}
      </section>
    </div>
  );
}

/** What the agent remembers about the user's progress in the game: editable, with an add field. */
function ProgressSection({ exe, notes, memoryOn, send }: { exe: string; notes: ProgressNote[]; memoryOn: boolean | null; send: Props["send"] }) {
  const t = useT();
  const [draft, setDraft] = useState("");
  const add = () => {
    const text = draft.trim();
    if (!text) return;
    send({ type: "progress.add", exe, text });
    setDraft("");
  };
  return (
    <section className="game-progress">
      <h2>
        {t("progress")}
        {notes.length ? ` · ${notes.length}` : ""}
      </h2>
      <p className="field-hint">{t("progress_hint")}</p>
      {memoryOn === false && <p className="settings-warn">{t("progress_off")}</p>}
      {notes.length === 0 && <div className="history-empty">{t("progress_empty")}</div>}
      <ul className="progress-list">
        {notes.map((n) => (
          <ProgressRow key={n.id} note={n} send={send} />
        ))}
      </ul>
      <form
        className="progress-add"
        onSubmit={(e) => {
          e.preventDefault();
          add();
        }}
      >
        <input value={draft} onChange={(e) => setDraft(e.target.value)} placeholder={t("progress_add_placeholder")} maxLength={300} />
        <button type="submit" className="btn" disabled={!draft.trim()}>
          <Plus size={13} aria-hidden /> {t("add")}
        </button>
      </form>
    </section>
  );
}

/** A note: edited in place, saved on Enter or when the field loses focus. */
function ProgressRow({ note, send }: { note: ProgressNote; send: Props["send"] }) {
  const t = useT();
  const [text, setText] = useState(note.text);
  useEffect(() => setText(note.text), [note.text]);
  const save = () => {
    const next = text.trim();
    if (!next) setText(note.text);
    else if (next !== note.text) send({ type: "progress.edit", id: note.id, text: next });
  };
  return (
    <li className="progress-row">
      <input
        value={text}
        maxLength={300}
        onChange={(e) => setText(e.target.value)}
        onBlur={save}
        onKeyDown={(e) => {
          if (e.key === "Enter") (e.target as HTMLInputElement).blur();
          if (e.key === "Escape") setText(note.text);
        }}
      />
      <button type="button" className="icon-btn" title={t("delete")} onClick={() => send({ type: "progress.delete", id: note.id })}>
        <X size={14} />
      </button>
    </li>
  );
}

/** The game's widgets (shown over the game window): what they hold, and deleting them. */
function WidgetsSection({ widgets, send }: { widgets: Widget[]; send: Props["send"] }) {
  const t = useT();
  return (
    <section className="game-widgets">
      <h2>
        {t("widgets")}
        {widgets.length ? ` · ${widgets.length}` : ""}
      </h2>
      <p className="field-hint">{t("widgets_hint")}</p>
      {widgets.length === 0 && <div className="history-empty">{t("widgets_empty")}</div>}
      {widgets.map((w) => {
        const done = w.items.filter((i) => i.done).length;
        return (
          <div key={w.id} className="widget-row">
            <div className="widget-row-text">
              <span className="widget-row-title">{w.title}</span>
              {w.items.length > 0 && <span className="widget-row-meta">{t("widget_items_done", { done, total: w.items.length })}</span>}
            </div>
            <button
              type="button"
              className="icon-btn"
              title={t("delete")}
              onClick={() => {
                if (confirm(t("widget_delete_confirm", { title: w.title }))) send({ type: "widget.delete", id: w.id });
              }}
            >
              <Trash2 size={14} />
            </button>
          </div>
        );
      })}
    </section>
  );
}
