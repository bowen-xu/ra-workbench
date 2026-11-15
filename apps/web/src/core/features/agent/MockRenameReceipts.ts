import {
  AGENT_RENAME_FAMILY, decodeAgentRenameProof, decodeAgentRenameRequest, decodeAgentRenameStatus,
  type AgentRenameProof, type AgentRenameRequest, type AgentRenameStatusResponse,
} from "@research-assistant/core/agent";

interface StoredRename { readonly fingerprint: string; readonly proof: AgentRenameProof }
export interface MockRenameSnapshot { readonly epoch: string; readonly entries: ReadonlyMap<string, StoredRename> }

/** Synthetic receipt data only. Production rename controllers and readers remain shared. */
export class MockRenameReceipts {
  private epoch: string = crypto.randomUUID();
  private readonly entries = new Map<string, StoredRename>();
  constructor(private readonly foreignOperation: (id: string) => boolean = () => false) {}
  epochId(): string { return this.epoch; }
  has(operationId: string): boolean { return this.entries.has(operationId); }
  snapshot(): MockRenameSnapshot { return structuredClone({ epoch: this.epoch, entries: this.entries }); }
  restore(snapshot: MockRenameSnapshot | undefined): void {
    if (!snapshot) { this.entries.clear(); return; }
    if (!/^[\da-f]{8}-(?:[\da-f]{4}-){3}[\da-f]{12}$/iu.test(snapshot.epoch)
      || snapshot.epoch === "00000000-0000-0000-0000-000000000000") throw new Error("Invalid mock rename receipt");
    const entries = new Map<string, StoredRename>();
    for (const [id, entry] of snapshot.entries) {
      const proof = decodeAgentRenameProof(entry.proof);
      let request: AgentRenameRequest;
      try { request = decodeAgentRenameRequest(JSON.parse(entry.fingerprint)); }
      catch { throw new Error("Invalid mock rename receipt"); }
      if (proof.epoch !== snapshot.epoch || proof.operationId !== id || request.operationId !== id
        || request.id !== proof.conversationId || JSON.stringify(request) !== entry.fingerprint) throw new Error("Invalid mock rename receipt");
      entries.set(id, { fingerprint: entry.fingerprint, proof });
    }
    this.epoch = snapshot.epoch;
    this.entries.clear(); for (const entry of entries) this.entries.set(...entry);
  }
  lookup(input: AgentRenameRequest): AgentRenameProof | undefined {
    const request = decodeAgentRenameRequest(input), saved = this.entries.get(request.operationId);
    if (this.foreignOperation(request.operationId) || (saved && saved.fingerprint !== JSON.stringify(request))) throw {
      code: "storage", reason: { code: "DATA_SQL_OPERATION_CONFLICT", reason: {
        operationId: request.operationId, epoch: this.epoch, sqlFence: "0", phase: "prepare",
        sqliteCode: null, sqliteExtendedCode: null, safeCause: "operation_id_content_changed",
      } },
    };
    if (!saved) return undefined;
    return { ...saved.proof, replayed: true };
  }
  /** No await separates the fixture's CAS, metadata mutation and receipt record. */
  apply(input: AgentRenameRequest, mutate: (request: AgentRenameRequest) => void): AgentRenameProof {
    const request = decodeAgentRenameRequest(input), replay = this.lookup(request);
    if (replay) return replay;
    mutate(request);
    const proof = decodeAgentRenameProof({ schemaVersion: 1, operationId: request.operationId,
      conversationId: request.id, family: AGENT_RENAME_FAMILY, epoch: this.epoch, sqlFence: "0", replayed: false, result: null });
    this.entries.set(request.operationId, { fingerprint: JSON.stringify(request), proof });
    return proof;
  }
  status(input: AgentRenameRequest): AgentRenameStatusResponse {
    const request = decodeAgentRenameRequest(input), proof = this.lookup(request);
    return decodeAgentRenameStatus({ schemaVersion: 1, operationId: request.operationId, conversationId: request.id,
      status: proof ? { type: "committed", proof } : { type: "noSqlReceipt", epoch: this.epoch, sqlFence: "0", closed: false } });
  }
}
