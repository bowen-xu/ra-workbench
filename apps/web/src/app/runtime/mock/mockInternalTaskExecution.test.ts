// @vitest-environment happy-dom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { AiCandidate, AiRun, AiTaskSpecDraft, PaperAnalysisStartDraft, PaperPortraitCandidateV1, PaperOverviewUnitCandidateV1, PortraitTemplateField } from "@research-assistant/core/ai";
import type { LiteratureItem } from "@research-assistant/core/literature";
import type { AiAssistanceGateway } from "../../../core/services/aiAssistanceGateway";
import { overviewCandidate, panelItem, workflow } from "../../../../../../plugins/literature_library/src/features/ai/paperOverviewTestFixtures";
import { selectAppRuntime } from "../../../core/runtime/appRuntime";
import { selectAppStorage } from "../../../core/runtime/appStorage";
import { createMockStructuredResources } from "./mockStructuredResources";
import { MockIndependentAgents } from "./MockIndependentAgents";
import { mockInternalConversations } from "./mockInternalConversations";

const state = vi.hoisted(() => ({paper: undefined as LiteratureItem | undefined, candidates: [] as AiCandidate[]}));
vi.mock("@research-assistant/literature-library-plugin/literatureGateway", () => ({getLiteratureGateway: () => ({list: async () => [state.paper], get: async () => state.paper})}));
vi.mock("../../../core/features/notes/gateways", () => ({getNotesGateway: () => ({listPages: async () => [], listReadingNotes: async () => [], listLiteratureNotes: async () => []})}));
vi.mock("@research-assistant/writing-projects-plugin/writingGateway", () => ({getWritingGateway: () => ({listManuscripts: async () => []})}));
vi.mock("@research-assistant/literature-library-plugin/knowledgeGateway", () => ({getResearchKnowledgeGateway: () => ({listReadingUnits: async () => []})}));
vi.mock("../../../core/services/aiAssistanceGateway", () => ({getAiAssistanceGateway: () => ({listCandidates: async () => state.candidates})}));

const controls = {delayMs: 0, nextFailure: null, beforeOperation: async () => undefined, configure() {}, reset() {}, dispose() {}};
let agents: MockIndependentAgents;
beforeEach(() => {
  localStorage.clear(); state.candidates = [];
  const original = panelItem();
  state.paper = {...original, id: "paper", attachments: [{...original.attachments[0]!, id: "attachment", mimeType: "application/pdf", displayName: "selected.pdf"}]};
  selectAppStorage(localStorage);
  agents = new MockIndependentAgents(localStorage, controls);
});
afterEach(() => { agents.dispose(); localStorage.clear(); });

function fixture(withTemplate = false, configurations?: Map<string, import("@research-assistant/core/agent").AgentContextState>) {
  const resources = createMockStructuredResources(localStorage);
  selectAppRuntime({mode: "preview", storage: localStorage, gateways: {getStructuredResourceGateway: resources}, dispose() {}});
  let lastRun: AiRun | undefined;
  const base = {
    listCandidates: async () => state.candidates,
    getRun: async () => lastRun!,
    startRun: async (draft: AiTaskSpecDraft): Promise<AiRun> => {
      const id = crypto.randomUUID();
      const candidate = overviewCandidate(id, "paper");
      const portrait: PaperPortraitCandidateV1 = {kind: "paper_portrait_v1", schemaVersion: 1, coverage: "Only the selected excerpt.",
        claims: (["genre", "mode", "paradigm"] as const).map((dimension, index) => ({id: `claim_${index}`, dimension, label: "不确定",
          description: "缺少足够分类依据。", certainty: "uncertain", evidence: []})),
        template: [{id: "question", title: "问题", question: "回应什么问题？", claimIds: []}]};
      state.candidates.push({...candidate, runId: id, candidateKind: "paper_portrait_v1",
        revisions: [{...candidate.revisions[0]!, payload: portrait}]});
      lastRun = {id, taskKind: draft.taskKind, status: "needs_review", userGoal: draft.userGoal, providerId: "fixture", modelId: "mock",
        scopeHash: "a".repeat(64), inputItemCount: 1, pipelineVersion: "v1", promptVersion: "v1", outputSchemaVersion: 1,
        maxInputTokens: 1000, maxOutputTokens: 1000, timeoutMs: 1000, cancelRequested: false,
        createdAt: "now", updatedAt: "now", scopeItems: [], stages: []};
      return lastRun;
    },
    startPaperAnalysis: async () => {
      let candidate = overviewCandidate("reading", "paper");
      if (withTemplate) {
        const template = (await resources.list()).find(value => value.kind === "readingTemplate" && !value.paperId)!;
        const fields = (template.content as {fields: readonly PortraitTemplateField[]}).fields.map(field => ({...field, claimIds: []}));
        const payload: PaperOverviewUnitCandidateV1 = {kind: "paper_overview_unit_v1", schemaVersion: 1, profileId: "paper_template_v1",
          paperTemplate: {templateResourceId: template.id, templateResourceRevision: template.revision, fields},
          fields: fields.map(field => ({field: field.id, status: "not_found", statements: []}))};
        candidate = {...candidate, revisions: [{...candidate.revisions[0]!, payload}]};
      }
      state.candidates.push(candidate);
      return {...workflow("needs_review"), turns: [{...workflow("needs_review").turns[0]!, candidateId: candidate.id}]};
    },
    getLatestPaperAnalysisWorkflow: async () => undefined,
  } as unknown as AiAssistanceGateway;
  return {resources, base, gateway: mockInternalConversations(base, agents, localStorage, resources, (id,value)=>{if(configurations && !configurations.has(id))configurations.set(id,structuredClone(value));}, id=>configurations?.get(id))};
}
const draft: AiTaskSpecDraft = {taskKind: "paper_profile_classification", providerId: "fixture-generation", userGoal: "保留配置 🧪",
  pipelineVersion: "v1", promptVersion: "v1", outputSchemaVersion: 1, maxInputTokens: 1000, maxOutputTokens: 1000,
  timeoutMs: 1000, scopeItems: [{itemKind: "attachment", objectId: "attachment", parentObjectId: "paper"}]};

async function history(conversationId: string) {
  const events = (await agents.gateway.sessions.readHistory({conversationId, afterSeq: 0, limit: 1000})).events;
  const messages = events.flatMap(event => {
    const payload = event.payload;
    return payload.type === "itemStarted" ? [{...payload, text: events.flatMap(row =>
      row.payload.type === "textDelta" && row.payload.itemId === payload.itemId ? [row.payload.text] : []).join("")}] : [];
  });
  return {events, messages};
}

it("records the portrait's real prompt, source reads and receipts from actual preview writes", async () => {
  const {gateway, resources, base} = fixture();
  const run = await gateway.startRun(draft);
  const {messages} = await history(run.sharedConversationId!);
  const prompt = messages.find(value => value.kind.type === "userMessage")!.text;
  expect(prompt).toContain("@generate-paper-portrait");
  expect(prompt).toContain("[selected.pdf](source://library/paper/attachment)");
  expect(prompt).toContain("保留配置 🧪");
  expect(prompt).toContain("本文模板建议写入到");
  expect(prompt).not.toContain('"quoteExact"');
  const calls = messages.filter(value => value.kind.type === "toolCall");
  expect(calls.map(value => value.kind.type === "toolCall" && value.kind.name)).toContain("research_read_source");
  const reads = calls.filter(value => value.kind.type === "toolCall" && value.kind.name === "research_read_file");
  expect(reads).toHaveLength(2);
  const writes = calls.filter(value => value.kind.type === "toolCall" && value.kind.name === "research_apply_patch");
  expect(writes).toHaveLength(2);
  for (const write of writes) { expect(JSON.parse(write.text).patch).toContain("*** Update File: source://library/paper/"); expect(write.text).not.toContain("*** Add File:"); }
  const results = messages.filter(value => value.kind.type === "toolResult").map(value => JSON.parse(value.text));
  expect(results.filter(value => value.path && value.text === "")).toHaveLength(2);
  expect(results.some(value => value.targets?.[0].structuredResource.status === "applied")).toBe(true);
  expect(results.filter(value => value.targets?.[0].structuredResource.status === "applied")).toHaveLength(2);
  const portrait = (await resources.list("paper")).find(value => value.kind === "portrait")!;
  expect(portrait.revision).toBe(1);
  expect(messages.at(-1)?.text).toContain("Keep／Undo");
  agents.dispose(); agents = new MockIndependentAgents(localStorage, controls);
  const restored = mockInternalConversations(base, agents, localStorage, resources);
  expect((await restored.getLatestPaperPortraitRun("paper"))?.sharedConversationId).toBe(run.sharedConversationId);
  expect((await history(run.sharedConversationId!)).messages).toEqual(messages);
});

it("records structured reading with its own skill, Update File and saved resource", async () => {
  const {gateway, resources} = fixture(true);
  const result = await gateway.startPaperAnalysis({paperId: "paper"} as PaperAnalysisStartDraft);
  const conversationId = localStorage.getItem(`internal-workflow-conversation:${result.id}`)!;
  const {messages} = await history(conversationId);
  const prompt = messages.find(value => value.kind.type === "userMessage")!.text;
  expect(prompt).toContain("@edit-structured-resources");
  expect(prompt).toContain('"kind": "paper_overview_unit_v1"');
  expect(prompt).not.toContain('"text": "question"');
  expect(messages.filter(value => value.kind.type === "toolCall").length).toBeGreaterThanOrEqual(4);
  const saved = (await resources.list("paper")).find(value => value.kind === "structuredReading")!;
  expect(saved.revision).toBeGreaterThan(0);
  const reference = saved.templateRef!;
  const template = await resources.getVersion(reference.resourceId, reference.revision);
  expect(prompt).toContain(template.sourceUri);
  expect(messages.some(value => value.kind.type === "toolResult" && JSON.parse(value.text).path === template.sourceUri &&
    JSON.parse(value.text).version === `structured:${reference.revision}::0`)).toBe(true);
  expect(messages.at(-1)?.text).toContain("结构化阅读记录已保存");
});

it("retains failed tool diagnostics in history without a false saved confirmation", async () => {
  const {gateway, resources} = fixture();
  vi.spyOn(resources, "write").mockRejectedValueOnce(new Error("storage is unavailable"));
  await expect(gateway.startRun(draft)).rejects.toThrow("storage is unavailable");
  const latest = await gateway.getLatestPaperPortraitRun("paper");
  expect(latest?.status).toBe("failed");
  const {events, messages} = await history(latest!.sharedConversationId!);
  expect(messages.at(-1)?.text).toContain("生成未完成");
  expect(messages.at(-1)?.text).not.toContain("文献画像已保存");
  expect(events.at(-1)?.payload).toMatchObject({type: "turnFinished", outcome: "failed"});
  expect(messages.some(value => value.kind.type === "toolResult" && value.kind.isError === true && value.text.includes("storage is unavailable"))).toBe(true);
  expect((await resources.list("paper")).find(value => value.kind === "portrait")?.content).toBeNull();
});

it("regeneration updates the same portrait and retains both tool transcripts", async () => {
  const {gateway, resources} = fixture();
  const first = await gateway.startRun(draft);
  const target = (await resources.list("paper")).find(value => value.kind === "portrait")!;
  const firstTemplate = (await resources.list("paper")).find(value => value.kind === "readingTemplate")!;
  const second = await gateway.startRun(draft);
  expect(second.sharedConversationId).toBe(first.sharedConversationId);
  expect(second.sharedRequestId).not.toBe(first.sharedRequestId);
  expect((await resources.get(target.id)).revision).toBe(2);
  expect((await resources.get(firstTemplate.id)).draft?.revision).toBe(firstTemplate.draft?.revision);
  const {events, messages} = await history(first.sharedConversationId!);
  expect(events.filter(value => value.payload.type === "turnFinished")).toHaveLength(2);
  const patches = messages.filter(value => value.kind.type === "toolCall" && value.kind.name === "research_apply_patch");
  expect(patches).toHaveLength(3);
  expect(JSON.parse(patches.at(-1)!.text).patch).toContain(`*** Update File: ${target.sourceUri}`);
  expect(JSON.parse(patches.at(-1)!.text).patch).toContain('-  "coverage"');
});

it("portrait and reading append to the same history and archive resets the next stage", async () => {
  const {gateway} = fixture();
  const portrait = await gateway.startRun(draft);
  const request = {clientRequestId: crypto.randomUUID(), paperId: "paper", attachmentIds: ["pdf"], overviewProfileSelectionRevision: 1, imageAssets: []};
  const reading = await gateway.startPaperAnalysis(request);
  expect(reading.conversationId).toBe(portrait.sharedConversationId);
  expect((await history(reading.conversationId!)).events.filter(event => event.payload.type === "turnFinished")).toHaveLength(2);
  const repeated = await gateway.startRun(draft);
  expect(repeated.sharedConversationId).toBe(reading.conversationId);
  await agents.archives.setArchived({operationId: crypto.randomUUID(), id: reading.conversationId!, archived: true, expectedWatermark: (await agents.metadata.read({id: reading.conversationId!})).durableCount});
  const next = await gateway.startPaperAnalysis({...request, clientRequestId: crypto.randomUUID(), regenerate: true});
  expect(next.conversationId).not.toBe(reading.conversationId);
  await agents.archives.setArchived({operationId: crypto.randomUUID(), id: reading.conversationId!, archived: false, expectedWatermark: null});
  expect((await gateway.startRun(draft)).sharedConversationId).toBe(next.conversationId);
});

it("keeps large patch arguments and results intact through bounded event deltas", async () => {
  const conversationId = crypto.randomUUID(), requestId = crypto.randomUUID();
  const large = "🧪".repeat(70_000);
  await agents.seedConversation({schemaVersion: 1, conversationId, requestId, turnId: crypto.randomUUID(), contextSnapshotId: crypto.randomUUID(),
    input: {text: "更新长记录", context: [], reasoningEffort: null, model: {providerId: "fixture", modelId: "mock", revision: null}}},
  "长记录", "已完成", true, {steps: [{name: "research_apply_patch", arguments: {patch: large}, output: {text: large}}]});
  const {events, messages} = await history(conversationId);
  const call = messages.find(value => value.kind.type === "toolCall")!;
  expect(JSON.parse(call.text).patch).toBe(large);
  const result = messages.find(value => value.kind.type === "toolResult")!;
  expect(JSON.parse(result.text).text).toBe(large);
  expect(events.filter(value => value.payload.type === "textDelta").every(value =>
    value.payload.type === "textDelta" && Array.from(value.payload.text).length <= 65_536)).toBe(true);
});

it("regeneration keeps saved context, refuses readonly or removed targets, and does not restore excluded sources", async () => {
  const configurations=new Map<string, import("@research-assistant/core/agent").AgentContextState>();
  const {gateway}=fixture(false,configurations);
  const first=await gateway.startRun(draft), id=first.sharedConversationId!;
  const configured=structuredClone(configurations.get(id)!);
  const group=configured.persistent[0]!.resource;
  if(group.kind!=="paperGroup")throw new Error("fixture");
  const target=group.parts.find(part=>part.resource.kind==="structuredObject" && part.resource.access==="readWrite")!;
  const selected={...configured,persistent:[{...configured.persistent[0]!,resource:{...group,parts:group.parts.map(part=>({...part,selected:part===target}))}}]};
  configurations.set(id,selected);
  const second=await gateway.startRun(draft);
  expect(second.sharedConversationId).toBe(id);
  expect(configurations.get(id)).toEqual(selected);
  const {messages}=await history(id);
  const requests=messages.filter(value=>value.kind.type==="toolCall");
  // The first default turn read the PDF; regeneration cannot read it again.
  expect(requests.filter(value=>value.kind.type==="toolCall" && value.kind.name==="research_read_source")).toHaveLength(1);
  configurations.set(id,{...selected,persistent:[{...selected.persistent[0]!,resource:{...group,groupAccess:"readOnly"}}]});
  await expect(gateway.startRun(draft)).rejects.toThrow("读写权限");
  configurations.set(id,{schemaVersion:1,persistent:[],temporary:[]});
  await expect(gateway.startRun(draft)).rejects.toThrow("读写权限");
  expect(configurations.get(id)!.persistent).toEqual([]);
});
