import { useEffect, useRef } from "react";
import {
  Brain,
  Camera,
  CameraOff,
  Check,
  ChevronRight,
  Globe,
  LoaderCircle,
  Search,
  ShieldQuestion,
  Wrench,
  X,
} from "lucide-react";
import type { ChatMessage } from "../api/generated/ChatMessage";
import type { Part } from "../api/generated/Part";
import type { PermissionPrompt } from "../api/generated/PermissionPrompt";
import type { ToolInfo } from "../api/generated/ToolInfo";
import { Markdown } from "./Markdown";
import { ShotThumb } from "./Shots";
import { useT, type T } from "../i18n";

/** Our MCP tool (src-tauri/src/mcp.rs). */
const SCREENSHOT_TOOL = "screenshot_game";

interface Props {
  messages: ChatMessage[];
  permissions: PermissionPrompt[];
  onPermission(id: string, optionId: string | null): void;
  emptyHint: string;
}

export function MessageList({ messages, permissions, onPermission, emptyHint }: Props) {
  const ref = useRef<HTMLDivElement>(null);
  const stick = useRef(true);

  // Follow the stream unless the user scrolled up to read.
  const onScroll = () => {
    const el = ref.current;
    if (el) stick.current = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
  };
  useEffect(() => {
    const el = ref.current;
    if (el && stick.current) el.scrollTop = el.scrollHeight;
  }, [messages, permissions]);

  return (
    <div className="messages" ref={ref} onScroll={onScroll}>
      {messages.length === 0 ? (
        <div className="empty">{emptyHint}</div>
      ) : (
        messages.map((m) => (m.role === "user" ? <UserMessage key={m.id} m={m} /> : <AssistantMessage key={m.id} m={m} />))
      )}
      {permissions.map((p) => (
        <PermissionCard key={p.id} prompt={p} onRespond={onPermission} />
      ))}
    </div>
  );
}

function UserMessage({ m }: { m: ChatMessage }) {
  const t = useT();
  const text = m.parts.map((p) => (p.type === "text" ? p.text : "")).join("");
  return (
    <div className="msg user">
      {m.screenshot && <ShotThumb shot={m.screenshot} width={200} label={t("shot_for_question")} />}
      <div className="bubble">{text}</div>
      {m.error && (
        <div className="note with-icon">
          <CameraOff size={12} aria-hidden />
          {m.error}
        </div>
      )}
    </div>
  );
}

function AssistantMessage({ m }: { m: ChatMessage }) {
  const t = useT();
  const streaming = m.status === "streaming";
  const hasText = m.parts.some((p) => p.type === "text" && p.text.trim());
  return (
    <div className="msg assistant">
      {m.parts.map((p, i) => (
        <PartView key={i} part={p} live={streaming && i === m.parts.length - 1} />
      ))}
      {streaming && !hasText && <Typing />}
      {m.status === "cancelled" && <div className="note">{t("stopped")}</div>}
      {m.status === "error" && m.error && <div className="error-box">{m.error}</div>}
    </div>
  );
}

function PartView({ part, live }: { part: Part; live: boolean }) {
  switch (part.type) {
    case "text":
      return <Markdown text={part.text} />;
    case "thought":
      return <Thought text={part.text} live={live} />;
    case "tool":
      return <ToolChip tool={part.tool} />;
  }
}

function Thought({ text, live }: { text: string; live: boolean }) {
  const t = useT();
  return (
    <details className="thought">
      <summary>
        <ChevronRight size={14} className="chev" aria-hidden />
        <Brain size={13} aria-hidden />
        {live ? t("thinking") : t("thoughts")}
      </summary>
      <div className="thought-text">{text}</div>
    </details>
  );
}

function toolIcon(tool: ToolInfo) {
  if (tool.title === SCREENSHOT_TOOL) return <Camera size={13} aria-hidden />;
  if (tool.kind === "search") return <Search size={13} aria-hidden />;
  if (tool.kind === "fetch") return <Globe size={13} aria-hidden />;
  return <Wrench size={13} aria-hidden />;
}

function toolLabel(tool: ToolInfo, t: T) {
  if (tool.title === SCREENSHOT_TOOL) return t("tool_screenshot");
  return tool.title || t("tool");
}

function ToolChip({ tool }: { tool: ToolInfo }) {
  const t = useT();
  const state =
    tool.status === "done" ? (
      <Check size={13} className="ok" aria-label={t("tool_done")} />
    ) : tool.status === "failed" ? (
      <X size={13} className="bad" aria-label={t("tool_failed")} />
    ) : (
      <LoaderCircle size={13} className="spin" aria-label={t("tool_running")} />
    );
  return (
    <>
      <div className="tool" title={tool.title}>
        {toolIcon(tool)}
        <span className="tool-title">{toolLabel(tool, t)}</span>
        {state}
      </div>
      {tool.shot && <ShotThumb shot={tool.shot} width={160} label={t("shot_seen_by_agent")} />}
    </>
  );
}

function Typing() {
  const t = useT();
  return (
    <div className="typing" aria-label={t("model_answering")}>
      <span />
      <span />
      <span />
    </div>
  );
}

function PermissionCard({ prompt, onRespond }: { prompt: PermissionPrompt; onRespond: Props["onPermission"] }) {
  const t = useT();
  return (
    <div className="permission">
      <div className="permission-title">
        <ShieldQuestion size={15} aria-hidden />
        <span>
          {t("permission_request")} <b>{prompt.title}</b>
        </span>
      </div>
      <div className="permission-actions">
        {prompt.options.map((o) => (
          <button
            key={o.id}
            type="button"
            className={`btn ${o.kind.startsWith("allow") ? "primary" : ""}`}
            onClick={() => onRespond(prompt.id, o.id)}
          >
            {o.name}
          </button>
        ))}
      </div>
    </div>
  );
}
