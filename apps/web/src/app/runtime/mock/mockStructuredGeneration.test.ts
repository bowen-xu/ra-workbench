// @vitest-environment happy-dom
import { expect, it, vi } from "vitest";
import type { AiAssistanceGateway } from "../../../core/services/aiAssistanceGateway";
import type { StructuredResourceGateway } from "../../../core/services/structuredResourceGateway";
import type { PaperAnalysisStartDraft } from "@research-assistant/core/ai";
import type { StructuredResource } from "@research-assistant/core/structured-resources";
import { overviewCandidate, workflow } from "../../../../../../plugins/literature_library/src/features/ai/paperOverviewTestFixtures";
import { mockStructuredGeneration } from "./mockStructuredGeneration";
import { createMockStructuredResources } from "./mockStructuredResources";
import { selectAppStorage } from "../../../core/runtime/appStorage";

it("adopts the frozen published template before the first reading write so its internal grant is discoverable", async () => {
  const candidate = overviewCandidate("reading-candidate", "paper");
  const payload = candidate.revisions[0]!.payload;
  if (payload.kind !== "paper_overview_unit_v1") throw new Error("fixture");
  const reference = {templateResourceId: "template", templateResourceRevision: 2};
  const generated = {...candidate, revisions: [{...candidate.revisions[0]!, payload: {
    ...payload, paperTemplate: {...reference, fields: [{id: "question", title: "问题", question: "为什么？", claimIds: []}]},
  }}]};
  const result = {...workflow("needs_review"), turns: [{...workflow("needs_review").turns[0]!, candidateId: candidate.id}]};
  const ai = {
    startPaperAnalysis: vi.fn(async () => result), listCandidates: vi.fn(async () => [generated]),
  } as unknown as AiAssistanceGateway;
  const target: StructuredResource = {id: "reading", kind: "structuredReading", paperId: "paper",
    title: "结构化阅读", sourceUri: "source://library/paper/reading", revision: 0,
    content: null, reviewBaseline: null, pendingFieldIds: [], reviewedFieldIds: []};
  const bindTemplate = vi.fn(async () => ({...target, revision: 1,
    templateRef: {resourceId: reference.templateResourceId, revision: reference.templateResourceRevision}}));
  const write = vi.fn(async () => ({status: "applied", resource: target, errors: []}));
  const resources = {importLegacy: async () => [], list: async () => [target],
    get: async () => ({...target, revision: 1}), bindTemplate, write} as unknown as StructuredResourceGateway;
  await mockStructuredGeneration(ai, resources).startPaperAnalysis({paperId: "paper"} as PaperAnalysisStartDraft);
  expect(bindTemplate).toHaveBeenCalledWith(expect.objectContaining({templateId: "template", templateRevision: 2, confirmed: false}));
  expect(write).toHaveBeenCalledWith(expect.objectContaining({resourceId: "reading", expectedRevision: 1}));
});

it.each([false, true])("regenerates identical content as additions after durable clear (accepted=%s)", async accepted => {
  const values = new Map<string, string>();
  const storage: Storage = {get length() {return values.size;}, key: index => [...values.keys()][index] ?? null,
    getItem: key => values.get(key) ?? null, setItem: (key, value) => {values.set(key, value);},
    removeItem: key => {values.delete(key);}, clear: () => values.clear()};
  selectAppStorage(storage);
  let resources = createMockStructuredResources(storage);
  const target = (await resources.list("paper")).find(value => value.kind === "structuredReading")!;
  const candidate = overviewCandidate("same", "paper");
  const payload = candidate.revisions[0]!.payload;
  const generated = (await resources.write({resourceId: target.id, expectedRevision: 0,
    requestId: "first", content: JSON.stringify(payload), editorKind: "agent"})).resource;
  const before = accepted ? await resources.review({resourceId: target.id, expectedRevision: generated.revision,
    requestId: "keep", action: "keep", scope: "all"}) : generated;
  const cleared = await resources.review({resourceId: target.id, expectedRevision: before.revision,
    requestId: "clear", action: "clear"});
  expect(cleared).toMatchObject({content: null, reviewBaseline: null, pendingFieldIds: [], reviewedFieldIds: []});
  resources = createMockStructuredResources(storage);
  expect((await resources.get(target.id)).content).toBeNull();
  expect((await resources.getVersion(target.id, before.revision)).content).toEqual(payload);
  await expect(resources.review({resourceId: target.id, expectedRevision: before.revision,
    requestId: "stale-clear", action: "clear"})).rejects.toThrow();
  const result = {...workflow("needs_review"), turns: [{...workflow("needs_review").turns[0]!, candidateId: candidate.id}]};
  const ai = {startPaperAnalysis: async () => result, listCandidates: async () => [candidate]} as unknown as AiAssistanceGateway;
  await mockStructuredGeneration(ai, resources).startPaperAnalysis({paperId: "paper"} as PaperAnalysisStartDraft);
  const regenerated = await resources.get(target.id);
  expect(regenerated.content).toEqual(payload);
  expect(regenerated.reviewBaseline).toBeNull();
  expect(regenerated.pendingFieldIds).toContain("research_question");
  resources = createMockStructuredResources(storage);
  expect((await resources.get(target.id)).pendingFieldIds).toContain("research_question");
});
