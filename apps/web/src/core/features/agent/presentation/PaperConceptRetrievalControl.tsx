import { Loader2, Network, X } from "lucide-react";
import type { ConceptRetrievalPreview } from "@research-assistant/core/ai";
import "./PaperConceptRetrieval.css";

export interface PendingConceptRetrieval {
  readonly text: string;
  readonly preview?: ConceptRetrievalPreview;
  readonly isLoading: boolean;
}

interface PaperConceptRetrievalControlProps {
  readonly checked: boolean;
  readonly disabled: boolean;
  readonly pending?: PendingConceptRetrieval;
  readonly onCancel: () => void;
  readonly onChange: (checked: boolean) => void;
  readonly onConfirm: () => void;
}

export function PaperConceptRetrievalControl({
  checked,
  disabled,
  pending,
  onCancel,
  onChange,
  onConfirm,
}: PaperConceptRetrievalControlProps): React.JSX.Element {
  return (
    <section className="paper-concept-retrieval" aria-label="概念网络检索">
      <label>
        <input
          checked={checked}
          disabled={disabled}
          onChange={(event) => onChange(event.target.checked)}
          type="checkbox"
        />
        <Network aria-hidden="true" />
        <span>
          <strong>参考概念网络</strong>
          <small>发送前预览；问题会先发送给 embedding provider</small>
        </span>
      </label>
      {pending !== undefined && <div className="paper-concept-retrieval-preview" aria-live="polite">
        {pending.isLoading
          ? <p><Loader2 className="spin" aria-hidden="true" />正在检索已接受的概念和来源…</p>
          : pending.preview === undefined
            ? <p>没有可预览的概念网络上下文。</p>
            : <RetrievalPreview preview={pending.preview} />}
        <div>
          <button onClick={onCancel} type="button"><X aria-hidden="true" />取消</button>
          <button
            disabled={pending.isLoading || pending.preview === undefined}
            onClick={onConfirm}
            type="button"
          >确认来源并发送</button>
        </div>
      </div>}
    </section>
  );
}

function RetrievalPreview({ preview }: { readonly preview: ConceptRetrievalPreview }): React.JSX.Element {
  const selectedSources = preview.sources.filter((source) => source.selected);
  return (
    <>
      <p>
        <strong>{preview.seeds.length} 个概念 · {selectedSources.length} 条来源</strong>
        <span>{preview.semanticStatus === "used" ? "词法 + 向量" : "仅词法（向量不可用）"}</span>
      </p>
      {preview.seeds.length > 0 && <ul className="paper-concept-retrieval-concepts">
        {preview.seeds.map((seed) => <li key={seed.conceptId}>{seed.label}</li>)}
      </ul>}
      {selectedSources.length > 0 && <details>
        <summary>查看将发送的来源</summary>
        <ol>
          {selectedSources.map((source) => <li key={`${source.anchorId}:${source.conceptId}`}>
            <strong>{source.paperTitle}</strong>
            <span>{source.conceptLabel} · 第 {source.pageIndex + 1} 页</span>
            <q>{source.quoteExact}</q>
          </li>)}
        </ol>
      </details>}
      {selectedSources.length === 0 && <p>未找到带原文锚点的已接受来源；确认后只发送概念身份和关系。</p>}
      {preview.externalPaperCount > 0 && <small>其中包含 {preview.externalPaperCount} 篇其他论文的来源。</small>}
    </>
  );
}
