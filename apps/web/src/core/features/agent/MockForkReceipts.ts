import {
  AGENT_FORK_FAMILY, decodeAgentForkProof, decodeAgentForkRequest, decodeAgentForkStatus,
  type AgentForkProof, type AgentForkRequest, type AgentForkStatusResponse,
} from "@research-assistant/core/agent";

interface StoredFork { readonly fingerprint: string; readonly proof: AgentForkProof }
export interface MockForkSnapshot { readonly epoch: string; readonly entries: ReadonlyMap<string, StoredFork> }

/** Receipt data adapter only; production fork owners handle intent and recovery. */
export class MockForkReceipts {
  private readonly entries = new Map<string, StoredFork>();
  constructor(private readonly epoch: () => string,
    private readonly foreignOperation: (id: string) => boolean = () => false) {}
  has(operationId: string): boolean { return this.entries.has(operationId); }
  snapshot(): MockForkSnapshot { return structuredClone({ epoch: this.epoch(), entries: this.entries }); }
  restore(snapshot: MockForkSnapshot | undefined): void {
    const entries = new Map<string, StoredFork>();
    if (snapshot) {
      if (snapshot.epoch !== this.epoch()) throw new Error("Invalid mock fork receipt");
      for (const [id, saved] of snapshot.entries) {
        const proof = decodeAgentForkProof(saved.proof);
        let request: AgentForkRequest;
        try { request = decodeAgentForkRequest(JSON.parse(saved.fingerprint)); }
        catch { throw new Error("Invalid mock fork receipt"); }
        if (request.childId !== id || proof.operationId !== id || request.childId !== proof.conversationId
          || proof.epoch !== snapshot.epoch || JSON.stringify(request) !== saved.fingerprint) throw new Error("Invalid mock fork receipt");
        entries.set(id, { fingerprint: saved.fingerprint, proof });
      }
    }
    this.entries.clear(); for (const saved of entries) this.entries.set(...saved);
  }
  lookup(input: AgentForkRequest): AgentForkProof | undefined {
    const request = decodeAgentForkRequest(input), saved = this.entries.get(request.childId);
    if (this.foreignOperation(request.childId) || (saved && saved.fingerprint !== JSON.stringify(request))) throw {
      code: "storage", reason: { code: "DATA_SQL_OPERATION_CONFLICT", reason: {
        operationId: request.childId, epoch: this.epoch(), sqlFence: "0", phase: "prepare",
        sqliteCode: null, sqliteExtendedCode: null, safeCause: "operation_id_content_changed",
      } },
    };
    return saved ? { ...saved.proof, replayed: true } : undefined;
  }
  /** The fixture's current-history CAS, mutation and receipt have no await gap. */
  apply(input: AgentForkRequest, mutate: (request: AgentForkRequest) => void): AgentForkProof {
    const request = decodeAgentForkRequest(input), replay = this.lookup(request);
    if (replay) return replay;
    mutate(request);
    const proof = decodeAgentForkProof({ schemaVersion: 1, operationId: request.childId,
      conversationId: request.childId, family: AGENT_FORK_FAMILY, epoch: this.epoch(), sqlFence: "0", replayed: false, result: null });
    this.entries.set(request.childId, { fingerprint: JSON.stringify(request), proof });
    return proof;
  }
  status(input: AgentForkRequest): AgentForkStatusResponse {
    const request = decodeAgentForkRequest(input), proof = this.lookup(request);
    return decodeAgentForkStatus({ schemaVersion: 1, operationId: request.childId, conversationId: request.childId,
      status: proof ? { type: "committed", proof } : { type: "noSqlReceipt", epoch: this.epoch(), sqlFence: "0", closed: false } });
  }
}
