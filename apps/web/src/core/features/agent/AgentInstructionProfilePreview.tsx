import type { AgentSubmissionInput } from "@research-assistant/core/agent";

const labels = { included: "已包含", truncated: "已裁剪", empty: "空指令", budgetExcluded: "预算排除" };

export function effectiveInstructionBudget(input: AgentSubmissionInput): number | null {
  const layers = input.instructionConfigs?.[0]?.layers ?? [];
  let budget = layers[0]?.instructions.max_bytes ?? null;
  for (const layer of layers.slice(1)) budget = layer.instructions.max_bytes ?? budget;
  return budget;
}

/** Read only the receipt; current configuration never replaces reviewed text. */
export function AgentInstructionProfilePreview({ input, subject }: { readonly input: AgentSubmissionInput; readonly subject: "笔记" | "论文" }): React.JSX.Element | null {
  if (!input.instructions?.length) return null;
  const budget = effectiveInstructionBudget(input);
  return <details className="note-agent-instructions">
    <summary>查看冻结的{subject}指令</summary>
    <p>这些指令仅适用于当前{subject}，不授予工具或资料访问权限。</p>
    {input.instructions.map(fragment => <section key={`${fragment.environmentId}/${fragment.sourceId}`}>
      <strong>{fragment.sourceId}</strong>
      <p>{labels[fragment.disposition]} · 原始 {fragment.originalBytes} 字节 · 最终上限 {budget ?? "默认"} 字节</p>
      <p>范围：{fragment.scopeId} · 版本：{fragment.revision}</p>
      {fragment.text ? <pre>{fragment.text}</pre> : <p>本条指令没有正文发送给模型。</p>}
    </section>)}
  </details>;
}
