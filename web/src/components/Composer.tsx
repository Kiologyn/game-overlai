import type { KeyboardEvent, RefObject } from "react";
import { ArrowUp, Camera, CameraOff, Gamepad2, GripVertical, Square, X } from "lucide-react";
import type { GameInfo } from "../api/generated/GameInfo";

interface Props {
  value: string;
  onChange(value: string): void;
  onSubmit(): void;
  /** An answer is being generated: the send button becomes "stop". */
  busy: boolean;
  onStop(): void;
  /** Overlay only: hide the panel. */
  onClose?: () => void;
  screenshot: boolean;
  onScreenshotChange(on: boolean): void;
  game: GameInfo | null;
  /** Shown name of the game (the profile's name if the user gave one). */
  gameName?: string;
  /** Click on the game chip: open the game's profile. */
  onOpenGame?: () => void;
  agentName: string;
  placeholder: string;
  inputRef?: RefObject<HTMLInputElement | null>;
  /** Overlay, compact panel: a grip to move the panel by. */
  onDragStart?: () => void;
}

export function Composer(props: Props) {
  const { value, onChange, onSubmit, busy, onStop, onClose, screenshot, onScreenshotChange, game, agentName } = props;

  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter" && !e.nativeEvent.isComposing) {
      e.preventDefault();
      if (!busy) onSubmit();
    }
  };

  return (
    <div className="composer">
      {props.onDragStart && (
        <span
          className="drag-grip"
          title="Перетащить панель"
          onPointerDown={(e) => {
            if (e.button !== 0) return;
            e.preventDefault();
            props.onDragStart?.();
          }}
        >
          <GripVertical size={14} aria-hidden />
        </span>
      )}
      {game && (() => {
        const label = props.gameName || game.title || game.exe;
        const content = (
          <>
            <Gamepad2 size={14} aria-hidden />
            <span className="chip-text">{label}</span>
          </>
        );
        const about = `${game.title}${game.exe ? ` (${game.exe})` : ""}`;
        return props.onOpenGame && game.exe ? (
          <button type="button" className="chip game clickable" onClick={props.onOpenGame} title={`${about} — профиль игры`}>
            {content}
          </button>
        ) : (
          <span className="chip game" title={about}>
            {content}
          </span>
        );
      })()}
      <button
        type="button"
        className={`icon-btn toggle ${screenshot ? "on" : ""}`}
        onClick={() => onScreenshotChange(!screenshot)}
        title={screenshot ? "Кадр игры прикладывается к вопросу — выключить" : "Без кадра игры — включить"}
        aria-label="Прикладывать кадр игры"
        aria-pressed={screenshot}
      >
        {screenshot ? <Camera size={16} /> : <CameraOff size={16} />}
      </button>
      <input
        ref={props.inputRef}
        className="input"
        value={value}
        onChange={(e) => onChange(e.target.value)}
        onKeyDown={onKeyDown}
        placeholder={props.placeholder}
        autoComplete="off"
        spellCheck={false}
      />
      <span className="agent" title="Модель">
        {agentName}
      </span>
      {busy ? (
        <button type="button" className="icon-btn send stop" onClick={onStop} title="Остановить ответ">
          <Square size={12} fill="currentColor" />
        </button>
      ) : (
        <button type="button" className="icon-btn send" onClick={onSubmit} disabled={!value.trim()} title="Отправить (Enter)">
          <ArrowUp size={16} />
        </button>
      )}
      {onClose && (
        <button type="button" className="icon-btn close" onClick={onClose} title="Скрыть (Esc)">
          <X size={16} />
        </button>
      )}
    </div>
  );
}
