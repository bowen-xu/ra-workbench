import { notify } from "../../../../../../apps/web/src/core/public/feedback";
import { useEffect, useRef, useState } from "react";
import type {
  AiAttentionFeedback,
  AiAttentionFeedbackKind,
  AiCandidate,
} from "@research-assistant/core/ai";

import type { AiAssistanceGateway } from "../../../../../../apps/web/src/core/public/aiAssistance";
import { aiAssistanceErrorMessage } from "../../../../../../apps/web/src/core/public/aiAssistance";

interface AiAttentionFeedbackControlsProps {
  readonly candidate: AiCandidate;
  readonly gateway: AiAssistanceGateway;
  readonly suggestionId: string;
}

const feedbackOptions: readonly {
  readonly kind: AiAttentionFeedbackKind;
  readonly label: string;
}[] = [
  { kind: "useful", label: "帮助我注意到" },
  { kind: "not_useful", label: "没有帮助" },
  { kind: "dismissed", label: "暂时忽略" },
];

function latestFeedback(
  records: readonly AiAttentionFeedback[],
  candidateRevision: number,
  suggestionId: string,
): AiAttentionFeedback | undefined {
  return [...records].reverse().find((record) =>
    record.candidateRevision === candidateRevision && record.suggestionId === suggestionId);
}

export function AiAttentionFeedbackControls({
  candidate,
  gateway,
  suggestionId,
}: AiAttentionFeedbackControlsProps): React.JSX.Element {
  const [records, setRecords] = useState(candidate.attentionFeedback);
  const [busy, setBusy] = useState(false);
  const idempotencyKeys = useRef(new Map<AiAttentionFeedbackKind, string>());

  useEffect(() => {
    setRecords(candidate.attentionFeedback);

    idempotencyKeys.current.clear();
  }, [candidate.id, candidate.currentRevision, candidate.attentionFeedback]);

  const latest = latestFeedback(records, candidate.currentRevision, suggestionId);

  async function record(kind: AiAttentionFeedbackKind): Promise<void> {
    setBusy(true);

    let idempotencyKey = idempotencyKeys.current.get(kind);
    if (idempotencyKey === undefined) {
      idempotencyKey = `${candidate.id}:${candidate.currentRevision}:${suggestionId}:${kind}:${globalThis.crypto.randomUUID()}`;
      idempotencyKeys.current.set(kind, idempotencyKey);
    }
    try {
      const saved = await gateway.recordAttentionFeedback({
        candidateId: candidate.id,
        candidateRevision: candidate.currentRevision,
        suggestionId,
        feedbackKind: kind,
        note: "",
        idempotencyKey,
      });
      setRecords((current) => current.some((item) => item.id === saved.id)
        ? current : [...current, saved]);
      idempotencyKeys.current.delete(kind);
    } catch (reason) {
      notify.error(aiAssistanceErrorMessage(reason));
    } finally {
      setBusy(false);
    }
  }

  return <div className="ai-attention-feedback">
    <span>注意力反馈（不改变知识或审查状态）</span>
    <div role="group" aria-label="这条 AI 建议是否帮助你注意到重要内容">
      {feedbackOptions.map((option) => <button
        aria-pressed={latest?.feedbackKind === option.kind}
        disabled={busy || latest?.feedbackKind === option.kind}
        key={option.kind}
        onClick={() => void record(option.kind)}
        type="button"
      >{option.label}</button>)}
    </div>
  </div>;
}
