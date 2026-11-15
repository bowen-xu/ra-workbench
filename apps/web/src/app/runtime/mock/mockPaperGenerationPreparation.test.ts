// @vitest-environment happy-dom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { LiteratureItem } from "@research-assistant/core/literature";
import type { AiTaskSpecDraft } from "@research-assistant/core/ai";
import type { AiAssistanceGateway } from "../../../core/services/aiAssistanceGateway";
import { createPreparedPaperGenerationGateway } from "../../../../../../plugins/literature_library/src/services/paperGenerationPreparation";
import { prepareInitialReadingTemplate } from "../../../../../../plugins/literature_library/src/features/ai/resources/prepareInitialReadingTemplate";
import { panelItem } from "../../../../../../plugins/literature_library/src/features/ai/paperOverviewTestFixtures";
import { createMockDoclingGateway } from "./mockDoclingGateway";
import { mockPaperPortraitGateway } from "./mockPaperPortraitGateway";
import { readMockPaperContext, savedMockDoclingRuns } from "./mockParsedPaperSources";
import { MockIndependentAgents } from "./MockIndependentAgents";
import { mockInternalConversations } from "./mockInternalConversations";
import { createMockStructuredResources } from "./mockStructuredResources";
import { createPreviewControls } from "../../../core/runtime/previewControls";
import { selectAppStorage } from "../../../core/runtime/appStorage";
import { selectAppRuntime } from "../../../core/runtime/appRuntime";

const state = vi.hoisted(() => ({paper: undefined as LiteratureItem | undefined}));
vi.mock("@research-assistant/literature-library-plugin/literatureGateway", () => ({getLiteratureGateway: () => ({list: async () => [state.paper], get: async () => state.paper})}));
vi.mock("@research-assistant/literature-library-plugin/knowledgeGateway", () => ({getResearchKnowledgeGateway: () => ({listReadingUnits: async () => []})}));
vi.mock("../../../core/features/notes/gateways", () => ({getNotesGateway: () => ({listPages: async () => [], listReadingNotes: async () => [], listLiteratureNotes: async () => []})}));
vi.mock("@research-assistant/writing-projects-plugin/writingGateway", () => ({getWritingGateway: () => ({listManuscripts: async () => []})}));
vi.mock("../../../core/services/aiAssistanceGateway", () => ({getAiAssistanceGateway: () => ({listCandidates: async () => []})}));
vi.mock("../../../core/services/notifications", () => ({notify: {loading: vi.fn(() => "parse-notification"), update: vi.fn(), success: vi.fn(), info: vi.fn(), error: vi.fn()}}));

let agents: MockIndependentAgents;
beforeEach(() => {
  vi.useFakeTimers(); localStorage.clear(); selectAppStorage(localStorage);
  state.paper = {...panelItem(), id: "paper", attachments: [{...panelItem().attachments[0]!, id: "file", displayName: "paper.pdf"}]};
});
afterEach(() => {agents?.dispose(); vi.useRealTimers(); localStorage.clear();});

function fixture() {
  const controls = createPreviewControls(localStorage, () => {});
  controls.configure({delayMs: 0, nextFailure: null});
  agents = new MockIndependentAgents(localStorage, controls);
  const resources = createMockStructuredResources(localStorage);
  selectAppRuntime({mode: "preview", storage: localStorage, gateways: {getStructuredResourceGateway: resources}, dispose() {}});
  const raw = mockPaperPortraitGateway(localStorage) as AiAssistanceGateway;
  const conversations = mockInternalConversations(raw, agents, localStorage, resources);
  const docling = createMockDoclingGateway(localStorage, controls);
  const gateway = createPreparedPaperGenerationGateway(conversations, () => docling);
  const draft: AiTaskSpecDraft = {taskKind: "paper_profile_classification", providerId: "fixture-generation", userGoal: "画像",
    pipelineVersion: "sources-v2", promptVersion: "v1", outputSchemaVersion: 1, maxInputTokens: 24000, maxOutputTokens: 8000, timeoutMs: 5000,
    scopeItems: [{itemKind: "attachment", objectId: "file", parentObjectId: "paper"}]};
  return {gateway, docling, raw, controls, resources, draft};
}

async function settle<T>(operation: Promise<T>): Promise<T> {
  const tracked = operation.then(value => ({value}), error => ({error}));
  await vi.advanceTimersByTimeAsync(4_500);
  const result = await tracked;
  if ("error" in result) throw result.error;
  return result.value;
}

it("uses the production preparation gate before any preview conversation, then reuses the result across stages and archive", async () => {
  const {gateway, resources, draft} = fixture();
  // The normal preview resolves identity without parsing.
  const scope = await gateway.previewTask(draft);
  expect(scope.evidencePreview).toEqual([]);
  const request = {...draft, confirmedScopeHash: scope.scopeHash};
  expect((await gateway.preparePaperReadingContext("paper", ["file"])).attachmentStatuses[0]?.parseStatus).toBe("missing");
  const first = gateway.startRun(request);
  await vi.advanceTimersByTimeAsync(100);
  expect((await agents.catalog.list({archived: false, internal: true, binding: null, before: null, limit: 100})).conversations).toEqual([]);
  expect(savedMockDoclingRuns(localStorage)).toEqual([]);
  const portrait = await settle(first);
  expect(savedMockDoclingRuns(localStorage)).toHaveLength(1);
  expect((await gateway.preparePaperReadingContext("paper", ["file"])).attachmentStatuses[0]?.parseStatus).toBe("available");
  const template = await prepareInitialReadingTemplate(resources, "paper");
  if (!("templateResourceId" in template)) throw new Error("expected managed template");
  const readingRequest = {clientRequestId: crypto.randomUUID(), paperId: "paper", attachmentIds: ["file"], overviewProfileSelectionRevision: 0,
    imageAssets: [], paperTemplateRef: {templateResourceId: template.templateResourceId, templateResourceRevision: template.templateResourceRevision}};
  const reading = await gateway.startPaperAnalysis(readingRequest);
  expect(reading.conversationId).toBe(portrait.sharedConversationId);
  if (!reading.conversationId) throw new Error("missing reading conversation");
  await agents.archives.setArchived({operationId: crypto.randomUUID(), id: reading.conversationId, archived: true, expectedWatermark: (await agents.metadata.read({id: reading.conversationId})).durableCount});
  const next = await gateway.startPaperAnalysis({...readingRequest, clientRequestId: crypto.randomUUID(), regenerate: true});
  expect(next.conversationId).not.toBe(reading.conversationId);
  expect(savedMockDoclingRuns(localStorage)).toHaveLength(1);
});

it("records no Agent turn when conversion fails and permits a normal retry", async () => {
  const {gateway, controls, draft} = fixture();
  const scope = await gateway.previewTask(draft);
  const request = {...draft, confirmedScopeHash: scope.scopeHash};
  controls.configure({delayMs: 0, nextFailure: "docling"});
  await expect(settle(gateway.startRun(request))).rejects.toThrow("模拟操作失败：docling");
  expect(savedMockDoclingRuns(localStorage)).toEqual([]);
  expect((await agents.catalog.list({archived: false, internal: true, binding: null, before: null, limit: 100})).conversations).toEqual([]);
  expect((await settle(gateway.startRun(request))).sharedConversationId).toBeTruthy();
  expect(savedMockDoclingRuns(localStorage)).toHaveLength(1);
});

it("shares pending conversion, persists it across gateway recreation, and invalidates changed file/model inputs", async () => {
  const {docling} = fixture();
  const first = docling.parseAttachment("paper", "file");
  const second = docling.parseAttachment("paper", "file");
  const [a,b] = await settle(Promise.all([first, second]));
  expect(a.id).toBe(b.id); expect(savedMockDoclingRuns(localStorage)).toHaveLength(1);
  const reopened = createMockDoclingGateway(localStorage);
  expect((await settle(reopened.parseAttachment("paper", "file"))).id).toBe(a.id);
  const paper = state.paper;
  if (!paper) throw new Error("missing fixture paper");
  state.paper = {...paper, attachments: paper.attachments.map(file => ({...file, contentSha256: "e".repeat(64)}))};
  expect((await readMockPaperContext(localStorage,"paper",["file"])).attachmentStatuses[0]?.parseStatus).toBe("stale");
  const changed = await settle(reopened.parseAttachment("paper", "file"));
  expect(changed.id).not.toBe(a.id);
  localStorage.setItem("docling-model-identity", "preview-models-updated");
  expect((await readMockPaperContext(localStorage,"paper",["file"])).attachmentStatuses[0]?.parseStatus).toBe("stale");
  expect((await settle(reopened.parseAttachment("paper","file"))).id).not.toBe(changed.id);
});
