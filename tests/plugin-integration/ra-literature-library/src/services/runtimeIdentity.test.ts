// @vitest-environment happy-dom
import {afterEach, expect, it, vi} from "vitest";
import type {ConceptMergeDraft, ConceptSplitDraft} from "@research-assistant/core/knowledge";
import type {RuntimeLiteraturePort} from "@research-assistant/workbench-sdk/literature-runtime";
import {createRuntimeLiteraturePort, selectAppRuntime} from "../../../../../../integration/web/src/app/runtime/literature/runtimeLiteratureWorkspace.testSupport.tsx";
import {configureLiteratureServices} from "../../../../../../plugins/ra-literature-library/src/services/configureLiteratureServices";
import {getResearchKnowledgeGateway} from "../../../../../../plugins/ra-literature-library/src/services/knowledgeGateway";
const key = "b".repeat(64), caps = [{id: "literature.concepts", version: 1}];
const summary = {id: "concept", label: "Concept Ω", revision: 1, termCount: 1, definitionCount: 0, sourceCount: 0};
const impact = {survivor: summary, sources: [{...summary, id: "source"}], affectedRelationIds: ["relation"]};
const event = {id: "event", operation: "merge" as const, ...impact, activeSourceIds: ["source"], rationale: "Reason", createdAt: "created"};
const draft: ConceptMergeDraft = {survivorId: " survivor\u0000 ", survivorExpectedRevision: 1, sources: [{id: " source ", expectedRevision: 1}], rationale: " Reason Ω "};
const split: ConceptSplitDraft = {mergeEventId: " event ", sourceConceptIds: [" source "], rationale: " Split Ω "};
const cases = [
  {method: "previewConceptMerge" as const, request: {kind: "preview", draft}, output: {kind: "impact", impact}, args: [draft], run: (p: RuntimeLiteraturePort) => p.knowledge.previewConceptMerge!(draft), expected: impact},
  {method: "mergeConcepts" as const, request: {kind: "merge", draft}, output: {kind: "event", event}, args: [draft], run: (p: RuntimeLiteraturePort) => p.knowledge.mergeConcepts!(draft), expected: event},
  {method: "splitConcepts" as const, request: {kind: "split", draft: split}, output: {kind: "event", event}, args: [split], run: (p: RuntimeLiteraturePort) => p.knowledge.splitConcepts!(split), expected: event},
  {method: "listConceptIdentityEvents" as const, request: {kind: "list"}, output: {kind: "events", events: [event]}, args: [], run: (p: RuntimeLiteraturePort) => p.knowledge.listConceptIdentityEvents!(), expected: [event]},
];
afterEach(() => selectAppRuntime({mode: "browser", storage: localStorage, gateways: {}, dispose() {}}));
it.each(cases)("preserves $method arguments and original decoding through one fixed package", async operation => {
  const invoke = vi.fn(async () => operation.output), port = createRuntimeLiteraturePort(key, caps, () => undefined, invoke);
  const release = configureLiteratureServices(port);
  try {
    await expect(operation.run(port)).resolves.toEqual(operation.expected);
    const gateway = getResearchKnowledgeGateway();
    await expect(Reflect.apply(gateway[operation.method], gateway, operation.args)).resolves.toEqual(operation.expected);
    expect(invoke).toHaveBeenLastCalledWith("apply_runtime_concept_identity", {packageKey: key, request: operation.request});
    release();await expect(Reflect.apply(gateway[operation.method], gateway, operation.args)).rejects.toMatchObject({code: "literature_gateway_unavailable"});
  } finally {release();}
});
it.each(cases)("refuses $method without its exact capability, method or owner", async operation => {
  const invoke = vi.fn();
  for (const provided of [[], [{id: "literature.concepts", version: 2}]]) await expect(operation.run(createRuntimeLiteraturePort(key, provided, () => undefined, invoke))).rejects.toMatchObject({code: "literature_gateway_unavailable"});
  await expect(operation.run(createRuntimeLiteraturePort(key, caps, () => {throw Object.assign(new Error("closed"), {code: "plugin_activation_context_closed"});}, invoke))).rejects.toMatchObject({code: "plugin_activation_context_closed"});expect(invoke).not.toHaveBeenCalled();
  const port = createRuntimeLiteraturePort(key, caps, () => undefined, invoke), knowledge = {...port.knowledge};delete knowledge[operation.method];
  const release = configureLiteratureServices({...port, knowledge});
  try {await expect(Reflect.apply(getResearchKnowledgeGateway()[operation.method], undefined, operation.args)).rejects.toMatchObject({code: "literature_gateway_unavailable"});} finally {release();}
});
it.each(cases)("rejects wrong $method envelopes and malformed original DTOs", async operation => {
  const invoke = vi.fn(async () => ({kind: "unexpected"}));await expect(operation.run(createRuntimeLiteraturePort(key, caps, () => undefined, invoke))).rejects.toMatchObject({code: "plugin_host_response_mismatch"});
  const malformed = operation.output.kind === "impact" ? {kind: "impact", impact: {...impact, survivor: {...summary, revision: 0}}} : operation.output.kind === "event" ? {kind: "event", event: {...event, operation: "unknown"}} : {kind: "events", events: [{...event, activeSourceIds: 42}]};
  const release = configureLiteratureServices(createRuntimeLiteraturePort(key, caps, () => undefined, async () => malformed));
  try {await expect(Reflect.apply(getResearchKnowledgeGateway()[operation.method], undefined, operation.args)).rejects.toThrow("不兼容的研究知识数据");} finally {release();}
});
it("retains identity settlement, version and safe request diagnostics without replay", async () => {
  const cause = {code: "data_operation_outcome_unknown", message: "Original cause", currentRevision: 3, instanceId: "instance", requestId: "request", conceptIdentity: {phase: "rollback", outcome: "unknown", objectId: "event", rollbackError: {sqliteExtendedCode: 23}, privateBody: "private body"}};
  const invoke = vi.fn(async () => {throw cause;});const error = await createRuntimeLiteraturePort(key, caps, () => undefined, invoke).knowledge.mergeConcepts!(draft).catch(e => e);
  expect(error).toMatchObject({conceptIdentity: cause.conceptIdentity, currentRevision: 3, instanceId: "instance", requestId: "request", cause});expect(error.message).toContain("conceptIdentity.outcome=unknown");expect(error.message).not.toContain("private body");expect(invoke).toHaveBeenCalledOnce();
});
