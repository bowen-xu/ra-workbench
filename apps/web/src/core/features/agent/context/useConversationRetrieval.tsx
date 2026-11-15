import { useNotificationState } from "../../../services/useNotificationState";
import { useEffect, useMemo, useRef, useState } from "react";
import { getAiAssistanceGateway } from "../../../services/aiAssistanceGateway";
import { PaperConceptRetrievalControl, type PendingConceptRetrieval } from "../presentation/PaperConceptRetrievalControl";
import type { WritingAgentSubmitOptions } from "../presentation/WritingAgentPanel";

/** Reuses the application retrieval preview; the Agent only receives a confirmed trace. */
export function useConversationRetrieval(confirmed: (text: string, options: WritingAgentSubmitOptions, traceId: string, submissionRequestId?: string) => void) {
  const gateway = useMemo(() => getAiAssistanceGateway(), []);
  const [pending, setPending] = useState<(PendingConceptRetrieval & {options: WritingAgentSubmitOptions; submissionRequestId?: string})>();
  const [error, setError] = useNotificationState("error");
  const epoch = useRef(0), request = useRef<string | undefined>(undefined);
  function cancel(): void {
    epoch.current += 1;
    if (request.current) void gateway.cancelConceptRetrievalPreview(request.current).catch(() => undefined);
    request.current = undefined;
    setPending(undefined);
  }
  useEffect(() => () => {
    epoch.current += 1;
    if (request.current) void gateway.cancelConceptRetrievalPreview(request.current).catch(() => undefined);
  }, [gateway]);
  async function search(text: string, options: WritingAgentSubmitOptions, submissionRequestId?: string): Promise<void> {
    if (request.current) return;
    const current = ++epoch.current, clientRequestId = crypto.randomUUID();
    request.current = clientRequestId;
    setError(undefined);
    setPending({text, options, ...(submissionRequestId ? {submissionRequestId} : {}), isLoading: true});
    try {
      const preview = await gateway.previewConceptRetrieval({clientRequestId, query: text});
      if (epoch.current === current) setPending({text, options, ...(submissionRequestId ? {submissionRequestId} : {}), preview, isLoading: false});
    } catch {
      if (epoch.current === current) { setPending(undefined); request.current = undefined; setError("概念网络检索失败，消息已保留，请重试。"); }
    }
  }
  return { search, pending: !!pending, controls: pending || error ? <>
    {pending && <PaperConceptRetrievalControl checked disabled pending={pending} onCancel={cancel} onChange={() => undefined} onConfirm={() => {
      if (!pending.preview) return;
      const {text, options, preview, submissionRequestId} = pending;
      cancel(); confirmed(text, options, preview.traceId, submissionRequestId);
    }} />}

  </> : null };
}
