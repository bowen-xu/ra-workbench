// @vitest-environment happy-dom
import { afterEach, expect, it, vi } from "vitest";
import type { AiCandidate } from "@research-assistant/core/ai";
import { createMockStructuredResources } from "./mockStructuredResources";

const legacy = vi.hoisted(() => ({candidates: [] as AiCandidate[]}));
vi.mock("../../../core/services/aiAssistanceGateway", () => ({getAiAssistanceGateway: () => ({
  listCandidates: async () => legacy.candidates,
})}));
vi.mock("@research-assistant/literature-library-plugin/knowledgeGateway", () => ({getResearchKnowledgeGateway: () => ({listReadingUnits: async () => []})}));
afterEach(() => { legacy.candidates = []; });

function storage(): Storage {
  const data = new Map<string, string>();
  return {
    get length() { return data.size; },
    key: index => [...data.keys()][index] ?? null,
    getItem: key => data.get(key) ?? null,
    setItem: (key, value) => { data.set(key, value); },
    removeItem: key => { data.delete(key); }, clear: () => data.clear(),
  };
}

function candidate(id: string, profileId: string, field: string, text: string): AiCandidate {
  return {id, candidateKind: "paper_overview_unit_v1", currentRevision: 1, createdAt: id,
    reviewState: "unreviewed", overviewFieldConfirmations: [],
    evidenceBundle: {items: [{paperId: "mixed"}]}, revisions: [{revision: 1, payload: {
      kind: "paper_overview_unit_v1", schemaVersion: 1, profileId: "reviewed_profile_composition_v1",
      profileRefs: [{profileId, profileVersion: 1}],
      fields: [{field, status: "filled", statements: [{id: `${id}-statement`, text,
        epistemicRole: "working_note", evidenceItemIds: []}]}],
    }}]} as unknown as AiCandidate;
}

it("maps all three deployed reviewed profiles independently and retains exact resource versions after reopen", async () => {
  const saved = storage();
  const gateway = createMockStructuredResources(saved);
  const resources = await gateway.list();
  const names = ["通用阅读", "技术／实证", "哲学／概念"];
  const profiles = names.map(title => resources.find(resource => !resource.paperId && resource.title === title &&
    (resource.content as {fields: readonly {id: string}[]}).fields[0]?.id === "research_question")!);
  expect(profiles.every(resource => resource.revision === 1)).toBe(true);
  expect(new Set(profiles.map(resource => resource.id)).size).toBe(3);
  // The first two profiles deliberately share fields but remain separate identities.
  expect((profiles[0]!.content as {fields: unknown}).fields).toEqual((profiles[1]!.content as {fields: unknown}).fields);
  const reopened = createMockStructuredResources(saved);
  expect(await Promise.all(profiles.map(resource => reopened.getVersion(resource.id, 1)))).toEqual(profiles);
});

it("imports a selected profile even without reading text and pins its precise published version", async () => {
  const saved = storage();
  saved.setItem("research-assistant.paper-reading-profile-selections.v1", JSON.stringify([{
    paperId: "unopened", profiles: [{profileId: "technical_empirical", profileVersion: 1}],
    revision: 4, isPersisted: true,
  }]));
  const gateway = createMockStructuredResources(saved);
  const summary = await gateway.importStartup({presentations: [], contexts: []});
  expect(summary.paperIds).toContain("unopened");
  const reading = (await gateway.list("unopened")).find(resource => resource.kind === "structuredReading")!;
  const template = (await gateway.list()).find(resource => !resource.paperId && resource.title === "技术／实证")!;
  expect(reading.content).toBeNull();
  expect(reading.templateRef).toEqual({resourceId: template.id, revision: 1});
  const proposal = await gateway.write({resourceId: template.id, expectedRevision: 1, requestId: "update-profile",
    content: JSON.stringify({...template.content as object, name: "Updated profile"}), editorKind: "agent"});
  expect(proposal.status).toBe("applied");
  expect((await gateway.get(reading.id)).templateRef).toEqual({resourceId: template.id, revision: 1});
  expect((await gateway.getVersion(template.id, 1)).title).toBe("技术／实证");
});

it("composes mixed profile fields in the displayed order without changing their bodies", async () => {
  legacy.candidates = [candidate("a", "technical_empirical", "research_question", "Manual question"),
    candidate("b", "philosophical_conceptual", "argument", "Saved argument")];
  const gateway = createMockStructuredResources(storage());
  const resources = await gateway.importLegacy({paperId: "mixed", requestId: "mixed-import", presentation: {
    fields: [{candidateId: "b", fieldId: "argument"}, {candidateId: "a", fieldId: "research_question"}], hiddenUnitIds: [],
  }});
  const reading = resources.find(resource => resource.kind === "structuredReading")!;
  const template = resources.find(resource => resource.kind === "readingTemplate")!;
  expect(reading.templateRef).toEqual({resourceId: template.id, revision: 1});
  expect(template.content).toMatchObject({fields: [{id: "argument", title: "核心论证"}, {id: "research_question", title: "研究问题"}]});
  expect(reading.content).toMatchObject({profileId: "paper_template_v1",
    paperTemplate: {templateResourceId: template.id, templateResourceRevision: 1},
    fields: [{field: "argument", statements: [{text: "Saved argument"}]},
      {field: "research_question", statements: [{text: "Manual question"}]}]});
  expect(reading.content).not.toHaveProperty("profileRefs");
});
