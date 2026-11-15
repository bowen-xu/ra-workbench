import {expect, it} from "vitest";
import type {WritingQueuedStartSnapshot} from "./writingQueuedStart";
import {writingQueuedStartMessage} from "./writingQueuedStartMessage";
const id = "11111111-1111-4111-8111-111111111111", operationId = "22222222-2222-4222-8222-222222222222";
const state: WritingQueuedStartSnapshot = {phase: "localError", intent: {inputId: id, expectedRevision: 7, start: {conversationId: id, manuscriptId: id, requestId: id,
  contextSnapshotId: id, model: {providerId: "deepseek", modelId: "deepseek-v4-pro", revision: null}, mode: "deepSeekNonThinking"}}, turnId: id,
  status: null, result: {schemaVersion: 1, conversationId: id, requestId: id, turnId: id, alreadyStarted: true, outcome: null}, preview: null,
  priorUnknown: true, knownRejected: false, recovered: true, resumeCommand: null, resumeAcknowledged: false, queuePaused: false, draftAcknowledgment: null, cause: null};
const detail = {operationId, epoch: id, sqlFence: "17", phase: "queue-cancel-status", sqliteCode: "SQLITE_BUSY", sqliteExtendedCode: 5, safeCause: "mock_owner_unavailable"};
it("retains the matching cancellation reader's seven safe fields with the original start identity", () => {
  const message = writingQueuedStartMessage({...state, cause: {code: "QUEUE_START_CANCEL_UNCONFIRMED", cancelOperation: operationId,
    detail: {...detail, rawBody: "PRIVATE_BODY", localPath: "/private/source.tex"}}});
  for (const [field, value] of Object.entries(detail)) expect(message).toContain(`cancel.${field}=${value}`);
  for (const label of ["C", "R", "I", "T"]) expect(message).toContain(`${label}=${id}`);
  expect(message).toContain(`cancelOperation=${operationId}`); expect(message).not.toContain("PRIVATE_BODY"); expect(message).not.toContain("/private/source.tex");
});
it("does not attach a different operation's diagnostic or unsafe text to the original cancellation", () => {
  const mismatched = writingQueuedStartMessage({...state, cause: {code: "QUEUE_START_CANCEL_UNCONFIRMED", cancelOperation: operationId, detail: {...detail, operationId: id}}});
  expect(mismatched).not.toContain("cancel.safeCause");
  const unsafe = writingQueuedStartMessage({...state, cause: {code: "QUEUE_START_CANCEL_UNCONFIRMED", cancelOperation: operationId,
    detail: {...detail, phase: "PRIVATE BODY", safeCause: "PRIVATE BODY", sqliteCode: "PRIVATE BODY"}}});
  expect(unsafe).not.toContain("PRIVATE BODY"); expect(unsafe).toContain(`cancel.operationId=${operationId}`);
});
