import type { DocumentConflict } from "@research-assistant/core/hbs";
import { AgentRecoveryMessage } from "../AgentRecoveryMessage";

function selection(value: unknown): string {
  if (Array.isArray(value)) return value.length ? value.map(selection).join("；") : "无";
  if (value && typeof value === "object") {
    const row = value as Record<string, unknown>, resource = row.resource && typeof row.resource === "object" ? row.resource as Record<string, unknown> : row;
    const label = typeof row.label === "string" ? row.label : typeof resource.resourceId === "string" ? resource.resourceId : "上下文项";
    const access = row.access ?? resource.access;
    const permissions = access === "readWrite" ? "可编辑" : access === "readOnly" ? "只读" : undefined;
    const enabled = typeof row.selected === "boolean" ? (row.selected ? "已选择" : "未选择") : undefined;
    const parts = Array.isArray(resource.parts) ? selection(resource.parts) : undefined;
    return [label, enabled, permissions, parts].filter(Boolean).join(" · ");
  }
  return value === null || value === undefined ? "无" : String(value);
}

export function ContextConfigurationRecovery({ conversationId, recovery, detail, conflicts, onRetry, onRepair, onResolve }: {
  readonly conversationId: string; readonly recovery: "seed" | "save" | "local" | "conflict" | "error";
  readonly detail?: string | undefined; readonly conflicts: readonly DocumentConflict[];
  readonly onRetry: () => void; readonly onRepair: () => void; readonly onResolve: (id: string, action: "incoming" | "keep") => void;
}): React.JSX.Element {
  const text = recovery === "seed" ? "上下文初始化结果尚未确认，原请求已保留。"
    : recovery === "save" ? "上下文保存结果尚未确认，原请求与后续选择已保留。"
    : recovery === "local" ? "上下文本地保全失败，当前选择已保留，请修复后继续。"
    : recovery === "conflict" ? "上下文在其他窗口发生变更，请逐项核对。" : "上下文暂时无法恢复，请核对后重试。";
  const diagnostic = [`C=${conversationId}`, detail].filter(Boolean).join("；");
  return <AgentRecoveryMessage id="context-configuration-recovery" label="上下文配置恢复" content={`${text} 新发送和自动队列已暂停。`} diagnosticText={diagnostic}
      {...(recovery === "conflict" ? {} : {retry: {label: recovery === "local" ? "重试保存" : "核对并重试", onClick: recovery === "local" ? onRepair : onRetry}})}>
    {recovery === "conflict" ? conflicts.map(conflict => <div key={conflict.id}>
      <p>{conflict.path[0] === "persistent" ? "持续上下文" : conflict.path[0] === "temporary" ? "临时上下文" : "上下文"}</p>
      <p>当前选择：{selection(conflict.after)}</p><p>其他更新：{selection(conflict.incoming)}</p>
      <div className="agent-failure-actions"><button type="button" onClick={() => onResolve(conflict.id, "keep")}>保留当前选择</button>
      <button type="button" onClick={() => onResolve(conflict.id, "incoming")}>采用其他更新</button></div>
    </div>) : null}
  </AgentRecoveryMessage>;
}
