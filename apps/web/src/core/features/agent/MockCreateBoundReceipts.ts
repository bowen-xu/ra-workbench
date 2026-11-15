import { AGENT_CREATE_BOUND_FAMILY, decodeAgentCreateBoundRequest, decodeAgentCreateBoundProof, decodeAgentCreateBoundStatus,
  type AgentCreateBoundRequest, type AgentCreateBoundProof, type AgentCreateBoundStatusResponse } from "@research-assistant/core/agent";

interface StoredCreation { readonly fingerprint: string; readonly proof: AgentCreateBoundProof }
export interface MockCreateBoundSnapshot { readonly entries: ReadonlyMap<string, StoredCreation> }
/** Synthetic receipt records; no UI, model or runtime admission behavior. */
export class MockCreateBoundReceipts {
  private readonly entries = new Map<string, StoredCreation>();
  constructor(private readonly epoch: () => string, private readonly foreignOperation: (id: string) => boolean = () => false) {}
  has(operationId: string): boolean { return this.entries.has(operationId); }
  snapshot(): MockCreateBoundSnapshot { return structuredClone({entries: this.entries}); }
  restore(snapshot: MockCreateBoundSnapshot | undefined): void {
    const entries = new Map<string, StoredCreation>();
    for (const [key, entry] of snapshot?.entries ?? []) {
      const proof = decodeAgentCreateBoundProof(entry.proof);
      let request: AgentCreateBoundRequest;
      try { request = decodeAgentCreateBoundRequest(JSON.parse(entry.fingerprint)); }
      catch { throw new Error("Invalid mock creation receipt"); }
      if (proof.epoch !== this.epoch() || proof.operationId !== key || request.operationId !== key
        || request.id !== proof.conversationId || JSON.stringify(request) !== entry.fingerprint) throw new Error("Invalid mock creation receipt");
      entries.set(key, {fingerprint: entry.fingerprint, proof});
    }
    this.entries.clear(); for (const entry of entries) this.entries.set(...entry);
  }
  private conflict(operationId: string): never {
    throw {code: "storage", reason: {code: "DATA_SQL_OPERATION_CONFLICT", reason: {operationId,
      epoch: this.epoch(), sqlFence: "0", phase: "prepare", sqliteCode: null, sqliteExtendedCode: null,
      safeCause: "operation_id_content_changed"}}};
  }
  lookup(input: AgentCreateBoundRequest): AgentCreateBoundProof | undefined {
    const request = decodeAgentCreateBoundRequest(input), saved = this.entries.get(request.operationId);
    if (this.foreignOperation(request.operationId)) return this.conflict(request.operationId);
    if (!saved) return undefined;
    if (saved.fingerprint !== JSON.stringify(request)) return this.conflict(request.operationId);
    return {...saved.proof, replayed: true};
  }
  apply(input: AgentCreateBoundRequest, liveCheck: (request: AgentCreateBoundRequest) => void,
    mutate: (request: AgentCreateBoundRequest) => void): AgentCreateBoundProof {
    const request = decodeAgentCreateBoundRequest(input);
    liveCheck(request);
    const replay = this.lookup(request);
    if (replay) return replay;
    mutate(request);
    const proof = decodeAgentCreateBoundProof({schemaVersion: 1, operationId: request.operationId, conversationId: request.id,
      family: AGENT_CREATE_BOUND_FAMILY, epoch: this.epoch(), sqlFence: "0", replayed: false, result: null});
    this.entries.set(request.operationId, {fingerprint: JSON.stringify(request), proof});
    return proof;
  }
  status(input: AgentCreateBoundRequest): AgentCreateBoundStatusResponse {
    const request = decodeAgentCreateBoundRequest(input), proof = this.lookup(request);
    return decodeAgentCreateBoundStatus({schemaVersion: 1, operationId: request.operationId, conversationId: request.id,
      status: proof ? {type: "committed", proof} : {type: "noSqlReceipt", epoch: this.epoch(), sqlFence: "0", closed: false}});
  }
}
