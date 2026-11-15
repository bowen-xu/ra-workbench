import type {Claim, SourceAnchorSummary, EvidenceLink} from "@research-assistant/core/evidence";
import type {AttachmentRemovalResult, AttachmentSummary, ImportJob, LiteratureItem, MergeOperation, MetadataPreview} from "@research-assistant/core/literature";
import type {DerivedResearchView, EvidenceMatrix, EvidenceMatrixCell, ReferenceRecord} from "@research-assistant/core/synthesis";
import type {ConceptAlgorithmRun, Concept, ConceptIdentityEvent, ConceptMergeDraft, ConceptMergeImpact, ConceptSplitDraft, ConceptCaptureResult, DefinitionClaim, RelationAssertion, RelationNodeSummary, ResearchObjectRevision, StructuredReadingUnit} from "@research-assistant/core/knowledge";
import {decodeNalQueryDefinitions, decodeNalState} from "@research-assistant/core/nal";
import type {RuntimeLiteraturePort} from "@research-assistant/workbench-sdk/literature-runtime";
import type {BuiltinCapabilityContract} from "@research-assistant/workbench-sdk";
import type {NalQueryStore} from "@research-assistant/data/nal";
import {getAppRuntime} from "../../../core/runtime/appRuntime";
import {invoke} from "../../../core/runtime/platformCore";
import {diagnosticError, nativeTerminationMessage} from "../../../core/public/diagnostics";
import {attachmentBytes, readingState, readingStateArguments, type AttachmentByteResponse, type ReadingPositionResponse} from "./literatureAttachmentTransport";

type Invoke = (command: string, args: Record<string, unknown>) => Promise<unknown>;
function record(value: unknown): Record<string, unknown> | undefined {
  return value !== null && typeof value === "object" && !Array.isArray(value) ? value as Record<string, unknown> : undefined;
}
function identity(value: unknown): string | undefined {
  return typeof value === "string" && /^[\w:.-]{1,200}$/u.test(value) ? value : undefined;
}
function importProgressFields(value: unknown): string[] {
  const progress = record(value);
  if (!progress) return [];
  const fields = ["jobId", "resultId", "candidateId", "phase"].flatMap(key => {
    const id = identity(progress[key]); return id === undefined ? [] : [`${key}=${id}`];
  });
  if (["notStarted", "rolledBack", "returned", "statementFailed", "unknown"].includes(String(progress.outcome)))
    fields.push(`outcome=${progress.outcome}`);
  for (const key of ["headerCommitted", "candidateInsertReturned"] as const) {
    if (typeof progress[key] === "boolean" || progress[key] === null) fields.push(`${key}=${progress[key] === null ? "unknown" : progress[key]}`);
  }
  if (Number.isSafeInteger(progress.completedResultUpdates) && Number(progress.completedResultUpdates) >= 0)
    fields.push(`completedResultUpdates=${progress.completedResultUpdates}`);
  else if (progress.completedResultUpdates === null) fields.push("completedResultUpdates=unknown");
  return fields;
}
function saveOutcomeFields(value: unknown): string[] {
  const outcome = record(value);
  if (!outcome || !["notStarted", "rolledBack", "committed", "unknown"].includes(String(outcome.status))) return [];
  const itemId = identity(outcome.itemId);
  return [`saveOutcome=${outcome.status}`, ...(itemId === undefined ? [] : [`itemId=${itemId}`])];
}
function databaseFields(prefix: string, value: unknown): string[] {
  const error = record(value);
  if (!error) return [];
  const fields = ["code", "safeCause", "sqliteCode"].flatMap(key => {
    const token = identity(error[key]); return token === undefined ? [] : [`${prefix}.${key}=${token}`];
  });
  if (Number.isSafeInteger(error.sqliteExtendedCode)) fields.push(`${prefix}.sqliteExtendedCode=${error.sqliteExtendedCode}`);
  return fields;
}
function storageMutationFields(prefix: "evidenceMatrix" | "derivedView" | "evidenceWrite" | "relations" | "conceptIdentity" | "conceptAlgorithm", value: unknown): string[] {
  const failure = record(value); if (!failure) return [];
  const fields: string[] = [];
  const phases = ["validation", "execution", "write", "commit", "rollback", "postcommit-read", "finish"];
  const outcomes = ["notStarted", "rolledBack", "committed", "unknown"];
  if (prefix === "evidenceWrite") {phases.push("link-insert"); outcomes.push("statementFailed");}
  if (prefix === "conceptAlgorithm") {phases.push("start", "worker"); outcomes.push("statementFailed");}
  if (phases.includes(String(failure.phase))) fields.push(`${prefix}.phase=${failure.phase}`);
  if (outcomes.includes(String(failure.outcome))) fields.push(`${prefix}.outcome=${failure.outcome}`);
  const id = identity(failure.objectId); if (id !== undefined) fields.push(`${prefix}.objectId=${id}`);
  if (Number.isSafeInteger(failure.revision)) fields.push(`${prefix}.revision=${failure.revision}`);
  return [...fields, ...databaseFields(`${prefix}.databaseError`, failure.databaseError), ...databaseFields(`${prefix}.rollbackError`, failure.rollbackError)];
}
function managementFields(value: Record<string, unknown> | undefined): string[] {
  const fields: string[] = [], sql = record(value?.duplicateSql);
  if (sql && ["notDispatched", "returned", "failed", "unknown"].includes(String(sql.state))) {
    fields.push(`duplicateSql.state=${sql.state}`);
    if (Number.isSafeInteger(sql.affectedRows) && Number(sql.affectedRows) >= 0) fields.push(`duplicateSql.affectedRows=${sql.affectedRows}`);
    if (typeof sql.observedAutocommit === "boolean") fields.push(`duplicateSql.observedAutocommit=${sql.observedAutocommit}`);
  }
  for (const key of ["duplicateUndo", "duplicateMerge", "literatureLifecycle", "trashCleanup"] as const) {
    const failure = record(value?.[key]);
    if (!failure) continue;
    const transactional = key === "duplicateUndo" || key === "duplicateMerge";
    const outcome = transactional ? failure.outcome : record(failure.outcome)?.state;
    const states = key === "literatureLifecycle"
      ? ["notDispatched", "statementReturned", "statementFailed", "rolledBack", "unknown"] : ["notStarted", "rolledBack", "unknown"];
    if (states.includes(String(outcome))) fields.push(`${key}.outcome=${outcome}`);
    const phase = transactional ? record(failure.problem)?.phase : failure.phase;
    const phases = ["header", "snapshot", "preparation", "begin", "classificationLoad", "classificationCheck", "nalLoad", "nalCheck",
      "nalDelete", "nalRevision", "classificationDelete", "classificationRevision", "restore", "search", "commit", "unknown",
      "outer", "titles", "timestamp", "nal", "classification", "history", "owner", "admission", "dispatch", "statement",
      "statement-result", "transaction", "execution", "expired-ids", "impact", "content-objects", "cleanup-plan", "scope-root",
      "scope-soft", "collect-prepare", "collect", "post-scope", "owned-delete", "fts-delete", "item-delete",
      "gc-scheduled", "orphan-creators", "orphan-tags", "finish"];
    if (typeof phase === "string" && phases.includes(phase)) fields.push(`${key}.phase=${phase}`);
    for (const name of ["itemId", "operationId"] as const) {
      const id = identity(failure[name]); if (id !== undefined) fields.push(`${key}.${name}=${id}`);
    }
    const effect = record(failure.outcome)?.affectedRows;
    if (Number.isSafeInteger(effect) && Number(effect) >= 0) fields.push(`${key}.affectedRows=${effect}`);
    if (key === "trashCleanup" && Array.isArray(failure.committedItemIds)) {
      const ids = failure.committedItemIds.flatMap(value => {const id = identity(value); return id === undefined ? [] : [id];});
      fields.push(`trashCleanup.committedItemIds=${ids.join(",")}`);
    }
    fields.push(...databaseFields(`${key}.rollbackError`, failure.rollbackError));
  }
  return [...fields, ...databaseFields("duplicateDatabaseError", value?.duplicateDatabaseError)];
}
function referenceRecordFields(value: Record<string, unknown> | undefined): string[] {
  const fields = Number.isSafeInteger(value?.currentRevision) ? [`currentRevision=${value?.currentRevision}`] : [];
  const failure = record(value?.referenceRecords);
  if (!failure) return fields;
  const phases = ["owner", "resources", "admission", "preparation", "validation", "source-anchor", "usage-anchor", "matched-item",
    "begin", "header", "identity", "revision", "fields-encoding", "statement", "history-encoding", "history", "commit",
    "readback", "list-ids", "list-record", "fields-codec", "output", "execution"];
  if (typeof failure.phase === "string" && phases.includes(failure.phase)) fields.push(`referenceRecords.phase=${failure.phase}`);
  if (typeof failure.outcome === "string" && ["notStarted", "rolledBack", "committed", "unknown"].includes(failure.outcome))
    fields.push(`referenceRecords.outcome=${failure.outcome}`);
  const id = identity(failure.recordId);
  if (id !== undefined) fields.push(`referenceRecords.recordId=${id}`);
  if (Number.isSafeInteger(failure.currentRevision)) fields.push(`referenceRecords.currentRevision=${failure.currentRevision}`);
  return [...fields, ...databaseFields("referenceRecords.rollbackError", failure.rollbackError)];
}
function attachmentFields(value: Record<string, unknown> | undefined): string[] {
  const failure = record(value?.attachments);
  if (!failure) return [];
  const fields: string[] = [];
  const phases = ["owner", "resources", "preparation", "validation", "execution", "begin", "commit", "rename",
    "attach-active-parent", "attach-probe", "attach-duplicate", "content-publish", "attach-insert", "attach-other-items",
    "attach-duplicate-insert", "remove-owner", "remove-header", "remove-path", "remove-delete", "remove-references",
    "remove-fts", "reading-state-save", "index-job-running", "index-pages-delete", "index-page-insert", "index-fts", "index-job-completed"];
  if (typeof failure.phase === "string" && phases.includes(failure.phase)) fields.push(`attachments.phase=${failure.phase}`);
  if (typeof failure.sql === "string" && ["notStarted", "statementFailed", "returned", "rolledBack", "committed", "unknown"].includes(failure.sql))
    fields.push(`attachments.sql=${failure.sql}`);
  const id = identity(failure.attachmentId);
  if (id !== undefined) fields.push(`attachments.attachmentId=${id}`);
  for (const key of ["contentPublishReturned", "attachmentInsertReturned"] as const)
    if (typeof failure[key] === "boolean") fields.push(`attachments.${key}=${failure[key]}`);
  if (Number.isSafeInteger(failure.duplicateInsertReturns) && Number(failure.duplicateInsertReturns) >= 0)
    fields.push(`attachments.duplicateInsertReturns=${failure.duplicateInsertReturns}`);
  return [...fields, ...databaseFields("attachments.database", failure.database), ...databaseFields("attachments.rollback", failure.rollback)];
}
function nalFields(value: Record<string, unknown> | undefined): string[] {
  const fields: string[] = [];
  for (const name of ["mutation", "annotation"] as const) {
    const outcome = record(value?.[`${name}Outcome`]), problem = record(value?.[`${name}Problem`]);
    if (outcome && ["notStarted", "rolledBack", "committed", "unknown"].includes(String(outcome.status))) {
      fields.push(`${name}Outcome=${outcome.status}`);
      for (const key of ["revision", "observedRevision"] as const)
        if (Number.isSafeInteger(outcome[key])) fields.push(`${name}Outcome.${key}=${outcome[key]}`);
      const annotation = record(outcome.annotation);
      for (const key of ["id", "targetId", "notePageId"] as const) {
        const id = identity(annotation?.[key]); if (id !== undefined) fields.push(`${name}Outcome.annotation.${key}=${id}`);
      }
    }
    const phases = ["validation", "begin", "startup", "decision", "createNote", "linkIdentity", "linkLoad", "linkConflict",
      "linkNoteActive", "linkTarget", "linkInsert", "revision", "commit", "revisionCheck", "mutation", "termRead",
      "kindUsage", "relationKind", "statementInsert", "link", "finalLoad"];
    if (typeof problem?.phase === "string" && phases.includes(problem.phase)) fields.push(`${name}Problem.phase=${problem.phase}`);
    const cause = record(problem?.cause);
    if (["startup", "link", "data", "database", "rejected", "conflict", "binding", "owner", "noteCreation", "annotationSnapshotChanged"].includes(String(cause?.kind)))
      fields.push(`${name}Problem.cause=${cause?.kind}`);
    if (cause?.kind === "database" || cause?.kind === "data") fields.push(...databaseFields(`${name}Problem.${cause.kind}`, cause.diagnostic));
    fields.push(...databaseFields(`${name}RollbackError`, value?.[`${name}RollbackError`]));
  }
  return fields;
}
function conceptFields(value: Record<string, unknown> | undefined): string[] {
  const failure = record(value?.concepts);
  if (!failure) return [];
  const fields: string[] = [];
  const phases = ["owner", "preparation", "validation", "execution", "begin", "commit", "postcommit-read", "search",
    "name-active", "concept-name-available", "term-validation", "term-lookup", "term-read", "term-insert", "term-revision",
    "merged", "current-revision", "alias-update", "preferred-upsert", "update", "concept-insert", "concept-preferred-insert",
    "concept-main", "concept-terms", "preferred-term", "revision", "semantic", "codec"];
  if (typeof failure.phase === "string" && phases.includes(failure.phase)) fields.push(`concepts.phase=${failure.phase}`);
  if (["notStarted", "rolledBack", "committed", "unknown"].includes(String(failure.outcome))) fields.push(`concepts.outcome=${failure.outcome}`);
  const semanticPhases = ["semanticRender", "representationLookup", "representationInsert", "embeddingSettings", "spaceInsert",
    "activeGeneration", "nextGeneration", "generationInsert", "queueLookup", "recordInsert", "jobInsert"];
  if (typeof failure.semanticPhase === "string" && semanticPhases.includes(failure.semanticPhase)) fields.push(`concepts.semanticPhase=${failure.semanticPhase}`);
  const id = identity(failure.conceptId);
  if (id !== undefined) fields.push(`concepts.conceptId=${id}`);
  if (Number.isSafeInteger(failure.currentRevision)) fields.push(`concepts.currentRevision=${failure.currentRevision}`);
  const problem = record(failure.problem);
  if (["domain", "missing", "revisionConflict", "data", "database", "owner", "binding", "codec"].includes(String(problem?.kind)))
    fields.push(`concepts.problem=${problem?.kind}`);
  if (problem?.kind === "database" || problem?.kind === "data") fields.push(...databaseFields(`concepts.${problem.kind}`, problem.diagnostic));
  return [...fields, ...databaseFields("concepts.rollbackError", failure.rollbackError)];
}
function readingUnitFields(value: Record<string, unknown> | undefined): string[] {
  const failure = record(value?.readingUnits);
  if (!failure) return [];
  const fields: string[] = [];
  const phases = ["owner", "preparation", "validation", "begin", "list", "read", "template-read", "current-revision",
    "update", "insert", "revision", "evidence-upsert", "commit", "postcommit-read", "codec", "execution"];
  if (typeof failure.phase === "string" && phases.includes(failure.phase)) fields.push(`readingUnits.phase=${failure.phase}`);
  if (["notStarted", "rolledBack", "committed", "unknown"].includes(String(failure.outcome))) fields.push(`readingUnits.outcome=${failure.outcome}`);
  const id = identity(failure.readingUnitId);
  if (id !== undefined) fields.push(`readingUnits.readingUnitId=${id}`);
  if (Number.isSafeInteger(failure.currentRevision)) fields.push(`readingUnits.currentRevision=${failure.currentRevision}`);
  const problem = record(failure.problem);
  if (["domain", "missing", "revisionConflict", "data", "database", "owner", "binding", "codec"].includes(String(problem?.kind)))
    fields.push(`readingUnits.problem=${problem?.kind}`);
  if (problem?.kind === "database" || problem?.kind === "data") fields.push(...databaseFields(`readingUnits.${problem.kind}`, problem.diagnostic));
  return [...fields, ...databaseFields("readingUnits.rollbackError", failure.rollbackError)];
}
function cleanupMessage(value: unknown): string | undefined {
  if (typeof value !== "string") return undefined;
  // Reuse Core's safe text/category boundary. The typed process operation form
  // is also closed; no stderr, URLs, paths or unknown payload are formatted.
  const plain = /^[\p{L}\p{N}_:：().,，；（） -]{1,300}$/u.test(value.replaceAll("I/O", "IO").replaceAll("／", ""));
  const operation = /^process I\/O failed \(operation=(?:group_interrupt|group_terminate|child_start_kill|child_wait|group_cleanup_after_wait), kind=[A-Za-z]+, raw_os_error=(?:None|Some\(-?\d{1,10}\))\)$/u.test(value);
  return `cleanup=${plain || operation ? value : diagnosticError(new Error(value)).cause}`;
}
export class RuntimeLiteratureError extends Error {
  readonly code: string;
  readonly pluginId = "literature_library";
  readonly packageKey: string;
  readonly instanceId?: string;
  readonly requestId?: string;
  readonly operationId?: string;
  readonly candidateId?: string;
  readonly jobId?: string;
  readonly resultId?: string;
  readonly status?: number;
  readonly httpStatus?: number;
  readonly cleanupError?: string;
  readonly terminationError?: unknown;
  readonly literatureMetadata?: unknown;
  readonly importJobs?: unknown;
  readonly saveOutcome?: unknown;
  readonly duplicateSql?: unknown;
  readonly duplicateDatabaseError?: unknown;
  readonly duplicateUndo?: unknown;
  readonly duplicateMerge?: unknown;
  readonly literatureLifecycle?: unknown;
  readonly trashCleanup?: unknown;
  readonly referenceRecords?: unknown;
  readonly evidenceMatrix?: unknown;
  readonly derivedView?: unknown;
  readonly relations?: unknown;
  readonly conceptIdentity?: unknown;
  readonly conceptAlgorithm?: unknown;
  readonly evidenceWrite?: unknown;
  readonly attachments?: unknown;
  readonly mutationOutcome?: unknown;
  readonly mutationProblem?: unknown;
  readonly mutationRollbackError?: unknown;
  readonly annotationOutcome?: unknown;
  readonly annotationProblem?: unknown;
  readonly annotationRollbackError?: unknown;
  readonly concepts?: unknown;
  readonly readingUnits?: unknown;
  readonly currentRevision?: number;
  constructor(reason: unknown, packageKey: string, fallback = "plugin_host_failed") {
    const value = record(reason), safe = diagnosticError(reason);
    const code = identity(safe.code) ?? fallback;
    const message = typeof value?.message === "string" ? value.message
      : reason instanceof Error ? reason.message : "原生文献操作失败。";
    const ids = Object.fromEntries(["instanceId", "requestId", "operationId", "candidateId", "jobId", "resultId"].flatMap(key => {
      const id = identity(value?.[key]); return id === undefined ? [] : [[key, id]];
    }));
    const termination = nativeTerminationMessage(value?.terminationError);
    const cleanup = cleanupMessage(value?.cleanupError);
    const status = value?.httpStatus ?? value?.status;
    const httpStatus = typeof status === "number" && Number.isInteger(status) && status >= 100 && status <= 599 ? status : undefined;
    super([`${code}: ${message}`, `packageKey=${packageKey}`, ...Object.entries(ids).map(([key, id]) => `${key}=${id}`),
      ...importProgressFields(value?.importJobs),
      ...saveOutcomeFields(value?.saveOutcome),
      ...managementFields(value),
      ...referenceRecordFields(value),
      ...storageMutationFields("evidenceMatrix", value?.evidenceMatrix),
      ...storageMutationFields("derivedView", value?.derivedView),
      ...storageMutationFields("relations", value?.relations),
      ...storageMutationFields("conceptIdentity", value?.conceptIdentity),
      ...storageMutationFields("conceptAlgorithm", value?.conceptAlgorithm),
      ...storageMutationFields("evidenceWrite", value?.evidenceWrite),
      ...attachmentFields(value),
      ...nalFields(value),
      ...conceptFields(value),
      ...readingUnitFields(value),
      ...(httpStatus === undefined ? [] : [`HTTP ${httpStatus}`]),
      ...(cleanup === undefined ? [] : [cleanup]),
      ...(termination === undefined ? [] : [termination])].join(" · "), {cause: reason});
    this.name = "RuntimeLiteratureError"; this.code = code; this.packageKey = packageKey;
    Object.assign(this, ids);
    if (httpStatus !== undefined) {this.status = httpStatus; this.httpStatus = httpStatus;}
    if (typeof value?.cleanupError === "string") this.cleanupError = value.cleanupError;
    if (value?.terminationError !== undefined) this.terminationError = value.terminationError;
    if (value?.literatureMetadata !== undefined) this.literatureMetadata = value.literatureMetadata;
    if (value?.importJobs !== undefined) this.importJobs = value.importJobs;
    if (value?.saveOutcome !== undefined) this.saveOutcome = value.saveOutcome;
    if (value?.duplicateSql !== undefined) this.duplicateSql = value.duplicateSql;
    if (value?.duplicateDatabaseError !== undefined) this.duplicateDatabaseError = value.duplicateDatabaseError;
    if (value?.duplicateUndo !== undefined) this.duplicateUndo = value.duplicateUndo;
    if (value?.duplicateMerge !== undefined) this.duplicateMerge = value.duplicateMerge;
    if (value?.literatureLifecycle !== undefined) this.literatureLifecycle = value.literatureLifecycle;
    if (value?.trashCleanup !== undefined) this.trashCleanup = value.trashCleanup;
    if (value?.referenceRecords !== undefined) this.referenceRecords = value.referenceRecords;
    if (value?.evidenceMatrix !== undefined) this.evidenceMatrix = value.evidenceMatrix;
    if (value?.derivedView !== undefined) this.derivedView = value.derivedView;
    if (value?.relations !== undefined) this.relations = value.relations;
    if (value?.conceptIdentity !== undefined) this.conceptIdentity = value.conceptIdentity;
    if (value?.conceptAlgorithm !== undefined) this.conceptAlgorithm = value.conceptAlgorithm;
    if (value?.evidenceWrite !== undefined) this.evidenceWrite = value.evidenceWrite;
    if (value?.attachments !== undefined) this.attachments = value.attachments;
    for (const key of ["mutationOutcome", "mutationProblem", "mutationRollbackError", "annotationOutcome", "annotationProblem", "annotationRollbackError"] as const)
      if (value?.[key] !== undefined) this[key] = value[key];
    if (value?.concepts !== undefined) this.concepts = value.concepts;
    if (value?.readingUnits !== undefined) this.readingUnits = value.readingUnits;
    if (typeof value?.currentRevision === "number") this.currentRevision = value.currentRevision;
  }
}

/** App-only finite adapter; package identity comes from discovery, never a URL.
 * Default static assembly is untouched. Native waits retain the existing owner. */
export function createRuntimeLiteraturePort(packageKey: string, capabilities: readonly BuiltinCapabilityContract[],
  assertCurrent: () => void, invokeCommand?: Invoke): RuntimeLiteraturePort {
  const runtime = getAppRuntime();
  const transport = invokeCommand ?? (runtime.mode === "desktop" ? invoke
    : runtime.nativeAgentPreview ? runtime.invoke : undefined);
  const usesGlobalDesktopTransport = invokeCommand === undefined && runtime.mode === "desktop";
  // Only candidates actually returned by this instance's admitted preview can
  // finish rejection after the activation context closes. This is not a retry.
  const unsettledPreviews = new Set<string>();
  const mismatch = (): never => {throw new RuntimeLiteratureError({code: "plugin_host_response_mismatch", message: "原生文献回复种类不匹配。"}, packageKey);};
  function queryStore(value: unknown): NalQueryStore {
    const row = record(value);
    if (!row || !Number.isSafeInteger(row.revision) || Number(row.revision) < 0 || typeof row.legacyImported !== "boolean")
      throw new Error("查询集合存储格式无效");
    return {...row, revision: row.revision as number, legacyImported: row.legacyImported,
      definitions: decodeNalQueryDefinitions(row.definitions), cache: row.cache};
  }
  async function call<T>(capability: string | readonly string[], command: string, args: Record<string, unknown> = {}, settlingPreview?: string): Promise<T> {
    try {
      if (!settlingPreview) assertCurrent();
      if (usesGlobalDesktopTransport && getAppRuntime() !== runtime) throw {
        code: "literature_gateway_unavailable", message: "原文献运行时已替换；未向新数据源重放操作。",
        ...(settlingPreview === undefined ? {} : {candidateId: settlingPreview}),
      };
      if (!transport) throw {code: "literature_gateway_unavailable", message: "当前工作台未接入原生文献运行时。"};
      for (const id of typeof capability === "string" ? [capability] : capability) {
        if (!capabilities.some(value => value.id === id && value.version === 1)) {
          throw {code: "literature_gateway_unavailable", message: `当前文献包未提供 ${id} v1。`};
        }
      }
      return await transport(command, {packageKey, ...args}) as T;
    } catch (reason) {throw reason instanceof RuntimeLiteratureError ? reason : new RuntimeLiteratureError(reason, packageKey);}
  }
  async function metadata<T>(request: Record<string, unknown>, kind: "item" | "done", settlingPreview?: string): Promise<T> {
    const value = record(await call("literature.metadata", "apply_runtime_literature_metadata", {request}, settlingPreview));
    if (!value || value.kind !== kind || (kind === "item" && !record(value.item))) return mismatch();
    return (kind === "item" ? value.item : undefined) as T;
  }
  async function lifecycle(request: Record<string, unknown>): Promise<void> {
    await call("literature.lifecycle", "apply_runtime_literature_lifecycle", {request});
  }
  async function conceptIdentity<T>(request: {kind: "preview" | "merge"; draft: ConceptMergeDraft} | {kind: "split"; draft: ConceptSplitDraft} | {kind: "list"}, kind: "impact" | "event" | "events"): Promise<T> {
    const result = record(await call("literature.concepts", "apply_runtime_concept_identity", {request}));
    if (result?.kind !== kind || result[kind] === undefined) return mismatch();
    return result[kind] as T;
  }
  return Object.freeze({
    researchExport: Object.freeze({
      exportAll: (targetDirectory: string) => call<import("@research-assistant/data/literature").ResearchExportReport>("literature.research-export", "export_runtime_research_knowledge", {request: {targetDirectory}}),
      cancel: () => call<boolean>("literature.research-export", "cancel_runtime_research_export"),
    }),
    literature: Object.freeze({
      list: (query?: string) => call<readonly LiteratureItem[]>("literature.catalog-read", "list_runtime_literature", {query}),
      // The original read first attempts expiry cleanup; both methods must be declared.
      listTrash: (query?: string) => call<readonly LiteratureItem[]>(["literature.catalog-read", "literature.trash-cleanup"], "list_runtime_trashed_literature", {query}),
      get: (id: string) => call<LiteratureItem>("literature.catalog-read", "get_runtime_literature_item", {id}),
      save: draft => call<LiteratureItem>("literature.catalog-save", "save_runtime_literature_item", {draft}),
      updateMetadataField: (itemId, patch) => metadata<LiteratureItem>({kind: "update", itemId, patch}, "item"),
      confirmMetadataFields: (itemId, fieldNames) => metadata<LiteratureItem>({kind: "confirm", itemId, fieldNames}, "item"),
      async importBibtex(drafts): Promise<ImportJob> {
        const value = record(await call("literature.import-jobs", "apply_runtime_literature_import_jobs", {request: {kind: "bibtex", drafts}}));
        if (!value || value.kind !== "job" || typeof record(value.job)?.id !== "string") return mismatch();
        return value.job as unknown as ImportJob;
      },
      importPdfs: candidates => call<ImportJob>("literature.pdf-import", "start_runtime_pdf_import", {candidates}),
      attachPdf: (itemId, path) => call<AttachmentSummary>("literature.attachments", "attach_runtime_pdf_path", {itemId, path}),
      async renameAttachment(itemId, attachmentId, displayName): Promise<void> {
        await call("literature.attachments", "rename_runtime_literature_attachment", {itemId, attachmentId, displayName});
      },
      removeAttachment: (itemId, attachmentId) => call<AttachmentRemovalResult>("literature.attachments", "remove_runtime_literature_attachment", {itemId, attachmentId}),
      async readAttachment(attachmentId): Promise<Uint8Array> {
        return attachmentBytes(await call<AttachmentByteResponse>("literature.attachments", "read_runtime_attachment", {attachmentId}));
      },
      async revealAttachments(attachmentIds): Promise<void> {
        await call("literature.attachments", "reveal_runtime_attachments_in_file_manager", {attachmentIds});
      },
      async saveReadingState(itemId, attachmentId, viewState): Promise<void> {
        await call("literature.attachments", "save_runtime_reading_state", {itemId, attachmentId, ...readingStateArguments(viewState)});
      },
      async getReadingState(itemId, attachmentId) {
        return readingState(await call<ReadingPositionResponse | null>("literature.attachments", "get_runtime_reading_state", {itemId, attachmentId}));
      },
      isAttachmentIndexed: attachmentId => call<boolean>("literature.attachments", "runtime_attachment_is_indexed", {attachmentId}),
      async indexAttachment(itemId, attachmentId, pages): Promise<void> {
        await call("literature.attachments", "index_runtime_attachment_text", {itemId, attachmentId, pages});
      },
      listDuplicates: () => call<Awaited<ReturnType<RuntimeLiteraturePort["literature"]["listDuplicates"]>>>("literature.duplicate-review", "list_runtime_duplicate_candidates"),
      listMergeOperations: () => call<Awaited<ReturnType<RuntimeLiteraturePort["literature"]["listMergeOperations"]>>>("literature.duplicate-review", "list_runtime_merge_operations"),
      async decideDuplicate(candidateId, decision): Promise<void> {
        await call("literature.duplicate-review", "decide_runtime_duplicate_candidate", {candidateId, decision});
      },
      mergeDuplicate: (candidateId, survivorItemId) => call<MergeOperation>("literature.duplicate-merge", "merge_runtime_duplicate_candidate", {candidateId, survivorItemId}),
      async undoMerge(operationId): Promise<void> {
        await call("literature.duplicate-undo", "undo_runtime_merge_operation", {operationId});
      },
      archive: itemId => lifecycle({kind: "archive", itemId}), restore: itemId => lifecycle({kind: "restore", itemId}),
      trash: itemId => lifecycle({kind: "trash", itemId}), trashMany: itemIds => lifecycle({kind: "trashMany", itemIds}),
      restoreTrash: itemId => lifecycle({kind: "restoreTrash", itemId}),
      async previewDoi(doi: string, itemId?: string): Promise<MetadataPreview> {
        const preview = await call<MetadataPreview>("literature.doi-preview", "preview_runtime_doi_metadata", {doi, itemId});
        unsettledPreviews.add(preview.id);
        return preview;
      },
      async applyMetadataCandidate(candidateId, draft) {
        const item = await metadata<LiteratureItem>({kind: "fullApply", candidateId, draft}, "item");
        unsettledPreviews.delete(candidateId); return item;
      },
      async applyMetadataCandidateFields(candidateId, decisions, candidateTrust) {
        const item = await metadata<LiteratureItem>({kind: "fieldsApply", candidateId, decisions, candidateTrust}, "item");
        unsettledPreviews.delete(candidateId); return item;
      },
      rejectMetadataCandidate: (candidateId: string) => metadata<void>({kind: "reject", candidateId}, "done",
        unsettledPreviews.delete(candidateId) ? candidateId : undefined),
      async getImportJob(id: string): Promise<ImportJob> {
        const value = record(await call("literature.import-jobs", "apply_runtime_literature_import_jobs", {request: {kind: "get", id}}));
        if (!value || value.kind !== "job" || record(value.job)?.id !== id) return mismatch();
        return value.job as unknown as ImportJob;
      },
      async cancelImportJob(id: string): Promise<void> {
        const value = record(await call("literature.import-jobs", "apply_runtime_literature_import_jobs", {request: {kind: "cancel", id}}));
        if (value?.kind !== "done") mismatch();
      },
    } satisfies RuntimeLiteraturePort["literature"]),
    evidence: Object.freeze({
      saveClaim: draft => call<Claim>("literature.evidence-write", "save_runtime_evidence_claim", {draft}),
      linkEvidence: draft => call<EvidenceLink>("literature.evidence-write", "link_runtime_claim_to_anchor", {draft}),
      createClaimWithEvidence: (claim, evidence) => call<Claim>("literature.evidence-write", "create_runtime_claim_with_evidence", {claim, evidence}),
      listClaims: projectId => call<readonly Claim[]>("literature.evidence-read", "list_runtime_evidence_claims", {projectId}),
      listSources: projectId => call<readonly SourceAnchorSummary[]>("literature.evidence-read", "list_runtime_evidence_sources", {projectId}),
      listClaimEvidence: claimId => call<readonly EvidenceLink[]>("literature.evidence-read", "list_runtime_claim_evidence", {claimId}),
    } satisfies NonNullable<RuntimeLiteraturePort["evidence"]>),
    derivedViews: Object.freeze({
      list: projectId => call<readonly DerivedResearchView[]>("literature.derived-views", "list_runtime_derived_research_views", {projectId}),
      save: draft => call<DerivedResearchView>("literature.derived-views", "save_runtime_derived_research_view", {draft}),
    } satisfies NonNullable<RuntimeLiteraturePort["derivedViews"]>),
    evidenceMatrix: Object.freeze({
      list: projectId => call<readonly EvidenceMatrix[]>("literature.evidence-matrix", "list_runtime_evidence_matrices", {projectId}),
      save: draft => call<EvidenceMatrix>("literature.evidence-matrix", "save_runtime_evidence_matrix", {draft}),
      listCells: matrixId => call<readonly EvidenceMatrixCell[]>("literature.evidence-matrix", "list_runtime_evidence_matrix_cells", {matrixId}),
      saveCell: draft => call<EvidenceMatrixCell>("literature.evidence-matrix", "save_runtime_evidence_matrix_cell", {draft}),
    } satisfies NonNullable<RuntimeLiteraturePort["evidenceMatrix"]>),
    referenceRecords: Object.freeze({
      list: paperId => call<readonly ReferenceRecord[]>("literature.reference-records", "list_runtime_reference_records", {paperId}),
      save: draft => call<ReferenceRecord>("literature.reference-records", "save_runtime_reference_record", {draft}),
    } satisfies RuntimeLiteraturePort["referenceRecords"]),
    nal: Object.freeze({
      read: async () => decodeNalState(await call("literature.nal-bootstrap", "read_runtime_nal_network")),
      mutate: async (mutation, expectedRevision) => decodeNalState(await call("literature.nal-mutation", "mutate_runtime_nal_network", {mutation, expectedRevision})),
      ensureAnnotation: request => call("literature.nal-annotation", "ensure_runtime_nal_annotation", {request}),
    } satisfies RuntimeLiteraturePort["nal"]),
    knowledge: Object.freeze({
      analyzeSharedFoundations: draft => call<ConceptAlgorithmRun>("literature.concepts", "analyze_runtime_shared_foundations", {draft}),
      analyzeConceptEvolution: conceptId => call<ConceptAlgorithmRun>("literature.concepts", "analyze_runtime_concept_evolution", {conceptId}),
      getConceptAlgorithmRun: runId => call<ConceptAlgorithmRun>("literature.concepts", "get_runtime_concept_algorithm_run", {runId}),
      previewConceptMerge: draft => conceptIdentity<ConceptMergeImpact>({kind: "preview", draft}, "impact"),
      mergeConcepts: draft => conceptIdentity<ConceptIdentityEvent>({kind: "merge", draft}, "event"),
      splitConcepts: draft => conceptIdentity<ConceptIdentityEvent>({kind: "split", draft}, "event"),
      listConceptIdentityEvents: () => conceptIdentity<readonly ConceptIdentityEvent[]>({kind: "list"}, "events"),
      searchConcepts: query => call<readonly Concept[]>("literature.concepts", "search_runtime_research_concepts", {query}),
      saveConcept: draft => call<Concept>("literature.concepts", "save_runtime_research_concept", {draft}),
      captureConceptUsage: draft => call<ConceptCaptureResult>("literature.concepts", "capture_runtime_research_concept_usage", {draft}),
      saveDefinitionClaim: draft => call<DefinitionClaim>("literature.concepts", "save_runtime_definition_claim", {draft}),
      listDefinitionClaims: conceptId => call<readonly DefinitionClaim[]>("literature.concepts", "list_runtime_definition_claims", {conceptId}),
      searchRelationNodes: query => call<readonly RelationNodeSummary[]>("literature.concepts", "search_runtime_relation_nodes", {query}),
      saveRelationAssertion: draft => call<RelationAssertion>("literature.concepts", "save_runtime_relation_assertion", {draft}),
      listRelationAssertions: projectId => call<readonly RelationAssertion[]>("literature.concepts", "list_runtime_relation_assertions", {projectId}),
      listRevisions: (objectKind, objectId) => call<readonly ResearchObjectRevision[]>("literature.concepts", "list_runtime_research_object_revisions", {objectKind, objectId}),
      listReadingUnits: (projectId, paperId) => call<readonly StructuredReadingUnit[]>("literature.reading-units", "list_runtime_structured_reading_units", {projectId, paperId}),
      saveReadingUnit: draft => call<StructuredReadingUnit>("literature.reading-units", "save_runtime_structured_reading_unit", {draft}),
    } satisfies RuntimeLiteraturePort["knowledge"]),
    queries: Object.freeze({
      read: async () => queryStore(await call("literature.nal-query-collections", "read_runtime_nal_query_collections")),
      save: async (value, expectedRevision) => queryStore(await call("literature.nal-query-collections", "save_runtime_nal_query_collections", {value, expectedRevision})),
    } satisfies RuntimeLiteraturePort["queries"]),
  });
}
