// The model and reasoning effort of the agent, as the agent offers them: a button with the
// current choice in the input row and a menu above it. The choice applies to the current chat
// at once and to later chats (the core remembers it).

import { useEffect, useRef, useState } from "react";
import { Check, ChevronUp } from "lucide-react";
import type { AgentOption } from "../api/generated/AgentOption";

interface Props {
  options: AgentOption[];
  /** Shown when the agent offers no choices. */
  agentName: string;
  onSet(id: string, value: string): void;
  /** Overlay, compact panel: make room for the menu first. */
  onBeforeOpen?: () => void;
}

const TITLES = { model: "Модель", effort: "Размышление" } as const;

/** Russian names of the usual effort levels; others keep the agent's name. */
const EFFORT_NAMES: Record<string, string> = {
  default: "По умолчанию",
  low: "Низкое",
  medium: "Среднее",
  high: "Высокое",
  xhigh: "Очень высокое",
  max: "Максимальное",
};

/** Long lists (Claude lists every model version): the first ones, the rest on demand. */
const SHORT_LIST = 5;

const valueName = (o: AgentOption, value: string) =>
  (o.kind === "effort" ? EFFORT_NAMES[value] : undefined) ?? o.values.find((v) => v.value === value)?.name ?? value;

/** "Default (recommended)" -> "Default" for the button. */
const short = (name: string) => name.replace(/\s*\(.*\)\s*$/, "").trim() || name;

export function ModelPicker({ options, agentName, onSet, onBeforeOpen }: Props) {
  const [open, setOpen] = useState(false);
  const [all, setAll] = useState(false);
  const box = useRef<HTMLDivElement>(null);

  // Closes on a click elsewhere and on Esc (which must not also hide the overlay).
  useEffect(() => {
    if (!open) return;
    const onDown = (e: PointerEvent) => {
      if (!box.current?.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.preventDefault();
      e.stopImmediatePropagation();
      setOpen(false);
    };
    window.addEventListener("pointerdown", onDown, true);
    window.addEventListener("keydown", onKey, true);
    return () => {
      window.removeEventListener("pointerdown", onDown, true);
      window.removeEventListener("keydown", onKey, true);
    };
  }, [open]);

  if (options.length === 0) {
    return (
      <span className="agent" title="Модель">
        {agentName}
      </span>
    );
  }

  // "Sonnet 5.5 · Высокое"; the default effort is not worth the space.
  const label = options
    .filter((o) => !(o.kind === "effort" && o.current === "default"))
    .map((o) => short(valueName(o, o.current)))
    .join(" · ");

  return (
    <div className="model-picker" ref={box}>
      <button
        type="button"
        className={`model-btn ${open ? "open" : ""}`}
        onClick={() => {
          if (!open) onBeforeOpen?.();
          setAll(false);
          setOpen(!open);
        }}
        title="Модель и глубина размышления"
        aria-haspopup="menu"
        aria-expanded={open}
      >
        <span className="model-btn-text">{label}</span>
        <ChevronUp size={12} aria-hidden />
      </button>
      {open && (
        <div className="model-menu" role="menu">
          {options.map((o) => (
            <OptionList key={o.id} option={o} all={all} onMore={() => setAll(true)} onSet={onSet} />
          ))}
          <p className="model-menu-note">Для этого чата сразу и для следующих чатов.</p>
        </div>
      )}
    </div>
  );
}

interface ListProps {
  option: AgentOption;
  all: boolean;
  onMore(): void;
  onSet(id: string, value: string): void;
}

function OptionList({ option: o, all, onMore, onSet }: ListProps) {
  const cut = o.values.length > SHORT_LIST + 1 && !all;
  const shown = cut ? o.values.filter((v, i) => i < SHORT_LIST || v.value === o.current) : o.values;
  return (
    <section>
      <div className="model-menu-title">{TITLES[o.kind] ?? o.name}</div>
      {shown.map((v) => (
        <button
          key={v.value}
          type="button"
          role="menuitemradio"
          aria-checked={v.value === o.current}
          className={`model-item ${v.value === o.current ? "on" : ""}`}
          onClick={() => v.value !== o.current && onSet(o.id, v.value)}
        >
          <span className="model-item-text">
            <span>{valueName(o, v.value)}</span>
            {v.description && <small>{v.description}</small>}
          </span>
          {v.value === o.current && <Check size={14} aria-hidden />}
        </button>
      ))}
      {cut && (
        <button type="button" className="model-item more" onClick={onMore}>
          Другие версии ({o.values.length - shown.length})
        </button>
      )}
    </section>
  );
}
