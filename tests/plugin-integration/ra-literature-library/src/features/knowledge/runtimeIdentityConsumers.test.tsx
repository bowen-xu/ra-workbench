// @vitest-environment happy-dom
import {act, useState} from "react";
import {createRoot} from "react-dom/client";
import {expect, it, vi} from "vitest";
import type {Concept, ConceptIdentityEvent, ConceptMergeDraft} from "@research-assistant/core/knowledge";
import {notify} from "@research-assistant/workbench-sdk/feedback";
import {createRuntimeLiteraturePort, selectAppRuntime} from "../../../../../../../integration/web/src/app/runtime/literature/runtimeLiteratureWorkspace.testSupport.tsx";
import {configureLiteratureServices} from "../../../../../../../plugins/ra-literature-library/src/services/configureLiteratureServices";
import {getResearchKnowledgeGateway} from "../../../../../../../plugins/ra-literature-library/src/services/knowledgeGateway";
import {ConceptIdentityWorkspace} from "../../../../../../../plugins/ra-literature-library/src/features/knowledge/ConceptIdentityWorkspace";
it("uses the original identity controls for preview, merge, split and failed re-preview", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  selectAppRuntime({mode: "browser", storage: localStorage, gateways: {}, dispose() {}});
  const info = vi.spyOn(notify, "info").mockReturnValue("info"), error = vi.spyOn(notify, "error").mockReturnValue("error");
  const concept = (id: string, label: string): Concept => {
    const term = {id: `${id}-term`, text: label, normalizedText: label.toLowerCase(), language: "en", provenanceKind: "human_original" as const, revision: 1, createdAt: "created", updatedAt: "updated"};
    return {id, label, preferredTerm: term, terms: [term], explanation: "", disambiguation: "", domain: "", tradition: "", period: "", provenanceKind: "human_original", revision: 1, definitionCount: 0, sourceCount: 0, createdAt: "created", updatedAt: "updated"};
  };
  const survivor = concept("survivor", "Survivor Ω"), source = concept("source", "Source Ω");
  const initial = [survivor, source], afterMerge = [{...survivor, revision: 2}], afterSplit = [{...survivor, revision: 3}, {...source, revision: 2}];
  let serverConcepts = initial, events: readonly ConceptIdentityEvent[] = [], reject = false;
  const summary = (value: Concept) => ({id: value.id, label: value.label, revision: value.revision, termCount: 1, definitionCount: value.definitionCount, sourceCount: value.sourceCount});
  const invoke = vi.fn(async (command: string, args: Record<string, unknown>): Promise<unknown> => {
    if (command !== "apply_runtime_concept_identity") throw new Error(`Unexpected ${command}`);
    const request = args.request as {kind: string; draft?: ConceptMergeDraft};
    if (request.kind === "list") return {kind: "events", events};
    if (request.kind === "preview") return {kind: "impact", impact: {survivor: summary(serverConcepts[0]!), sources: [summary(serverConcepts[1]!)], affectedRelationIds: ["relation"]}};
    if (request.kind === "merge") {
      if (reject) throw {code: "revision_conflict", message: "Original conflict", currentRevision: 4, requestId: "identity-request"};
      const event: ConceptIdentityEvent = {id: "merge", operation: "merge", survivor: summary(survivor), sources: [summary(source)], affectedRelationIds: ["relation"], activeSourceIds: ["source"], rationale: request.draft!.rationale.trim(), createdAt: "2026-11-12T00:00:00Z"};
      events = [event]; serverConcepts = afterMerge; return {kind: "event", event};
    }
    const event: ConceptIdentityEvent = {id: "split", operation: "split", survivor: summary(survivor), sources: [summary(source)], affectedRelationIds: ["relation"], activeSourceIds: [], rationale: "人工拆回误合并概念", createdAt: "2026-11-12T00:01:00Z"};
    events = [event, {...events[0]!, activeSourceIds: []}]; serverConcepts = afterSplit; return {kind: "event", event};
  });
  const release = configureLiteratureServices(createRuntimeLiteraturePort("e".repeat(64), [{id: "literature.concepts", version: 1}], () => undefined, invoke));
  const gateway = getResearchKnowledgeGateway();
  function Harness(): React.JSX.Element {
    const [concepts, setConcepts] = useState(initial);
    return <ConceptIdentityWorkspace concepts={concepts} gateway={gateway} onChanged={async () => setConcepts([...serverConcepts])}/>;
  }
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  const button = (text: string) => {const found = [...host.querySelectorAll<HTMLButtonElement>("button")].find(b => b.textContent?.trim() === text);if (!found) throw new Error(`Missing production button ${text}`);return found;};
  const choose = async () => act(async () => {
    const select = host.querySelector<HTMLSelectElement>("select")!;
    Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value")!.set!.call(select, "survivor");select.dispatchEvent(new Event("change", {bubbles: true}));
  });
  const reason = async (value: string) => act(async () => {const textarea = host.querySelector<HTMLTextAreaElement>("textarea")!;Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(textarea, value);textarea.dispatchEvent(new Event("input", {bubbles: true}));});
  try {
    await act(async () => root.render(<Harness/>)); host.querySelector("details")!.open = true;
    await choose();await act(async () => host.querySelectorAll<HTMLInputElement>('input[type="checkbox"]')[1]!.click());await reason(" Reason Ω ");
    await act(async () => button("预览 ID、来源与关系影响").click());expect(host.querySelector(".concept-identity-preview")?.textContent).toContain("受影响关系 1 条");
    await act(async () => button("取消").click());expect(invoke.mock.calls.filter(([,args]) => (args.request as {kind: string}).kind === "merge")).toHaveLength(0);
    await act(async () => button("预览 ID、来源与关系影响").click());await act(async () => button("确认建立可拆回映射").click());
    expect(invoke).toHaveBeenCalledWith("apply_runtime_concept_identity", {packageKey: "e".repeat(64), request: {kind: "merge", draft: {survivorId: "survivor", survivorExpectedRevision: 1, sources: [{id: "source", expectedRevision: 1}], rationale: " Reason Ω "}}});
    expect(host.querySelector(".concept-identity-history")?.textContent).toContain("合并映射");
    await act(async () => button("拆回 Source Ω").click());expect(invoke).toHaveBeenCalledWith("apply_runtime_concept_identity", {packageKey: "e".repeat(64), request: {kind: "split", draft: {mergeEventId: "merge", sourceConceptIds: ["source"], rationale: "人工拆回误合并概念"}}});
    expect(host.querySelector(".concept-identity-history")?.textContent).toContain("拆回 · Survivor Ω");
    await choose();await act(async () => host.querySelectorAll<HTMLInputElement>('input[type="checkbox"]')[1]!.click());await reason(" Retained reason ");await act(async () => button("预览 ID、来源与关系影响").click());reject = true;const successes = info.mock.calls.length;
    await act(async () => button("确认建立可拆回映射").click());expect(info).toHaveBeenCalledTimes(successes);expect(error).toHaveBeenLastCalledWith(expect.stringMatching(/revision_conflict.*identity-request.*重新预览/u));expect(host.querySelector<HTMLTextAreaElement>("textarea")?.value).toBe(" Retained reason ");expect(host.querySelector(".concept-identity-preview")).toBeNull();expect(button("预览 ID、来源与关系影响").disabled).toBe(false);
  } finally {await act(async () => root.unmount());host.remove();release();vi.restoreAllMocks();vi.unstubAllGlobals();}
});
