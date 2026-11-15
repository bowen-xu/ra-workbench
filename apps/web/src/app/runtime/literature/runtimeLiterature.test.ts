// @vitest-environment happy-dom
import {afterEach, expect, it, vi} from "vitest";
import type {ImportJob, LiteratureItem, MetadataPreview} from "@research-assistant/core/literature";
import type {ReferenceRecord, ReferenceRecordDraft} from "@research-assistant/core/synthesis";
import {emptyNalState} from "@research-assistant/core/nal";
import type {BuiltinCapabilityContract} from "@research-assistant/workbench-sdk";
import type {RuntimeLiteraturePort} from "@research-assistant/workbench-sdk/literature-runtime";
import {createRuntimeLiteraturePort} from "./runtimeLiterature";
import {selectAppRuntime} from "../../../core/runtime/appRuntime";

const platform = vi.hoisted(() => ({invoke: vi.fn()}));
vi.mock("../../../core/runtime/platformCore", () => ({invoke: platform.invoke}));

const key = "b".repeat(64);
const caps = ["literature.catalog-read", "literature.catalog-save", "literature.duplicate-review", "literature.doi-preview", "literature.metadata",
  "literature.import-jobs", "literature.nal-bootstrap", "literature.nal-query-collections", "literature.duplicate-merge",
  "literature.duplicate-undo", "literature.lifecycle", "literature.trash-cleanup", "literature.reference-records"].map(id => ({id, version: 1}));
const item: LiteratureItem = {id: "paper", itemType: "journalArticle", title: "Original", abstractText: "", doi: "10.1000/test",
  containerTitle: "", volume: "", issue: "", pages: "", url: "", language: "", bibtexKey: "", creators: [], tags: [],
  attachments: [], projectIds: [], metadataSources: [], readingStatus: "unread", createdAt: "created", updatedAt: "updated"};
const preview: MetadataPreview = {id: "candidate", importJobId: "job", sourceKind: "doi.org:csl-json",
  sourceReference: "10.1000/test", draft: item, diffs: []};
const job: ImportJob = {id: "job", sourceKind: "doi", state: "queued", totalCount: 1, completedCount: 0,
  cancellationRequested: false, results: [], createdAt: "created", updatedAt: "updated"};
const stored = {revision: 1, definitions: [], cache: null, legacyImported: true};
const content = {definitions: [], cache: null, legacyImported: true};
const reference: ReferenceRecord = {id: "reference-1", citingPaperId: "paper", sourceAnchorId: "anchor-1", rawText: "Original reference",
  fields: [{name: "title", value: "Original title", sourceKind: "reference_text", confirmedByUser: false}], parseState: "parsed",
  identityState: "unverified", semanticState: "unreviewed", usageSummary: "", provenanceKind: "human_original", revision: 1,
  createdAt: "created", updatedAt: "updated"};
const referenceDraft: ReferenceRecordDraft = {id: reference.id, citingPaperId: reference.citingPaperId, sourceAnchorId: reference.sourceAnchorId,
  rawText: " Original reference\n", fields: reference.fields, parseState: reference.parseState, identityState: reference.identityState,
  semanticState: reference.semanticState, usageSummary: "", expectedRevision: 1, changeReason: " human review "};
const browser = () => ({mode: "browser" as const, storage: window.localStorage, gateways: {}, dispose() {}});
afterEach(() => {platform.invoke.mockReset(); selectAppRuntime(browser());});

it("routes the original reference list and save through the selected package without rewriting IDs, draft or result", async () => {
  const rows = [reference], invoke = vi.fn(async (command: string): Promise<unknown> => command === "list_runtime_reference_records" ? rows : reference);
  const port = createRuntimeLiteraturePort(key, caps, () => undefined, invoke);
  await expect(port.referenceRecords.list(" paper\u0000 ")).resolves.toBe(rows);
  await expect(port.referenceRecords.save(referenceDraft)).resolves.toBe(reference);
  expect(invoke.mock.calls).toEqual([
    ["list_runtime_reference_records", {packageKey: key, paperId: " paper\u0000 "}],
    ["save_runtime_reference_record", {packageKey: key, draft: referenceDraft}],
  ]);
});

it("requires the explicit reference capability and current activation before either original operation", async () => {
  const invoke = vi.fn(), operations = [
    (port: RuntimeLiteraturePort) => port.referenceRecords.list("paper"),
    (port: RuntimeLiteraturePort) => port.referenceRecords.save(referenceDraft),
  ];
  for (const run of operations) {
    for (const version of [undefined, 2]) {
      const declared = caps.filter(value => value.id !== "literature.reference-records");
      if (version !== undefined) declared.push({id: "literature.reference-records", version});
      await expect(run(createRuntimeLiteraturePort(key, declared, () => undefined, invoke)))
        .rejects.toMatchObject({code: "literature_gateway_unavailable"});
    }
    await expect(run(createRuntimeLiteraturePort(key, caps, () => {throw Object.assign(new Error("closed"), {code: "plugin_activation_context_closed"});}, invoke)))
      .rejects.toMatchObject({code: "plugin_activation_context_closed"});
  }
  expect(invoke).not.toHaveBeenCalled();
});

it("retains the original reference failure evidence and identifiers without formatting unknown content or replaying", async () => {
  const details = {futureField: "private reference body"};
  const reason = {code: "storage_error", message: "参考条目保存未完成。", instanceId: "instance-1", requestId: "request-1",
    operationId: "operation-1", referenceRecords: details};
  const invoke = vi.fn(async () => {throw reason;}), port = createRuntimeLiteraturePort(key, caps, () => undefined, invoke);
  const error = await port.referenceRecords.save(referenceDraft).catch(value => value);
  expect(error).toMatchObject({code: "storage_error", packageKey: key, instanceId: "instance-1", requestId: "request-1", operationId: "operation-1", cause: reason});
  expect(error.referenceRecords).toBe(details);
  expect(error.message).not.toContain("private reference body");
  expect(invoke).toHaveBeenCalledExactlyOnceWith("save_runtime_reference_record", {packageKey: key, draft: referenceDraft});
});

it.each([
  {phase: "validation", outcome: "notStarted"}, {phase: "statement", outcome: "rolledBack"},
  {phase: "readback", outcome: "committed"}, {phase: "commit", outcome: "unknown"},
] as const)("preserves reference $phase/$outcome settlement and safe revision evidence without replay", async ({phase, outcome}) => {
  const rollback = {code: "storage_error", safeCause: "sqlite_operation_failed", sqliteCode: "AuthorizationForStatementDenied", sqliteExtendedCode: 23};
  const hasIdentity = outcome !== "notStarted";
  const details = {phase, outcome, ...(hasIdentity ? {recordId: "reference-1", currentRevision: 7} : {}),
    problem: {kind: "database", diagnostic: "private reference SQL/body"}, ...(outcome === "unknown" ? {rollbackError: rollback} : {})};
  const reason = {code: "storage_error", message: "参考条目操作未完成。", instanceId: "instance-1", requestId: "request-1",
    ...(hasIdentity ? {currentRevision: 7} : {}), referenceRecords: details};
  const invoke = vi.fn(async () => {throw reason;}), port = createRuntimeLiteraturePort(key, caps, () => undefined, invoke);
  const error = await port.referenceRecords.save(referenceDraft).catch(value => value);
  expect(error).toMatchObject({code: "storage_error", packageKey: key, instanceId: "instance-1", requestId: "request-1", cause: reason});
  expect(error.referenceRecords).toBe(details);
  expect(error.message).toContain(`referenceRecords.phase=${phase}`);
  expect(error.message).toContain(`referenceRecords.outcome=${outcome}`);
  if (hasIdentity) {
    expect(error.currentRevision).toBe(7);
    expect(error.message.split(" · ")).toContain("referenceRecords.recordId=reference-1");
    expect(error.message.split(" · ")).toContain("referenceRecords.currentRevision=7");
    expect(error.message.split(" · ")).toContain("currentRevision=7");
  } else {
    expect(error.currentRevision).toBeUndefined();
    expect(error.message).not.toContain("referenceRecords.recordId="); expect(error.message).not.toContain("currentRevision=");
  }
  if (outcome === "unknown") expect(error.message).toContain("referenceRecords.rollbackError.sqliteExtendedCode=23");
  else expect(error.message).not.toContain("referenceRecords.rollbackError");
  expect(error.message).not.toContain("private reference SQL/body");
  expect(invoke).toHaveBeenCalledExactlyOnceWith("save_runtime_reference_record", {packageKey: key, draft: referenceDraft});
});

it("retains Core's full revision message and raw numeric evidence without clamping a large i64 or displaying unknown diagnostics", async () => {
  const currentRevision = Number("9223372036854775807");
  const details = {phase: "private unknown phase", outcome: "private unknown outcome", recordId: "/private/record",
    currentRevision, problem: {kind: "database", diagnostic: "private reference body"}};
  const reason = {code: "revision_conflict", message: "参考条目版本冲突；当前版本 v9223372036854775807", currentRevision, referenceRecords: details};
  const invoke = vi.fn(async () => {throw reason;}), port = createRuntimeLiteraturePort(key, caps, () => undefined, invoke);
  const error = await port.referenceRecords.save(referenceDraft).catch(value => value);
  expect(error.currentRevision).toBe(currentRevision); expect(error.referenceRecords).toBe(details);
  expect(error.message).toContain(reason.message);
  expect(error.message).not.toContain("currentRevision=");
  expect(error.message).not.toContain("referenceRecords.phase=");
  expect(error.message).not.toContain("referenceRecords.outcome=");
  expect(error.message).not.toContain("referenceRecords.recordId=");
  expect(error.message).not.toContain("private");
  expect(invoke).toHaveBeenCalledOnce();
});

it("routes the actual startup and DOI consumers through fixed commands and one supplied package identity", async () => {
  const invoke = vi.fn(async (command: string, args: Record<string, unknown>): Promise<unknown> => {
    expect(args.packageKey).toBe(key);
    if (command === "list_runtime_literature") return [item];
    if (command === "get_runtime_literature_item") return item;
    if (command === "list_runtime_duplicate_candidates" || command === "list_runtime_merge_operations") return [];
    if (command === "preview_runtime_doi_metadata") return preview;
    if (command === "apply_runtime_literature_metadata") return {kind: (args.request as {kind: string}).kind === "reject" ? "done" : "item", item};
    if (command === "apply_runtime_literature_import_jobs") return (args.request as {kind: string}).kind === "get" ? {kind: "job", job} : {kind: "done"};
    if (command === "read_runtime_nal_network") return emptyNalState();
    if (command === "read_runtime_nal_query_collections" || command === "save_runtime_nal_query_collections") return stored;
    throw new Error("Unexpected command");
  });
  const port = createRuntimeLiteraturePort(key, caps, () => undefined, invoke);
  await expect(port.literature.list("needle")).resolves.toEqual([item]);
  await expect(port.literature.get(item.id)).resolves.toEqual(item);
  await port.literature.listDuplicates(); await port.literature.listMergeOperations();
  await expect(port.literature.previewDoi(" DOI: 10.1000/Test ", item.id)).resolves.toEqual(preview);
  await port.literature.applyMetadataCandidate("candidate", item);
  const decisions = [{fieldName: "title" as const, choice: "current" as const}];
  await port.literature.applyMetadataCandidateFields("candidate", decisions, "trusted");
  await port.literature.rejectMetadataCandidate("candidate");
  await expect(port.literature.getImportJob("job")).resolves.toEqual(job);
  await port.literature.cancelImportJob("job");
  await expect(port.nal.read()).resolves.toEqual(emptyNalState());
  await expect(port.queries.read()).resolves.toEqual(stored);
  await expect(port.queries.save(content, 1)).resolves.toEqual(stored);
  expect(invoke.mock.calls).toEqual([
    ["list_runtime_literature", {packageKey: key, query: "needle"}],
    ["get_runtime_literature_item", {packageKey: key, id: "paper"}],
    ["list_runtime_duplicate_candidates", {packageKey: key}], ["list_runtime_merge_operations", {packageKey: key}],
    ["preview_runtime_doi_metadata", {packageKey: key, doi: " DOI: 10.1000/Test ", itemId: "paper"}],
    ["apply_runtime_literature_metadata", {packageKey: key, request: {kind: "fullApply", candidateId: "candidate", draft: item}}],
    ["apply_runtime_literature_metadata", {packageKey: key, request: {kind: "fieldsApply", candidateId: "candidate", decisions, candidateTrust: "trusted"}}],
    ["apply_runtime_literature_metadata", {packageKey: key, request: {kind: "reject", candidateId: "candidate"}}],
    ["apply_runtime_literature_import_jobs", {packageKey: key, request: {kind: "get", id: "job"}}],
    ["apply_runtime_literature_import_jobs", {packageKey: key, request: {kind: "cancel", id: "job"}}],
    ["read_runtime_nal_network", {packageKey: key}], ["read_runtime_nal_query_collections", {packageKey: key}],
    ["save_runtime_nal_query_collections", {packageKey: key, value: content, expectedRevision: 1}],
  ]);
});

it("keeps the exact failure and available native identities without retry or a static command", async () => {
  const reason = {code: "connector_error", message: "DOI resolver returned HTTP 503", instanceId: "instance-1", requestId: "request-2",
    httpStatus: 503, literatureMetadata: {phase: "postRead", outcome: {status: "committed"}}};
  const invoke = vi.fn(async () => {throw reason;});
  const port = createRuntimeLiteraturePort(key, caps, () => undefined, invoke);
  await expect(port.literature.previewDoi("10.1000/test")).rejects.toMatchObject({code: "connector_error", packageKey: key,
    instanceId: "instance-1", requestId: "request-2", status: 503, cause: reason, literatureMetadata: reason.literatureMetadata});
  expect(invoke).toHaveBeenCalledTimes(1);
});

it("routes manual input, original field patches, confirmations and BibTeX drafts without normalizing their input", async () => {
  const draft = {...item, id: undefined, title: "  新标题\u0000  ", sourceKind: "bibtex", rawBibtex: "@article{raw, title={原文}}",
    creators: [{familyName: "Xu", givenName: "Bo", literalName: "", role: " editor "}], tags: ["same", "same"]};
  const {id: _id, ...input} = draft;
  expect(_id).toBeUndefined();
  const patch = {fieldName: "issuedYear" as const, value: null};
  const fieldNames = ["title", "title", "creators"];
  const drafts = [input, {...input, title: "Second", sourceKind: "user"}];
  const imported = {...job, id: "new-job", sourceKind: "bibtex", state: "completed_with_errors"};
  const invoke = vi.fn(async (command: string): Promise<unknown> => command === "save_runtime_literature_item" ? item
    : command === "apply_runtime_literature_metadata" ? {kind: "item", item} : {kind: "job", job: imported});
  const port = createRuntimeLiteraturePort(key, caps, () => undefined, invoke);
  await expect(port.literature.save(input)).resolves.toBe(item);
  await expect(port.literature.updateMetadataField(item.id, patch)).resolves.toBe(item);
  await expect(port.literature.confirmMetadataFields(item.id, fieldNames)).resolves.toBe(item);
  await expect(port.literature.importBibtex(drafts)).resolves.toBe(imported);
  expect(invoke.mock.calls).toEqual([
    ["save_runtime_literature_item", {packageKey: key, draft: input}],
    ["apply_runtime_literature_metadata", {packageKey: key, request: {kind: "update", itemId: item.id, patch}}],
    ["apply_runtime_literature_metadata", {packageKey: key, request: {kind: "confirm", itemId: item.id, fieldNames}}],
    ["apply_runtime_literature_import_jobs", {packageKey: key, request: {kind: "bibtex", drafts}}],
  ]);
  // Empty input and repeated field names retain the backend's original validation/order.
  await port.literature.confirmMetadataFields(item.id, []); await port.literature.importBibtex([]);
  expect(invoke).toHaveBeenCalledTimes(6);
});

it("retains committed and unknown manual-save diagnostics without replay or invented item identity", async () => {
  for (const saveOutcome of [{status: "committed", itemId: "saved-paper"}, {status: "unknown"}]) {
    const reason = {code: "plugin_host_exited", message: "题录保存后的回复未完成。", instanceId: "instance-1", requestId: "request-1", saveOutcome};
    const invoke = vi.fn(async () => {throw reason;}), port = createRuntimeLiteraturePort(key, caps, () => undefined, invoke);
    const error = await port.literature.save(item).catch(value => value);
    expect(error).toMatchObject({code: reason.code, packageKey: key, instanceId: "instance-1", requestId: "request-1", cause: reason});
    expect(error.saveOutcome).toBe(saveOutcome);
    expect(error.message).toContain(`saveOutcome=${saveOutcome.status}`);
    if (saveOutcome.status === "committed") expect(error.message).toContain("itemId=saved-paper");
    else expect(error.message).not.toContain("itemId=");
    expect(invoke).toHaveBeenCalledTimes(1);
  }
});

it("does not grant the four input writes without their capability or after the original context closes", async () => {
  const operations = [
    {capability: "literature.catalog-save", run: (port: ReturnType<typeof createRuntimeLiteraturePort>) => port.literature.save(item)},
    {capability: "literature.metadata", run: (port: ReturnType<typeof createRuntimeLiteraturePort>) => port.literature.updateMetadataField(item.id, {fieldName: "title", value: "Changed"})},
    {capability: "literature.metadata", run: (port: ReturnType<typeof createRuntimeLiteraturePort>) => port.literature.confirmMetadataFields(item.id, [])},
    {capability: "literature.import-jobs", run: (port: ReturnType<typeof createRuntimeLiteraturePort>) => port.literature.importBibtex([item])},
  ];
  const invoke = vi.fn();
  for (const operation of operations) {
    await expect(operation.run(createRuntimeLiteraturePort(key, caps.filter(value => value.id !== operation.capability), () => undefined, invoke)))
      .rejects.toMatchObject({code: "literature_gateway_unavailable"});
    await expect(operation.run(createRuntimeLiteraturePort(key, caps, () => {throw Object.assign(new Error("closed"), {code: "plugin_activation_context_closed"});}, invoke)))
      .rejects.toMatchObject({code: "plugin_activation_context_closed"});
  }
  expect(invoke).not.toHaveBeenCalled();
});

it("routes all original duplicate and recycle actions with unchanged IDs and batch order, preserving void IPC results", async () => {
  const operation = {id: "merge-1", survivorItemId: "survivor", survivorTitle: "Kept", mergedItemId: "merged",
    mergedTitle: "Original", state: "applied", createdAt: "created"};
  const invoke = vi.fn(async (command: string): Promise<unknown> => command === "merge_runtime_duplicate_candidate" ? operation
    : command === "list_runtime_trashed_literature" ? [item] : null);
  const port = createRuntimeLiteraturePort(key, caps, () => undefined, invoke), ids = ["z", "a", "z"];
  await expect(port.literature.decideDuplicate(" candidate ", "keep_both")).resolves.toBeUndefined();
  await expect(port.literature.decideDuplicate("candidate", "dismiss")).resolves.toBeUndefined();
  await expect(port.literature.mergeDuplicate("candidate", "survivor")).resolves.toBe(operation);
  await expect(port.literature.undoMerge("merge-1")).resolves.toBeUndefined();
  await expect(port.literature.archive("paper")).resolves.toBeUndefined();
  await expect(port.literature.restore("paper")).resolves.toBeUndefined();
  await expect(port.literature.trash("paper")).resolves.toBeUndefined();
  await expect(port.literature.trashMany(ids)).resolves.toBeUndefined();
  await expect(port.literature.restoreTrash("paper")).resolves.toBeUndefined();
  await expect(port.literature.listTrash(" needle\u0000 ")).resolves.toEqual([item]);
  expect(invoke.mock.calls).toEqual([
    ["decide_runtime_duplicate_candidate", {packageKey: key, candidateId: " candidate ", decision: "keep_both"}],
    ["decide_runtime_duplicate_candidate", {packageKey: key, candidateId: "candidate", decision: "dismiss"}],
    ["merge_runtime_duplicate_candidate", {packageKey: key, candidateId: "candidate", survivorItemId: "survivor"}],
    ["undo_runtime_merge_operation", {packageKey: key, operationId: "merge-1"}],
    ...["archive", "restore", "trash"].map(kind => ["apply_runtime_literature_lifecycle", {packageKey: key, request: {kind, itemId: "paper"}}]),
    ["apply_runtime_literature_lifecycle", {packageKey: key, request: {kind: "trashMany", itemIds: ids}}],
    ["apply_runtime_literature_lifecycle", {packageKey: key, request: {kind: "restoreTrash", itemId: "paper"}}],
    ["list_runtime_trashed_literature", {packageKey: key, query: " needle\u0000 "}],
  ]);
});

it("requires each exact management capability and original context, including both capabilities before reading trash", async () => {
  const operations = [
    {capability: "literature.duplicate-review", run: (port: RuntimeLiteraturePort) => port.literature.decideDuplicate("candidate", "dismiss")},
    {capability: "literature.duplicate-merge", run: (port: RuntimeLiteraturePort) => port.literature.mergeDuplicate("candidate", "survivor")},
    {capability: "literature.duplicate-undo", run: (port: RuntimeLiteraturePort) => port.literature.undoMerge("operation")},
    ...(["archive", "restore", "trash", "restoreTrash"] as const).map(method => ({capability: "literature.lifecycle", run: (port: RuntimeLiteraturePort) => port.literature[method]("paper")})),
    {capability: "literature.lifecycle", run: (port: RuntimeLiteraturePort) => port.literature.trashMany(["z", "a", "z"])},
    {capability: "literature.catalog-read", run: (port: RuntimeLiteraturePort) => port.literature.listTrash()},
    {capability: "literature.trash-cleanup", run: (port: RuntimeLiteraturePort) => port.literature.listTrash()},
  ];
  const invoke = vi.fn();
  for (const operation of operations) {
    for (const version of [undefined, 2]) {
      const declarations = caps.filter(value => value.id !== operation.capability);
      if (version !== undefined) declarations.push({id: operation.capability, version});
      await expect(operation.run(createRuntimeLiteraturePort(key, declarations, () => undefined, invoke)))
        .rejects.toMatchObject({code: "literature_gateway_unavailable"});
    }
    await expect(operation.run(createRuntimeLiteraturePort(key, caps, () => {throw Object.assign(new Error("closed"), {code: "plugin_activation_context_closed"});}, invoke)))
      .rejects.toMatchObject({code: "plugin_activation_context_closed"});
  }
  expect(invoke).not.toHaveBeenCalled();
});

it("keeps the actual management failure shapes, partial cleanup prefix and safe rollback diagnostics without replay", async () => {
  const database = {code: "storage_error", phase: "rollback", safeCause: "sqlite_operation_failed", sqliteCode: "AuthorizationForStatementDenied", sqliteExtendedCode: 23};
  const failures = {
    duplicateSql: {state: "returned", affectedRows: 2, observedAutocommit: true},
    duplicateUndo: {problem: {phase: "commit", cause: {kind: "snapshotCodec", diagnostic: "private snapshot body"}}, outcome: "unknown", rollbackError: database},
    duplicateMerge: {problem: {phase: "nal", cause: {kind: "owner", diagnostic: "private source body"}}, outcome: "rolledBack", operationId: "merge-1", rollbackError: database},
    literatureLifecycle: {phase: "statement", problem: {kind: "database", diagnostic: "private statement"}, outcome: {state: "statementFailed"}},
    trashCleanup: {phase: "commit", problem: {kind: "owner", diagnostic: "private item body"}, outcome: {state: "unknown"}, itemId: "next",
      committedItemIds: ["done-1", "done-2"], rollbackError: database},
  };
  for (const [field, value] of Object.entries(failures)) {
    const reason = {code: "storage_error", message: "原操作未完成。", instanceId: "instance-1", requestId: "request-1", operationId: "operation-1",
      [field]: value, ...(field === "duplicateSql" ? {duplicateDatabaseError: database} : {})};
    const invoke = vi.fn(async () => {throw reason;}), port = createRuntimeLiteraturePort(key, caps, () => undefined, invoke);
    const error = await port.literature.decideDuplicate("candidate", "dismiss").catch(value => value);
    expect(error[field]).toBe(value);
    expect(error).toMatchObject({code: "storage_error", packageKey: key, instanceId: "instance-1", requestId: "request-1", operationId: "operation-1", cause: reason});
    expect(error.message).not.toContain("private");
    if (field === "duplicateSql") {
      expect(error.duplicateDatabaseError).toBe(database);
      expect(error.message).toContain("duplicateSql.state=returned");
      expect(error.message).toContain("duplicateSql.affectedRows=2");
      expect(error.message).toContain("duplicateSql.observedAutocommit=true");
      expect(error.message).toContain("sqliteExtendedCode=23");
    } else if (field === "duplicateUndo") expect(error.message).toContain("duplicateUndo.outcome=unknown");
    else if (field === "duplicateMerge") expect(error.message).toContain("duplicateMerge.outcome=rolledBack");
    else if (field === "literatureLifecycle") {
      expect(error.message).toContain("literatureLifecycle.outcome=statementFailed");
      expect(error.message).not.toContain("rolledBack");
    } else {
      expect(error.message).toContain("trashCleanup.outcome=unknown");
      expect(error.message).toContain("trashCleanup.committedItemIds=done-1,done-2");
      expect(error.message).toContain("trashCleanup.itemId=next");
    }
    if ("rollbackError" in value) expect(error.message).toContain(`${field}.rollbackError.sqliteExtendedCode=23`);
    expect(invoke).toHaveBeenCalledTimes(1);
  }
});

it("preserves the real Rust importJobs partial prefix and displays only safe known progress and cleanup details", async () => {
  const importJobs = {phase: "job-queued", outcome: "statementFailed", jobId: "job-1", resultId: "result-2",
    headerCommitted: true, completedResultUpdates: 0, lastSave: null, candidateId: "candidate-3", candidateInsertReturned: true};
  const cleanupError = "process I/O failed (operation=group_terminate, kind=PermissionDenied, raw_os_error=Some(1))";
  const reason = {code: "storage_error", message: "文献数据库操作失败：sqlite_failure", pluginId: "literature_library",
    instanceId: "instance-4", requestId: "request-5", importJobs, cleanupError};
  const port = createRuntimeLiteraturePort(key, caps, () => undefined, async () => {throw reason;});
  const error = await port.literature.previewDoi("doi").catch(value => value);
  expect(error).toMatchObject({code: "storage_error", importJobs, cleanupError, cause: reason});
  expect(error.importJobs).toBe(importJobs);
  for (const token of ["jobId=job-1", "resultId=result-2", "candidateId=candidate-3", "phase=job-queued",
    "headerCommitted=true", "candidateInsertReturned=true", "outcome=statementFailed", cleanupError]) expect(error.message).toContain(token);
  const unknown = {...importJobs, headerCommitted: null, candidateInsertReturned: null, completedResultUpdates: null, outcome: "unknown"};
  const failed = createRuntimeLiteraturePort(key, caps, () => undefined, async () => {
    throw {...reason, importJobs: unknown, cleanupError: "/private/synthetic-note-body PermissionDenied"};
  });
  const unknownError = await failed.literature.previewDoi("doi").catch(value => value);
  expect(unknownError.importJobs).toBe(unknown);
  expect(unknownError.message).toContain("headerCommitted=unknown");
  expect(unknownError.message).toContain("candidateInsertReturned=unknown");
  expect(unknownError.message).toContain("cleanup=permission_denied");
  expect(unknownError.message).not.toContain("synthetic-note-body");
});

it("allows only the original admitted preview candidate to settle once after its context closes", async () => {
  let resolve: (value: MetadataPreview) => void = () => {throw new Error("Fixture not initialized");};
  const pending = new Promise<MetadataPreview>(accept => {resolve = accept;});
  let current = true;
  const invoke = vi.fn(async (command: string): Promise<unknown> => command === "preview_runtime_doi_metadata" ? pending : {kind: "done"});
  const port = createRuntimeLiteraturePort(key, caps, () => {
    if (!current) throw Object.assign(new Error("closed"), {code: "plugin_activation_context_closed"});
  }, invoke);
  const accepted = port.literature.previewDoi("doi");
  current = false;
  await expect(port.literature.previewDoi("new")).rejects.toMatchObject({code: "plugin_activation_context_closed"});
  await expect(port.literature.rejectMetadataCandidate("unobserved")).rejects.toMatchObject({code: "plugin_activation_context_closed"});
  resolve(preview); await expect(accepted).resolves.toBe(preview);
  await expect(port.literature.rejectMetadataCandidate(preview.id)).resolves.toBeUndefined();
  await expect(port.literature.rejectMetadataCandidate(preview.id)).rejects.toMatchObject({code: "plugin_activation_context_closed"});
  expect(invoke.mock.calls).toEqual([["preview_runtime_doi_metadata", {packageKey: key, doi: "doi", itemId: undefined}],
    ["apply_runtime_literature_metadata", {packageKey: key, request: {kind: "reject", candidateId: preview.id}}]]);
});

it("does not route an old desktop cleanup lease into a replacement App runtime", async () => {
  const original = {mode: "desktop" as const, storage: window.localStorage, gateways: {}, dispose() {}};
  selectAppRuntime(original);
  let resolve: (value: MetadataPreview) => void = () => {throw new Error("Fixture not initialized");};
  const pending = new Promise<MetadataPreview>(accept => {resolve = accept;});
  platform.invoke.mockReturnValueOnce(pending);
  let current = true;
  const port = createRuntimeLiteraturePort(key, caps, () => {
    if (!current) throw Object.assign(new Error("closed"), {code: "plugin_activation_context_closed"});
  });
  const accepted = port.literature.previewDoi("doi");
  current = false;
  const replacement = vi.fn(async () => ({kind: "done"}));
  selectAppRuntime({mode: "preview", storage: window.localStorage, gateways: {}, invoke: replacement, dispose() {}});
  resolve(preview); await accepted;
  await expect(port.literature.rejectMetadataCandidate(preview.id)).rejects.toMatchObject({code: "literature_gateway_unavailable", candidateId: preview.id});
  expect(platform.invoke).toHaveBeenCalledTimes(1);
  expect(replacement).not.toHaveBeenCalled();
});

it("rejects wrong result phases and task identities after one call", async () => {
  for (const response of [{kind: "done"}, {kind: "job", job: {...job, id: "different"}}]) {
    const invoke = vi.fn(async () => response), port = createRuntimeLiteraturePort(key, caps, () => undefined, invoke);
    await expect(port.literature.getImportJob("job")).rejects.toMatchObject({code: "plugin_host_response_mismatch"});
    expect(invoke).toHaveBeenCalledTimes(1);
  }
  const invoke = vi.fn(async () => ({kind: "job", job})), port = createRuntimeLiteraturePort(key, caps, () => undefined, invoke);
  await expect(port.literature.rejectMetadataCandidate("candidate")).rejects.toMatchObject({code: "plugin_host_response_mismatch"});
  await expect(port.literature.updateMetadataField(item.id, {fieldName: "title", value: "Changed"})).rejects.toMatchObject({code: "plugin_host_response_mismatch"});
  await expect(port.literature.confirmMetadataFields(item.id, [])).rejects.toMatchObject({code: "plugin_host_response_mismatch"});
  for (const response of [{kind: "done"}, {kind: "job"}, {kind: "job", job: {id: 1}}]) {
    const dispatch = vi.fn(async () => response), owner = createRuntimeLiteraturePort(key, caps, () => undefined, dispatch);
    await expect(owner.literature.importBibtex([item])).rejects.toMatchObject({code: "plugin_host_response_mismatch"});
    expect(dispatch).toHaveBeenCalledTimes(1);
  }
});

it("checks current ownership and exact capability before dispatch and fails closed without a native adapter", async () => {
  const invoke = vi.fn(async () => preview);
  for (const capabilities of [[], [{id: "literature.doi-preview", version: 2}]] satisfies BuiltinCapabilityContract[][]) {
    await expect(createRuntimeLiteraturePort(key, capabilities, () => undefined, invoke).literature.previewDoi("doi"))
      .rejects.toMatchObject({code: "literature_gateway_unavailable"});
  }
  await expect(createRuntimeLiteraturePort(key, caps, () => {throw Object.assign(new Error("closed"), {code: "plugin_activation_context_closed"});}, invoke)
    .literature.previewDoi("doi")).rejects.toMatchObject({code: "plugin_activation_context_closed"});
  expect(invoke).not.toHaveBeenCalled();
  selectAppRuntime(browser());
  await expect(createRuntimeLiteraturePort(key, caps, () => undefined).literature.previewDoi("doi"))
    .rejects.toMatchObject({code: "literature_gateway_unavailable"});
});
