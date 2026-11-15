// @vitest-environment happy-dom
import {afterEach, expect, it, vi} from "vitest";
import {emptyNalState} from "@research-assistant/core/nal";
import type {Concept, ConceptDraft} from "@research-assistant/core/knowledge";
import type {RuntimeLiteraturePort} from "@research-assistant/workbench-sdk/literature-runtime";
import type {BuiltinCapabilityContract} from "@research-assistant/workbench-sdk";
import {createRuntimeLiteraturePort} from "./runtimeLiterature";
import {selectAppRuntime} from "../../../core/runtime/appRuntime";

const platform = vi.hoisted(() => ({invoke: vi.fn()}));
vi.mock("../../../core/runtime/platformCore", () => ({invoke: platform.invoke}));
const key = "c".repeat(64);
const capabilities = ["literature.nal-bootstrap", "literature.nal-mutation", "literature.nal-annotation", "literature.concepts"]
  .map(id => ({id, version: 1}));
const mutation = {kind: "delete_concept" as const, id: " concept\u0000 "};
const request = {targetKind: "concept" as const, targetId: "concept", title: "Original title"};
const annotation = {id: "annotation", targetKind: "concept" as const, targetId: "concept", notePageId: "core-note"};
const draft: ConceptDraft = {id: "concept", expectedRevision: 7, preferredTerm: " Original\u0000 ", label: " Original\u0000 ",
  explanation: "", disambiguation: "", domain: "", tradition: "", period: ""};
const term = {id: "term", text: "Original", normalizedText: "original", language: "und", provenanceKind: "human_original" as const,
  revision: 1, createdAt: "created", updatedAt: "updated"};
const concept: Concept = {id: "concept", preferredTerm: term, terms: [term], label: "Original", explanation: "", disambiguation: "",
  domain: "", tradition: "", period: "", provenanceKind: "human_original", revision: 8, definitionCount: 0, sourceCount: 0,
  createdAt: "created", updatedAt: "updated"};
const captureDraft = {termText: " Human term Ω ", anchorId: "anchor", usageText: " Original usage ", resolution: {kind: "unresolved" as const}};
const captureSource = {paperId: "paper", paperTitle: "Synthetic capture", attachmentId: "pdf", contentSha256: "a".repeat(64), pageIndex: 0, quoteExact: "Original quote"};
const definitionDraft = {id: "definition", expectedRevision: 3, conceptId: "old-concept", statement: " Definition Ω ", scopeNote: " Scope ", changeReason: " original reason "};
const definition = {id: "definition", conceptId: "old-concept", statement: "Definition Ω", scopeNote: "Scope", provenanceKind: "human_original" as const, epistemicRole: "definition" as const, revision: 4, evidenceCount: 1, createdAt: "created", updatedAt: "updated"};
const revisions = [{id: "revision", objectKind: "definition_claim" as const, objectId: "definition", revision: 4, snapshot: {statement: definition.statement}, editorKind: "human" as const, changeReason: " original reason ", createdAt: "created"}];
const cases = [
  {capability: "literature.concepts", command: "save_runtime_definition_claim", args: {draft: definitionDraft}, run: (port: RuntimeLiteraturePort) => port.knowledge.saveDefinitionClaim!(definitionDraft), result: definition},
  {capability: "literature.concepts", command: "list_runtime_definition_claims", args: {conceptId: " old-concept "}, run: (port: RuntimeLiteraturePort) => port.knowledge.listDefinitionClaims!(" old-concept "), result: [definition]},
  {capability: "literature.concepts", command: "list_runtime_research_object_revisions", args: {objectKind: "definition_claim", objectId: " definition "}, run: (port: RuntimeLiteraturePort) => port.knowledge.listRevisions!("definition_claim", " definition "), result: revisions},
  {capability: "literature.concepts", command: "capture_runtime_research_concept_usage", args: {draft: captureDraft},
    run: (port: RuntimeLiteraturePort) => port.knowledge.captureConceptUsage!(captureDraft, captureSource), result: {term, usage: {...captureSource, id: "usage", term, anchorId: "anchor", usageText: "Original usage", provenanceKind: "human_original", revision: 1, createdAt: "created", updatedAt: "updated"}}},
  {capability: "literature.nal-mutation", command: "mutate_runtime_nal_network", args: {mutation, expectedRevision: 7},
    run: (port: RuntimeLiteraturePort) => port.nal.mutate(mutation, 7), result: emptyNalState()},
  {capability: "literature.nal-annotation", command: "ensure_runtime_nal_annotation", args: {request},
    run: (port: RuntimeLiteraturePort) => port.nal.ensureAnnotation(request), result: annotation},
  {capability: "literature.concepts", command: "search_runtime_research_concepts", args: {query: " Original\u0000 "},
    run: (port: RuntimeLiteraturePort) => port.knowledge.searchConcepts(" Original\u0000 "), result: [concept]},
  {capability: "literature.concepts", command: "save_runtime_research_concept", args: {draft},
    run: (port: RuntimeLiteraturePort) => port.knowledge.saveConcept(draft), result: concept},
] as const;
afterEach(() => {platform.invoke.mockReset(); selectAppRuntime({mode: "browser", storage: window.localStorage, gateways: {}, dispose() {}});});

it.each(cases)("binds $command to the selected package and retains original arguments/results", async operation => {
  const invoke = vi.fn(async () => operation.result);
  const port = createRuntimeLiteraturePort(key, capabilities, () => undefined, invoke);
  await expect(operation.run(port)).resolves.toEqual(operation.result);
  expect(invoke).toHaveBeenCalledExactlyOnceWith(operation.command, {packageKey: key, ...operation.args});
});

it.each(cases)("refuses $command before IPC without its exact capability or current instance", async operation => {
  const invoke = vi.fn();
  const declarations: readonly (readonly BuiltinCapabilityContract[])[] = [[], capabilities.filter(cap => cap.id !== operation.capability),
    capabilities.map(cap => cap.id === operation.capability ? {...cap, version: 2} : cap)];
  for (const caps of declarations) await expect(operation.run(createRuntimeLiteraturePort(key, caps, () => undefined, invoke)))
    .rejects.toMatchObject({code: "literature_gateway_unavailable", packageKey: key});
  await expect(operation.run(createRuntimeLiteraturePort(key, capabilities, () => {
    throw Object.assign(new Error("closed"), {code: "plugin_activation_context_closed"});
  }, invoke))).rejects.toMatchObject({code: "plugin_activation_context_closed", packageKey: key});
  await expect(operation.run(createRuntimeLiteraturePort(key, capabilities, () => undefined)))
    .rejects.toMatchObject({code: "literature_gateway_unavailable"});
  expect(invoke).not.toHaveBeenCalled();
});

it("keeps the original NAL state decoder on mutation and does not replay an invalid response", async () => {
  const invoke = vi.fn(async () => ({...emptyNalState(), revision: -1}));
  await expect(createRuntimeLiteraturePort(key, capabilities, () => undefined, invoke).nal.mutate(mutation, 7))
    .rejects.toThrow("关系数据格式无效");
  expect(invoke).toHaveBeenCalledOnce();
});

it.each(cases)("retains original failure identity and revision for $command without replay", async operation => {
  const failure = {code: "revision_conflict", message: "Original revision conflict", currentRevision: 9,
    instanceId: "native-1", requestId: "request-1"};
  const invoke = vi.fn(async () => {throw failure;});
  await expect(operation.run(createRuntimeLiteraturePort(key, capabilities, () => undefined, invoke)))
    .rejects.toMatchObject({code: failure.code, currentRevision: 9, instanceId: "native-1", requestId: "request-1", packageKey: key, cause: failure});
  expect(invoke).toHaveBeenCalledOnce();
});

it("does not redirect a captured concept write to a replacement App runtime", async () => {
  const desktop = () => ({mode: "desktop" as const, storage: window.localStorage, gateways: {}, dispose() {}});
  selectAppRuntime(desktop()); const port = createRuntimeLiteraturePort(key, capabilities, () => undefined);
  selectAppRuntime(desktop());
  await expect(port.knowledge.saveConcept(draft)).rejects.toMatchObject({code: "literature_gateway_unavailable"});
  expect(platform.invoke).not.toHaveBeenCalled();
});

it("formats only closed NAL outcome fields and preserves unknown/committed distinctions without replay", async () => {
  const failure = {code: "plugin_host_failed", message: "Original NAL failure", instanceId: "native-1", requestId: "request-1",
    mutationOutcome: {status: "unknown", observedRevision: 9}, mutationProblem: {phase: "commit", cause: {kind: "database",
      diagnostic: {code: "database_error", safeCause: "database_busy", sqliteCode: "DatabaseBusy", sqliteExtendedCode: 5, rawSql: "must-not-display"}}},
    annotationOutcome: {status: "committed", annotation: {...annotation, quote: "must-not-display"}},
    annotationProblem: {phase: "linkInsert", cause: {kind: "owner", diagnostic: "must-not-display"}}};
  const invoke = vi.fn(async () => {throw failure;});
  const error = await createRuntimeLiteraturePort(key, capabilities, () => undefined, invoke).nal.mutate(mutation, 7)
    .then(() => {throw new Error("Expected the original NAL failure");}, reason => reason as Error);
  expect(error).toMatchObject({mutationOutcome: failure.mutationOutcome, annotationOutcome: failure.annotationOutcome});
  expect(error.message).toContain("mutationOutcome=unknown"); expect(error.message).toContain("mutationOutcome.observedRevision=9");
  expect(error.message).toContain("annotationOutcome=committed"); expect(error.message).toContain("annotationOutcome.annotation.notePageId=core-note");
  expect(error.message).toContain("mutationProblem.phase=commit"); expect(error.message).toContain("mutationProblem.database.sqliteExtendedCode=5");
  expect(error.message).not.toContain("must-not-display"); expect(invoke).toHaveBeenCalledOnce();
});

it.each(["notStarted", "rolledBack", "committed", "unknown"] as const)("preserves the %s concept prefix with only safe structured diagnostics", async outcome => {
  const concepts = {phase: "semantic", outcome, conceptId: "concept", currentRevision: 9, semanticPhase: "recordInsert",
    problem: {kind: "database", diagnostic: {code: "database_error", safeCause: "database_busy", sqliteCode: "DatabaseBusy", sqliteExtendedCode: 5,
      sql: "must-not-display", draft: "must-not-display"}}, rollbackError: {code: "database_error", safeCause: "connection_unavailable"}};
  const failure = {code: "storage_error", message: "Original concept failure", currentRevision: 9, instanceId: "native-1", requestId: "request-1", concepts};
  const invoke = vi.fn(async () => {throw failure;});
  const error = await createRuntimeLiteraturePort(key, capabilities, () => undefined, invoke).knowledge.saveConcept(draft)
    .then(() => {throw new Error("Expected the original concept failure");}, reason => reason as Error);
  expect(error).toMatchObject({concepts, currentRevision: 9, packageKey: key, instanceId: "native-1", requestId: "request-1"});
  expect(error.message).toContain(`concepts.outcome=${outcome}`); expect(error.message).toContain("concepts.conceptId=concept");
  expect(error.message).toContain("concepts.currentRevision=9"); expect(error.message).toContain("concepts.semanticPhase=recordInsert");
  expect(error.message).toContain("concepts.database.sqliteExtendedCode=5"); expect(error.message).not.toContain("must-not-display");
  expect(invoke).toHaveBeenCalledExactlyOnceWith("save_runtime_research_concept", {packageKey: key, draft});
});
