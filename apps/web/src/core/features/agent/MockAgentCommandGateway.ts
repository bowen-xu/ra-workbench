import type { AgentCommandGateway, AgentCommandRequest, AgentCompactRequest, AgentCommandUsage, AgentCompactResult } from "./agentCommandGateway";

/** Synthetic token snapshots only: no provider, transcript or file mutations. */
export class MockAgentCommandGateway implements AgentCommandGateway {
  private readonly usages = new Map<string, AgentCommandUsage>();
  private readonly receipts = new Map<string, AgentCompactResult>();
  private readonly pending = new Set<() => void>();
  private disposed = false;
  constructor(public delayMs = 1000) {}

  async readUsage({ conversationId, signal }: AgentCommandRequest): Promise<AgentCommandUsage> {
    if (signal.aborted || this.disposed) throw new DOMException("Canceled", "AbortError");
    return this.usages.get(conversationId) ?? { inputTokens: 82000, contextWindowTokens: 128000 };
  }
  async compact(request: AgentCompactRequest): Promise<AgentCompactResult> {
    const delayMs = this.delayMs;
    const before = await this.readUsage(request);
    const key = request.requestId ? JSON.stringify([request.conversationId, request.requestId]) : undefined;
    const receipt = key ? this.receipts.get(key) : undefined;
    if (receipt) return receipt;
    await new Promise<void>((resolve, reject) => {
      const cancel = () => { cleanup(); reject(new DOMException("Canceled", "AbortError")); };
      const timer = setTimeout(() => { cleanup(); resolve(); }, delayMs);
      const cleanup = () => { clearTimeout(timer); request.signal.removeEventListener("abort", cancel); this.pending.delete(cancel); };
      this.pending.add(cancel); request.signal.addEventListener("abort", cancel, { once: true });
      if (request.signal.aborted || this.disposed) cancel();
    });
    if (request.signal.aborted || this.disposed) throw new DOMException("Canceled", "AbortError");
    const usage = { ...before, inputTokens: Math.min(before.inputTokens, 18000) };
    this.usages.set(request.conversationId, usage);
    const result = { beforeTokens: before.inputTokens, usage };
    if (key) this.receipts.set(key, result);
    return result;
  }
  dispose(): void { this.disposed = true; for (const cancel of this.pending) cancel(); this.usages.clear(); this.receipts.clear(); }
}
