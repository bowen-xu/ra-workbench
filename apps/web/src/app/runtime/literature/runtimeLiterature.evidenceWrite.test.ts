import {expect, it, vi} from "vitest";
import type {ClaimDraft, EvidenceLinkDraft, NewEvidenceDraft} from "@research-assistant/core/evidence";
import {createRuntimeLiteraturePort} from "./runtimeLiterature";

const key = "e".repeat(64);
const capabilities = [{id: "literature.evidence-write", version: 1}];
const claim: ClaimDraft = {
  projectId: " p ", statement: " Manual Ω ", epistemicRole: "hypothesis", changeReason: " human ",
};
const evidence: NewEvidenceDraft = {anchorId: " a\u0000 ", stance: "context", rationale: " chosen "};
const link: EvidenceLinkDraft = {claimId: " c ", ...evidence};

it("routes all three original evidence writes with exact values and package identity", async () => {
  const value = {original: true};
  const invoke = vi.fn(async () => value);
  const port = createRuntimeLiteraturePort(key, capabilities, () => undefined, invoke).evidence!;
  expect(await port.saveClaim!(claim)).toBe(value);
  expect(await port.linkEvidence!(link)).toBe(value);
  expect(await port.createClaimWithEvidence!(claim, evidence)).toBe(value);
  expect(invoke.mock.calls).toEqual([
    ["save_runtime_evidence_claim", {packageKey: key, draft: claim}],
    ["link_runtime_claim_to_anchor", {packageKey: key, draft: link}],
    ["create_runtime_claim_with_evidence", {packageKey: key, claim, evidence}],
  ]);
});

it("rejects missing or incompatible capability before transport and does not replay partial publication", async () => {
  const invoke = vi.fn();
  for (const declared of [[], [{id: "literature.evidence-write", version: 2}]]) {
    const port = createRuntimeLiteraturePort(key, declared, () => undefined, invoke).evidence!;
    await expect(port.saveClaim!(claim)).rejects.toMatchObject({code: "literature_gateway_unavailable"});
    await expect(port.linkEvidence!(link)).rejects.toMatchObject({code: "literature_gateway_unavailable"});
    await expect(port.createClaimWithEvidence!(claim, evidence)).rejects.toMatchObject({code: "literature_gateway_unavailable"});
  }
  expect(invoke).not.toHaveBeenCalled();
  const evidenceWrite = {
    phase: "postcommit-read", outcome: "committed", objectId: "stored-link",
    databaseError: {safeCause: "sqlite_operation_failed", sqliteExtendedCode: 23},
    unknownBody: "private synthetic body",
  };
  const cause = {
    code: "storage_error", message: "证据操作已提交但读回失败，禁止自动重放。", operationId: "operation", evidenceWrite,
  };
  invoke.mockRejectedValue(cause);
  const error = await createRuntimeLiteraturePort(key, capabilities, () => undefined, invoke)
    .evidence!.linkEvidence!(link).catch(error => error);
  expect(error).toMatchObject({code: "storage_error", evidenceWrite, cause});
  expect(error.message).toContain("evidenceWrite.outcome=committed");
  expect(error.message).toContain("sqliteExtendedCode=23");
  expect(error.message).not.toContain("private synthetic body");
  expect(invoke).toHaveBeenCalledTimes(1);
});

it("shows standalone statement failure without changing other storage diagnostic states", async () => {
  const invoke = vi.fn().mockRejectedValue({
    code: "storage_error", message: "Original statement failed",
    evidenceWrite: {phase: "link-insert", outcome: "statementFailed"},
    derivedView: {phase: "link-insert", outcome: "statementFailed"},
    saveOutcome: {status: "statementFailed"},
  });
  const error = await createRuntimeLiteraturePort(key, capabilities, () => undefined, invoke)
    .evidence!.linkEvidence!(link).catch(error => error);
  expect(error.message).toContain("evidenceWrite.phase=link-insert");
  expect(error.message).toContain("evidenceWrite.outcome=statementFailed");
  expect(error.message).not.toContain("derivedView.phase=link-insert");
  expect(error.message).not.toContain("derivedView.outcome=statementFailed");
  expect(error.message).not.toContain("saveOutcome=statementFailed");
});
