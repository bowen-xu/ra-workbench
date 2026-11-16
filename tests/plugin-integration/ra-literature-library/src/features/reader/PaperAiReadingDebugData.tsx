import { useMemo, useRef, useState } from "react";
import { ChevronDown } from "lucide-react";
import type { PaperAiReadingTurn } from "@research-assistant/core/ai";

import {
  aiAssistanceErrorMessage,
  type AiAssistanceGateway,
} from "../../../../../../apps/web/src/core/public/aiAssistance";

interface PaperAiReadingDebugDataProps {
  readonly aiGateway: AiAssistanceGateway;
  readonly turn: PaperAiReadingTurn;
  readonly turns: readonly PaperAiReadingTurn[];
}

interface LoadedTurnDebugRecord {
  readonly turnId: string;
  readonly data: Awaited<ReturnType<AiAssistanceGateway["getPaperReadingTurnDebugData"]>>;
}

function turnReturnData(turn: PaperAiReadingTurn): Record<string, unknown> {
  return {
    turnId: turn.id,
    sessionId: turn.sessionId,
    ordinal: turn.ordinal,
    status: turn.status,
    providerId: turn.providerId,
    modelId: turn.modelId,
    userMessage: turn.userMessage,
    assistantMessage: turn.assistantMessage,
    messages: turn.messages,
    inputHash: turn.inputHash,
    outputHash: turn.outputHash,
    inputTokens: turn.inputTokens,
    outputTokens: turn.outputTokens,
    errorCode: turn.errorCode,
    errorMessage: turn.errorMessage,
    createdAt: turn.createdAt,
    finishedAt: turn.finishedAt,
  };
}

function DebugJsonField({
  label,
  value,
}: {
  readonly label: string;
  readonly value: unknown;
}): React.JSX.Element {
  const serialized = useMemo(() => JSON.stringify(value, null, 2) ?? "null", [value]);
  return <label className="paper-ai-reading-debug-json">
    <strong>{label}</strong>
    <textarea aria-label={label} readOnly spellCheck={false} value={serialized} wrap="off" />
  </label>;
}

export function PaperAiReadingDebugData({
  aiGateway,
  turn,
  turns,
}: PaperAiReadingDebugDataProps): React.JSX.Element {
  const requestIdRef = useRef(0);
  const [record, setRecord] = useState<LoadedTurnDebugRecord>();
  const [expandedTurnId, setExpandedTurnId] = useState<string>();
  const [loading, setLoading] = useState(false);
  const [loadError, setLoadError] = useState<string>();

  async function toggle(item: PaperAiReadingTurn): Promise<void> {
    if (expandedTurnId === item.id) {
      requestIdRef.current += 1;
      setExpandedTurnId(undefined);
      setRecord(undefined);
      setLoading(false);
      setLoadError(undefined);
      return;
    }

    const requestId = requestIdRef.current + 1;
    requestIdRef.current = requestId;
    setExpandedTurnId(item.id);
    setRecord(undefined);
    setLoading(true);
    setLoadError(undefined);
    try {
      const data = await aiGateway.getPaperReadingTurnDebugData(item.id);
      if (requestIdRef.current === requestId) setRecord({ turnId: item.id, data });
    } catch (reason: unknown) {
      if (requestIdRef.current === requestId) setLoadError(aiAssistanceErrorMessage(reason));
    } finally {
      if (requestIdRef.current === requestId) setLoading(false);
    }
  }

  return <details className="paper-ai-reading-raw">
    <summary>查看返回数据（调试）</summary>
    <div className="paper-ai-reading-debug-records">
      <strong>会话调试时间线（截至第 {turn.ordinal + 1} 轮）</strong>
      {turns.map((item) => {
        const expanded = expandedTurnId === item.id;
        const contentId = `paper-ai-reading-debug-${item.id}`;
        const loadedData = record?.turnId === item.id ? record.data : undefined;
        return <div className="paper-ai-reading-debug-record" key={item.id}>
          <button
            aria-controls={contentId}
            aria-expanded={expanded}
            onClick={() => void toggle(item)}
            type="button"
          >
            <span>第 {item.ordinal + 1} 轮 · {item.providerId} · {item.modelId}</span>
            <ChevronDown aria-hidden="true" />
          </button>
          {expanded && <section id={contentId}>
            {loading && <p className="empty">正在读取第 {item.ordinal + 1} 轮调试记录…</p>}
            {loadError !== undefined && <p className="paper-ai-reading-error">{loadError}</p>}
            {loadedData !== undefined && <>
              <DebugJsonField label="原始发送记录" value={loadedData.rawRequest} />
              <DebugJsonField label="原始返回记录" value={loadedData.rawResponse} />
              <DebugJsonField label="运行元数据" value={turnReturnData(item)} />
            </>}
          </section>}
        </div>;
      })}
    </div>
  </details>;
}
