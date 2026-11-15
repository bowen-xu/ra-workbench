import { ChevronRight, FileDiff } from "lucide-react";
import type { ReactNode } from "react";
import {ControlHint} from "../../components/ControlHint";
import "./AgentInlineChangeSummary.css";
import "./AgentChangeSummary.css";

export interface AgentChangeSummaryItem {
  readonly id: string;
  readonly identity?:string;
  readonly label: string;
  readonly path?: string;
  readonly stats?: { readonly additions: number; readonly deletions: number };
  readonly disabled?: boolean;
  readonly onOpen: () => void;
}
/** Shared file/note change presentation. Storage, review and decisions belong
 * to their existing adapters; this component owns no execution or persistence. */
export function AgentChangeSummary({ title, ariaLabel, stats, items, headerActions, children }: {
  readonly title: string;
  readonly ariaLabel: string;
  readonly stats?: { readonly additions: number; readonly deletions: number };
  readonly items: readonly AgentChangeSummaryItem[];
  readonly headerActions?: ReactNode;
  readonly children?: ReactNode;
}): React.JSX.Element {
  const single=items.length===1?items[0]:undefined;
  const singleName=single?.path?single.path.split(/[\\/]/u).at(-1)??single.label:single?.label;
  const fileButton=single?<button type="button" className="agent-change-summary-file" disabled={single.disabled} onClick={single.onOpen}>{singleName}</button>:null;
  const heading=single?<>Edited file {single.path?<ControlHint content={single.path}>{fileButton!}</ControlHint>:fileButton}</>:title;
  return <section className={`writing-agent-inline-diff writing-agent-inline-diff-card${single?" single-file":""}`} role="status" aria-label={ariaLabel}>
    <header className="writing-agent-inline-diff-header">
      <div className="writing-agent-inline-diff-summary">
        <span className="writing-agent-inline-diff-icon"><FileDiff aria-hidden="true" size={20} /></span>
        <div><strong>{heading}</strong>{stats && <span className="writing-agent-inline-diff-counts"><b>+{stats.additions}</b> <em>−{stats.deletions}</em></span>}</div>
      </div>
      {headerActions}
    </header>
    {!single&&<div className="writing-agent-inline-diff-list">{items.map(item => <button type="button" key={item.id} disabled={item.disabled} onClick={item.onOpen}>
      <span>{item.label}</span>{item.stats && <small><b>+{item.stats.additions}</b> <em>−{item.stats.deletions}</em></small>}
      <ChevronRight aria-hidden="true" size={14} />
    </button>)}</div>}
    {children}
  </section>;
}
