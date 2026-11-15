import { decodeAgentContextConfigurationEnvironmentRequest } from "@research-assistant/core/agent";
import type { AgentContextConfigurationSaveGateway } from "@research-assistant/data/agent";
import type { ConversationOperationLock } from "../conversationOperationLock";
import { ContextConfigurationOwner } from "./contextConfigurationOwner";

interface Entry {
  readonly opened: Promise<ContextConfigurationOwner>;
  owner?: ContextConfigurationOwner;
  unsubscribe?: () => void;
  views: number;
}

/** Opens one owner per conversation even when several views mount at once. */
export class ContextConfigurationOwners {
  private readonly entries = new Map<string, Entry>();
  constructor(private readonly open: (conversationId: string) => Promise<ContextConfigurationOwner>) {}

  async acquire(conversationId: string): Promise<{ readonly owner: ContextConfigurationOwner; readonly release: () => void }> {
    const id = decodeAgentContextConfigurationEnvironmentRequest({ operationId: conversationId, conversationId }).conversationId;
    let entry = this.entries.get(id);
    if (!entry) {
      const opened = Promise.resolve().then(() => this.open(id));
      entry = { opened, views: 0 };
      this.entries.set(id, entry);
      const created = entry;
      void opened.then(owner => {
        created.owner = owner;
        created.unsubscribe = owner.subscribe(() => this.collect(id, created));
        this.collect(id, created);
      }, () => {
        if (this.entries.get(id) === created) this.entries.delete(id);
      });
    }
    const acquired = entry;
    acquired.views += 1;
    const owner = await acquired.opened;
    let released = false;
    return { owner, release: () => {
      if (released) return;
      released = true;
      acquired.views -= 1;
      this.collect(id, acquired);
    } };
  }

  private collect(id: string, entry: Entry): void {
    if (this.entries.get(id) !== entry || entry.views || !entry.owner || entry.owner.getSnapshot().phase !== "idle") return;
    this.entries.delete(id);
    entry.unsubscribe?.();
    entry.owner.dispose();
  }
}

const scopes = new WeakMap<Storage, WeakMap<AgentContextConfigurationSaveGateway, ContextConfigurationOwners>>();

/** Runtime storage and the authority gateway both form part of owner identity. */
export function contextConfigurationOwners(storage: Storage, gateway: AgentContextConfigurationSaveGateway,
  lock?: ConversationOperationLock): ContextConfigurationOwners {
  let gateways = scopes.get(storage);
  if (!gateways) { gateways = new WeakMap(); scopes.set(storage, gateways); }
  let registry = gateways.get(gateway);
  if (!registry) {
    registry = new ContextConfigurationOwners(conversationId => ContextConfigurationOwner.open({ storage, conversationId,
      gateway, newId: () => crypto.randomUUID(), ...(lock ? { lock } : {}) }));
    gateways.set(gateway, registry);
  }
  return registry;
}
