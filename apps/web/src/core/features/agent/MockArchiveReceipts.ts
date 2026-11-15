import {
  AGENT_ARCHIVE_FAMILY, decodeAgentArchiveProof, decodeAgentArchiveRequest, decodeAgentArchiveStatus,
  type AgentArchiveProof, type AgentArchiveRequest, type AgentArchiveStatusResponse,
} from "@research-assistant/core/agent";

interface StoredArchive { readonly fingerprint: string; readonly proof: AgentArchiveProof }
export interface MockArchiveSnapshot { readonly epoch: string; readonly entries: ReadonlyMap<string, StoredArchive> }

/** Receipt data adapter only; production archive owners handle intent and recovery. */
export class MockArchiveReceipts {
  private readonly entries = new Map<string, StoredArchive>();
  constructor(private readonly epoch: () => string,
    private readonly foreignOperation: (id: string) => boolean = () => false) {}
  has(operationId: string): boolean { return this.entries.has(operationId); }
  snapshot(): MockArchiveSnapshot { return structuredClone({ epoch: this.epoch(), entries: this.entries }); }
  restore(snapshot: MockArchiveSnapshot | undefined): void {
    const entries = new Map<string, StoredArchive>();
    if (snapshot) {
      if (snapshot.epoch !== this.epoch()) throw new Error("Invalid mock archive receipt");
      for (const [id, saved] of snapshot.entries) {
        const proof = decodeAgentArchiveProof(saved.proof);
        let request: AgentArchiveRequest;
        try { request = decodeAgentArchiveRequest(JSON.parse(saved.fingerprint)); }
        catch { throw new Error("Invalid mock archive receipt"); }
        if (request.operationId !== id || proof.operationId !== id || request.id !== proof.conversationId
          || proof.epoch !== snapshot.epoch || JSON.stringify(request) !== saved.fingerprint) throw new Error("Invalid mock archive receipt");
        entries.set(id, { fingerprint: saved.fingerprint, proof });
      }
    }
    this.entries.clear(); for (const saved of entries) this.entries.set(...saved);
  }
  lookup(input: AgentArchiveRequest): AgentArchiveProof | undefined {
    const request = decodeAgentArchiveRequest(input), saved = this.entries.get(request.operationId);
    if (this.foreignOperation(request.operationId) || (saved && saved.fingerprint !== JSON.stringify(request))) throw {
      code: "storage", reason: { code: "DATA_SQL_OPERATION_CONFLICT", reason: {
        operationId: request.operationId, epoch: this.epoch(), sqlFence: "0", phase: "prepare",
        sqliteCode: null, sqliteExtendedCode: null, safeCause: "operation_id_content_changed",
      } },
    };
    return saved ? { ...saved.proof, replayed: true } : undefined;
  }
  /** The fixture's current-history CAS, mutation and receipt have no await gap. */
  apply(input: AgentArchiveRequest, mutate: (request: AgentArchiveRequest) => void): AgentArchiveProof {
    const request = decodeAgentArchiveRequest(input), replay = this.lookup(request);
    if (replay) return replay;
    mutate(request);
    const proof = decodeAgentArchiveProof({ schemaVersion: 1, operationId: request.operationId,
      conversationId: request.id, family: AGENT_ARCHIVE_FAMILY, epoch: this.epoch(), sqlFence: "0", replayed: false, result: null });
    this.entries.set(request.operationId, { fingerprint: JSON.stringify(request), proof });
    return proof;
  }
  status(input: AgentArchiveRequest): AgentArchiveStatusResponse {
    const request = decodeAgentArchiveRequest(input), proof = this.lookup(request);
    return decodeAgentArchiveStatus({ schemaVersion: 1, operationId: request.operationId, conversationId: request.id,
      status: proof ? { type: "committed", proof } : { type: "noSqlReceipt", epoch: this.epoch(), sqlFence: "0", closed: false } });
  }
}
