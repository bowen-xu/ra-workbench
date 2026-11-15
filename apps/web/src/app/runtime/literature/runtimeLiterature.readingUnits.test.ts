// @vitest-environment happy-dom
import {afterEach, expect, it, vi} from "vitest";
import type {StructuredReadingUnit, StructuredReadingUnitDraft} from "@research-assistant/core/knowledge";
import type {RuntimeLiteraturePort} from "@research-assistant/workbench-sdk/literature-runtime";
import {createRuntimeLiteraturePort, RuntimeLiteratureError} from "./runtimeLiterature";
import {selectAppRuntime} from "../../../core/runtime/appRuntime";

const platform = vi.hoisted(() => ({invoke: vi.fn()}));
vi.mock("../../../core/runtime/platformCore", () => ({invoke: platform.invoke}));
const key = "e".repeat(64), capabilities = [{id: "literature.reading-units", version: 1}];
const unit: StructuredReadingUnit = {id: "unit", projectId: "project", paperId: "paper", unitKind: "other", title: "Original", body: "Original body",
  provenanceKind: "human_original", epistemicRole: "working_note", revision: 8, evidenceCount: 1, createdAt: "created", updatedAt: "updated"};
const draft: StructuredReadingUnitDraft = {id: " unit\u0000 ", projectId: " project ", paperId: " paper\u0000 ",
  unitKind: "other", title: " Original ", body: " Original body\n", epistemicRole: "working_note", anchorId: "anchor",
  expectedRevision: 7, evidenceStance: "context", evidenceRationale: " Original rationale ", changeReason: " Original change "};
const cases = [
  {command: "list_runtime_structured_reading_units", args: {projectId: undefined, paperId: " paper\u0000 "}, result: [unit],
    run: (port: RuntimeLiteraturePort) => port.knowledge.listReadingUnits(undefined, " paper\u0000 ")},
  {command: "save_runtime_structured_reading_unit", args: {draft}, result: unit,
    run: (port: RuntimeLiteraturePort) => port.knowledge.saveReadingUnit(draft)},
] as const;
afterEach(() => {platform.invoke.mockReset(); selectAppRuntime({mode: "browser", storage: localStorage, gateways: {}, dispose() {}});});

it.each(cases)("binds $command to the selected package without rewriting original arguments", async operation => {
  const invoke = vi.fn(async () => operation.result);
  await expect(operation.run(createRuntimeLiteraturePort(key, capabilities, () => undefined, invoke))).resolves.toBe(operation.result);
  expect(invoke).toHaveBeenCalledExactlyOnceWith(operation.command, {packageKey: key, ...operation.args});
});

it.each(cases)("refuses $command before IPC when capability, current activation or transport is unavailable", async operation => {
  const invoke = vi.fn();
  for (const caps of [[], [{id: "literature.concepts", version: 1}], [{id: "literature.reading-units", version: 2}]])
    await expect(operation.run(createRuntimeLiteraturePort(key, caps, () => undefined, invoke)))
      .rejects.toMatchObject({code: "literature_gateway_unavailable", packageKey: key});
  await expect(operation.run(createRuntimeLiteraturePort(key, capabilities, () => {
    throw Object.assign(new Error("closed"), {code: "plugin_activation_context_closed"});
  }, invoke))).rejects.toMatchObject({code: "plugin_activation_context_closed", packageKey: key});
  await expect(operation.run(createRuntimeLiteraturePort(key, capabilities, () => undefined)))
    .rejects.toMatchObject({code: "literature_gateway_unavailable"});
  expect(invoke).not.toHaveBeenCalled();
});

it.each(cases)("keeps failure identity and the original current revision for $command without replay", async operation => {
  const failure = {code: "revision_conflict", message: "Original conflict", currentRevision: 9,
    instanceId: "native-reading", requestId: "reading-request"};
  const invoke = vi.fn(async () => {throw failure;});
  await expect(operation.run(createRuntimeLiteraturePort(key, capabilities, () => undefined, invoke)))
    .rejects.toMatchObject({code: failure.code, currentRevision: 9, instanceId: "native-reading", requestId: "reading-request", packageKey: key, cause: failure});
  expect(invoke).toHaveBeenCalledOnce();
});

it("never redirects a captured manual-reading write to a replacement desktop runtime", async () => {
  const runtime = () => ({mode: "desktop" as const, storage: localStorage, gateways: {}, dispose() {}});
  selectAppRuntime(runtime()); const port = createRuntimeLiteraturePort(key, capabilities, () => undefined);
  selectAppRuntime(runtime());
  await expect(port.knowledge.saveReadingUnit(draft)).rejects.toMatchObject({code: "literature_gateway_unavailable"});
  expect(platform.invoke).not.toHaveBeenCalled();
});

it.each(["notStarted", "rolledBack", "committed", "unknown"])("preserves %s reading-unit outcome and only formats the agreed safe diagnostic fields", async outcome => {
  const readingUnits = {phase: "postcommit-read", outcome, readingUnitId: "unit", currentRevision: 9,
    problem: {kind: "database", diagnostic: {code: "database_error", safeCause: "database_busy", sqliteCode: "DatabaseBusy", sqliteExtendedCode: 5,
      sql: "private-sql", body: "private-body"}}, rollbackError: {code: "database_error", safeCause: "database_busy", sqliteExtendedCode: 5}, draft};
  const failure = {code: "storage_error", message: "Original reading failure", instanceId: "native-reading", requestId: "reading-request", readingUnits};
  const invoke = vi.fn(async () => {throw failure;});
  const error = await createRuntimeLiteraturePort(key, capabilities, () => undefined, invoke).knowledge.saveReadingUnit(draft)
    .then(() => {throw new Error("Expected original failure");}, reason => reason as RuntimeLiteratureError);
  expect(error.readingUnits).toBe(readingUnits); expect(error.cause).toBe(failure);
  for (const detail of ["readingUnits.phase=postcommit-read", `readingUnits.outcome=${outcome}`, "readingUnits.readingUnitId=unit",
    "readingUnits.currentRevision=9", "readingUnits.problem=database", "sqliteExtendedCode=5"])
    expect(error.message).toContain(detail);
  expect(error.message).not.toMatch(/private-|Original body|Original rationale/u);
  expect(invoke).toHaveBeenCalledOnce();
});

it("omits unrecognized reading-unit phases, outcomes and problem diagnostics without dropping the original failure", async () => {
  const readingUnits = {phase: "private-phase", outcome: "private-outcome", problem: {kind: "private-kind", diagnostic: "private-diagnostic"}};
  const failure = {code: "invalid_input", message: "Original invalid reading input", readingUnits};
  const invoke = vi.fn(async () => {throw failure;});
  const error = await createRuntimeLiteraturePort(key, capabilities, () => undefined, invoke).knowledge.saveReadingUnit(draft)
    .then(() => {throw new Error("Expected original failure");}, reason => reason as RuntimeLiteratureError);
  expect(error.readingUnits).toBe(readingUnits); expect(error.message).toContain("invalid_input");
  expect(error.message).not.toContain("private-"); expect(invoke).toHaveBeenCalledOnce();
});
