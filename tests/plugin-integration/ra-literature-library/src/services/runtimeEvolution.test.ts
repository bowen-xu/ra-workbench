// @vitest-environment happy-dom
import {afterEach, expect, it, vi} from "vitest";
import type {ConceptAlgorithmRun} from "@research-assistant/core/knowledge";
import type {RuntimeLiteraturePort} from "@research-assistant/workbench-sdk/literature-runtime";
import {createRuntimeLiteraturePort, selectAppRuntime} from "../../../../../../integration/web/src/app/runtime/literature/runtimeLiteratureWorkspace.testSupport.tsx";
import {configureLiteratureServices} from "../../../../../../plugins/ra-literature-library/src/services/configureLiteratureServices";
import {getResearchKnowledgeGateway} from "../../../../../../plugins/ra-literature-library/src/services/knowledgeGateway";

const key = "e".repeat(64);
const capabilities = [{id: "literature.concepts", version: 1}];
const run: ConceptAlgorithmRun = {
  id: "run", algorithmKind: "concept_evolution", algorithmId: "accepted-paper-concept-timeline",
  algorithmVersion: 1, parametersHash: "a".repeat(64), conceptRevisionWatermark: 3,
  relationRevisionWatermark: 2, paperScopeHash: "b".repeat(64), status: "succeeded", resultCount: 1,
  createdAt: "created", startedAt: "started", finishedAt: "finished",
  evolutionEvents: [{
    ordinal: 0, conceptId: "concept", conceptRevision: 3, conceptLabel: "Concept Ω",
    paperId: "paper", paperTitle: "Paper Ω", assertionId: "assertion", assertionRevision: 2,
    paperFunction: "focal", noveltyStatus: "introduced_by_paper", eventKind: "introduced", explanation: "Original explanation",
  }],
  foundationSimilarities: [],
};
const methods = [
  {method: "analyzeConceptEvolution", command: "analyze_runtime_concept_evolution", args: {conceptId: " concept Ω "}},
  {method: "getConceptAlgorithmRun", command: "get_runtime_concept_algorithm_run", args: {runId: " run Ω "}},
] as const;

afterEach(() => vi.restoreAllMocks());
function select(): void {
  selectAppRuntime({mode: "browser", storage: localStorage, gateways: {}, dispose() {}});
}

it.each(methods)("$method uses the selected package and original decoder", async operation => {
  select();
  const invoke = vi.fn(async () => run);
  const port = createRuntimeLiteraturePort(key, capabilities, () => undefined, invoke);
  const release = configureLiteratureServices(port);
  try {
    const argument = Object.values(operation.args)[0]!;
    const gateway = getResearchKnowledgeGateway();
    const decoded = await gateway[operation.method](argument);
    expect(decoded).toEqual(run);
    expect(invoke).toHaveBeenCalledWith(operation.command, {packageKey: key, ...operation.args});
    release();
    await expect(gateway[operation.method](argument)).rejects.toMatchObject({code: "literature_gateway_unavailable"});
    expect(invoke).toHaveBeenCalledOnce();
  } finally {
    release();
  }
});

it.each(methods)("$method rejects missing methods and mismatched capability versions", async operation => {
  select();
  const invoke = vi.fn();
  const port = createRuntimeLiteraturePort(key, [{id: "literature.concepts", version: 2}], () => undefined, invoke);
  await expect(port.knowledge[operation.method]!("value")).rejects.toMatchObject({code: "literature_gateway_unavailable"});
  expect(invoke).not.toHaveBeenCalled();
  const knowledge = {...port.knowledge};
  delete knowledge[operation.method];
  const release = configureLiteratureServices({...port, knowledge} as RuntimeLiteraturePort);
  try {
    await expect(getResearchKnowledgeGateway()[operation.method]("value")).rejects.toMatchObject({code: "literature_gateway_unavailable"});
  } finally {
    release();
  }
});

it.each(methods)("$method rejects incompatible original result rows", async operation => {
  select();
  const release = configureLiteratureServices(createRuntimeLiteraturePort(key, capabilities, () => undefined, async () => ({...run, resultCount: 3})));
  try {
    await expect(getResearchKnowledgeGateway()[operation.method]("value")).rejects.toThrow("不兼容的概念算法数据");
  } finally {
    release();
  }
});

it.each([
  {phase: "finish", outcome: "statementFailed", code: "storage_error"},
  {phase: "worker", outcome: "unknown", code: "data_operation_outcome_unknown"},
])("preserves the original known run and $phase diagnostics without repeating analysis", async ({phase, outcome, code}) => {
  const failure = {
    code, message: "Original cause", requestId: "request", instanceId: "instance",
    conceptAlgorithm: {phase, outcome, objectId: "run", databaseError: {sqliteExtendedCode: 19}, privateBody: "private"},
  };
  const invoke = vi.fn(async () => {throw failure;});
  const error = await createRuntimeLiteraturePort(key, capabilities, () => undefined, invoke).knowledge.analyzeConceptEvolution!("concept").catch(value => value);
  expect(error).toMatchObject({conceptAlgorithm: failure.conceptAlgorithm, requestId: "request", instanceId: "instance", cause: failure});
  expect(error.message).toContain("conceptAlgorithm.objectId=run");
  expect(error.message).toContain(`conceptAlgorithm.phase=${phase}`);
  expect(error.message).toContain(`conceptAlgorithm.outcome=${outcome}`);
  expect(error.message).not.toContain("private");
  expect(invoke).toHaveBeenCalledOnce();
});

it("retains the original decoder refusal for a persisted running run whose finish statement failed", async () => {
  select();
  const {finishedAt: _finished, ...unfinished} = run;
  const release = configureLiteratureServices(createRuntimeLiteraturePort(key, capabilities, () => undefined, async () => ({...unfinished, status: "running", resultCount: 0})));
  try {
    await expect(getResearchKnowledgeGateway().getConceptAlgorithmRun("run")).rejects.toThrow("不兼容的概念算法数据");
  } finally {
    release();
  }
});
