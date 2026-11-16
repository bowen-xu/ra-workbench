// @vitest-environment happy-dom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {expect, it, vi} from "vitest";
import type {RelationAssertion, RelationAssertionDraft} from "@research-assistant/core/knowledge";
import {notify} from "@research-assistant/workbench-sdk/feedback";
import {createRuntimeLiteraturePort, selectAppRuntime} from "../../../../../../../integration/web/src/app/runtime/literature/runtimeLiteratureWorkspace.testSupport.tsx";
import {configureLiteratureServices} from "../../../../../../../plugins/ra-literature-library/src/services/configureLiteratureServices";
import {getResearchKnowledgeGateway} from "../../../../../../../plugins/ra-literature-library/src/services/knowledgeGateway";
import {RelationWorkspace} from "../../../../../../../plugins/ra-literature-library/src/features/knowledge/RelationWorkspace";
it("uses the production relation editor and history with the selected runtime and retains rejected edits", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true); vi.useFakeTimers();
  selectAppRuntime({mode: "browser", storage: localStorage, gateways: {}, dispose() {}});
  const info = vi.spyOn(notify, "info").mockReturnValue("info"), error = vi.spyOn(notify, "error").mockReturnValue("error");
  const nodes = [{kind: "paper" as const, id: "paper", label: "Paper Ω", detail: "journalArticle"}, {kind: "note_page" as const, id: "core-note", label: "Core note Ω", detail: "笔记页面"}];
  let saved: RelationAssertion | undefined, reject = false;
  const invoke = vi.fn(async (command: string, args: Record<string, unknown>): Promise<unknown> => {
    if (command === "search_runtime_relation_nodes") return nodes;
    if (command === "list_runtime_relation_assertions") return saved ? [saved] : [];
    if (command === "save_runtime_relation_assertion") {
      if (reject) throw {code: "revision_conflict", message: "Original conflict", currentRevision: 3, requestId: "relation-request"};
      const draft = args.draft as RelationAssertionDraft;
      saved = {id: "relation", subject: nodes[0]!, object: nodes[1]!, predicateKind: draft.predicateKind, scopeNote: draft.scopeNote.trim(), provenanceKind: "human_original", epistemicRole: "assertion", revision: (saved?.revision ?? 0) + 1, evidenceCount: 0, createdAt: "created", updatedAt: "updated"};
      return saved;
    }
    if (command === "list_runtime_research_object_revisions") return [{id: "revision", objectKind: "relation_assertion", objectId: "relation", revision: saved!.revision, snapshot: {scopeNote: saved!.scopeNote}, editorKind: "human", changeReason: "manual_edit", createdAt: "2026-11-12T00:00:00Z"}];
    throw new Error(`Unexpected ${command}`);
  });
  const release = configureLiteratureServices(createRuntimeLiteraturePort("e".repeat(64), [{id: "literature.concepts", version: 1}], () => undefined, invoke));
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  const button = (parent: Element, text: string) => {const result = [...parent.querySelectorAll<HTMLButtonElement>("button")].find(b => b.textContent === text);if (!result) throw new Error(`Missing production button ${text}`);return result;};
  const change = async (element: HTMLSelectElement | HTMLTextAreaElement, value: string) => act(async () => {
    const prototype = element instanceof HTMLSelectElement ? HTMLSelectElement.prototype : HTMLTextAreaElement.prototype;
    Object.getOwnPropertyDescriptor(prototype, "value")!.set!.call(element, value);
    element.dispatchEvent(new Event(element instanceof HTMLSelectElement ? "change" : "input", {bubbles: true}));
  });
  try {
    await act(async () => root.render(<RelationWorkspace gateway={getResearchKnowledgeGateway()}/>));
    await act(async () => vi.advanceTimersByTimeAsync(150));
    const selects = host.querySelectorAll<HTMLSelectElement>(".relation-editor-row select");
    await change(selects[0]!, "paper:paper");await change(selects[2]!, "note_page:core-note");
    await change(host.querySelector<HTMLTextAreaElement>("textarea")!, " New relation Ω ");
    await act(async () => button(host, "创建关系").click());
    expect(invoke).toHaveBeenCalledWith("save_runtime_relation_assertion", {packageKey: "e".repeat(64), draft: {subject: {kind: "paper", id: "paper"}, object: {kind: "note_page", id: "core-note"}, predicateKind: "related_to", scopeNote: " New relation Ω ", changeReason: "manual_create"}});
    expect(host.querySelector(".relation-list")?.textContent).toContain("New relation Ω");
    await act(async () => button(host.querySelector(".relation-list")!, "编辑").click());
    await change(host.querySelector<HTMLTextAreaElement>("textarea")!, " Edited relation Ω ");
    await act(async () => button(host, "保存新版本").click());expect(saved?.revision).toBe(2);
    expect(invoke).toHaveBeenCalledWith("save_runtime_relation_assertion", {packageKey: "e".repeat(64), draft: {id: "relation", expectedRevision: 1, subject: {kind: "paper", id: "paper"}, object: {kind: "note_page", id: "core-note"}, predicateKind: "related_to", scopeNote: " Edited relation Ω ", changeReason: "manual_edit"}});
    await act(async () => button(host.querySelector(".relation-list")!, "版本").click());expect(host.querySelector('[aria-label="版本历史"]')?.textContent).toContain("v2人工保存");
    expect(invoke).toHaveBeenCalledWith("list_runtime_research_object_revisions", {packageKey: "e".repeat(64), objectKind: "relation_assertion", objectId: "relation"});
    await act(async () => button(host.querySelector(".relation-list")!, "编辑").click());await change(host.querySelector<HTMLTextAreaElement>("textarea")!, " Retained draft ");reject = true;const successes = info.mock.calls.length;
    await act(async () => button(host, "保存新版本").click());expect(host.querySelector<HTMLTextAreaElement>("textarea")?.value).toBe(" Retained draft ");expect(info).toHaveBeenCalledTimes(successes);expect(error).toHaveBeenLastCalledWith(expect.stringMatching(/revision_conflict.*relation-request.*仍保留/u));expect(button(host, "保存新版本").disabled).toBe(false);
  } finally {await act(async () => root.unmount());host.remove();release();vi.useRealTimers();vi.restoreAllMocks();vi.unstubAllGlobals();}
});
