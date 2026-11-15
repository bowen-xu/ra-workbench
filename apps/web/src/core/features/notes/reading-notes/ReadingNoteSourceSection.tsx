import { ExternalLink } from "lucide-react";
import type { ReadingNoteBlock } from "@research-assistant/core/notes";
import { useReadingNoteResearchActions } from "./ReadingNotePresentation";

const SOURCE_LABELS = {
  exact: "可精确定位", page_only: "可定位到页面", needs_relocation: "附件已变化，需要重定位",
  paper_in_trash: "来源文献位于回收站", unavailable: "来源不可用，已保留快照",
};
export function ReadingNoteSourceSection({ note, onColorChange }: {
  readonly note: ReadingNoteBlock; readonly onColorChange?: (color: string) => void;
}): React.JSX.Element {
  const ResearchActions = useReadingNoteResearchActions();
  const available = note.source.state !== "unavailable" && note.source.state !== "paper_in_trash";
  const needsAttention = note.source.state !== "exact" && note.source.state !== "page_only";
  return <section className="reading-note-source-section">
    <details>
      <summary>来源 · 第 {note.source.pageLabel ?? note.source.pageIndex + 1} 页{needsAttention ? ` · ${SOURCE_LABELS[note.source.state]}` : ""}</summary>
      <p className="reading-note-source-paper" data-selectable-text>{note.source.paperTitle}</p>
      <div className="reading-note-section-label">
        <span>{SOURCE_LABELS[note.source.state]}</span>
        {onColorChange && <label>颜色<input type="color" aria-label="阅读笔记颜色" value={note.color} onChange={event => onColorChange(event.target.value)} /></label>}
      </div>
      <blockquote>{note.source.quoteExact || "无选区页级笔记"}</blockquote>
      {ResearchActions && <ResearchActions note={note} />}
    </details>
    <button type="button" aria-label="返回原文" disabled={!available} onClick={() => window.dispatchEvent(new CustomEvent("research-assistant:open-reading-source", { detail: note.source }))}><ExternalLink aria-hidden="true" size={14} />原文</button>
  </section>;
}
