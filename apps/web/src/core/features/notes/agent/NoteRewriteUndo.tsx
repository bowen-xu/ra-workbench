import { ControlHint } from "../../../components/ControlHint";
import { useEffect, useRef, useState } from "react";
import type { AgentNoteApplication, AgentNoteUndo } from "@research-assistant/core/agent";
import { agentNoteUndoGateway } from "../../../services/agentGateway";
import { notify } from "../../../services/notifications";
import { useAgentRequestGuard } from "../../agent/useAgentMountedGuard";

export function NoteRewriteUndo({application, currentRevision, isSaved, blocked, onUndo}: {
  readonly application: AgentNoteApplication;
  readonly currentRevision: number | undefined;
  readonly isSaved: boolean;
  readonly blocked: boolean;
  readonly onUndo: (application: AgentNoteApplication) => Promise<void>;
}) {
  const [phase, setPhase] = useState<"loading" | "ready" | "failed" | "undoing">("loading");
  const [receipt, setReceipt] = useState<AgentNoteUndo | null>(null);
  const [refresh, setRefresh] = useState(0);
  const busy = useRef(false);
  const beginRequest = useAgentRequestGuard();
  useEffect(() => {
    const active = beginRequest();
    setPhase("loading"); setReceipt(null); busy.current = false;
    void agentNoteUndoGateway.status(application).then(value => {
      if (active()) { setReceipt(value); setPhase("ready"); }
    }, () => {
      if (active()) { setPhase("failed"); notify.error("无法读取撤销状态，请重试。"); }
    });
  }, [application, beginRequest, refresh, blocked]);
  const stale = !isSaved || currentRevision !== application.appliedRevision;
  async function undo() {
    if (busy.current || phase !== "ready" || receipt || stale || blocked) return;
    busy.current = true; setPhase("undoing"); const active = beginRequest();
    try { await onUndo(application); }
    catch { /* Workspace retains the original operation and recovery controls. */ }
    finally { if (active()) { busy.current = false; setRefresh(value => value + 1); } }
  }
  if (phase === "loading") return <p role="status">正在读取撤销状态…</p>;
  if (phase === "failed") return <button type="button" onClick={() => setRefresh(value => value + 1)}>重试读取撤销状态</button>;
  if (receipt) return <p role="status">已撤销至版本 {receipt.restoredRevision} · <time dateTime={receipt.createdAt}>{new Date(receipt.createdAt).toLocaleString()}</time>。这是历史记录，查看不会重写正文。</p>;
  return <div>
    <p role="status">{blocked ? "当前修改结果尚待核对，请使用工作区恢复入口。" : stale ? "笔记已有后续编辑或尚未保存，不能直接撤销原提案。" : "可撤销这次改写，保留原应用记录。"}</p>
    <ControlHint content={stale || blocked ? "需要匹配原应用版本并先核对在途操作" : "恢复改写前正文并保存为新版本"}><button type="button" disabled={phase !== "ready" || stale || blocked} onClick={() => void undo()}
      >{phase === "undoing" ? "正在撤销…" : "撤销这次改写"}</button></ControlHint>
  </div>;
}
