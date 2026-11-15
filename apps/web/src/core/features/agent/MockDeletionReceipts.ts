import {
  AGENT_DELETION_FAMILY, decodeAgentDeletionProof, decodeAgentDeletionRequest, decodeAgentDeletionStatus, decodeAgentDeletionWriteOptions,
  type AgentDeletionProof, type AgentDeletionRequest, type AgentDeletionStatusResponse, type AgentDeletionWriteOptions,
} from "@research-assistant/core/agent";

interface StoredDeletion { readonly fingerprint: string; readonly proof: AgentDeletionProof | null }
export interface MockDeletionSnapshot { readonly epoch: string; readonly entries: ReadonlyMap<string, StoredDeletion> }

/** Data and receipts only. A legacy tombstone never becomes a managed SQL proof. */
export class MockDeletionReceipts {
  private readonly entries = new Map<string, StoredDeletion>();
  constructor(private readonly epoch: () => string,
    private readonly foreignOperation: (id: string) => boolean = () => false) {}
  has(operationId: string): boolean { return this.entries.has(operationId); }
  snapshot(): MockDeletionSnapshot { return structuredClone({ epoch: this.epoch(), entries: this.entries }); }
  restore(snapshot: MockDeletionSnapshot | undefined, legacy: ReadonlyMap<string, string> = new Map()): void {
    const entries = new Map<string, StoredDeletion>();
    if (snapshot && snapshot.epoch !== this.epoch()) throw new Error("Invalid mock deletion receipt");
    const source = snapshot?.entries ?? new Map([...legacy].map(([id, fingerprint]) => [id, {fingerprint, proof: null}]));
    for (const [id, saved] of source) {
      const request = decodeAgentDeletionRequest(JSON.parse(saved.fingerprint));
      const proof = saved.proof === null ? null : decodeAgentDeletionProof(saved.proof);
      if (request.requestId !== id || JSON.stringify(request) !== saved.fingerprint || proof &&
        (proof.operationId !== id || proof.conversationId !== request.conversationId || proof.epoch !== this.epoch())) throw new Error("Invalid mock deletion receipt");
      entries.set(id, { fingerprint: saved.fingerprint, proof });
    }
    this.entries.clear(); for (const saved of entries) this.entries.set(...saved);
  }
  private conflict(request: AgentDeletionRequest, phase: string, cause: string): never {
    throw {code: "storage", reason: {code: "DATA_SQL_OPERATION_CONFLICT", reason: {
      operationId: request.requestId, epoch: this.epoch(), sqlFence: "0", phase, safeCause: cause,
    }}};
  }
  validateWrite(input: AgentDeletionRequest, options?: AgentDeletionWriteOptions): AgentDeletionRequest {
    const request = decodeAgentDeletionRequest(input);
    if (options !== undefined && decodeAgentDeletionWriteOptions(options).expectedEpoch !== this.epoch())
      this.conflict(request, "delete-epoch", "data_epoch_changed");
    this.lookup(request);
    return request;
  }
  private lookup(input: AgentDeletionRequest): StoredDeletion | undefined {
    const request = decodeAgentDeletionRequest(input), saved = this.entries.get(request.requestId);
    if (this.foreignOperation(request.requestId) || saved && saved.fingerprint !== JSON.stringify(request))
      this.conflict(request, "prepare", "operation_id_content_changed");
    return saved;
  }
  replay(input: AgentDeletionRequest): boolean { return this.lookup(input) !== undefined; }
  /** Called with the fixture's accepted deletion, with no await before publication. */
  apply(input: AgentDeletionRequest, mutate: () => void): void {
    const request = decodeAgentDeletionRequest(input);
    if (this.lookup(request)) return;
    const proof = decodeAgentDeletionProof({schemaVersion: 1, operationId: request.requestId, conversationId: request.conversationId,
      family: AGENT_DELETION_FAMILY, epoch: this.epoch(), sqlFence: "0", replayed: false, result: null});
    mutate();
    this.entries.set(request.requestId, {fingerprint: JSON.stringify(request), proof});
  }
  status(input: AgentDeletionRequest): AgentDeletionStatusResponse {
    const request = decodeAgentDeletionRequest(input), saved = this.lookup(request);
    return decodeAgentDeletionStatus({schemaVersion: 1, operationId: request.requestId, conversationId: request.conversationId,
      status: saved?.proof ? {type: "committed", proof: {...saved.proof, replayed: true}}
        : saved ? {type: "unconfirmed", epoch: this.epoch(), sqlFence: "0", blocked: false, closed: false, reason: "legacy_sql_receipt_unavailable"}
        : {type: "noSqlReceipt", epoch: this.epoch(), sqlFence: "0", closed: false}});
  }
}
