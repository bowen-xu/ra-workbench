import { type CSSProperties } from "react";
import { ControlHint } from "../../../public/overlay";

export type WritingAgentContextScope = "current-file" | "manuscript";

import type { AgentContextUsage as WritingAgentContextUsage } from "../../../contracts/agentContext";
export type { AgentContextUsage as WritingAgentContextUsage } from "../../../contracts/agentContext";

interface WritingAgentContextControlProps {
  readonly disabled?: boolean;
  readonly usage?: WritingAgentContextUsage;
}

function formatTokens(value: number): string {
  return `${Math.round(value / 1000)}k`;
}

export function WritingAgentContextControl({
  disabled = false,
  usage,
}: WritingAgentContextControlProps): React.JSX.Element {
  const percentage = usage?.contextWindowTokens === undefined
    ? 0
    : Math.min(100, Math.max(0, Math.round((usage.inputTokens / usage.contextWindowTokens) * 100)));

  return <div className="writing-agent-context">
    <ControlHint content={<div className="writing-agent-context-tooltip">
      <strong>上下文窗口</strong>
      {usage === undefined
        ? <strong>用量尚不可用</strong>
        : <><strong>{usage.contextWindowTokens === undefined ? "窗口大小未知" : usage.measurement === "stale" ? `最近采样 ${percentage}%` : `${percentage}% 已使用`}</strong><span>{formatTokens(usage.inputTokens)} / {usage.contextWindowTokens === undefined ? "?" : formatTokens(usage.contextWindowTokens)} tokens</span>{usage.measurement && <span>{usage.measurement === "stale" ? "最近一次采样" : "估算用量"}</span>}</>}
    </div>}>
    <button
      aria-label="Agent 上下文用量"
      className="writing-agent-context-trigger"
      disabled={disabled}
      type="button"
    ><span
      aria-hidden="true"
      className="writing-agent-context-ring"
      style={{ "--writing-agent-context-progress": `${percentage * 3.6}deg` } as CSSProperties}
    /></button></ControlHint>
  </div>;
}
