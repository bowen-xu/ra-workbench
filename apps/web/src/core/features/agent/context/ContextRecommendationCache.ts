import type {AgentContextSelection} from "@research-assistant/core/agent";
import type {AgentContextGateway} from "../../../services/agentContextGateway";

interface Entry {
  readonly candidate: AgentContextSelection;
  readonly promise: Promise<AgentContextSelection>;
  readonly controller: AbortController;
  users: number;
  settled: boolean;
  selection?: AgentContextSelection;
}
/** Provider-owned, 128 suggestion identities. Never used for prepare/start/access. */
export class ContextRecommendationCache {
  private readonly entries = new Map<string, Entry>();
  private readonly listeners = new Set<() => void>();
  private version = 0;
  constructor(private readonly gateway: Pick<AgentContextGateway, "recommend">) {}
  getVersion = (): number => this.version;
  subscribe = (listener: () => void): (() => void) => {this.listeners.add(listener); return () => {this.listeners.delete(listener);};};
  peek(candidate: AgentContextSelection | null): AgentContextSelection | undefined {
    return candidate ? this.entries.get(JSON.stringify(candidate))?.selection : undefined;
  }
  private entry(candidate: AgentContextSelection): Entry {
    const key = JSON.stringify(candidate), found = this.entries.get(key);
    if (found) {this.entries.delete(key); this.entries.set(key, found); return found;}
    if (this.entries.size >= 128) {
      const unused = [...this.entries].find(([, entry]) => entry.users === 0);
      if (!unused) throw new Error("推荐资源过多");
      unused[1].controller.abort(); this.entries.delete(unused[0]);
    }
    const controller = new AbortController();
    const entry: Entry = {candidate, controller, users: 0, settled: false,
      promise: Promise.resolve().then(() => {
        controller.signal.throwIfAborted();
        if (!this.gateway.recommend) throw new Error("资源推荐接口不可用");
        return this.gateway.recommend(candidate, controller.signal);
      }).then(selection => {
        controller.signal.throwIfAborted();
        if (this.entries.get(key) === entry) entry.selection = selection;
        return selection;
      }).catch(reason => {if (this.entries.get(key) === entry) this.entries.delete(key); throw reason;})
        .finally(() => {entry.settled = true;}),
    };
    this.entries.set(key, entry);
    return entry;
  }
  resolve(candidate: AgentContextSelection): Promise<AgentContextSelection> {return this.entry(candidate).promise;}
  acquire(candidate: AgentContextSelection): {readonly promise: Promise<AgentContextSelection>; readonly release: () => void} {
    const entry = this.entry(candidate), key = JSON.stringify(candidate);
    entry.users++;
    let released = false;
    return {promise: entry.promise, release: () => {
      if (released) return;
      released = true; entry.users--;
      // Activation changes and StrictMode may immediately reacquire this identity.
      queueMicrotask(() => {
        if (!entry.users && !entry.settled && this.entries.get(key) === entry) {
          this.entries.delete(key); entry.controller.abort();
        }
      });
    }};
  }
  invalidate(matches: (candidate: AgentContextSelection) => boolean = () => true, refreshActive = true): void {
    let changed = false;
    for (const [key, entry] of this.entries) if (matches(entry.candidate)) {
      this.entries.delete(key); entry.controller.abort(); changed = true;
    }
    if (!changed || !refreshActive) return;
    this.version++;
    for (const listener of this.listeners) listener();
  }
}
