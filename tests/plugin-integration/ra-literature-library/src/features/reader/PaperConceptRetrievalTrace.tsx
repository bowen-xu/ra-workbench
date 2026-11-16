import { useState } from "react";
import type { ConceptRetrievalPreview } from "@research-assistant/core/ai";

import {
  aiAssistanceErrorMessage,
  type AiAssistanceGateway,
} from "../../../../../../apps/web/src/core/public/aiAssistance";

interface PaperConceptRetrievalTraceProps {
  readonly aiGateway: AiAssistanceGateway;
  readonly traceId: string;
}

export function PaperConceptRetrievalTrace({
  aiGateway,
  traceId,
}: PaperConceptRetrievalTraceProps): React.JSX.Element {
  const [preview, setPreview] = useState<ConceptRetrievalPreview>();
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string>();

  async function load(isOpen: boolean): Promise<void> {
    if (!isOpen || preview !== undefined || isLoading) return;
    setIsLoading(true);
    setError(undefined);
    try {
      setPreview(await aiGateway.getConceptRetrievalTrace(traceId));
    } catch (reason: unknown) {
      setError(aiAssistanceErrorMessage(reason));
    } finally {
      setIsLoading(false);
    }
  }

  const selectedSources = preview?.sources.filter((source) => source.selected) ?? [];
  return <details
    className="paper-concept-retrieval-trace"
    onToggle={(event) => void load(event.currentTarget.open)}
  >
    <summary>本轮检索上下文</summary>
    {isLoading && <p className="empty">正在读取检索 trace…</p>}
    {error !== undefined && <p className="paper-ai-reading-error">{error}</p>}
    {preview !== undefined && <div>
      <p>{preview.seeds.length} 个种子概念 · {preview.edges.length} 条扩展关系 · {selectedSources.length} 条来源</p>
      <ul>
        {preview.seeds.map((seed) => <li key={seed.conceptId}>
          <strong>{seed.label}</strong>
          <span>{seed.matchedBy.join(" + ")}</span>
        </li>)}
      </ul>
      <ol>
        {selectedSources.map((source, index) => <li key={`${source.anchorId}:${source.conceptId}`}>
          <strong>[CN-S{index + 1}] {source.paperTitle}</strong>
          <span>{source.conceptLabel} · 第 {source.pageIndex + 1} 页</span>
          <q>{source.quoteExact}</q>
        </li>)}
      </ol>
    </div>}
  </details>;
}
