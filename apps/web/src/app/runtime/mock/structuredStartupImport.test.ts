// @vitest-environment happy-dom
import { afterEach, expect, it, vi } from "vitest";
import { EMPTY_AGENT_CONTEXT } from "@research-assistant/core/agent";
import { DEFAULT_PORTRAIT_CATALOG } from "@research-assistant/core/ai";
import type { StructuredReadingUnit } from "@research-assistant/core/knowledge";
import type { AgentContextConfiguration } from "../../../core/services/agentContextGateway";
import type { AppRuntime } from "../../../core/runtime/appRuntime";
import { createMockStructuredResources } from "./mockStructuredResources";
import { importMockStartupContexts } from "./importMockStartupContexts";
import { ensureStructuredResourceStartup } from "../../../core/runtime/structuredResourceStartup";

const legacy = vi.hoisted(() => ({units: [] as StructuredReadingUnit[]}));
vi.mock("../../../core/services/aiAssistanceGateway", () => ({getAiAssistanceGateway: () => ({listCandidates: async () => []})}));
vi.mock("@research-assistant/literature-library-plugin/knowledgeGateway", () => ({getResearchKnowledgeGateway: () => ({
  listReadingUnits: async (_project?: string, paperId?: string) => legacy.units.filter(unit => !paperId || unit.paperId === paperId),
})}));
afterEach(() => { legacy.units = []; });
function storage(): Storage { const data = new Map<string, string>(); return {
  get length() { return data.size; }, key: index => [...data.keys()][index] ?? null,
  getItem: key => data.get(key) ?? null, setItem: (key, value) => { data.set(key, value); },
  removeItem: key => { data.delete(key); }, clear: () => data.clear(),
}; }

it("imports a never-opened paper's human body before an ordinary conversation can first edit it", async () => {
  const saved = storage();
  legacy.units = [{id: "manual", paperId: "unopened", unitKind: "research_question", body: "Human preserved body",
    revision: 3, updatedAt: "2026-01-01"} as StructuredReadingUnit];
  const gateway = createMockStructuredResources(saved);
  const before = (await gateway.list("unopened")).find(resource => resource.kind === "structuredReading")!;
  expect(before.content).toBeNull();
  const runtime: AppRuntime = {mode: "preview", storage: saved, gateways: {getStructuredResourceGateway: gateway}, dispose() {}};
  await ensureStructuredResourceStartup(runtime);
  const imported = await gateway.get(before.id);
  expect(imported.content).toMatchObject({fields: expect.arrayContaining([
    expect.objectContaining({field: "research_question", statements: [expect.objectContaining({text: "Human preserved body"})]}),
  ])});
  const changed = {...imported.content as object, coverage: "Ordinary chat edit"};
  const result = await gateway.write({resourceId: imported.id, expectedRevision: imported.revision,
    requestId: "ordinary-chat", content: JSON.stringify(changed), editorKind: "agent"});
  expect(result.status).toBe("applied");
  expect(result.resource.content).toMatchObject({fields: expect.arrayContaining([
    expect.objectContaining({field: "research_question", statements: [expect.objectContaining({text: "Human preserved body"})]}),
  ])});
});

it("imports all custom client templates into the global resource library", async () => {
  const saved = storage();
  saved.setItem("research-assistant.portrait-catalog.v1", JSON.stringify({...DEFAULT_PORTRAIT_CATALOG,
    templates: [{id: "custom", label: "Custom legacy", fields: [{id: "q", title: "Q", question: "Why?", claimIds: []}]}]}));
  const gateway = createMockStructuredResources(saved);
  await ensureStructuredResourceStartup({mode: "preview", storage: saved, gateways: {getStructuredResourceGateway: gateway}, dispose() {}});
  expect((await gateway.list()).some(resource => !resource.paperId && resource.title === "Custom legacy")).toBe(true);
});

it("prefers an existing native empty context over corrupt cache, skips deleted conversations, and strictly imports missing rows", async () => {
  const saved = storage();
  saved.setItem("research-assistant:agent-context:v1:existing", "{");
  saved.setItem("research-assistant:agent-context:v1:deleted", "bad cache");
  saved.setItem("research-assistant:agent-context:v1:new", JSON.stringify({...EMPTY_AGENT_CONTEXT,
    temporary: [{label: "one turn", resource: {kind: "paper", resourceId: "unopened"}}]}));
  const records = new Map<string, AgentContextConfiguration>([["existing", {revision: 7, state: EMPTY_AGENT_CONTEXT}]]);
  const gateway = createMockStructuredResources(saved, {importContexts: async contexts =>
    importMockStartupContexts(records, contexts, id => id !== "deleted")});
  await ensureStructuredResourceStartup({mode: "preview", storage: saved, gateways: {getStructuredResourceGateway: gateway}, dispose() {}});
  expect(records.get("existing")).toEqual({revision: 7, state: EMPTY_AGENT_CONTEXT});
  expect(records.has("deleted")).toBe(false);
  expect(records.get("new")?.state.temporary).toHaveLength(1);
  expect(saved.getItem("research-assistant:agent-context:v1:existing")).toBe("{");
});

it("blocks a corrupt context without native authority and permits retry after its cache is repaired", async () => {
  const saved = storage(), key = "research-assistant:agent-context:v1:new";
  saved.setItem(key, "{");
  const records = new Map<string, AgentContextConfiguration>();
  const gateway = createMockStructuredResources(saved, {importContexts: async contexts =>
    importMockStartupContexts(records, contexts, () => true)});
  const runtime: AppRuntime = {mode: "preview", storage: saved, gateways: {getStructuredResourceGateway: gateway}, dispose() {}};
  await expect(ensureStructuredResourceStartup(runtime)).rejects.toThrow("new");
  expect(records.size).toBe(0);
  saved.setItem(key, JSON.stringify(EMPTY_AGENT_CONTEXT));
  await ensureStructuredResourceStartup(runtime);
  expect(records.get("new")?.state).toEqual(EMPTY_AGENT_CONTEXT);
});
