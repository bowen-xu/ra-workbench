import { useRef, useState } from "react";
import { agentContextKey, type AgentContextSelection, type AgentContextState } from "@research-assistant/core/agent";
import { getAppStorage } from "../../../runtime/appStorage";
import { readConversationContext, writeConversationContext } from "./contextStorage";
import { applyRecommendationChoices, decodeRecommendationChoices, recommendationDraftKey, recommendationSubmissionKey, type ContextRecommendationChoice } from "./contextRecommendationDraft";
import { selectedCatalogItem } from "./contextCatalogSelection";
import { useAgentContextCandidates, useAgentContextSuggestions } from "./AgentContextProvider";

/** Recommendation checkmarks belong to the draft until an admitted message consumes them. */
export function useContextRecommendations(id: string | null, context: AgentContextState, pending = false) {
  const storage = getAppStorage();
  const [initial] = useState(() => {
    try {return {choices: id ? decodeRecommendationChoices(JSON.parse(storage.getItem(recommendationDraftKey(id)) ?? "[]")) : [], error: undefined as string | undefined};}
    catch {return {choices: [], error: "无法恢复推荐选择，请重新核对上下文。"};}
  });
  const [choices, setChoices] = useState<readonly ContextRecommendationChoice[]>(initial.choices);
  const currentChoices = useRef(choices); currentChoices.current = choices;
  const [error, setError] = useState(initial.error);
  const active = useAgentContextCandidates(), suggestions = useAgentContextSuggestions();
  // Drafts can preview other open materials. Established conversations only
  // recommend the active page; switching pages never clears saved choices.
  const candidates = pending ? [...active, ...suggestions] : active;
  const items = [...new Map(candidates
    .filter(item => pending || !selectedCatalogItem(item, context))
    .map(item => [agentContextKey(item.resource), item])).values()];
  const effective = applyRecommendationChoices(context, choices);
  const selected = items.flatMap(item => {const selected = selectedCatalogItem(item, effective); return selected ? [selected] : [];});
  async function choose(item: AgentContextSelection, remove: boolean): Promise<void> {
    if (!id) return;
    try {
      const key = agentContextKey(item.resource);
      const next = decodeRecommendationChoices([...currentChoices.current.filter(row => agentContextKey(row.item.resource) !== key), {item, selected: !remove}]);
      applyRecommendationChoices(context, next);
      storage.setItem(recommendationDraftKey(id), JSON.stringify(next));
      currentChoices.current = next; setChoices(next); setError(undefined);
    } catch {setError("无法保存推荐选择，请重试。");}
  }
  function capture(requestId: string): AgentContextState {
    if (!id || error) throw new Error(error ?? "会话身份无效");
    storage.setItem(recommendationSubmissionKey(id), JSON.stringify({requestId, choices: currentChoices.current}));
    return applyRecommendationChoices(context, currentChoices.current);
  }
  function clear(item: AgentContextSelection): void {
    if (!id) return;
    const next = currentChoices.current.filter(row => agentContextKey(row.item.resource) !== agentContextKey(item.resource));
    storage.setItem(recommendationDraftKey(id), JSON.stringify(next));
    currentChoices.current = next; setChoices(next); setError(undefined);
  }
  function submittedChoices(requestId: string): readonly ContextRecommendationChoice[] | null {
    if (!id) throw new Error("会话身份无效");
    const raw = storage.getItem(recommendationSubmissionKey(id));
    if (!raw) return null;
    const captured = JSON.parse(raw) as {requestId: string; choices: unknown};
    if (captured.requestId !== requestId) throw new Error("推荐选择与发送请求不匹配");
    return decodeRecommendationChoices(captured.choices);
  }
  function prepareCommit(requestId: string): AgentContextState {
    if (!id) throw new Error("会话身份无效");
    const submitted = submittedChoices(requestId);
    const current = readConversationContext(id);
    return submitted ? applyRecommendationChoices(current, submitted) : current;
  }
  function completeCommit(requestId: string): void {
    if (!id) throw new Error("会话身份无效");
    const submitted = submittedChoices(requestId);
    if (!submitted) return;
    // Retain deselections so reopening a resource does not silently authorize it.
    const current = decodeRecommendationChoices(JSON.parse(storage.getItem(recommendationDraftKey(id)) ?? "[]"));
    const remaining = current.filter(row => !submitted.some(sent => sent.selected && row.selected
      && agentContextKey(sent.item.resource) === agentContextKey(row.item.resource) && JSON.stringify(sent.item) === JSON.stringify(row.item)));
    storage.setItem(recommendationDraftKey(id), JSON.stringify(remaining));
    storage.removeItem(recommendationSubmissionKey(id));
    currentChoices.current = remaining; setChoices(remaining);
  }
  function commit(requestId: string): AgentContextState {
    if (!id) throw new Error("会话身份无效");
    if (!submittedChoices(requestId)) return readConversationContext(id);
    const next = prepareCommit(requestId);
    writeConversationContext(id, next);
    completeCommit(requestId);
    return next;
  }
  return {items, selected, effective, choose, capture, prepareCommit, completeCommit, commit, clear, error};
}
