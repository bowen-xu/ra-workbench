import { createContext, useCallback, useContext, useMemo, useRef, useState, useSyncExternalStore, type ReactNode } from "react";
import { agentDraftFromInput, insertAgentReference, type AgentUserInput, type AgentReferenceInput, type AgentInputDraft } from "@research-assistant/core/agent";
import { ConversationDraftStore, conversationDraftKey, type ConversationDraftScope } from "./conversationDraftStore";
import type {ConversationDraftStore as ConversationDraftStorePort} from "@research-assistant/workbench-sdk/agent-drafts";

import type { ConversationDraftPersistence } from "./conversationDraftPersistence";
import type { AgentQueueState } from "@research-assistant/core/agent";
const DraftContext = createContext<ConversationDraftStore | null>(null);

export function AgentDraftProvider({ children, persistence }: { readonly children: ReactNode;readonly persistence?:ConversationDraftPersistence }): React.JSX.Element {
  const [store] = useState(() => new ConversationDraftStore(persistence));
  return <DraftContext.Provider value={store}>{children}</DraftContext.Provider>;
}

/** Standalone hosts may own their store; the application provider survives module unmounts. */
export function useAgentDraftStore(): ConversationDraftStorePort {
  const shared = useContext(DraftContext);
  const [local] = useState(() => new ConversationDraftStore());
  return shared ?? local;
}

export function useConversationDraft(store: ConversationDraftStorePort, scope: ConversationDraftScope | null, observeText = true) {
  const key = scope ? conversationDraftKey(scope) : null;
  const subscribe = useCallback((listener: () => void) => key === null ? () => undefined : store.subscribe(key, listener), [store, key]);
  // Execution hosts observe placement; the composer owns ordinary text edits.
  const placementCache = useRef<{value: unknown; serialized: string}>({value: undefined, serialized:"null"});
  const read = useCallback(() => {
    const current = store.readStructured(key ?? "");
    if (observeText) return current;
    if (placementCache.current.value !== current.queuePlacement) placementCache.current = {
      value: current.queuePlacement, serialized: JSON.stringify(current.queuePlacement ?? null),
    };
    return placementCache.current.serialized;
  }, [store, key, observeText]);
  const observed = useSyncExternalStore(subscribe, read, read);
  const draft = typeof observed === "string" ? store.readStructured(key ?? "").text : observed.text;
  // A delayed acknowledgment keeps the original scope even after the view switches away.
  const setDraft = useCallback((next: string | ((current: string) => string)) => {
    if (key !== null) store.update(key, next);
  }, [store, key]);
  const captureAcknowledgment = useCallback((text: string) => key === null ? () => undefined : store.captureAcknowledgment(key, text), [store, key]);
  const resources = useMemo(() => ({
    subscribe,
    error:()=>store.error(),
    draft: () => store.readStructured(key ?? ""),
    set: (value: AgentInputDraft) => {if (key !== null) store.updateStructured(key, value);},
    input: (text: string): readonly AgentUserInput[] => key === null ? [] : store.input(key, text),
    acknowledgmentToken: (text: string) => key === null ? null : store.captureAcknowledgmentToken(key, text),
    insert: (reference: AgentReferenceInput, start?: number, end?: number) => { if (key !== null) store.updateStructured(key, insertAgentReference(store.readStructured(key), reference, start, end)); },
    restore: (text: string, input: readonly AgentUserInput[] = []) => { if (key !== null) store.updateStructured(key, agentDraftFromInput(text, input), null); },
    restoreQueue: (input: AgentQueueState["inputs"][number], index: number, queue: AgentQueueState) => {
      if (key !== null) store.restoreQueue(key, {input: structuredClone(input), index,
        preceding: queue.inputs.slice(0, index).map(item => item.inputId), following: queue.inputs.slice(index + 1).map(item => item.inputId)});
    },
  }), [store, key, subscribe]);
  return [draft, setDraft, captureAcknowledgment, resources] as const;
}
