// @vitest-environment happy-dom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {expect, it, vi} from "vitest";
import type {Concept, DefinitionClaim} from "@research-assistant/core/knowledge";
import {notify} from "@research-assistant/workbench-sdk/feedback";
import {createRuntimeLiteraturePort, selectAppRuntime} from "../../../../../../../integration/web/src/app/runtime/literature/runtimeLiteratureWorkspace.testSupport.tsx";
import {configureLiteratureServices} from "../../../../../../../plugins/ra-literature-library/src/services/configureLiteratureServices";
import {getResearchKnowledgeGateway} from "../../../../../../../plugins/ra-literature-library/src/services/knowledgeGateway";
import {ConceptWorkspace} from "../../../../../../../plugins/ra-literature-library/src/features/knowledge/ConceptWorkspace";

it("uses the original definition editor and history with the selected runtime and retains rejected drafts", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.useFakeTimers();
  selectAppRuntime({mode: "browser", storage: localStorage, gateways: {}, dispose() {}});
  const info = vi.spyOn(notify, "info").mockReturnValue("info");
  const error = vi.spyOn(notify, "error").mockReturnValue("error");
  const term = {id: "term", text: "Original term", normalizedText: "original term", language: "en", provenanceKind: "human_original" as const, revision: 1, createdAt: "created", updatedAt: "updated"};
  const concept: Concept = {id: "concept", label: "Original concept", preferredTerm: term, terms: [term], explanation: "", disambiguation: "", domain: "", tradition: "", period: "", provenanceKind: "human_original", revision: 1, definitionCount: 0, sourceCount: 0, createdAt: "created", updatedAt: "updated"};
  let saved: DefinitionClaim | undefined, rejectSave = false;
  const invoke = vi.fn(async (command: string, args: Record<string, unknown>): Promise<unknown> => {
    if (command === "search_runtime_research_concepts") return [concept];
    if (command === "list_runtime_definition_claims") return saved === undefined ? [] : [saved];
    if (command === "save_runtime_definition_claim") {
      if (rejectSave) throw {code: "revision_conflict", message: "Original conflict", currentRevision: 3, requestId: "definition-request"};
      const draft = args.draft as {statement: string; scopeNote: string};
      saved = {id: "definition", conceptId: concept.id, statement: draft.statement.trim(), scopeNote: draft.scopeNote.trim(), provenanceKind: "human_original", epistemicRole: "definition", revision: (saved?.revision ?? 0) + 1, evidenceCount: 0, createdAt: "created", updatedAt: "updated"};
      return saved;
    }
    if (command === "list_runtime_research_object_revisions") return [{id: "revision", objectKind: "definition_claim", objectId: "definition", revision: saved!.revision, snapshot: {statement: saved!.statement}, editorKind: "human", changeReason: "manual_edit", createdAt: "2026-11-12T00:00:00Z"}];
    throw new Error(`Unexpected ${command}`);
  });
  const port = createRuntimeLiteraturePort("e".repeat(64), [{id: "literature.concepts", version: 1}], () => undefined, invoke);
  const partialKnowledge = {...port.knowledge};
  delete partialKnowledge.listConceptIdentityEvents;
  const release = configureLiteratureServices({...port, knowledge: partialKnowledge});
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  const button = (parent: Element, text: string) => {
    const found = [...parent.querySelectorAll<HTMLButtonElement>("button")].find(value => value.textContent === text);
    if (!found) throw new Error(`Missing production button ${text}`);
    return found;
  };
  const input = async (selector: string, value: string) => {
    const element = host.querySelector<HTMLInputElement | HTMLTextAreaElement>(selector);
    if (!element) throw new Error(`Missing production input ${selector}`);
    await act(async () => {
      const prototype = element instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
      Object.getOwnPropertyDescriptor(prototype, "value")!.set!.call(element, value);
      element.dispatchEvent(new Event("input", {bubbles: true}));
    });
  };
  try {
    await act(async () => root.render(<ConceptWorkspace gateway={getResearchKnowledgeGateway()}/>));
    await act(async () => vi.advanceTimersByTimeAsync(150));
    // Identity management retains its declared unavailable state in this finite package port.
    expect(error).toHaveBeenCalledWith(expect.stringContaining("knowledge.listConceptIdentityEvents"));
    await act(async () => host.querySelector<HTMLButtonElement>(".concept-result-main")!.click());
    await input(".definition-workspace textarea", " New definition Ω ");
    await input(".definition-workspace input", " Scope ");
    await act(async () => button(host, "保存定义").click());
    expect(invoke).toHaveBeenCalledWith("save_runtime_definition_claim", {packageKey: "e".repeat(64), draft: {conceptId: "concept", statement: " New definition Ω ", scopeNote: " Scope ", changeReason: "manual_create"}});
    expect(host.querySelector(".definition-workspace .knowledge-object-list")?.textContent).toContain("New definition Ω");
    await act(async () => button(host.querySelector(".definition-workspace .knowledge-object-list")!, "编辑").click());
    await input(".definition-workspace textarea", " Revised definition Ω ");
    await act(async () => button(host, "保存定义").click());
    expect(invoke).toHaveBeenCalledWith("save_runtime_definition_claim", {packageKey: "e".repeat(64), draft: {id: "definition", expectedRevision: 1, conceptId: "concept", statement: " Revised definition Ω ", scopeNote: "Scope", changeReason: "manual_edit"}});
    await act(async () => button(host.querySelector(".definition-workspace .knowledge-object-list")!, "版本").click());
    expect(host.querySelector('[aria-label="版本历史"]')?.textContent).toContain("Revised definition Ω");
    expect(invoke).toHaveBeenCalledWith("list_runtime_research_object_revisions", {packageKey: "e".repeat(64), objectKind: "definition_claim", objectId: "definition"});
    await act(async () => button(host.querySelector(".definition-workspace .knowledge-object-list")!, "编辑").click());
    await input(".definition-workspace textarea", " Retained rejected draft "); rejectSave = true;
    const successes = info.mock.calls.length;
    await act(async () => button(host, "保存定义").click());
    expect(host.querySelector<HTMLTextAreaElement>(".definition-workspace textarea")?.value).toBe(" Retained rejected draft ");
    expect(button(host, "保存定义").disabled).toBe(false); expect(info).toHaveBeenCalledTimes(successes);
    expect(error).toHaveBeenLastCalledWith(expect.stringMatching(/revision_conflict.*definition-request.*草稿仍保留/u));
  } finally {
    await act(async () => root.unmount()); host.remove(); release(); vi.useRealTimers(); vi.restoreAllMocks(); vi.unstubAllGlobals();
  }
});
