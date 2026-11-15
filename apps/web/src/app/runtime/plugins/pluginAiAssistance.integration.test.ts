// @vitest-environment happy-dom
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import type {AiTaskSpecDraft, PaperAiReadingPrepareResult, PaperAnalysisStartDraft} from "@research-assistant/core/ai";
import {createPreparedPaperGenerationGateway} from "@research-assistant/literature-library-plugin/paperPreparation";
import {selectAppRuntime} from "../../../core/runtime/appRuntime";
import {subscribeConversationCatalog} from "../../../core/runtime/conversationCatalogEvents";
import {getAiAssistanceGateway as getWideGateway, type AiAssistanceGateway as WideGateway} from "../../../core/public/aiAssistance";
import type {DoclingParserGateway} from "../../../core/public/docling";
import {getAiAssistanceGateway} from "../../../core/public/pluginAiAssistance";

function gateway(overrides: Partial<WideGateway> = {}): WideGateway {
  const unexpected = (): Promise<never> => Promise.reject(new Error("Unexpected synthetic gateway call"));
  return {
    getLlmConfiguration: unexpected, saveLlmProviderConfig: unexpected,
    saveLlmModelRoutes: unexpected, fetchLlmProviderModels: unexpected,
    getChatGptAccountStatus: unexpected, startChatGptLogin: unexpected,
    cancelChatGptLogin: unexpected, logoutChatGpt: unexpected,
    listProviders: unexpected, saveProviderSettings: unexpected,
    setCredential: unexpected, deleteCredential: unexpected, testProvider: unexpected,
    previewTask: unexpected, startRun: unexpected, startPaperAnalysis: unexpected,
    cancelPaperAnalysis: unexpected, getPaperAnalysisWorkflow: unexpected,
    getLatestPaperAnalysisWorkflow: unexpected, getRun: unexpected,
    getLatestPaperPortraitRun: unexpected, listRuns: unexpected, cancelRun: unexpected,
    listCandidates: unexpected, reviseCandidate: unexpected,
    confirmOverviewFields: unexpected, undoOverviewConfirmation: unexpected,
    confirmPaperReview: unexpected, undoPaperReviewConfirmation: unexpected,
    decideCandidate: unexpected, decideSuggestionItem: unexpected,
    decidePaperConceptItem: unexpected, recordCitationAssessment: unexpected,
    recordEvidenceInspection: unexpected, recordAttentionFeedback: unexpected,
    createCandidateSummaryVersion: unexpected, selectCandidateSummaryVersion: unexpected,
    discardOtherCandidateSummaryVersions: unexpected, preparePaperReadingContext: unexpected,
    preparePaperReadingBootstrap: unexpected, previewConceptRetrieval: unexpected,
    cancelConceptRetrievalPreview: unexpected, getConceptRetrievalTrace: unexpected,
    startPaperReadingTurn: unexpected, steerPaperReadingTurn: unexpected,
    cancelPaperReadingTurn: unexpected, listPaperReadingSessions: unexpected,
    getPaperReadingSession: unexpected, getPaperReadingTurnDebugData: unexpected,
    deletePaperReadingSession: unexpected, ...overrides,
  };
}

let storage: Storage;
function memoryStorage(): Storage {
  const values = new Map<string, string>();
  return {
    get length() {return values.size;},
    getItem: key => values.get(key) ?? null,
    setItem: (key, value) => {values.set(key, value);},
    removeItem: key => {values.delete(key);},
    key: index => [...values.keys()][index] ?? null,
    clear: () => {values.clear();},
  };
}

function select(source?: WideGateway): void {
  selectAppRuntime({mode: "preview", storage,
    gateways: source ? {getAiAssistanceGateway: source} : {}, dispose() {}});
}

function deferred<T>() {
  let resolve!: (value: T | PromiseLike<T>) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((done, fail) => {resolve = done; reject = fail;});
  return {promise, resolve, reject};
}

const portrait: AiTaskSpecDraft = {taskKind: "paper_profile_classification", providerId: "fixture-generation",
  userGoal: "Synthetic portrait", pipelineVersion: "sources", promptVersion: "v1", outputSchemaVersion: 1,
  maxInputTokens: 1000, maxOutputTokens: 1000, timeoutMs: 5000,
  scopeItems: [{itemKind: "attachment", objectId: "file", parentObjectId: "paper"}]};
const reading: PaperAnalysisStartDraft = {clientRequestId: "synthetic-request", paperId: "paper",
  attachmentIds: ["file"], overviewProfileSelectionRevision: 1, imageAssets: []};

beforeEach(() => {storage = memoryStorage(); select();});
afterEach(() => {select(); vi.restoreAllMocks();});

describe("finite live AI assistance projection", () => {
  it("retains one frozen narrow identity without reading methods or exposing the wide owner", () => {
    let reads = 0;
    let retrievalReads = 0, cancellationReads = 0;
    const source = gateway();
    const readRun = source.getRun;
    const previewRetrieval = source.previewConceptRetrieval, cancelRetrieval = source.cancelConceptRetrievalPreview;
    Object.defineProperty(source, "getRun", {enumerable: true, get() {reads++; return readRun;}});
    Object.defineProperty(source, "previewConceptRetrieval", {enumerable: true, get() {retrievalReads++; return previewRetrieval;}});
    Object.defineProperty(source, "cancelConceptRetrievalPreview", {enumerable: true, get() {cancellationReads++; return cancelRetrieval;}});
    select(source);
    const port = getAiAssistanceGateway();
    expect(getAiAssistanceGateway()).toBe(port);
    expect(Object.isFrozen(port)).toBe(true);
    expect(Object.keys(port).sort()).toEqual([
      "getLlmConfiguration", "fetchLlmProviderModels", "saveLlmModelRoutes", "previewTask", "startRun",
      "getRun", "cancelRun", "getLatestPaperPortraitRun", "startPaperAnalysis", "cancelPaperAnalysis",
      "getLatestPaperAnalysisWorkflow", "listCandidates", "reviseCandidate", "confirmOverviewFields",
      "undoOverviewConfirmation", "recordEvidenceInspection", "preparePaperReadingContext",
      "previewConceptRetrieval", "cancelConceptRetrievalPreview",
    ].sort());
    for (const descriptor of Object.values(Object.getOwnPropertyDescriptors(port))) {
      expect(descriptor.get).toBeTypeOf("function");
      expect(descriptor).not.toHaveProperty("value");
    }
    expect(reads).toBe(0);
    expect(retrievalReads).toBe(0); expect(cancellationReads).toBe(0);
    void port.getRun;
    expect(reads).toBe(1);
    void port.previewConceptRetrieval; void port.cancelConceptRetrievalPreview;
    expect(retrievalReads).toBe(1); expect(cancellationReads).toBe(1);
    for (const key of ["setCredential", "deleteCredential", "startChatGptLogin", "getChatGptAccountStatus",
      "saveLlmProviderConfig", "saveProviderSettings", "invoke", "wide", "raw", "transport"]) {
      expect(Reflect.get(port, key)).toBeUndefined();
    }
    expect("setCredential" in getWideGateway()).toBe(true);
  });

  it("uses current read-time selection while previously read functions retain this and Promise identity", async () => {
    const first = Promise.resolve(true), second = Promise.resolve(false);
    const owners: object[] = [];
    const sourceA = gateway({cancelRun() {owners.push(this); return first;}});
    const sourceB = gateway({cancelRun() {owners.push(this); return second;}});
    select(sourceA);
    const port = getAiAssistanceGateway();
    const captured = port.cancelRun;
    select(sourceB);
    expect(getAiAssistanceGateway()).toBe(port);
    expect(captured("one")).toBe(first);
    expect(port.cancelRun("two")).toBe(second);
    expect(owners).toEqual([sourceA, sourceB]);
    await expect(first).resolves.toBe(true);
    await expect(second).resolves.toBe(false);
    const failure = Object.assign(new Error("Synthetic original cause"), {code: "synthetic_failure", requestId: "request"});
    select(gateway({listCandidates: () => Promise.reject(failure)}));
    await expect(port.listCandidates()).rejects.toBe(failure);
  });

  it("preserves concept retrieval and cancellation read-time owners, Promises and original failures", async () => {
    const first = deferred<never>(), second = deferred<never>();
    const firstCancellation = Promise.resolve(true), secondCancellation = Promise.resolve(false);
    const owners: object[] = [];
    const sourceA = gateway({
      previewConceptRetrieval() {owners.push(this); return first.promise;},
      cancelConceptRetrievalPreview() {owners.push(this); return firstCancellation;},
    });
    const sourceB = gateway({
      previewConceptRetrieval() {owners.push(this); return second.promise;},
      cancelConceptRetrievalPreview() {owners.push(this); return secondCancellation;},
    });
    select(sourceA);
    const port = getAiAssistanceGateway(), capturedPreview = port.previewConceptRetrieval;
    const capturedCancellation = port.cancelConceptRetrievalPreview;
    select(sourceB);
    expect(getAiAssistanceGateway()).toBe(port);
    const draft = {clientRequestId: "retrieval", query: "Synthetic concept"};
    expect(capturedPreview(draft)).toBe(first.promise);
    expect(port.previewConceptRetrieval(draft)).toBe(second.promise);
    expect(capturedCancellation("first")).toBe(firstCancellation);
    expect(port.cancelConceptRetrievalPreview("second")).toBe(secondCancellation);
    expect(owners).toEqual([sourceA, sourceB, sourceA, sourceB]);
    const originalFailure = Object.assign(new Error("Synthetic retrieval failure"), {code: "retrieval_unavailable", requestId: draft.clientRequestId});
    const nextFailure = new Error("Synthetic current retrieval failure");
    const observedFirst = expect(first.promise).rejects.toBe(originalFailure), observedSecond = expect(second.promise).rejects.toBe(nextFailure);
    first.reject(originalFailure); second.reject(nextFailure);
    await observedFirst; await observedSecond;
    await expect(firstCancellation).resolves.toBe(true); await expect(secondCancellation).resolves.toBe(false);
  });

  it("retains original catalog invalidation on failure and its captured-runtime finally guard", async () => {
    const pending = deferred<never>();
    const failure = new Error("Synthetic admission failed");
    select(gateway({startRun: () => pending.promise}));
    const port = getAiAssistanceGateway();
    const invalidated = vi.fn();
    const unsubscribe = subscribeConversationCatalog(invalidated);
    try {
      const request = port.startRun(portrait);
      const observed = expect(request).rejects.toBe(failure);
      expect(invalidated).not.toHaveBeenCalled();
      pending.reject(failure);
      await observed;
      expect(invalidated).toHaveBeenCalledTimes(1);
      const oldMethod = port.startRun;
      select(gateway({startRun: () => Promise.reject(new Error("New owner"))}));
      await expect(oldMethod(portrait)).rejects.toBe(failure);
      expect(invalidated).toHaveBeenCalledTimes(1);
      await expect(port.startRun(portrait)).rejects.toThrow("New owner");
      expect(invalidated).toHaveBeenCalledTimes(2);
    } finally {unsubscribe();}
  });
});

function preparedFixture() {
  let available = false;
  const parsing = deferred<Awaited<ReturnType<DoclingParserGateway["parseAttachment"]>>>();
  const started = deferred<void>();
  const order: string[] = [];
  const failure = new Error("Synthetic original execution result");
  const raw = gateway({
    preparePaperReadingContext: vi.fn(async (): Promise<PaperAiReadingPrepareResult> => {
      order.push("prepare");
      return {attachmentStatuses: [{attachmentId: "file", displayName: "synthetic.pdf",
        contentSha256: "a".repeat(64), parseStatus: available ? "available" : "missing"}],
      contextItems: [], imageRequests: [], textItemCount: 0, tableItemCount: 0, pictureItemCount: 0};
    }),
    startRun: vi.fn(() => {order.push("startRun"); return Promise.reject(failure);}),
    startPaperAnalysis: vi.fn(() => {order.push("startPaperAnalysis"); return Promise.reject(failure);}),
    cancelPaperAnalysis: vi.fn(async () => false),
  });
  const parser: Pick<DoclingParserGateway, "parseAttachment"> = {
    parseAttachment: vi.fn(() => {order.push("parse"); started.resolve(undefined); return parsing.promise;}),
  };
  const prepared = createPreparedPaperGenerationGateway(raw, () => parser);
  select(prepared);
  return {raw, order, failure, started: started.promise, port: getAiAssistanceGateway(),
    finishParse() {
      available = true;
      parsing.resolve({id: "parse", itemId: "paper", attachmentId: "file", contentSha256: "a".repeat(64),
        serviceBaseUrl: "http://synthetic.invalid", doclingStatus: "success", parseStatus: "success",
        createdAt: "2026-11-05T00:00:00Z", finishedAt: "2026-11-05T00:00:00Z", rawResponse: {synthetic: true}});
    },
  };
}

describe("original prepared gateway remains ahead of plugin AI execution", () => {
  it.each(["portrait", "analysis"] as const)("%s preserves saved-parse admission before execution", async stage => {
    const fixture = preparedFixture();
    const pending = stage === "portrait" ? fixture.port.startRun(portrait) : fixture.port.startPaperAnalysis(reading);
    const observed = expect(pending).rejects.toBe(fixture.failure);
    await fixture.started;
    expect(fixture.order).toEqual(["prepare", "parse"]);
    expect(fixture.raw.startRun).not.toHaveBeenCalled();
    expect(fixture.raw.startPaperAnalysis).not.toHaveBeenCalled();
    fixture.finishParse();
    await observed;
    expect(fixture.order).toEqual(["prepare", "parse", "prepare", stage === "portrait" ? "startRun" : "startPaperAnalysis"]);
  });

  it("preserves original cancellation during preparation and requires the next explicit call", async () => {
    const fixture = preparedFixture();
    const pending = fixture.port.startPaperAnalysis(reading);
    const observed = expect(pending).rejects.toMatchObject({code: "canceled"});
    await fixture.started;
    await expect(fixture.port.cancelPaperAnalysis(reading.clientRequestId)).resolves.toBe(true);
    fixture.finishParse();
    await observed;
    expect(fixture.raw.startPaperAnalysis).not.toHaveBeenCalled();
    expect(fixture.raw.cancelPaperAnalysis).not.toHaveBeenCalled();
    await expect(fixture.port.startPaperAnalysis({...reading, clientRequestId: "next-explicit-request"}))
      .rejects.toBe(fixture.failure);
    expect(fixture.raw.startPaperAnalysis).toHaveBeenCalledTimes(1);
  });
});
