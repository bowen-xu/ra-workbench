import {beforeEach, expect, it, vi} from "vitest";
import type {AiAssistanceGateway} from "./aiAssistance";
import {getAiAssistanceGateway} from "./pluginAiAssistance";

const state = vi.hoisted(() => ({gateway: undefined as AiAssistanceGateway | undefined}));
vi.mock("./aiAssistance", () => ({getAiAssistanceGateway: () => state.gateway}));
beforeEach(() => {state.gateway = undefined;});

it("reads the two original retrieval methods lazily and retains the selected gateway projection", async () => {
  const result = {traceId: "synthetic-trace"};
  const first = vi.fn(async () => result), next = vi.fn(async () => result), cancel = vi.fn(async () => false);
  let selected = first;
  const reads = {preview: 0, cancel: 0};
  state.gateway = {
    get previewConceptRetrieval() {reads.preview += 1; return selected;},
    get cancelConceptRetrievalPreview() {reads.cancel += 1; return cancel;},
  } as unknown as AiAssistanceGateway;
  const gateway = getAiAssistanceGateway();
  expect(getAiAssistanceGateway()).toBe(gateway);
  expect(reads).toEqual({preview: 0, cancel: 0});
  const captured = gateway.previewConceptRetrieval;
  expect(captured).toBe(first);
  selected = next;
  expect(gateway.previewConceptRetrieval).toBe(next);
  const draft = {clientRequestId: "synthetic-request", query: "synthetic-query"};
  await expect(captured(draft)).resolves.toBe(result);
  await expect(gateway.cancelConceptRetrievalPreview(draft.clientRequestId)).resolves.toBe(false);
  expect(first).toHaveBeenCalledExactlyOnceWith(draft);
  expect(next).not.toHaveBeenCalled();
  expect(cancel).toHaveBeenCalledExactlyOnceWith(draft.clientRequestId);
  expect(gateway).not.toHaveProperty("setCredential");
  expect(gateway).not.toHaveProperty("invoke");
  expect(Object.isFrozen(gateway)).toBe(true);
});

it("preserves the original retrieval rejection object and selected-owner function binding", async () => {
  const failure = Object.assign(new Error("synthetic_retrieval_failure"), {code: "synthetic_failure", requestId: "synthetic-request"});
  const owner = {
    async preview(_draft: unknown) {throw failure;},
    async cancel(_requestId: string) {expect(this).toBe(owner); return true;},
  };
  const preview = owner.preview.bind(owner), cancel = owner.cancel.bind(owner);
  state.gateway = {previewConceptRetrieval: preview, cancelConceptRetrievalPreview: cancel} as unknown as AiAssistanceGateway;
  const gateway = getAiAssistanceGateway();
  expect(gateway.previewConceptRetrieval).toBe(preview);
  expect(gateway.cancelConceptRetrievalPreview).toBe(cancel);
  await expect(gateway.previewConceptRetrieval({clientRequestId: "synthetic-request", query: "synthetic"})).rejects.toBe(failure);
  await expect(gateway.cancelConceptRetrievalPreview("synthetic-request")).resolves.toBe(true);
});
