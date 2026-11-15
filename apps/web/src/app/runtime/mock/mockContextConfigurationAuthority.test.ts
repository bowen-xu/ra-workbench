// @vitest-environment happy-dom
import { beforeEach, expect, it } from "vitest";
import { EMPTY_AGENT_CONTEXT, type AgentContextState } from "@research-assistant/core/agent";
import { configurePreviewDatabase } from "../../../core/runtime/previewIndexedDatabase";
import { createIndexedPreviewStorage } from "../../../core/runtime/previewIndexedStorage";
import { createMemoryPreviewDatabase } from "../testing/memoryPreviewDatabase";
import { decodeMockState } from "../../../core/runtime/mockStateCodec";
import { createMockContextConfigurationAuthority } from "./mockContextConfigurationAuthority";

const conversationId = "11111111-1111-4111-8111-111111111111";
const operationId = "22222222-2222-4222-8222-222222222222";
const laterOperation = "33333333-3333-4333-8333-333333333333";
type Configurations = Map<string, { revision: number; state: AgentContextState }>;
let database: ReturnType<typeof createMemoryPreviewDatabase>;
beforeEach(() => { database = createMemoryPreviewDatabase(); configurePreviewDatabase(database.open); });
function configurations(storage: Storage): Configurations {
  const raw = storage.getItem("native-context-configurations");
  return raw ? decodeMockState<Configurations>(raw) : new Map();
}

it("preserves the exact mock save receipt after later edits and reopening", async () => {
  const storage = await createIndexedPreviewStorage("mock-context-reopen:"), rows = configurations(storage);
  const first = await createMockContextConfigurationAuthority(storage, rows);
  const environment = await first.configurationSave.environment({ operationId, conversationId });
  const intent = { operationId, conversationId, expectedEpoch: environment.epoch, expectedRevision: 0, state: EMPTY_AGENT_CONTEXT };
  expect((await first.configurationSave.save(intent)).replayed).toBe(false);
  await first.configurationSave.save({ ...intent, operationId: laterOperation, expectedRevision: 1 });
  const reopened = await createIndexedPreviewStorage("mock-context-reopen:"), next = await createMockContextConfigurationAuthority(reopened, configurations(reopened));
  expect((await next.configurationSave.status(intent)).type).toBe("committed");
  expect((await next.configurationSave.save(intent)).replayed).toBe(true);
  expect((await next.configurationSave.environment({ operationId, conversationId })).configuration?.revision).toBe(2);
  await expect(next.configurationSave.status({ ...intent, expectedRevision: 1 })).rejects.toThrow("MOCK_CONTEXT_RECEIPT_CONFLICT");
});

it("does not claim a failed mock storage flush committed and reopens without a partial transaction", async () => {
  const storage = await createIndexedPreviewStorage("mock-context-failure:"), authority = await createMockContextConfigurationAuthority(storage, configurations(storage));
  const environment = await authority.configurationSave.environment({ operationId, conversationId });
  const intent = { operationId, conversationId, expectedEpoch: environment.epoch, expectedRevision: 0, state: EMPTY_AGENT_CONTEXT };
  database.failNext(new DOMException("full", "QuotaExceededError"));
  await expect(authority.configurationSave.save(intent)).rejects.toThrow();
  expect((await authority.configurationSave.status(intent)).type).toBe("unconfirmed");
  const reopened = await createIndexedPreviewStorage("mock-context-failure:"), next = await createMockContextConfigurationAuthority(reopened, configurations(reopened));
  expect((await next.configurationSave.status(intent)).type).toBe("noSqlReceipt");
  expect((await next.configurationSave.environment({ operationId, conversationId })).configuration).toBeNull();
  expect((await next.configurationSave.save(intent)).replayed).toBe(false);
});

it("records a mock seed without inventing a configuration and rejects wrong epochs", async () => {
  const storage = await createIndexedPreviewStorage("mock-context-seed:"), authority = await createMockContextConfigurationAuthority(storage, configurations(storage));
  const environment = await authority.configurationSave.environment({ operationId, conversationId });
  const intent = { operationId, conversationId, expectedEpoch: environment.epoch };
  expect((await authority.configurationSeed.seed(intent)).family).toBe("agent.context.configuration.seed");
  expect((await authority.configurationSeed.status(intent)).type).toBe("committed");
  expect((await authority.configurationSave.environment({ operationId, conversationId })).configuration).toBeNull();
  await expect(authority.configurationSeed.status({ ...intent, expectedEpoch: laterOperation })).rejects.toThrow("MOCK_CONTEXT_EPOCH_MISMATCH");
});

it("applies host archive authorization before writing or replaying", async () => {
  const storage = await createIndexedPreviewStorage("mock-context-archived:"); let archived = false;
  const authority = await createMockContextConfigurationAuthority(storage, configurations(storage), async () => undefined,
    async () => { if (archived) throw "conversationUnavailable"; });
  const environment = await authority.configurationSave.environment({ operationId, conversationId });
  const intent = { operationId, conversationId, expectedEpoch: environment.epoch, expectedRevision: 0, state: EMPTY_AGENT_CONTEXT };
  await authority.configurationSave.save(intent); archived = true;
  await expect(authority.configurationSave.save(intent)).rejects.toBe("conversationUnavailable");
  expect((await authority.configurationSave.status(intent)).type).toBe("committed");
});
