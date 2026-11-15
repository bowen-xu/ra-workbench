import { Check, Copy } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import type { WritingAgentMessage } from "./presentation/WritingAgentMessage";
import { copyText } from "./copyText";

export function AgentFailureMessage({ message, retry }: { readonly message: WritingAgentMessage; readonly retry?: {readonly label: string; readonly onClick: () => void} }): React.JSX.Element {
  const [state, setState] = useState<"idle" | "copying" | "copied" | "failed">("idle");
  const timer = useRef<number | undefined>(undefined);
  useEffect(() => () => window.clearTimeout(timer.current), []);
  async function copy(): Promise<void> {
    window.clearTimeout(timer.current);
    setState("copying");
    const value = message.failureDiagnosticText
      ? `${message.content}\n\n运行诊断：\n${message.failureDiagnosticText}` : message.content;
    const copied = await copyText(value);
    setState(copied ? "copied" : "failed");
    if (copied) timer.current = window.setTimeout(() => setState("idle"), 2000);
  }
  const label = state === "copied" ? "已复制" : state === "failed" ? "复制失败，请重试"
    : state === "copying" ? "正在复制…" : "复制诊断信息";
  return <>
    <p>{message.content}</p>
    <div className="agent-failure-actions" data-agent-message-actions>
      {retry && <button type="button" onClick={retry.onClick}>{retry.label}</button>}
      <button type="button" aria-label="复制诊断信息" disabled={state === "copying"} onClick={() => void copy()}>
        {state === "copied" ? <Check size={14} aria-hidden="true" /> : <Copy size={14} aria-hidden="true" />}
        <span aria-live="polite">{label}</span>
      </button>
    </div>
  </>;
}

/** A transient local failure uses the same presentation without writing history. */
export function AgentTransientFailure({ id, content, retry, diagnosticText }: {readonly id: string; readonly content: string; readonly retry: () => void; readonly diagnosticText?: string}): React.JSX.Element {
  return <article className="writing-agent-message assistant failure" role="alert">
    <div className="writing-agent-message-bubble" data-selectable-text>
      <AgentFailureMessage message={{id, role: "assistant", content, failure: true, ...(diagnosticText ? {failureDiagnosticText: diagnosticText} : {})}} retry={{label: "重新读取", onClick: retry}} />
    </div>
  </article>;
}
