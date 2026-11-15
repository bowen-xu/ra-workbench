import type { AgentInputDraft } from "@research-assistant/core/agent";

const identity = (draft: AgentInputDraft) => JSON.stringify([draft.text, draft.bindings, draft.unboundReferences ?? [], draft.evidence ?? []]);
const metadata = (draft: AgentInputDraft) => JSON.stringify([draft.bindings, draft.unboundReferences ?? [], draft.evidence ?? []]);
const empty = (draft: AgentInputDraft) => !draft.text && !draft.bindings.length && !draft.evidence?.length && !draft.unboundReferences?.length;

/** One mounted composer owns its history; submissions and scope changes are boundaries.
 * Store complete identity-bearing snapshots, never reconstruct mentions from labels. */
export class ComposerDraftHistory {
  private past: AgentInputDraft[] = [];
  private future: AgentInputDraft[] = [];
  private current: AgentInputDraft;
  private lastTyping = 0;
  constructor(draft: AgentInputDraft) {this.current = structuredClone(draft);}
  observe(draft: AgentInputDraft): void {
    if (identity(draft) === identity(this.current)) return;
    if (empty(draft)) this.reset(draft);
    else this.record(draft);
  }
  reset(draft: AgentInputDraft): void {this.current = structuredClone(draft); this.past = []; this.future = []; this.lastTyping = 0;}
  record(draft: AgentInputDraft, now = Date.now()): void {
    if (identity(draft) === identity(this.current)) return;
    const typing = metadata(draft) === metadata(this.current) && draft.text !== this.current.text
      && Math.abs(draft.text.length - this.current.text.length) <= 2;
    if (!typing || !this.lastTyping || now - this.lastTyping > 750 || this.future.length) {
      this.past.push(this.current);
      if (this.past.length > 100) this.past.shift();
    }
    this.lastTyping = typing ? now : 0;
    this.current = structuredClone(draft); this.future = [];
  }
  undo(): AgentInputDraft | undefined {
    const previous = this.past.pop();
    if (!previous) return;
    this.future.push(this.current); this.current = previous; this.lastTyping = 0;
    return structuredClone(previous);
  }
  redo(): AgentInputDraft | undefined {
    const next = this.future.pop();
    if (!next) return;
    this.past.push(this.current); this.current = next; this.lastTyping = 0;
    return structuredClone(next);
  }
  retainedDrafts(): readonly AgentInputDraft[] {return [...this.past, this.current, ...this.future];}
}
