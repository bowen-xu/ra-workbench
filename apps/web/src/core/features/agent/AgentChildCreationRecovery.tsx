import type { ChildCreationRecoveryIdentity } from "../../contracts/agentMessage";
import { useEffect, useRef, useState } from "react";
import type { AgentChildCreationStatusGateway } from "@research-assistant/data/agent";
import { decodeAgentSqlImmediateFailure, type AgentChildCreationStatusResponse } from "@research-assistant/core/agent";
import { agentChildCreationStatusGateway } from "../../services/agentGateway";
import { diagnosticError } from "../../services/diagnosticError";
import { AgentRecoveryMessage } from "./AgentRecoveryMessage";
import "./AgentChildCreationRecovery.css";

export type { ChildCreationRecoveryIdentity } from "../../contracts/agentMessage";

/** Inspection never retries the tool or removes the durable recovery barrier. */
export function AgentChildCreationRecovery({ identity, gateway = agentChildCreationStatusGateway, diagnosticText, embedded = false, failureStage }:
  { readonly identity: ChildCreationRecoveryIdentity; readonly gateway?: AgentChildCreationStatusGateway;
    readonly diagnosticText?: string | undefined; readonly embedded?: boolean; readonly failureStage?: "creation" | "cleanup" | undefined }): React.JSX.Element {
  return <Recovery key={JSON.stringify(identity)} identity={identity} gateway={gateway} diagnosticText={diagnosticText} embedded={embedded} failureStage={failureStage} />;
}

function Recovery({ identity, gateway, diagnosticText, embedded, failureStage }: { readonly identity: ChildCreationRecoveryIdentity;
  readonly gateway: AgentChildCreationStatusGateway; readonly diagnosticText?: string | undefined; readonly embedded: boolean; readonly failureStage?: "creation" | "cleanup" | undefined }): React.JSX.Element {
  const [state, setState] = useState<{ loading: boolean; result?: AgentChildCreationStatusResponse; error?: string }>({loading: false});
  const generation = useRef(0);
  useEffect(() => { generation.current += 1; setState({loading: false}); return () => { generation.current += 1; }; }, [gateway]);
  async function inspect(): Promise<void> {
    const revision = ++generation.current;
    setState({loading: true});
    try {
      const result = await gateway.read({schemaVersion: 1, conversationId: identity.conversationId, turnId: identity.turnId, callId: identity.callId});
      if (generation.current !== revision) return;
      if (result.watermark < identity.watermark || result.creation.conversationId !== identity.childId || result.creation.operationId !== identity.operationId
        || (result.cleanup && result.cleanup.conversationId !== identity.childId)
        || (result.submission && result.submission.requestId !== identity.submissionRequestId)) {
        setState({loading: false, error: "原调用身份或历史水位不匹配，请重新打开对话后核对。 [CHILD_CREATION_IDENTITY_MISMATCH]"});
        return;
      }
      setState({loading: false, result});
    } catch (error) {
      if (generation.current !== revision) return;
      const code = error && typeof error === "object" && "code" in error && typeof error.code === "string"
        && /^[a-zA-Z][a-zA-Z0-9_-]{0,100}$/u.test(error.code) ? error.code : "CHILD_CREATION_STATUS_UNAVAILABLE";
      const nested = error && typeof error === "object" && "reason" in error ? error.reason : undefined;
      const storage = decodeAgentSqlImmediateFailure(nested);
      const detail = storage?.diagnostic?.operationId && ![identity.operationId, identity.submissionRequestId].includes(storage.diagnostic.operationId) ? undefined : storage;
      const safe = diagnosticError(error), fields = [code, `conversation=${identity.conversationId}`, `turn=${identity.turnId}`, `call=${identity.callId}`, `operation=${identity.operationId}`];
      if (detail) for (const value of [detail.code, detail.diagnostic?.phase, detail.diagnostic?.safeCause, detail.diagnostic?.sqliteCode, detail.diagnostic?.sqliteExtendedCode])
        if (value !== null && value !== undefined) fields.push(String(value));
      if (typeof safe.httpStatus === "number" && Number.isInteger(safe.httpStatus) && safe.httpStatus >= 100 && safe.httpStatus <= 599) fields.push(`HTTP ${safe.httpStatus}`);
      setState({loading: false, error: `无法核对子任务创建状态，请稍后重试。 [${fields.join("; ")}]`});
    }
  }
  const status = state.result?.creation.status;
  const cleanup = state.result?.cleanup?.status;
  const submission = state.result?.submission;
  return <AgentRecoveryMessage id="child-creation-recovery" label={failureStage === "cleanup" ? "子任务清理恢复" : "子任务创建恢复"}
    content={state.error ? "无法核对子任务状态，请稍后重试。" : failureStage === "cleanup" ? "子任务清理结果尚待核对，原操作已保留；请勿重复执行。" : "子任务创建结果尚待核对，原操作已保留；请勿重复执行。"}
    diagnosticText={[`conversation=${identity.conversationId}`, `turn=${identity.turnId}`, `call=${identity.callId}`, `operation=${identity.operationId}`, diagnosticText, state.error].filter(Boolean).join("；")}
    embedded={embedded}>
    <div className="agent-failure-actions"><button type="button" disabled={state.loading} onClick={() => void inspect()}>{state.loading ? "正在核对…" : failureStage === "cleanup" ? "核对子任务状态" : "核对子任务创建"}</button></div>
    {status && <p role="status">{status.type === "committed" ? "已找到原创建回执。" : status.type === "noSqlReceipt"
      ? "未找到原创建回执。" : `创建回执仍无法确认。（${status.reason}）`}
      {submission && (submission.record ? "当前保留子任务的原提交记录。" : "当前未找到子任务的原提交记录。")}
      {cleanup && (cleanup.type === "committed" ? "已找到原清理归档回执。" : cleanup.type === "noSqlReceipt"
        ? "未找到原清理归档回执。" : `清理归档回执仍无法确认。（${cleanup.reason}）`)}
      {cleanup || submission ? "这些查询只反映对应阶段的当前状态；" : "这只反映创建阶段；"}请勿重复发送或重试子任务，对话仍需恢复。</p>}
  </AgentRecoveryMessage>;
}
