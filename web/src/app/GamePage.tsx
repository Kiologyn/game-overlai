// A game's page in the browser chat: its profile (what the agent is told about the game) and
// its chats. The profile applies to the game's chats from their next question.

import { useEffect, useRef, useState } from "react";
import { ArrowLeft, MessageSquare } from "lucide-react";
import type { ChatSummary } from "../api/generated/ChatSummary";
import type { GameProfile } from "../api/generated/GameProfile";

interface Props {
  profile: GameProfile;
  chats: ChatSummary[];
  chatId: string | null;
  onSave(profile: GameProfile): void;
  onOpenChat(id: string): void;
  onBack(): void;
}

type ShotChoice = "panel" | "on" | "off";

interface Form {
  name: string;
  instructions: string;
  sources: string;
  screenshot: ShotChoice;
}

const toForm = (p: GameProfile): Form => ({
  name: p.name ?? "",
  instructions: p.instructions,
  sources: p.sources,
  screenshot: p.screenshot === null ? "panel" : p.screenshot ? "on" : "off",
});

const sameForm = (a: Form, b: Form) =>
  a.name.trim() === b.name.trim() &&
  a.instructions.trim() === b.instructions.trim() &&
  a.sources.trim() === b.sources.trim() &&
  a.screenshot === b.screenshot;

const DATE = new Intl.DateTimeFormat("ru-RU", { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" });

export function GamePage({ profile, chats, chatId, onSave, onOpenChat, onBack }: Props) {
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
      screenshot: form.screenshot === "panel" ? null : form.screenshot === "on",
    });
  };

  const title = profile.name || profile.title || profile.exe;

  return (
    <div className="game-page">
      <div className="game-page-head">
        <button type="button" className="icon-btn" onClick={onBack} title="К чату">
          <ArrowLeft size={16} />
        </button>
        <div className="game-page-title">
          <h1>{title}</h1>
          <span>
            {profile.exe}
            {profile.title && profile.title !== title ? ` · окно «${profile.title}»` : ""}
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
          <span className="field-label">Название</span>
          <input
            value={form.name}
            onChange={(e) => set("name", e.target.value)}
            placeholder={profile.title || profile.exe}
            maxLength={100}
          />
          <span className="field-hint">Если заголовок окна не похож на название игры, агент будет знать, о какой игре речь.</span>
        </label>

        <label className="field">
          <span className="field-label">Указания агенту</span>
          <textarea
            value={form.instructions}
            onChange={(e) => set("instructions", e.target.value)}
            rows={6}
            maxLength={4000}
            placeholder={"Например:\nбез спойлеров дальше 3-й главы\nя играю за мага\nотвечай по-английски"}
          />
          <span className="field-hint">Агент получает их в каждом чате этой игры.</span>
        </label>

        <label className="field">
          <span className="field-label">Где искать</span>
          <textarea
            value={form.sources}
            onChange={(e) => set("sources", e.target.value)}
            rows={3}
            maxLength={1000}
            placeholder={"Сайты, по одному в строке:\nfextralife.com\nwiki.example.org"}
          />
          <span className="field-hint">Агент сначала ищет на этих сайтах, остальные — если там нет ответа.</span>
        </label>

        <label className="field">
          <span className="field-label">Кадр игры (📷) при открытии панели</span>
          <select value={form.screenshot} onChange={(e) => set("screenshot", e.target.value as ShotChoice)}>
            <option value="panel">как было в панели</option>
            <option value="on">прикладывать</option>
            <option value="off">не прикладывать</option>
          </select>
        </label>

        <div className="game-form-actions">
          <button type="submit" className="btn primary" disabled={!dirty}>
            Сохранить
          </button>
          {dirty && (
            <button type="button" className="btn" onClick={() => setForm(saved)}>
              Отменить
            </button>
          )}
          <span className="game-form-status">
            {dirty ? "Есть несохранённые изменения" : "Применяется к чатам игры со следующего вопроса"}
          </span>
        </div>
      </form>

      <section className="game-chats">
        <h2>Чаты{chats.length ? ` · ${chats.length}` : ""}</h2>
        {chats.length === 0 ? (
          <div className="history-empty">Чатов по этой игре пока нет</div>
        ) : (
          chats.map((chat) => (
            <div key={chat.id} className={`history-item${chatId === chat.id ? " active" : ""}`}>
              <button type="button" className="history-open" onClick={() => onOpenChat(chat.id)}>
                <MessageSquare size={15} />
                <span className="history-item-text">
                  <span className="history-item-title">{chat.title}</span>
                </span>
                <span className="game-chat-date">{DATE.format(chat.updatedAt)}</span>
              </button>
            </div>
          ))
        )}
      </section>
    </div>
  );
}
