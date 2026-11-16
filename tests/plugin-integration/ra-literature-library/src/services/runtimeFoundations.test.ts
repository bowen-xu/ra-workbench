// @vitest-environment happy-dom
import {afterEach, expect, it, vi} from "vitest";
import type {ConceptAlgorithmRun} from "@research-assistant/core/knowledge";
import type {RuntimeLiteraturePort} from "@research-assistant/workbench-sdk/literature-runtime";
import {createRuntimeLiteraturePort, selectAppRuntime} from "../../../../../../integration/web/src/app/runtime/literature/runtimeLiteratureWorkspace.testSupport.tsx";
import {configureLiteratureServices} from "../../../../../../plugins/ra-literature-library/src/services/configureLiteratureServices";
import {getResearchKnowledgeGateway} from "../../../../../../plugins/ra-literature-library/src/services/knowledgeGateway";

const key = "f".repeat(64);
const capabilities = [{id: "literature.concepts", version: 1}];
const draft = {paperIds: [" paper-b ", "paper-a", "paper-b"]};
const run: ConceptAlgorithmRun = {
  id: "foundation-run", algorithmKind: "shared_foundations", algorithmId: "foundational-concept-weighted-jaccard",
  algorithmVersion: 1, parametersHash: "a".repeat(64), conceptRevisionWatermark: 3,
  relationRevisionWatermark: 2, paperScopeHash: "b".repeat(64), status: "succeeded", resultCount: 1,
  createdAt: "created", startedAt: "started", finishedAt: "finished", evolutionEvents: [],
  foundationSimilarities: [{ordinal: 0, leftPaperId: "paper-a", leftPaperTitle: "Left Ω", rightPaperId: "paper-b", rightPaperTitle: "Right Ω", sharedConcepts: [{conceptId: "concept", conceptRevision: 3, label: "Label Ω", weight: 0.5}], weightedJaccard: 0.5, sharedConceptCount: 1, leftFoundationCount: 2, rightFoundationCount: 1}],
};
afterEach(() => vi.restoreAllMocks());
function select(): void {
  selectAppRuntime({mode: "browser", storage: localStorage, gateways: {}, dispose() {}});
}

it("uses the selected package and original decoder, and stops after disposal", async () => {
  select();
  const invoke = vi.fn(async () => run);
  const release = configureLiteratureServices(createRuntimeLiteraturePort(key, capabilities, () => undefined, invoke));
  try {
    const gateway = getResearchKnowledgeGateway();
    expect(await gateway.analyzeSharedFoundations(draft)).toEqual(run);
    expect(invoke).toHaveBeenCalledWith("analyze_runtime_shared_foundations", {packageKey: key, draft});
    release();
    await expect(gateway.analyzeSharedFoundations(draft)).rejects.toMatchObject({code: "literature_gateway_unavailable"});
    expect(invoke).toHaveBeenCalledOnce();
  } finally { release(); }
});

it("requires the existing capability version and checks the optional method", async () => {
  select();
  const invoke = vi.fn();
  const port = createRuntimeLiteraturePort(key, [{id: "literature.concepts", version: 2}], () => undefined, invoke);
  await expect(port.knowledge.analyzeSharedFoundations!(draft)).rejects.toMatchObject({code: "literature_gateway_unavailable"});
  expect(invoke).not.toHaveBeenCalled();
  const knowledge = {...port.knowledge};
  delete knowledge.analyzeSharedFoundations;
  const release = configureLiteratureServices({...port, knowledge} as RuntimeLiteraturePort);
  try { await expect(getResearchKnowledgeGateway().analyzeSharedFoundations(draft)).rejects.toMatchObject({code: "literature_gateway_unavailable"}); }
  finally { release(); }
});

it.each([
  {...run, resultCount: 2},
  {...run, foundationSimilarities: [{...run.foundationSimilarities[0], weightedJaccard: Number.NaN}]},
  {...run, foundationSimilarities: [{...run.foundationSimilarities[0], sharedConceptCount: 2}]},
])("retains original foundation row validation", async invalid => {
  select();
  const release = configureLiteratureServices(createRuntimeLiteraturePort(key, capabilities, () => undefined, async () => invalid));
  try { await expect(getResearchKnowledgeGateway().analyzeSharedFoundations(draft)).rejects.toThrow("不兼容的概念算法数据"); }
  finally { release(); }
});

it("preserves known-run failure diagnostics with safe messages and one invocation", async () => {
  const failure = {code: "storage_error", message: "Original cause", requestId: "request", instanceId: "instance", conceptAlgorithm: {phase: "finish", outcome: "statementFailed", objectId: "foundation-run", databaseError: {sqliteExtendedCode: 19}, privateBody: "private"}};
  const invoke = vi.fn(async () => {throw failure;});
  const error = await createRuntimeLiteraturePort(key, capabilities, () => undefined, invoke).knowledge.analyzeSharedFoundations!(draft).catch(value => value);
  expect(error).toMatchObject({conceptAlgorithm: failure.conceptAlgorithm, requestId: "request", instanceId: "instance", cause: failure});
  expect(error.message).toContain("conceptAlgorithm.objectId=foundation-run");
  expect(error.message).not.toContain("private");
  expect(invoke).toHaveBeenCalledOnce();
});
