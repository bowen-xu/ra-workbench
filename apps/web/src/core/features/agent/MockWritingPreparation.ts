import {AGENT_WRITING_PREPARE_FAMILY, decodeAgentSubmissionPreview, decodeAgentWritingPrepareRequest,
  decodeAgentWritingPrepareStatusResponse, type AgentSubmissionPreview, type AgentWritingPrepareProof,
  type AgentWritingPrepareRequest, type AgentWritingPrepareStatusResponse, type AgentContextText} from "@research-assistant/core/agent";

export type MockWritingPreparationPurpose = "regular" | "manualCompact" | "queuedRegular" | "queuedCompact";
export interface MockWritingPreparationReceipt {
  readonly fingerprint: string;
  readonly resultFingerprint: string;
  readonly proofFingerprint: string;
  readonly proof: AgentWritingPrepareProof;
}
/** Replace only retrieval storage I/O. Commit must publish its one-shot trace
 * consumption atomically with accept, or leave both unchanged on rejection. */
export interface MockWritingPreparationSources {
  readonly context: readonly AgentContextText[];
  commit(accept: () => void): void;
}
interface FrozenRecord {
  readonly request: AgentWritingPrepareRequest;
  readonly preview: AgentSubmissionPreview;
  readonly purpose?: MockWritingPreparationPurpose;
  readonly preparation?: MockWritingPreparationReceipt;
}
function fingerprint(request: AgentWritingPrepareRequest, purpose: MockWritingPreparationPurpose, expectedRevision: number | null): string {
  return JSON.stringify({request: decodeAgentWritingPrepareRequest(request), purpose, expectedRevision});
}
/** Mock SQL data only. A proof is created with the actual frozen record, never
 * reconstructed from execution flags or a later queue/history revision. */
export function mockWritingPreparationReceipt(request: AgentWritingPrepareRequest, preview: AgentSubmissionPreview,
  purpose: "regular" | "manualCompact", expectedRevision: number | null, epoch: string): MockWritingPreparationReceipt {
  const proof: AgentWritingPrepareProof = {schemaVersion: 1, operationId: request.requestId, conversationId: request.conversationId,
    family: AGENT_WRITING_PREPARE_FAMILY, epoch, sqlFence: "0", replayed: false, result: null};
  return {fingerprint: fingerprint(request, purpose, expectedRevision), resultFingerprint: JSON.stringify(decodeAgentSubmissionPreview(preview)),
    proof, proofFingerprint: JSON.stringify(proof)};
}
export function mockWritingPreparationUnconfirmed(request: AgentWritingPrepareRequest, safeCause: string): AgentWritingPrepareStatusResponse["status"] {
  return {type: "unconfirmed", detail: {operationId: request.requestId, epoch: null, sqlFence: null,
    phase: "writing-prepare-status", sqliteCode: null, sqliteExtendedCode: null, safeCause}};
}
export function mockWritingPreparationStatus(request: AgentWritingPrepareRequest, record: FrozenRecord | undefined,
  purpose: "regular" | "manualCompact", expectedRevision: number | null): AgentWritingPrepareStatusResponse["status"] {
  if (!record) return {type: "notFound"};
  if (record.purpose && record.purpose !== purpose) return mockWritingPreparationUnconfirmed(request, "mock_writing_prepare_purpose_mismatch");
  if (!record.preparation) return mockWritingPreparationUnconfirmed(request, "mock_writing_prepare_legacy_unconfirmed");
  try {
    const saved = record.preparation, actual = fingerprint(record.request, purpose, expectedRevision);
    if (record.purpose !== purpose || saved.fingerprint !== actual || actual !== fingerprint(request, purpose, expectedRevision)
      || saved.resultFingerprint !== JSON.stringify(decodeAgentSubmissionPreview(record.preview)) || saved.proofFingerprint !== JSON.stringify(saved.proof))
      return mockWritingPreparationUnconfirmed(request, "mock_writing_prepare_identity_mismatch");
    const decoded = decodeAgentWritingPrepareStatusResponse({schemaVersion: 1, intent: request,
      status: {type: "committed", result: record.preview, proof: {...saved.proof, replayed: true}}});
    return decoded.status;
  } catch {return mockWritingPreparationUnconfirmed(request, "mock_writing_prepare_record_invalid");}
}
/** A legacy or inconsistent record cannot gain a new typed proof on retry. */
export function requireMockWritingPreparation(status: AgentWritingPrepareStatusResponse["status"]): AgentSubmissionPreview {
  if (status.type === "committed") return structuredClone(status.result);
  throw {code: "recoveryRequired", ...(status.type === "unconfirmed" ? {detail: status.detail} : {})};
}
