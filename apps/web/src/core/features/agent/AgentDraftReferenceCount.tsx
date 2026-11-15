import { createContext, useContext, useLayoutEffect, type ReactNode } from "react";
import type { AgentInputDraft } from "@research-assistant/core/agent";

const ReferenceCountContext = createContext<((count: number) => void) | null>(null);

export function draftReferenceCount(draft: AgentInputDraft): number {
  return draft.bindings.length + (draft.unboundReferences?.length ?? 0) + (draft.evidence?.length ?? 0);
}

export function AgentDraftReferenceCountProvider({onChange, children}: {
  readonly onChange: (count: number) => void; readonly children: ReactNode;
}) {
  return <ReferenceCountContext.Provider value={onChange}>{children}</ReferenceCountContext.Provider>;
}

/** The mounted composer publishes its current draft, including updates while hidden. */
export function useAgentDraftReferenceCount(draft: AgentInputDraft): number {
  const publish = useContext(ReferenceCountContext), count = draftReferenceCount(draft);
  useLayoutEffect(() => {publish?.(count);}, [publish, count]);
  useLayoutEffect(() => () => publish?.(0), [publish]);
  return count;
}

export function AgentReferenceCountBadge({count}: {readonly count: number}) {
  return count > 0 ? <span className="agent-reference-count-badge" aria-hidden="true"><span>{count}</span></span> : null;
}
