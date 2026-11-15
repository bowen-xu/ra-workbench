// @vitest-environment happy-dom
import {afterEach, expect, it, vi} from "vitest";
import type {AttachmentRemovalResult, AttachmentSummary, ImportJob, LiteratureDraft, PageText} from "@research-assistant/core/literature";
import type {ReaderViewStateV2} from "@research-assistant/core/reader";
import type {RuntimeLiteraturePort} from "@research-assistant/workbench-sdk/literature-runtime";
import {selectAppRuntime} from "../../../core/runtime/appRuntime";
import {createRuntimeLiteraturePort} from "./runtimeLiterature";

const platform = vi.hoisted(() => ({invoke: vi.fn()}));
vi.mock("../../../core/runtime/platformCore", () => ({invoke: platform.invoke}));
const packageKey = "a".repeat(64);
const capabilities = ["literature.attachments", "literature.pdf-import"].map(id => ({id, version: 1}));
const draft: LiteratureDraft = {itemType: "document", title: " 原PDF\u0000 ", abstractText: "", containerTitle: "", volume: "", issue: "", pages: "",
  doi: "", url: "", language: "", bibtexKey: "", creators: [], tags: ["same", "same"]};
const candidates = [{path: " /synthetic 原文.pdf ", draft}];
const attachment: AttachmentSummary = {id: "attachment", displayName: "原文.pdf", mimeType: "application/pdf", byteSize: 3, contentSha256: "b".repeat(64)};
const removal: AttachmentRemovalResult = {fileStatus: "cleanup_failed", warning: "ATTACHMENT_STORAGE_CLEANUP_FAILED"};
const job: ImportJob = {id: "job", sourceKind: "pdf", state: "running", totalCount: 1, completedCount: 0,
  cancellationRequested: false, results: [], createdAt: "created", updatedAt: "updated"};
const state: ReaderViewStateV2 = {schemaVersion: 2, pageIndex: 7, scale: {kind: "page-width"}, top: 5, left: 3, scrollMode: "vertical", spreadMode: "none"};
const pages: readonly PageText[] = [{pageIndex: 2, text: " 原文\u0000 "}, {pageIndex: 0, text: "Second\n"}];
type Gateway = RuntimeLiteraturePort["literature"];
const calls = [
  {method: "attachPdf", run: (gateway: Gateway) => gateway.attachPdf(" paper ", " /synthetic 原文.pdf "), command: "attach_runtime_pdf_path", args: {itemId: " paper ", path: " /synthetic 原文.pdf "}},
  {method: "renameAttachment", run: (gateway: Gateway) => gateway.renameAttachment(" paper ", "attachment", " New\u0000.pdf "), command: "rename_runtime_literature_attachment", args: {itemId: " paper ", attachmentId: "attachment", displayName: " New\u0000.pdf "}},
  {method: "removeAttachment", run: (gateway: Gateway) => gateway.removeAttachment(" paper ", "attachment"), command: "remove_runtime_literature_attachment", args: {itemId: " paper ", attachmentId: "attachment"}},
  {method: "readAttachment", run: (gateway: Gateway) => gateway.readAttachment("attachment"), command: "read_runtime_attachment", args: {attachmentId: "attachment"}},
  {method: "revealAttachments", run: (gateway: Gateway) => gateway.revealAttachments(["z", "a", "z"]), command: "reveal_runtime_attachments_in_file_manager", args: {attachmentIds: ["z", "a", "z"]}},
  {method: "saveReadingState", run: (gateway: Gateway) => gateway.saveReadingState(" paper ", "attachment", state), command: "save_runtime_reading_state", args: {itemId: " paper ", attachmentId: "attachment", pageIndex: 7, zoom: 1, viewStateJson: JSON.stringify(state)}},
  {method: "getReadingState", run: (gateway: Gateway) => gateway.getReadingState(" paper ", "attachment"), command: "get_runtime_reading_state", args: {itemId: " paper ", attachmentId: "attachment"}},
  {method: "isAttachmentIndexed", run: (gateway: Gateway) => gateway.isAttachmentIndexed("attachment"), command: "runtime_attachment_is_indexed", args: {attachmentId: "attachment"}},
  {method: "indexAttachment", run: (gateway: Gateway) => gateway.indexAttachment(" paper ", "attachment", pages), command: "index_runtime_attachment_text", args: {itemId: " paper ", attachmentId: "attachment", pages}},
  {method: "importPdfs", run: (gateway: Gateway) => gateway.importPdfs(candidates), command: "start_runtime_pdf_import", args: {candidates}},
] as const;
const browser = () => ({mode: "browser" as const, storage: window.localStorage, gateways: {}, dispose() {}});
afterEach(() => {platform.invoke.mockReset(); selectAppRuntime(browser());});

it("routes all original attachment methods and PDF import to their fixed commands without rewriting input", async () => {
  const bytes = new Uint8Array([0, 255, 42]);
  const invoke = vi.fn(async (command: string): Promise<unknown> => {
    if (command === "attach_runtime_pdf_path") return attachment;
    if (command === "remove_runtime_literature_attachment") return removal;
    if (command === "read_runtime_attachment") return bytes.buffer;
    if (command === "get_runtime_reading_state") return {pageIndex: 0, zoom: 1, viewStateJson: JSON.stringify(state)};
    if (command === "runtime_attachment_is_indexed") return false;
    if (command === "start_runtime_pdf_import") return job;
    return undefined;
  });
  const gateway = createRuntimeLiteraturePort(packageKey, capabilities, () => undefined, invoke).literature;
  const actual = [];
  for (const call of calls) actual.push(await call.run(gateway));
  expect(actual).toEqual([attachment, undefined, removal, bytes, undefined, undefined, state, false, undefined, job]);
  expect(invoke.mock.calls).toEqual(calls.map(call => [call.command, {packageKey, ...call.args}]));
});

it("keeps each original binary response form rather than exposing an object path or encoded string", async () => {
  const bytes = new Uint8Array([0, 255, 42]);
  const invoke = vi.fn(), gateway = createRuntimeLiteraturePort(packageKey, capabilities, () => undefined, invoke).literature;
  for (const value of [bytes, bytes.buffer, [...bytes]]) {
    invoke.mockResolvedValueOnce(value);
    const result = await gateway.readAttachment("attachment");
    expect(result).toEqual(bytes);
    if (value === bytes) expect(result).toBe(bytes);
    if (value === bytes.buffer) expect(result.buffer).toBe(bytes.buffer);
  }
  expect(invoke.mock.calls).toEqual(Array.from({length: 3}, () => ["read_runtime_attachment", {packageKey, attachmentId: "attachment"}]));
});

it("keeps V2-first decoding, nullable absence, invalid JSON fallback and legacy percentage zoom", async () => {
  const invoke = vi.fn(), gateway = createRuntimeLiteraturePort(packageKey, capabilities, () => undefined, invoke).literature;
  invoke.mockResolvedValueOnce({pageIndex: 0, zoom: 1, viewStateJson: JSON.stringify({...state, extra: "discarded"})});
  await expect(gateway.getReadingState("paper", "attachment")).resolves.toEqual(state);
  invoke.mockResolvedValueOnce(null);
  await expect(gateway.getReadingState("paper", "attachment")).resolves.toBeUndefined();
  for (const viewStateJson of [undefined, null, "invalid JSON", JSON.stringify({...state, schemaVersion: 1})]) {
    invoke.mockResolvedValueOnce({pageIndex: 2, zoom: 197.75, ...(viewStateJson === undefined ? {} : {viewStateJson})});
    await expect(gateway.getReadingState("paper", "attachment")).resolves.toEqual({schemaVersion: 2, pageIndex: 2,
      scale: {kind: "numeric", value: 1.9775}, scrollMode: "vertical", spreadMode: "none"});
  }
  const numeric: ReaderViewStateV2 = {...state, scale: {kind: "numeric", value: 2.65}};
  invoke.mockResolvedValueOnce(undefined);
  await gateway.saveReadingState("paper", "attachment", numeric);
  expect(invoke).toHaveBeenLastCalledWith("save_runtime_reading_state", {packageKey, itemId: "paper", attachmentId: "attachment",
    pageIndex: 7, zoom: 2.65, viewStateJson: JSON.stringify(numeric)});
});

it.each(calls)("requires the explicit capability/current activation for $method without consulting static IPC", async operation => {
  const invoke = vi.fn(), capability = operation.method === "importPdfs" ? "literature.pdf-import" : "literature.attachments";
  for (const version of [undefined, 2]) {
    const declared = capabilities.filter(value => value.id !== capability);
    if (version !== undefined) declared.push({id: capability, version});
    await expect(operation.run(createRuntimeLiteraturePort(packageKey, declared, () => undefined, invoke).literature))
      .rejects.toMatchObject({code: "literature_gateway_unavailable"});
  }
  await expect(operation.run(createRuntimeLiteraturePort(packageKey, capabilities, () => {
    throw Object.assign(new Error("closed"), {code: "plugin_activation_context_closed"});
  }, invoke).literature)).rejects.toMatchObject({code: "plugin_activation_context_closed"});
  await expect(operation.run(createRuntimeLiteraturePort(packageKey, capabilities, () => undefined).literature))
    .rejects.toMatchObject({code: "literature_gateway_unavailable"});
  expect(invoke).not.toHaveBeenCalled(); expect(platform.invoke).not.toHaveBeenCalled();
});

it.each(calls)("preserves safe $method failure identity after one dispatch without replay", async operation => {
  const reason = {code: "storage_error", message: "原附件操作失败", instanceId: "instance-1", requestId: "request-1", operationId: "operation-1"};
  const invoke = vi.fn(async () => {throw reason;}), gateway = createRuntimeLiteraturePort(packageKey, capabilities, () => undefined, invoke).literature;
  const error = await operation.run(gateway).catch(value => value);
  expect(error).toMatchObject({code: reason.code, instanceId: reason.instanceId, requestId: reason.requestId,
    operationId: reason.operationId, packageKey, cause: reason});
  expect(error.message).toContain(reason.message);
  expect(invoke).toHaveBeenCalledExactlyOnceWith(operation.command, {packageKey, ...operation.args});
});

it("does not direct a captured attachment method into a replacement desktop data owner", async () => {
  const original = {mode: "desktop" as const, storage: window.localStorage, gateways: {}, dispose() {}};
  selectAppRuntime(original);
  const gateway = createRuntimeLiteraturePort(packageKey, capabilities, () => undefined).literature;
  selectAppRuntime({...original});
  for (const operation of calls) await expect(operation.run(gateway)).rejects.toMatchObject({code: "literature_gateway_unavailable"});
  expect(platform.invoke).not.toHaveBeenCalled();
});

it("retains safe attachment settlement details without printing arbitrary SQL, paths or payloads", async () => {
  const attachments = {phase: "attach-other-items", sql: "returned", attachmentId: "attachment-1", contentPublishReturned: true,
    attachmentInsertReturned: true, duplicateInsertReturns: 2, data: {message: "private SQL payload"},
    database: {code: "storage_error", safeCause: "sqlite_operation_failed", sqliteExtendedCode: 1811, message: "private SQL payload"},
    rollback: {code: "storage_error", safeCause: "rollback_failed", message: "private path"}};
  const invoke = vi.fn(async () => {throw {code: "storage_error", message: "已提交，请刷新核对", attachments};});
  const gateway = createRuntimeLiteraturePort(packageKey, capabilities, () => undefined, invoke).literature;
  const error = await gateway.attachPdf("paper", "/synthetic.pdf").catch(value => value);
  expect(error.attachments).toBe(attachments);
  for (const field of ["attachments.phase=attach-other-items", "attachments.sql=returned", "attachments.attachmentId=attachment-1",
    "attachments.contentPublishReturned=true", "attachments.attachmentInsertReturned=true", "attachments.duplicateInsertReturns=2",
    "attachments.database.sqliteExtendedCode=1811", "attachments.rollback.safeCause=rollback_failed"])
    expect(error.message).toContain(field);
  expect(error.message).not.toMatch(/private|payload|synthetic\.pdf/u);
  expect(invoke).toHaveBeenCalledTimes(1);
});
