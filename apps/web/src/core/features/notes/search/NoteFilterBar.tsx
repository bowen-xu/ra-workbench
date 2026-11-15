import type { ReadingNoteBlock } from "@research-assistant/core/notes";
import type { NoteTag } from "@research-assistant/data/notes";

import type { ResearchObjectTarget } from "../links/useResearchObjectTargets";

export interface NoteFilters {
  readonly tagId?: string;
  readonly paperId?: string;
  readonly noteType?: ReadingNoteBlock["noteType"];
  readonly sourceState?: ReadingNoteBlock["source"]["state"];
}

interface NoteFilterBarProps {
  readonly filters: NoteFilters;
  readonly papers: readonly ResearchObjectTarget[];
  readonly tags: readonly NoteTag[];
  readonly onChange: (filters: NoteFilters) => void;
}

function updateFilter(filters: NoteFilters, key: keyof NoteFilters, value: string): NoteFilters {
  return Object.fromEntries([
    ...Object.entries(filters).filter(([candidate]) => candidate !== key),
    ...(value === "" ? [] : [[key, value]]),
  ]) as NoteFilters;
}

export function NoteFilterBar({ filters, papers, tags, onChange }: NoteFilterBarProps): React.JSX.Element {
  const hasFilters = Object.values(filters).some((value) => value !== undefined);
  return <div className="note-filter-bar" aria-label="笔记筛选">
    <label>标签<select onChange={(event) => onChange(updateFilter(filters, "tagId", event.target.value))} value={filters.tagId ?? ""}>
      <option value="">全部</option>{tags.map((tag) => <option key={tag.id} value={tag.id}>{tag.name}</option>)}
    </select></label>
    <label>文献<select onChange={(event) => onChange(updateFilter(filters, "paperId", event.target.value))} value={filters.paperId ?? ""}>
      <option value="">全部</option>{papers.map((paper) => <option key={paper.id} value={paper.id}>{paper.label}</option>)}
    </select></label>
    <label>类型<select onChange={(event) => onChange(updateFilter(filters, "noteType", event.target.value))} value={filters.noteType ?? ""}>
      <option value="">全部</option><option value="highlight">高亮</option><option value="annotation">批注</option><option value="page_note">页级笔记</option>
    </select></label>
    <label>来源<select onChange={(event) => onChange(updateFilter(filters, "sourceState", event.target.value))} value={filters.sourceState ?? ""}>
      <option value="">全部</option><option value="exact">精确定位</option><option value="page_only">页级定位</option><option value="needs_relocation">需要重定位</option><option value="paper_in_trash">文献在回收站</option><option value="unavailable">来源不可用</option>
    </select></label>
    {hasFilters && <button onClick={() => onChange({})} type="button">清除</button>}
  </div>;
}
