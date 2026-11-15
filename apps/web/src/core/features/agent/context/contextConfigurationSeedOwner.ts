import { decodeAgentContextConfigurationEnvironmentRequest, type AgentContextConfigurationSeedIntent } from "@research-assistant/core/agent";
import type { AgentContextConfigurationSaveGateway, AgentContextConfigurationSeedGateway } from "@research-assistant/data/agent";
import type { ContextConfigurationStorageOutbox } from "./contextConfigurationOutbox";

/** Coordinates only the trusted internal seed. Configuration editing remains
 * in HBS and uses the same outbox so the two original requests cannot race. */
export class ContextConfigurationSeedOwner {
  private readonly conversationId: string;
  constructor(conversationId: string, private readonly outbox: ContextConfigurationStorageOutbox,
    private readonly configuration: AgentContextConfigurationSaveGateway, private readonly gateway: AgentContextConfigurationSeedGateway,
    private readonly newId: () => string) {
    this.conversationId = decodeAgentContextConfigurationEnvironmentRequest({ operationId: conversationId, conversationId }).conversationId;
  }

  async start(canWrite: () => boolean = () => true): Promise<"existing" | "seeded"> {
    return this.outbox.run(async () => {
      if (!canWrite()) throw new Error("CONTEXT_CONFIGURATION_WRITE_REVOKED");
      if (await this.outbox.readSeed()) throw new Error("CONTEXT_CONFIGURATION_SEED_ORIGINAL_PENDING");
      const operationId = this.newId();
      const environment = await this.configuration.environment({ operationId, conversationId: this.conversationId });
      if (environment.configuration) return "existing";
      if (!canWrite()) throw new Error("CONTEXT_CONFIGURATION_WRITE_REVOKED");
      const intent: AgentContextConfigurationSeedIntent = { operationId, conversationId: this.conversationId, expectedEpoch: environment.epoch };
      await this.outbox.writeSeed(intent);
      if (!canWrite()) throw new Error("CONTEXT_CONFIGURATION_WRITE_REVOKED");
      await this.gateway.seed(structuredClone(intent));
      await this.clear(intent);
      return "seeded";
    });
  }

  async inspect(): Promise<"empty" | "committed" | "unconfirmed"> {
    return this.outbox.run(async () => {
      const intent = await this.outbox.readSeed();
      if (!intent) return "empty";
      if ((await this.gateway.status(structuredClone(intent))).type !== "committed") return "unconfirmed";
      await this.clear(intent);
      return "committed";
    });
  }

  async retry(canWrite: () => boolean = () => true): Promise<void> {
    await this.outbox.run(async () => {
      const intent = await this.outbox.readSeed();
      if (!intent) throw new Error("CONTEXT_CONFIGURATION_SEED_ORIGINAL_MISSING");
      const status = await this.gateway.status(structuredClone(intent));
      if (status.type !== "committed" && (status.closed || status.type === "unconfirmed" && status.blocked))
        throw new Error("CONTEXT_CONFIGURATION_SEED_RECOVERY_REQUIRED");
      if (status.type !== "committed") {
        if (!canWrite()) throw new Error("CONTEXT_CONFIGURATION_WRITE_REVOKED");
        await this.gateway.seed(structuredClone(intent));
      }
      await this.clear(intent);
    });
  }

  private async clear(intent: AgentContextConfigurationSeedIntent): Promise<void> {
    const current = await this.outbox.readSeed();
    if (!current || JSON.stringify(current) !== JSON.stringify(intent)) throw new Error("CONTEXT_CONFIGURATION_SEED_ORIGINAL_CHANGED");
    await this.outbox.writeSeed(null);
  }
}
