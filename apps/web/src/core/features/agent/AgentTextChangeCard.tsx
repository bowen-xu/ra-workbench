import { TextPatchReview } from "./TextPatchReview";
import { useEffect } from "react";
import { notifyTextPatchesChanged } from "./textPatchGateway";
import { textPatchTargets } from "./textChangeTargets";
import { StructuredChangeCard } from "./StructuredChangeCard";
import {AgentTurnChangeSummary} from "./AgentTurnChangeSummary";
export { textPatchTargets, type TextPatchTarget } from "./textChangeTargets";
export function AgentTextChangeCard({ content, conversationId }: { readonly content: string; readonly conversationId?: string }): React.JSX.Element | null {
  useEffect(() => { if (textPatchTargets(content).length) notifyTextPatchesChanged(); }, [content]);
  const targets = textPatchTargets(content); if (!targets.length) return null;
  const structured = targets.filter(target => target.status === "applied" && target.structuredResource && target.changeId);
  const notes = targets.filter(target => !target.structuredResource && ["applied", "proposed"].includes(target.status) && target.changeId && conversationId);
  const changes = [...new Set(notes.map(target => target.changeId!))];
  const otherTargets = targets.filter(target => !structured.includes(target) && !notes.includes(target));
  return <AgentTurnChangeSummary>{otherTargets.length > 0 && <ul>{otherTargets.map(target => <li key={target.path}>
    <span>{typeof target.name === "string" ? target.name : target.path.startsWith("source://") ? "资源" : target.path} · {(target.status === "applied" && target.path.startsWith("source://agents/")) ? "已保存并立即生效" : { applied: "执行副本已修改", proposed: "笔记待接受", failed: "失败", unknown: "结果待恢复" }[target.status]}</span>
    {(target.status === "failed" || target.status === "unknown") && <p>{target.message}</p>}
  </li>)}</ul>}{structured.length > 0 && <StructuredChangeCard changes={structured.map(target => ({ changeId: target.changeId!,
    target: { ...target.structuredResource!, path: target.path }, title: target.name ?? "资源" }))} />}
    {conversationId && changes.map(changeId => <TextPatchReview key={changeId} conversationId={conversationId} changeId={changeId} presentation="chat"
      {...(typeof notes.find(target => target.changeId === changeId)?.generation === "number" ? { generation: notes.find(target => target.changeId === changeId)!.generation! } : {})} />)}</AgentTurnChangeSummary>;
}
