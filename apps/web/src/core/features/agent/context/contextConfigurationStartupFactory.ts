import { decodeAgentContextConfigurationEnvironmentRequest, decodeAgentContextState, EMPTY_AGENT_CONTEXT } from "@research-assistant/core/agent";
import type { AgentContextConfigurationSaveGateway, AgentContextConfigurationSeedGateway } from "@research-assistant/data/agent";
import type { ConversationOperationLock } from "../conversationOperationLock";
import { ContextConfigurationStorageOutbox } from "./contextConfigurationOutbox";
import { contextConfigurationOwners } from "./contextConfigurationOwners";
import { ContextConfigurationSeedOwner } from "./contextConfigurationSeedOwner";
import { ContextConfigurationStartup } from "./contextConfigurationStartup";
import { contextStorageKey } from "./contextStorage";

type Startups = Map<string, WeakRef<ContextConfigurationStartup>>;
const scopes = new WeakMap<Storage, WeakMap<AgentContextConfigurationSaveGateway, WeakMap<AgentContextConfigurationSeedGateway, Startups>>>();

/** Same storage, authority and conversation share the startup gate. Capturing
 * the storage object prevents a runtime switch from importing another cache. */
export function contextConfigurationStartup(storage: Storage, conversationId: string,
  configuration: AgentContextConfigurationSaveGateway, seedGateway: AgentContextConfigurationSeedGateway,
  lock?: ConversationOperationLock): ContextConfigurationStartup {
  const id = decodeAgentContextConfigurationEnvironmentRequest({ operationId: conversationId, conversationId }).conversationId;
  let configurations = scopes.get(storage);
  if (!configurations) { configurations = new WeakMap(); scopes.set(storage, configurations); }
  let seeds = configurations.get(configuration);
  if (!seeds) { seeds = new WeakMap(); configurations.set(configuration, seeds); }
  let startups = seeds.get(seedGateway);
  if (!startups) { startups = new Map(); seeds.set(seedGateway, startups); }
  const existing = startups.get(id)?.deref();
  if (existing) return existing;
  for (const [key, value] of startups) if (!value.deref()) startups.delete(key);
  const raw = storage.getItem(contextStorageKey(id));
  const initial = raw === null ? EMPTY_AGENT_CONTEXT : decodeAgentContextState(JSON.parse(raw));
  const outbox = new ContextConfigurationStorageOutbox(storage, id, lock);
  const seed = new ContextConfigurationSeedOwner(id, outbox, configuration, seedGateway, () => crypto.randomUUID());
  const owner = contextConfigurationOwners(storage, configuration, lock);
  const startup = new ContextConfigurationStartup(id, initial, raw === null, outbox, seed, owner);
  startups.set(id, new WeakRef(startup));
  return startup;
}
